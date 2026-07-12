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
mod export;
mod face_resolution;
mod history;
mod input;
mod layout;
#[cfg(target_os = "macos")]
mod macos;
mod icon;
mod model;
mod render;
mod runtime;
pub mod selection;
mod title_policy;
mod toolbar;
mod user_keys;

use app::{
    AppCommand, AppEvent, Args, checked_config_paths, load_config, show_config_toml,
    user_config_path,
};
use diagnostics::log_error;
use editor::EditorState;
use export::{ExportOutcome, NativeExportPlatform};
use input::{
    ClipboardCommand, EditCommand, HistoryCommand, clipboard_command, edit_command,
    history_command, ordered_direction_command, pointer_position_to_coord,
    pointer_position_to_toolbar_position,
};
use render::{render, resize_surface};
use runtime::config_watch::{UserConfigWatch, poll_user_config_updates};
use runtime::window::{
    EditorWindow, close_window, create_editor_window, handle_command, save_windows_on_exit,
};
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
    #[cfg(target_os = "macos")]
    let mut should_apply_app_icon = true;

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
                            editor.ensure_cursor_in_viewport();
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
                            } else {
                                #[cfg(target_os = "macos")]
                                if should_apply_app_icon {
                                    should_apply_app_icon = false;
                                    if let Err(error) = icon::apply_app_icon() {
                                        log_error(format!("app icon setup failed: {error:#}"));
                                    }
                                }

                            }
                        }
                        WindowEvent::ModifiersChanged(modifiers) => {
                            editor.modifiers = modifiers.state();
                            editor.ordered_modifiers.update(editor.modifiers);
                        }
                        WindowEvent::Focused(false) => {
                            editor.modifiers = ModifiersState::empty();
                            editor.ordered_modifiers.update(editor.modifiers);
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
                            if let Some(command) =
                                history_command(&event.logical_key, editor.modifiers)
                            {
                                match command {
                                    HistoryCommand::Undo => {
                                        editor.undo();
                                    }
                                    HistoryCommand::Redo => {
                                        editor.redo();
                                    }
                                }
                            } else if clipboard_command(&event.logical_key, editor.modifiers)
                                .is_some()
                            {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                let mut platform = NativeExportPlatform;
                                let result = handle_clipboard_shortcut(
                                    &mut editor.state,
                                    &event.logical_key,
                                    editor.modifiers,
                                    &mut platform,
                                )
                                .expect("clipboard shortcut was already recognized");
                                match result {
                                    Ok(true) => {
                                        if editor.finish_state_change(
                                            previous_state,
                                            previous_viewport,
                                            true,
                                        ) {
                                            editor.mark_document_dirty();
                                        }
                                    }
                                    Ok(false) => {}
                                    Err(error) => {
                                        editor.state = previous_state;
                                        editor.viewport = previous_viewport;
                                        log_error(format!("Clipboard operation failed: {error:#}"));
                                    }
                                }
                                editor.request_redraw();
                            } else if edit_command(
                                &event.logical_key,
                                event.repeat,
                                editor.modifiers,
                                editor.state.cursor_mode,
                            ) == Some(EditCommand::CancelTextEntry)
                            {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                editor.state.cancel_text_entry();
                                editor.finish_state_change(
                                    previous_state,
                                    previous_viewport,
                                    false,
                                );
                                editor.request_redraw();
                            } else if let Some(action) =
                                user_keys.action_for_event(&event, editor.modifiers)
                            {
                                pending_command = Some(app_command_from_user_action(action));
                            } else {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                let handled = handle_editor_key_with_order(
                                    &mut editor.state,
                                    &event.logical_key,
                                    event.text.as_deref(),
                                    event.repeat,
                                    editor.modifiers,
                                    &editor.ordered_modifiers,
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
                                perform_pending_export(editor);
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
                                editor
                                    .mouse_toolbar_position
                                    .and_then(|(row, column, width)| {
                                        editor.state.toolbar.action_at(row, column, width)
                                    });
                            if let Some(action) = toolbar_action {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                editor.state.apply_toolbar_action(action);
                                editor.finish_state_change(
                                    previous_state,
                                    previous_viewport,
                                    false,
                                );
                                perform_pending_export(editor);
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
                        WindowEvent::ScaleFactorChanged { .. } => {
                            editor.ensure_cursor_in_viewport();
                            editor.request_redraw();
                        }
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
            Event::LoopExiting => save_windows_on_exit(&mut windows),
            _ => {}
        }
    })?;

    Ok(ExitCode::SUCCESS)
}

