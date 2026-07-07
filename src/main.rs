use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use winit::event::{ElementState, Event, Ime, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowId;

mod app;
mod diagnostics;
mod face_resolution;
mod icon;
mod input;
mod kakoune_integration;
mod kakoune_messages;
mod kakoune_process;
mod layout;
#[cfg(target_os = "macos")]
mod macos;
mod render;
mod runtime;
mod title_policy;
mod user_keys;

use app::{
    AppCommand, AppEvent, Args, apply_notification, checked_config_paths, load_config,
    show_config_toml, user_config_path,
};
use diagnostics::log_error;
use input::{
    key_event_to_kak, pointer_position_to_coord, scroll_delta_to_kak, send_keys,
    send_mouse_button, send_mouse_move, send_scroll,
};
use kakoune_integration::bootstrap_client_hooks;
use kakoune_messages::KakouneNotification;
#[cfg(unix)]
use kakoune_process::spawn_client_close_listener;
use kakoune_process::build_kakoune_help_command;
use render::{render, resize_surface};
use runtime::client::{
    ClientWindow, RuntimeContext, create_active_client_window, remove_closed_client,
};
use runtime::config_watch::{UserConfigWatch, event_loop_wait_duration, poll_user_config_updates};
use runtime::startup::{
    StartupOpenState, resolve_kakoune_session, should_create_fallback_startup_client,
    should_handle_startup_open_with_files, should_ignore_startup_open_files, startup_args,
    startup_open_files, startup_open_state_for_launch,
};
use title_policy::{decode_title_update, set_native_window_title};
use user_keys::{FontSizeAction, UserAction, UserKeys};

fn default_launch_directory(current_dir: &Path, home: Option<OsString>) -> Option<PathBuf> {
    if current_dir == Path::new("/") {
        home.map(PathBuf::from)
    } else {
        None
    }
}

fn apply_launch_directory() {
    let Ok(current_dir) = env::current_dir() else {
        return;
    };
    let Some(home) = default_launch_directory(&current_dir, env::var_os("HOME")) else {
        return;
    };
    if let Err(error) = env::set_current_dir(&home) {
        log_error(format!(
            "failed to set launch directory to {}: {error:#}",
            home.display()
        ));
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

fn main() -> ExitCode {
    if let Err(error) = diagnostics::init() {
        eprintln!("diagnostics setup failed: {error:#}");
    }
    diagnostics::install_panic_hook();

    let raw_args: Vec<OsString> = env::args_os().collect();
    match try_main(raw_args) {
        Ok(code) => code,
        Err(error) => {
            log_error(format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

#[allow(deprecated)]
fn try_main(raw_args: Vec<OsString>) -> Result<ExitCode> {
    if should_show_combined_help(&raw_args) {
        print_combined_help(&extract_kak_bin(&raw_args))?;
        return Ok(ExitCode::SUCCESS);
    }
    let args = Args::parse_from(raw_args);
    if let Some(output) = early_exit_output(&args)? {
        print!("{output}");
        return Ok(ExitCode::SUCCESS);
    }
    apply_launch_directory();

    let mut config = load_config()?;
    let mut user_keys = UserKeys::from_config(&config.keys)?;
    let mut user_config_watch = user_config_path().map(UserConfigWatch::new);
    if let Err(error) = icon::apply_app_icon() {
        log_error(format!("app icon setup failed: {error:#}"));
    }
    let window_icon = match icon::load_window_icon() {
        Ok(icon) => Some(icon),
        Err(error) => {
            log_error(format!("window icon setup failed: {error:#}"));
            None
        }
    };

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    #[cfg(target_os = "macos")]
    if let Err(error) = macos::install(proxy.clone(), args.kak_bin.clone()) {
        log_error(format!("macOS integration setup failed: {error:#}"));
    }

    #[cfg(unix)]
    let _client_close_listener = match spawn_client_close_listener(proxy.clone()) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log_error(format!("client close socket setup failed: {error:#}"));
            None
        }
    };
    #[cfg(unix)]
    let client_close_socket = _client_close_listener
        .as_ref()
        .map(|listener| listener.path().to_path_buf());
    #[cfg(not(unix))]
    let client_close_socket: Option<PathBuf> = None;

    let mut pending_startup_open_files =
        Some(startup_open_files(&args.kak_args)).filter(|paths| !paths.is_empty());
    let kak_bin = args.kak_bin.clone();
    let mut clients: HashMap<WindowId, ClientWindow> = HashMap::new();
    let mut kakoune_session: Option<OsString> = None;
    let mut startup_open_state =
        startup_open_state_for_launch(cfg!(target_os = "macos"), &args.kak_args);
    #[cfg(target_os = "macos")]
    let mut did_install_macos_menus_after_winit = false;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::WaitUntil(
            std::time::Instant::now() + event_loop_wait_duration(&clients, &config),
        ));

        match event {
            Event::Resumed => {
                #[cfg(target_os = "macos")]
                if !did_install_macos_menus_after_winit {
                    if let Err(error) = macos::install_menus() {
                        log_error(format!("macOS menu setup failed: {error:#}"));
                    }
                    did_install_macos_menus_after_winit = true;
                }

                for client in clients.values() {
                    client.send_resize(&config);
                    client.request_redraw();
                }
            }
            Event::UserEvent(AppEvent::Rpc(window_id, notification)) => {
                if let Some(client) = clients.get_mut(&window_id) {
                    bootstrap_client_hooks(client, notification.as_ref());
                    let should_force_resize =
                        matches!(notification.as_ref(), KakouneNotification::Draw { .. })
                            && !client.did_force_startup_resize;
                    let old_window_title = client.state.window_title.clone();
                    if client.client_id.is_none()
                        && let KakouneNotification::SetUiOptions { options } = notification.as_ref()
                    {
                        client.client_id =
                            decode_title_update(options).and_then(|update| update.client_name);
                    }
                    apply_notification(&mut client.state, *notification);
                    if client.state.window_title != old_window_title {
                        set_native_window_title(
                            &client.window,
                            &config,
                            &client.state.window_title,
                        );
                    }
                    if should_force_resize {
                        client.send_resize(&config);
                        client.did_force_startup_resize = true;
                    }
                    client.request_redraw();
                }
            }
            Event::UserEvent(AppEvent::KakouneExited(window_id)) => {
                clients.remove(&window_id);
                if clients.is_empty() {
                    elwt.exit();
                }
            }
            Event::UserEvent(AppEvent::ClientClosed { session, client_id }) => {
                remove_closed_client(&mut clients, &session, &client_id, elwt);
            }
            Event::UserEvent(AppEvent::OpenFiles(paths)) => {
                if should_handle_startup_open_with_files(
                    startup_open_state,
                    !clients.is_empty(),
                    kakoune_session.is_some(),
                ) {
                    let open_args = startup_args(&args, &paths);
                    match create_startup_client_window(
                        elwt,
                        &open_args,
                        proxy.clone(),
                        &config,
                        window_icon.clone(),
                        client_close_socket.as_deref(),
                    ) {
                        Ok(mut client) => {
                            let session =
                                resolve_kakoune_session(&open_args.kak_args, client.child.id());
                            client.session = session.clone();
                            kakoune_session = Some(session);
                            clients.insert(client.window_id(), client);
                            startup_open_state = StartupOpenState::StartupOpenHandled;
                        }
                        Err(error) => {
                            log_error(format!("startup open window creation failed: {error:#}"))
                        }
                    }
                    return;
                }
                if should_ignore_startup_open_files(&mut pending_startup_open_files, &paths) {
                    return;
                }
                let Some(session) = kakoune_session.as_deref() else {
                    log_error("open file requested before startup session was ready".to_string());
                    return;
                };
                RuntimeContext {
                    elwt,
                    clients: &mut clients,
                    proxy: proxy.clone(),
                    config: &config,
                    window_icon: window_icon.clone(),
                    kak_bin: &kak_bin,
                    kakoune_session: session,
                    client_close_socket: client_close_socket.as_deref(),
                }
                .open_session_window(session, &paths, "open file");
            }
            Event::UserEvent(AppEvent::Command(command)) => {
                let Some(session) = kakoune_session.as_deref() else {
                    log_error("command requested before startup session was ready".to_string());
                    return;
                };
                RuntimeContext {
                    elwt,
                    clients: &mut clients,
                    proxy: proxy.clone(),
                    config: &config,
                    window_icon: window_icon.clone(),
                    kak_bin: &kak_bin,
                    kakoune_session: session,
                    client_close_socket: client_close_socket.as_deref(),
                }
                .handle_command(command, None);
            }
            Event::AboutToWait => {
                if let Some(watch) = user_config_watch.as_mut() {
                    poll_user_config_updates(watch, &mut config, &mut user_keys, &mut clients);
                }
                for client in clients.values() {
                    if client.multi_cursor_indicator_active(&config) {
                        client.request_redraw();
                    }
                }
                if should_create_fallback_startup_client(
                    startup_open_state,
                    !clients.is_empty(),
                    kakoune_session.is_some(),
                ) {
                    let open_args = startup_args(&args, &[]);
                    match create_startup_client_window(
                        elwt,
                        &open_args,
                        proxy.clone(),
                        &config,
                        window_icon.clone(),
                        client_close_socket.as_deref(),
                    ) {
                        Ok(mut client) => {
                            let session =
                                resolve_kakoune_session(&open_args.kak_args, client.child.id());
                            client.session = session.clone();
                            kakoune_session = Some(session);
                            clients.insert(client.window_id(), client);
                            startup_open_state = StartupOpenState::NoPendingStartupOpen;
                        }
                        Err(error) => {
                            log_error(format!("startup window creation failed: {error:#}"));
                            elwt.exit();
                        }
                    }
                }
            }
            Event::WindowEvent { window_id, event } => {
                let mut remove_client = false;
                let mut pending_command = None;
                if let Some(client) = clients.get_mut(&window_id) {
                    match event {
                        WindowEvent::CloseRequested => {
                            let _ = client.child.kill();
                            remove_client = true;
                        }
                        WindowEvent::Resized(size) => {
                            if let Err(error) = resize_surface(&mut client.surface, size) {
                                log_error(format!("surface resize failed: {error:#}"));
                            }
                            client.send_resize(&config);
                            client.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            if let Err(error) = render(
                                &client.window,
                                &mut client.surface,
                                &client.state,
                                &client.renderer,
                                &config,
                            ) {
                                log_error(format!("render failed: {error:#}"));
                                let _ = client.child.kill();
                                remove_client = true;
                            }
                        }
                        WindowEvent::ModifiersChanged(new_modifiers) => {
                            client.modifiers = new_modifiers.state();
                        }
                        WindowEvent::Ime(Ime::Commit(text)) => {
                            if !text.is_empty()
                                && !client.modifiers.control_key()
                                && !client.modifiers.alt_key()
                                && !client.modifiers.super_key()
                            {
                                send_keys(&client.command_tx, &[text.to_string()]);
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. } => {
                            if event.state == ElementState::Pressed {
                                if let Some(action) =
                                    user_keys.action_for_event(&event, client.modifiers)
                                {
                                    pending_command = Some(app_command_from_user_action(action));
                                } else if let Some(keys) =
                                    key_event_to_kak(&event, client.modifiers)
                                {
                                    send_keys(&client.command_tx, &[keys]);
                                }
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            client.mouse_cell = pointer_position_to_coord(
                                position.x,
                                position.y,
                                &client.renderer,
                                &client.window,
                                &config,
                            );
                            if client.mouse_motion_state.should_send_move() {
                                send_mouse_move(&client.command_tx, client.mouse_cell);
                            }
                        }
                        WindowEvent::MouseInput { state, button, .. } => match state {
                            ElementState::Pressed => {
                                client.mouse_motion_state.set_button(button, true);
                                send_mouse_button(
                                    &client.command_tx,
                                    true,
                                    button,
                                    client.mouse_cell,
                                )
                            }
                            ElementState::Released => {
                                send_mouse_button(
                                    &client.command_tx,
                                    false,
                                    button,
                                    client.mouse_cell,
                                );
                                client.mouse_motion_state.set_button(button, false);
                            }
                        },
                        WindowEvent::CursorLeft { .. } | WindowEvent::Focused(false) => {
                            client.mouse_motion_state.reset();
                        }
                        WindowEvent::MouseWheel { delta, .. } => {
                            if let Some(amount) = scroll_delta_to_kak(
                                delta,
                                config.mouse_scroll_rate.max(0.0) as f64,
                                &mut client.scroll_state,
                            ) {
                                send_scroll(&client.command_tx, amount, client.mouse_cell);
                            }
                        }
                        WindowEvent::ScaleFactorChanged { .. } => {
                            client.send_resize(&config);
                            client.request_redraw();
                        }
                        _ => {}
                    }
                }
                if let Some(command) = pending_command {
                    if let Some(session) = kakoune_session.as_deref() {
                        RuntimeContext {
                            elwt,
                            clients: &mut clients,
                            proxy: proxy.clone(),
                            config: &config,
                            window_icon: window_icon.clone(),
                            kak_bin: &kak_bin,
                            kakoune_session: session,
                            client_close_socket: client_close_socket.as_deref(),
                        }
                        .handle_command(command, Some(window_id));
                    } else {
                        log_error(
                            "window command requested before startup session was ready".to_string(),
                        );
                    }
                } else if remove_client {
                    clients.remove(&window_id);
                    if clients.is_empty() {
                        elwt.exit();
                    }
                }
            }
            _ => {}
        }
    })?;

    Ok(ExitCode::SUCCESS)
}

fn create_startup_client_window(
    elwt: &winit::event_loop::ActiveEventLoop,
    startup_args: &Args,
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    config: &app::AppConfig,
    window_icon: Option<winit::window::Icon>,
    client_close_socket: Option<&Path>,
) -> Result<ClientWindow> {
    create_active_client_window(
        elwt,
        startup_args,
        proxy,
        config,
        window_icon,
        client_close_socket,
    )
}

fn early_exit_output(args: &Args) -> Result<Option<String>> {
    if args.show_config {
        let config = load_config()?;
        let config_toml = show_config_toml(&config)?;
        let checked_paths = checked_config_paths();
        let mut output = String::from("Checked configuration paths:\n");
        for path in checked_paths {
            output.push_str(&format!("- {}\n", path.display()));
        }
        output.push_str("\nCurrent configuration:\n\n");
        output.push_str(&config_toml);
        Ok(Some(output))
    } else {
        Ok(None)
    }
}

fn should_show_combined_help(raw_args: &[OsString]) -> bool {
    let args: Vec<&OsString> = raw_args.iter().skip(1).collect();
    let split_at = args
        .iter()
        .position(|arg| arg.as_os_str() == OsStr::new("--"))
        .unwrap_or(args.len());

    if args[..split_at]
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("--help" | "-h")))
    {
        return true;
    }

    matches!(
        args.get(split_at + 1..),
        Some([arg]) if matches!(arg.to_str(), Some("--help" | "-help"))
    )
}

fn extract_kak_bin(raw_args: &[OsString]) -> OsString {
    let mut args = raw_args.iter().skip(1);
    while let Some(arg) = args.next() {
        if arg.as_os_str() == OsStr::new("--") {
            break;
        }
        if arg.as_os_str() == OsStr::new("--kak-bin")
            && let Some(value) = args.next()
        {
            return value.clone();
        }
    }

    OsString::from("kak")
}

fn print_combined_help(kak_bin: &OsStr) -> Result<()> {
    let mut command = Args::command();
    command.print_help()?;
    println!();
    println!();
    println!("Kakoune help:");
    println!();

    let mut help_command = build_kakoune_help_command(kak_bin);
    let output = help_command
        .output()
        .with_context(|| format!("failed to run {} --help", kak_bin.to_string_lossy()))?;

    io::stdout().write_all(&output.stdout)?;
    io::stderr().write_all(&output.stderr)?;

    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} --help exited with {}",
            kak_bin.to_string_lossy(),
            output.status
        )
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use std::ffi::OsString;
    use std::path::Path;
    use std::path::PathBuf;

    use super::{default_launch_directory, early_exit_output, extract_kak_bin, should_show_combined_help};
    use crate::app::Args;

    #[test]
    fn top_level_help_triggers_combined_help() {
        assert!(should_show_combined_help(&[
            OsString::from("kakvide"),
            OsString::from("--help"),
        ]));
    }

    #[test]
    fn forwarded_help_after_double_dash_does_not_trigger_combined_help() {
        assert!(should_show_combined_help(&[
            OsString::from("kakvide"),
            OsString::from("--"),
            OsString::from("--help"),
        ]));
    }

    #[test]
    fn forwarded_non_help_after_double_dash_does_not_trigger_combined_help() {
        assert!(!should_show_combined_help(&[
            OsString::from("kakvide"),
            OsString::from("--"),
            OsString::from("file.txt"),
        ]));
    }

    #[test]
    fn custom_kak_bin_is_extracted_for_help() {
        assert_eq!(
            extract_kak_bin(&[
                OsString::from("kakvide"),
                OsString::from("--kak-bin"),
                OsString::from("/tmp/kak"),
                OsString::from("--help"),
            ]),
            OsString::from("/tmp/kak")
        );
    }

    #[test]
    fn launch_directory_defaults_to_home_when_started_at_root() {
        assert_eq!(
            default_launch_directory(Path::new("/"), Some(OsString::from("/Users/example"))),
            Some(PathBuf::from("/Users/example"))
        );
    }

    #[test]
    fn launch_directory_preserves_non_root_current_directory() {
        assert_eq!(
            default_launch_directory(
                Path::new("/Users/example/project"),
                Some(OsString::from("/Users/example")),
            ),
            None
        );
    }

    #[test]
    fn launch_directory_ignores_missing_home() {
        assert_eq!(default_launch_directory(Path::new("/"), None), None);
    }

    #[test]
    fn early_exit_output_returns_none_without_show_config() {
        let args = Args::try_parse_from(["kakvide", "file.txt"]).expect("args should parse");

        assert_eq!(early_exit_output(&args).unwrap(), None);
    }

    #[test]
    fn early_exit_output_returns_effective_config_for_show_config() {
        let args = Args::try_parse_from(["kakvide", "--show-config"]).expect("args should parse");
        let output = early_exit_output(&args)
            .expect("show config should succeed")
            .expect("show config should early-exit");

        assert!(output.contains("Checked configuration paths:"));
        assert!(output.contains("Current configuration:\n\n"));
        assert!(output.contains("kakvide.toml"));
        assert!(output.contains("font-family = "));
        assert!(output.contains("[macos]"));
        assert!(output.contains("[keys]"));
    }
}
