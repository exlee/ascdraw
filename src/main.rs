use std::collections::HashMap;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use winit::event::{ElementState, Event, Ime, MouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState};
use winit::window::WindowId;

mod app;
mod diagnostics;
mod document;
mod drawing;
mod editor;
mod face_resolution;
mod input;
mod layout;
#[cfg(target_os = "macos")]
mod macos;
mod model;
mod render;
mod runtime;
mod title_policy;
mod toolbar;
mod user_keys;

use app::{
    AppCommand, AppEvent, Args, checked_config_paths, load_config, show_config_toml,
    user_config_path,
};
use diagnostics::log_error;
use editor::EditorState;
use input::{
    EditCommand, edit_command, pointer_position_to_coord, pointer_position_to_toolbar_position,
};
use render::{render, resize_surface};
use runtime::config_watch::{UserConfigWatch, poll_user_config_updates};
use runtime::window::{EditorWindow, close_window, create_editor_window, handle_command};
use user_keys::{FontSizeAction, UserAction, UserKeys};

fn main() -> ExitCode {
    if let Err(error) = diagnostics::init() {
        eprintln!("diagnostics setup failed: {error:#}");
    }
    diagnostics::install_panic_hook();

    match try_main() {
        Ok(code) => code,
        Err(error) => {
            log_error(format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

#[allow(deprecated)]
fn try_main() -> Result<ExitCode> {
    let args = Args::parse();
    let document_path = args.document.unwrap_or_else(document::default_path);
    let mut config = load_config()?;
    if args.show_config {
        println!("Checked configuration paths:");
        for path in checked_config_paths() {
            println!("  {}", path.display());
        }
        println!("\nCurrent configuration:\n{}", show_config_toml(&config)?);
        return Ok(ExitCode::SUCCESS);
    }

    let mut user_keys = UserKeys::from_config(&config.keys)?;
    let mut user_config_watch = user_config_path().map(UserConfigWatch::new);
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();

    #[cfg(target_os = "macos")]
    if let Err(error) = macos::install(proxy) {
        log_error(format!("macOS integration setup failed: {error:#}"));
    }

    let mut windows: HashMap<WindowId, EditorWindow> = HashMap::new();
    let mut last_autosave_check = Instant::now();
    #[cfg(target_os = "macos")]
    let mut installed_macos_menus = false;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(500),
        ));

        match event {
            Event::Resumed => {
                #[cfg(target_os = "macos")]
                if !installed_macos_menus {
                    if let Err(error) = macos::install_menus() {
                        log_error(format!("macOS menu setup failed: {error:#}"));
                    }
                    installed_macos_menus = true;
                }

                if windows.is_empty() {
                    match create_editor_window(elwt, &config, &document_path) {
                        Ok(editor) => {
                            windows.insert(editor.window_id(), editor);
                        }
                        Err(error) => {
                            log_error(format!("window creation failed: {error:#}"));
                            elwt.exit();
                        }
                    }
                } else {
                    for editor in windows.values() {
                        editor.request_redraw();
                    }
                }
            }
            Event::UserEvent(AppEvent::Command(command)) => {
                handle_command(command, None, &mut windows, elwt, &config, &document_path);
            }
            Event::AboutToWait => {
                if let Some(watch) = user_config_watch.as_mut() {
                    poll_user_config_updates(watch, &mut config, &mut user_keys, &mut windows);
                }
                let now = Instant::now();
                if now.saturating_duration_since(last_autosave_check) >= Duration::from_secs(1) {
                    for editor in windows.values_mut() {
                        if let Err(error) = editor.autosave_if_idle(now) {
                            log_error(format!("document autosave failed: {error:#}"));
                        }
                    }
                    last_autosave_check = now;
                }
            }
            Event::WindowEvent { window_id, event } => {
                let mut should_close = false;
                let mut pending_command = None;
                if let Some(editor) = windows.get_mut(&window_id) {
                    match event {
                        WindowEvent::CloseRequested => should_close = true,
                        WindowEvent::Resized(size) => {
                            if let Err(error) = resize_surface(&mut editor.surface, size) {
                                log_error(format!("surface resize failed: {error:#}"));
                                should_close = true;
                            }
                            editor.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            if let Err(error) = render(
                                &editor.window,
                                &mut editor.surface,
                                &editor.state,
                                &editor.renderer,
                                &config,
                                editor.viewport,
                            ) {
                                log_error(format!("render failed: {error:#}"));
                                should_close = true;
                            }
                        }
                        WindowEvent::ModifiersChanged(modifiers) => {
                            editor.modifiers = modifiers.state();
                        }
                        WindowEvent::Ime(Ime::Commit(text)) => {
                            if !text.is_empty()
                                && editor.state.cursor_mode.accepts_text()
                                && !editor.modifiers.control_key()
                                && !editor.modifiers.alt_key()
                                && !editor.modifiers.super_key()
                            {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                editor.state.write_text(&text);
                                if editor.finish_state_change(
                                    previous_state,
                                    previous_viewport,
                                    true,
                                ) {
                                    editor.mark_document_dirty();
                                }
                                editor.request_redraw();
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state == ElementState::Pressed =>
                        {
                            editor.note_keypress(Instant::now());
                            if edit_command(
                                &event.logical_key,
                                event.repeat,
                                editor.modifiers,
                                editor.state.cursor_mode,
                            ) == Some(EditCommand::CancelTextEntry)
                            {
                                editor.state.cancel_text_entry();
                                editor.request_redraw();
                            } else if let Some(action) =
                                user_keys.action_for_event(&event, editor.modifiers)
                            {
                                pending_command = Some(app_command_from_user_action(action));
                            } else {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                let handled = handle_editor_key(
                                    &mut editor.state,
                                    &event.logical_key,
                                    event.text.as_deref(),
                                    event.repeat,
                                    editor.modifiers,
                                );
                                if let Some(document_changed) = handled {
                                    let document_changed = editor.finish_state_change(
                                        previous_state,
                                        previous_viewport,
                                        document_changed,
                                    );
                                    if document_changed {
                                        editor.mark_document_dirty();
                                    }
                                    editor.request_redraw();
                                }
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            editor.mouse_toolbar_position = pointer_position_to_toolbar_position(
                                position.x,
                                position.y,
                                editor.window.inner_size().width as usize,
                                &editor.renderer,
                                editor.window.scale_factor(),
                                &config,
                                &editor.state.toolbar,
                            );
                            editor.mouse_cell = pointer_position_to_coord(
                                position.x,
                                position.y,
                                &editor.renderer,
                                editor.window.scale_factor(),
                                &config,
                                &editor.state.toolbar,
                                editor.viewport,
                            );
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } => {
                            let toolbar_action =
                                editor.mouse_toolbar_position.and_then(|(row, column)| {
                                    editor.state.toolbar.action_at(row, column)
                                });
                            if let Some(action) = toolbar_action {
                                editor.state.apply_toolbar_action(action);
                                editor.request_redraw();
                            } else if let Some(coord) = editor.mouse_cell {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                editor.state.move_to(coord);
                                editor.finish_state_change(
                                    previous_state,
                                    previous_viewport,
                                    false,
                                );
                                editor.request_redraw();
                            }
                        }
                        WindowEvent::ScaleFactorChanged { .. } => editor.request_redraw(),
                        _ => {}
                    }
                }

                if let Some(command) = pending_command {
                    handle_command(
                        command,
                        Some(window_id),
                        &mut windows,
                        elwt,
                        &config,
                        &document_path,
                    );
                } else if should_close {
                    close_window(&mut windows, window_id, elwt);
                }
            }
            _ => {}
        }
    })?;

    Ok(ExitCode::SUCCESS)
}

fn app_command_from_user_action(action: UserAction) -> AppCommand {
    match action {
        UserAction::FontSize(FontSizeAction::Increase) => AppCommand::FontScaleUp,
        UserAction::FontSize(FontSizeAction::Decrease) => AppCommand::FontScaleDown,
        UserAction::FontSize(FontSizeAction::Reset) => AppCommand::FontScaleReset,
        UserAction::WindowNew => AppCommand::WindowNew,
        UserAction::WindowClose => AppCommand::WindowClose,
    }
}

fn handle_editor_key(
    state: &mut EditorState,
    key: &Key,
    text: Option<&str>,
    repeat: bool,
    modifiers: ModifiersState,
) -> Option<bool> {
    if state.handle_toolbar_shortcut(key, modifiers) {
        return Some(false);
    }
    if let Some(command) = edit_command(key, repeat, modifiers, state.cursor_mode) {
        return Some(apply_edit_command(state, command));
    }
    if !modifiers.control_key()
        && state.cursor_mode.accepts_text()
        && !modifiers.alt_key()
        && !modifiers.super_key()
        && let Some(text) = text
        && !text.chars().all(char::is_control)
    {
        state.write_text(text);
        return Some(true);
    }
    None
}

fn apply_edit_command(state: &mut EditorState, command: EditCommand) -> bool {
    match command {
        EditCommand::Move(direction) => state.move_cursor(direction),
        EditCommand::Draw(direction) => {
            state.move_or_draw(direction, true);
            true
        }
        EditCommand::DrawStamp(direction) => {
            state.draw_stamp(direction);
            true
        }
        EditCommand::Erase(direction) => {
            state.erase(direction);
            true
        }
        EditCommand::Clear => {
            state.clear_cell();
            true
        }
        EditCommand::ToggleTextEntry => {
            state.toggle_text_entry();
            false
        }
        EditCommand::ToggleReplaceMode => {
            state.toggle_replace_mode();
            false
        }
        EditCommand::BeginSingleReplace => {
            state.begin_single_replace();
            false
        }
        EditCommand::CancelTextEntry => {
            state.cancel_text_entry();
            false
        }
        EditCommand::PlaceStamp => {
            state.place_stamp();
            true
        }
        EditCommand::ToggleShapePreview => {
            state.toggle_shape_preview();
            false
        }
        EditCommand::ConfirmShape => {
            state.confirm_shape();
            true
        }
        EditCommand::Home => {
            state.move_home();
            false
        }
        EditCommand::End => {
            state.move_end();
            false
        }
        EditCommand::Backspace => {
            state.backspace();
            true
        }
        EditCommand::Delete => {
            state.delete();
            true
        }
        EditCommand::Newline => {
            state.newline();
            true
        }
        EditCommand::InsertTab => {
            state.insert("    ");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppConfig, CursorMode};
    use crate::model::{Coord, Direction};
    use crate::toolbar::{MainMode, ToolbarAction};
    use winit::keyboard::NamedKey;

    #[test]
    fn shape_commands_start_preview_move_and_commit_a_rectangle() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));

        assert!(!apply_edit_command(
            &mut state,
            EditCommand::ToggleShapePreview
        ));
        for command in [
            EditCommand::Move(Direction::Right),
            EditCommand::Move(Direction::Right),
            EditCommand::Move(Direction::Right),
            EditCommand::Move(Direction::Down),
            EditCommand::Move(Direction::Down),
        ] {
            assert!(!apply_edit_command(&mut state, command));
        }

        let preview = state
            .lines_with_shape_preview()
            .expect("preview is visible");
        assert_eq!(line_contents(&preview[0]), "┌──┐");
        assert!(apply_edit_command(&mut state, EditCommand::ConfirmShape));
        assert!(state.lines_with_shape_preview().is_none());
        assert_eq!(line_contents(&state.grid.lines[2]), "└──┘");
    }

    #[test]
    fn structural_edge_movement_reports_a_document_change() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "ascdraw");
        assert!(apply_edit_command(
            &mut state,
            EditCommand::Move(model::Direction::Up)
        ));
        assert_eq!(state.grid.lines.len(), 2);
    }

    #[test]
    fn cancel_keys_route_out_of_text_replace_and_single_replace() {
        let config = AppConfig::default();
        for (key, modifiers) in [
            (Key::Named(NamedKey::Escape), ModifiersState::empty()),
            (Key::Character("c".into()), ModifiersState::CONTROL),
            (Key::Character("g".into()), ModifiersState::CONTROL),
        ] {
            for mode in [CursorMode::Text, CursorMode::Replace] {
                let mut state = EditorState::new(&config.theme, "test");
                assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
                state.cursor_mode = mode;

                assert_eq!(
                    handle_editor_key(&mut state, &key, None, false, modifiers),
                    Some(false)
                );
                assert_eq!(state.cursor_mode, CursorMode::Stamp);
            }

            let mut state = EditorState::new(&config.theme, "test");
            assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
            assert!(state.begin_single_replace());

            assert_eq!(
                handle_editor_key(&mut state, &key, None, false, modifiers),
                Some(false)
            );
            assert_eq!(state.cursor_mode, CursorMode::Shapes);
        }
    }

    #[test]
    fn digits_route_to_insert_replace_and_single_replace() {
        let config = AppConfig::default();
        let digit = Key::Character("2".into());

        let mut insert = EditorState::new(&config.theme, "test");
        insert.toggle_text_entry();
        assert_eq!(
            handle_editor_key(
                &mut insert,
                &digit,
                Some("2"),
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(line_contents(&insert.grid.lines[0]), "2");

        let mut replace = EditorState::new(&config.theme, "test");
        replace.insert("a");
        replace.move_to(Coord::default());
        replace.toggle_replace_mode();
        assert_eq!(
            handle_editor_key(
                &mut replace,
                &digit,
                Some("2"),
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(line_contents(&replace.grid.lines[0]), "2");
        assert_eq!(replace.cursor_mode, CursorMode::Replace);

        let mut single_replace = EditorState::new(&config.theme, "test");
        single_replace.insert("a");
        single_replace.move_to(Coord::default());
        assert!(single_replace.begin_single_replace());
        assert_eq!(
            handle_editor_key(
                &mut single_replace,
                &digit,
                Some("2"),
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(line_contents(&single_replace.grid.lines[0]), "2");
        assert_eq!(single_replace.grid.cursor_pos, Coord::default());
        assert_eq!(single_replace.cursor_mode, CursorMode::MoveDraw);
    }

    fn line_contents(line: &[crate::model::Atom]) -> String {
        line.iter().map(|atom| atom.contents.as_str()).collect()
    }
}