fn handle_clipboard_shortcut(
    state: &mut EditorState,
    key: &Key,
    modifiers: ModifiersState,
    platform: &mut impl export::ExportPlatform,
) -> Option<Result<bool>> {
    Some(match clipboard_command(key, modifiers)? {
        ClipboardCommand::Copy => export::copy_selection(state, platform).map(|()| false),
        ClipboardCommand::Paste => export::paste_selection(state, platform),
    })
}

fn perform_pending_export(editor: &mut EditorWindow) {
    let Some(action) = editor.state.toolbar.take_export_action() else {
        return;
    };
    if action.is_png() {
        log_error("PNG export is deferred; it will use an Egui canvas-only screenshot");
        return;
    }
    let previous_state = editor.state.clone();
    let previous_viewport = editor.viewport;
    let mut platform = NativeExportPlatform;
    match export::perform(action, &mut editor.state, &mut platform) {
        Ok(ExportOutcome::DocumentLoaded) => {
            editor.viewport = layout::ViewportOffset::default();
            if editor.finish_state_change(previous_state, previous_viewport, true) {
                editor.mark_document_dirty();
            }
        }
        Ok(ExportOutcome::Unchanged) => {}
        Err(error) => log_error(format!("Save/Load/Export failed: {error:#}")),
    }
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

fn handle_editor_key_with_order(
    state: &mut EditorState,
    key: &Key,
    text: Option<&str>,
    repeat: bool,
    modifiers: ModifiersState,
    ordered_modifiers: &input::OrderedModifierTracker,
) -> Option<bool> {
    if let Some(command) =
        ordered_direction_command(key, modifiers, ordered_modifiers, state.cursor_mode)
    {
        state.toolbar.cancel_shortcut();
        let mut document_changed = false;
        for _ in 0..command.steps {
            document_changed |= apply_edit_command(state, command.command);
        }
        if matches!(command.command, EditCommand::ExtendSelection(_)) {
            document_changed = false;
        }
        return Some(document_changed);
    }
    if let Some(command @ (EditCommand::ExtendSelection(_) | EditCommand::Erase(_))) =
        edit_command(key, repeat, modifiers, state.cursor_mode)
    {
        state.toolbar.cancel_shortcut();
        return Some(apply_edit_command(state, command));
    }
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

#[cfg(test)]
fn handle_editor_key(
    state: &mut EditorState,
    key: &Key,
    text: Option<&str>,
    repeat: bool,
    modifiers: ModifiersState,
) -> Option<bool> {
    let mut ordered = input::OrderedModifierTracker::default();
    ordered.update(modifiers);
    handle_editor_key_with_order(state, key, text, repeat, modifiers, &ordered)
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
        EditCommand::ApplyUtility(direction) => state.apply_utility(direction),
        EditCommand::ExtendSelection(direction) => state.extend_selection(direction),
        EditCommand::Erase(direction) => state.erase(direction),
        EditCommand::Clear => {
            state.clear_selection();
            true
        }
        EditCommand::ClearAndBack => {
            state.clear_selection();
            state.move_cursor(model::Direction::Left);
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
        EditCommand::ConfirmOrTextEntry => {
            if state.has_shape_preview() {
                state.start_shape_or_confirm();
            } else {
                state.toggle_text_entry();
            }
            false
        },
        EditCommand::ConfirmOrReplace => {
            if state.has_shape_preview() {
                state.start_shape_or_confirm();
            } else {
                state.toggle_replace_mode();
            }
            false
        },
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
        EditCommand::StartOrConfirmShape => {
            state.start_shape_or_confirm();
            false
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

    #[derive(Default)]
    struct ClipboardPlatform {
        text: String,
    }

    impl export::ExportPlatform for ClipboardPlatform {
        fn set_clipboard_text(&mut self, text: &str) -> Result<()> {
            self.text = text.to_string();
            Ok(())
        }

        fn clipboard_text(&mut self) -> Result<String> {
            Ok(self.text.clone())
        }

        fn choose_save_path(&mut self, _kind: export::FileKind) -> Option<std::path::PathBuf> {
            None
        }

        fn choose_open_path(&mut self, _kind: export::FileKind) -> Option<std::path::PathBuf> {
            None
        }
    }

    #[test]
    fn shape_commands_start_preview_move_and_commit_a_rectangle() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));

        assert!(!apply_edit_command(
            &mut state,
            EditCommand::StartOrConfirmShape
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
        assert!(!apply_edit_command(
            &mut state,
            EditCommand::StartOrConfirmShape
        ));
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
    fn erase_command_reports_only_real_canvas_erasure() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "ascdraw");
        assert!(!apply_edit_command(
            &mut state,
            EditCommand::Erase(model::Direction::Right)
        ));
        state.move_to(model::Coord::default());
        state.insert("x");
        state.move_to(model::Coord::default());
        assert!(apply_edit_command(
            &mut state,
            EditCommand::Erase(model::Direction::Right)
        ));
    }

    #[test]
    fn backspace_and_line_space_route_to_the_same_literal_clear() {
        let config = AppConfig::default();
        let mut cleared = Vec::new();
        for key in [Key::Named(NamedKey::Backspace), Key::Named(NamedKey::Space)] {
            let mut state = EditorState::new(&config.theme, "test");
            state.insert("│\n│\n│");
            state.move_to(Coord { line: 1, column: 0 });

            assert_eq!(
                handle_editor_key(&mut state, &key, None, false, ModifiersState::empty()),
                Some(true)
            );
            assert_eq!(line_contents(&state.grid.lines[0]), "│");
            assert_eq!(line_contents(&state.grid.lines[1]), " ");
            assert_eq!(line_contents(&state.grid.lines[2]), "│");
            cleared.push(state.edit_snapshot());
        }
        assert_eq!(cleared[0], cleared[1]);
    }

    #[test]
    fn modified_directions_precede_and_cancel_pending_toolbar_prefixes() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "ascdraw");
        assert!(
            state.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty())
        );
        assert!(state.toolbar.pending_shortcut().is_some());

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Character("l".into()),
                None,
                false,
                ModifiersState::CONTROL,
            ),
            Some(false)
        );
        assert_eq!(state.selection.active(), Coord { line: 0, column: 1 });
        assert!(state.toolbar.pending_shortcut().is_none());
    }

    fn ordered_modifiers(states: &[ModifiersState]) -> input::OrderedModifierTracker {
        let mut ordered = input::OrderedModifierTracker::default();
        for state in states {
            ordered.update(*state);
        }
        ordered
    }

    fn dispatch_ordered(
        state: &mut EditorState,
        key: Key,
        states: &[ModifiersState],
    ) -> Option<bool> {
        let modifiers = *states.last().expect("at least one modifier state");
        let ordered = ordered_modifiers(states);
        handle_editor_key_with_order(state, &key, None, false, modifiers, &ordered)
    }

    #[test]
    fn ordered_shift_draws_connected_five_and_ten_cell_paths() {
        for (secondary, steps) in [(ModifiersState::CONTROL, 5), (ModifiersState::ALT, 10)] {
            let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
            let combined = ModifiersState::SHIFT | secondary;
            assert_eq!(
                dispatch_ordered(
                    &mut state,
                    Key::Named(NamedKey::ArrowRight),
                    &[ModifiersState::SHIFT, combined],
                ),
                Some(true)
            );
            assert_eq!(state.grid.cursor_pos.column, steps);
            assert_eq!(state.grid.lines[0].len(), steps + 1);
            assert!(
                state.grid.lines[0]
                    .iter()
                    .all(|atom| { !atom.contents.chars().all(char::is_whitespace) })
            );
        }
    }

    #[test]
    fn ordered_alt_erases_every_intermediate_cell_even_after_a_blank() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
        state.insert(" abcdef");
        state.move_to(Coord::default());
        assert_eq!(
            dispatch_ordered(
                &mut state,
                Key::Character("l".into()),
                &[
                    ModifiersState::ALT,
                    ModifiersState::ALT | ModifiersState::CONTROL,
                ],
            ),
            Some(true)
        );
        assert_eq!(state.grid.cursor_pos.column, 5);
        assert_eq!(line_contents(&state.grid.lines[0]), "      f");
    }

    #[test]
    fn ordered_control_grows_from_the_anchor_by_five_and_ten_without_document_change() {
        for (secondary, steps) in [(ModifiersState::ALT, 5), (ModifiersState::SHIFT, 10)] {
            let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
            let combined = ModifiersState::CONTROL | secondary;
            assert_eq!(
                dispatch_ordered(
                    &mut state,
                    Key::Character("l".into()),
                    &[ModifiersState::CONTROL, combined],
                ),
                Some(false)
            );
            assert_eq!(state.selection.anchor(), Coord::default());
            assert_eq!(
                state.selection.active(),
                Coord {
                    line: 0,
                    column: steps
                }
            );
        }
    }

    #[test]
    fn ordered_shift_preserves_stamp_shape_and_utility_routing() {
        let states = [
            ModifiersState::SHIFT,
            ModifiersState::SHIFT | ModifiersState::CONTROL,
        ];

        let mut stamp = EditorState::new(&app::ThemeConfig::default(), "test");
        assert!(stamp.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
        assert_eq!(
            dispatch_ordered(&mut stamp, Key::Character("l".into()), &states),
            Some(true)
        );
        assert_eq!(stamp.grid.cursor_pos.column, 5);
        assert!(stamp.grid.lines[0].len() >= 6);

        let mut shape = EditorState::new(&app::ThemeConfig::default(), "test");
        assert!(shape.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
        assert!(!apply_edit_command(
            &mut shape,
            EditCommand::StartOrConfirmShape
        ));
        assert_eq!(
            dispatch_ordered(&mut shape, Key::Character("l".into()), &states),
            Some(false)
        );
        assert_eq!(shape.grid.cursor_pos.column, 5);
        assert!(shape.lines_with_shape_preview().is_some());

        let mut utility = EditorState::new(&app::ThemeConfig::default(), "test");
        utility.insert("x");
        utility.move_to(Coord::default());
        assert!(utility.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
        assert!(utility.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 1,
        }));
        assert_eq!(
            dispatch_ordered(&mut utility, Key::Character("l".into()), &states),
            Some(true)
        );
        assert_eq!(utility.grid.lines[0].len(), 6);
        assert_eq!(line_contents(&utility.grid.lines[0]), "x     ");
    }

    #[test]
    fn ordered_shift_pull_all_repeats_five_or_ten_times_as_one_document_change() {
        for (secondary, steps) in [(ModifiersState::CONTROL, 5), (ModifiersState::ALT, 10)] {
            let source = "abcdefghijkl";
            let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
            state.insert(source);
            state.move_to(Coord::default());
            assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
            assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
                submenu: 0,
                option: 2,
            }));
            let before = history::HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: layout::ViewportOffset::default(),
            };
            let combined = ModifiersState::SHIFT | secondary;

            assert_eq!(
                dispatch_ordered(
                    &mut state,
                    Key::Character("h".into()),
                    &[ModifiersState::SHIFT, combined],
                ),
                Some(true)
            );
            let expected = format!("a{}", &source[steps + 1..]);
            assert_eq!(line_contents(&state.grid.lines[0]), expected);

            let after = history::HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: layout::ViewportOffset::default(),
            };
            let mut history = history::EditHistory::default();
            assert!(history.record_change(before.clone(), &after));
            assert_eq!(history.undo(after), Some(before));
        }
    }

    #[test]
    fn one_ordered_keypress_is_one_history_record_and_origin_prepends_aggregate() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
        let before = history::HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: layout::ViewportOffset::default(),
        };
        assert_eq!(
            dispatch_ordered(
                &mut state,
                Key::Named(NamedKey::ArrowLeft),
                &[
                    ModifiersState::SHIFT,
                    ModifiersState::SHIFT | ModifiersState::CONTROL,
                ],
            ),
            Some(true)
        );
        assert_eq!(state.take_pending_prepend(), (5, 0));
        let after = history::HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: layout::ViewportOffset::default(),
        };
        let mut history = history::EditHistory::default();
        assert!(history.record_change(before.clone(), &after));
        assert_eq!(history.undo(after), Some(before));
    }

    #[test]
    fn cancel_keys_route_out_of_text_replace_and_single_replace() {
        let config = AppConfig::default();
        for (key, modifiers) in [
            (Key::Named(NamedKey::Escape), ModifiersState::empty()),
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
    fn clipboard_shortcuts_precede_all_modes_and_pending_toolbar_paths() {
        let config = AppConfig::default();
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
            CursorMode::Text,
            CursorMode::Insert,
            CursorMode::Replace,
        ] {
            for modifiers in [ModifiersState::CONTROL, ModifiersState::SUPER] {
                let mut state = EditorState::new(&config.theme, "test");
                state.cursor_mode = mode;
                let mut platform = ClipboardPlatform { text: "v".into() };
                assert!(
                    handle_clipboard_shortcut(
                        &mut state,
                        &Key::Character("V".into()),
                        modifiers,
                        &mut platform,
                    )
                    .unwrap()
                    .unwrap(),
                    "mode={mode:?}"
                );
                assert_eq!(state.selected_text(), "v");
            }
        }

        let mut one_shot = EditorState::new(&config.theme, "test");
        assert!(one_shot.begin_single_replace());
        let mut platform = ClipboardPlatform {
            text: "paste".into(),
        };
        assert!(
            handle_clipboard_shortcut(
                &mut one_shot,
                &Key::Character("v".into()),
                ModifiersState::CONTROL,
                &mut platform,
            )
            .unwrap()
            .unwrap()
        );
        assert_eq!(one_shot.selected_text(), "paste");
        assert_eq!(one_shot.cursor_mode, CursorMode::Replace);

        for prefix in ["2", "0"] {
            let mut state = EditorState::new(&config.theme, "test");
            assert!(
                state.handle_toolbar_shortcut(
                    &Key::Character(prefix.into()),
                    ModifiersState::empty()
                )
            );
            let mut platform = ClipboardPlatform { text: "x".into() };
            assert!(
                handle_clipboard_shortcut(
                    &mut state,
                    &Key::Character("v".into()),
                    ModifiersState::CONTROL,
                    &mut platform,
                )
                .unwrap()
                .unwrap()
            );
            assert_eq!(state.selected_text(), "x");
        }

        let mut copy = EditorState::new(&config.theme, "test");
        copy.insert("copy");
        copy.move_to(Coord::default());
        copy.extend_selection(Direction::Right);
        copy.extend_selection(Direction::Right);
        copy.extend_selection(Direction::Right);
        let before = copy.clone();
        let mut platform = ClipboardPlatform::default();
        assert!(
            !handle_clipboard_shortcut(
                &mut copy,
                &Key::Character("C".into()),
                ModifiersState::SUPER,
                &mut platform,
            )
            .unwrap()
            .unwrap()
        );
        assert_eq!(platform.text, "copy");
        assert_eq!(copy.grid.lines, before.grid.lines);
        assert_eq!(copy.selection, before.selection);
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

    #[test]
    fn escape_closes_export_menu_without_collapsing_canvas_selection() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        state.extend_selection(Direction::Right);
        let bounds = state.selection_bounds();

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Character("0".into()),
                Some("0"),
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert!(state.toolbar.export_menu_open());
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Escape),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert!(!state.toolbar.export_menu_open());
        assert_eq!(state.selection_bounds(), bounds);
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
    }

    fn line_contents(line: &[crate::model::Atom]) -> String {
        line.iter().map(|atom| atom.contents.as_str()).collect()
    }
}
