use std::collections::HashMap;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use winit::event::{ElementState, Event, Ime, MouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowId;

mod app;
mod diagnostics;
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
use input::{EditCommand, edit_command, pointer_position_to_coord};
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
                    match create_editor_window(elwt, &config) {
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
                handle_command(command, None, &mut windows, elwt, &config);
            }
            Event::AboutToWait => {
                if let Some(watch) = user_config_watch.as_mut() {
                    poll_user_config_updates(watch, &mut config, &mut user_keys, &mut windows);
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
                                editor.state.insert(&text);
                                editor.request_redraw();
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state == ElementState::Pressed =>
                        {
                            if let Some(action) =
                                user_keys.action_for_event(&event, editor.modifiers)
                            {
                                pending_command = Some(app_command_from_user_action(action));
                            } else if editor
                                .state
                                .cycle_toolbar_shortcut(&event.logical_key, editor.modifiers)
                            {
                                editor.request_redraw();
                            } else if let Some(command) =
                                edit_command(&event, editor.modifiers, editor.state.cursor_mode)
                            {
                                apply_edit_command(&mut editor.state, command);
                                editor.request_redraw();
                            } else if !editor.modifiers.control_key()
                                && editor.state.cursor_mode.accepts_text()
                                && !editor.modifiers.alt_key()
                                && !editor.modifiers.super_key()
                                && let Some(text) = event.text
                                && !text.chars().all(char::is_control)
                            {
                                editor.state.insert(&text);
                                editor.request_redraw();
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            editor.mouse_cell = pointer_position_to_coord(
                                position.x,
                                position.y,
                                &editor.renderer,
                                editor.window.scale_factor(),
                                &config,
                                editor.viewport,
                            );
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } => {
                            if let Some(coord) = editor.mouse_cell {
                                editor.state.move_to(coord);
                                editor.request_redraw();
                            }
                        }
                        WindowEvent::ScaleFactorChanged { .. } => editor.request_redraw(),
                        _ => {}
                    }
                }

                if let Some(command) = pending_command {
                    handle_command(command, Some(window_id), &mut windows, elwt, &config);
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

fn apply_edit_command(state: &mut EditorState, command: EditCommand) {
    match command {
        EditCommand::Move(direction) => state.move_cursor(direction),
        EditCommand::Draw(direction) => state.move_or_draw(direction, true),
        EditCommand::Clear => state.clear_cell(),
        EditCommand::ToggleTextEntry => state.toggle_text_entry(),
        EditCommand::Home => state.move_home(),
        EditCommand::End => state.move_end(),
        EditCommand::Backspace => state.backspace(),
        EditCommand::Delete => state.delete(),
        EditCommand::Newline => state.newline(),
        EditCommand::InsertTab => state.insert("    "),
    }
}
