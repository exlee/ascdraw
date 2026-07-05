use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, ExitCode};
use std::rc::Rc;
use std::sync::mpsc::Sender;

use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser};
use softbuffer::{Context as SoftContext, Surface};
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, Ime, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;
use winit::window::WindowLevel;
use winit::window::{Icon, Window, WindowAttributes, WindowId};

mod app;
mod diagnostics;
mod face_resolution;
mod icon;
mod input;
mod kakoune_messages;
mod kakoune_process;
mod layout;
#[cfg(target_os = "macos")]
mod macos_open_files;
mod render;
mod user_keys;

use app::{AppConfig, AppEvent, AppState, Args, apply_notification, load_config};
use diagnostics::log_error;
use input::{
    MouseMotionState, ScrollState, key_event_to_kak, pointer_position_to_coord,
    scroll_delta_to_kak, send_keys, send_mouse_button, send_mouse_move, send_resize, send_scroll,
};
use kakoune_messages::{Coord, KakouneNotification};
use kakoune_process::{build_kakoune_help_command, spawn_kakoune, spawn_stdin_writer};
use render::{Renderer, load_renderer, render, resize_surface};
use user_keys::{FontSizeAction, UserKeys};

#[cfg(target_os = "macos")]
fn apply_platform_window_attributes(
    attrs: WindowAttributes,
    config: &AppConfig,
) -> WindowAttributes {
    if config.transparent_menubar {
        attrs
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
    } else {
        attrs
    }
}

#[cfg(not(target_os = "macos"))]
fn apply_platform_window_attributes(
    attrs: WindowAttributes,
    _config: &AppConfig,
) -> WindowAttributes {
    attrs
}

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

struct ClientWindow {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    child: Child,
    command_tx: Sender<String>,
    modifiers: ModifiersState,
    mouse_cell: Coord,
    mouse_motion_state: MouseMotionState,
    scroll_state: ScrollState,
    did_force_startup_resize: bool,
    state: AppState,
    renderer: Renderer,
}

impl ClientWindow {
    fn window_id(&self) -> WindowId {
        self.window.id()
    }

    fn send_resize(&self, config: &AppConfig) {
        send_resize(&self.command_tx, &self.window, &self.renderer, config);
    }

    fn request_redraw(&self) {
        self.window.request_redraw();
    }
}

fn window_attributes(config: &AppConfig, window_icon: Option<Icon>) -> WindowAttributes {
    apply_platform_window_attributes(
        WindowAttributes::default()
            .with_title("kakvide")
            .with_window_level(WindowLevel::Normal)
            .with_inner_size(LogicalSize::new(1200.0, 800.0))
            .with_window_icon(window_icon),
        config,
    )
}

fn create_client_window(
    window: Rc<Window>,
    args: &Args,
    proxy: EventLoopProxy<AppEvent>,
    config: &AppConfig,
) -> Result<ClientWindow> {
    let context = SoftContext::new(window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    let mut surface =
        Surface::new(&context, window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    resize_surface(&mut surface, window.inner_size())?;

    let renderer = load_renderer(config);
    let mut child = spawn_kakoune(args, proxy, window.id())?;
    let command_tx = spawn_stdin_writer(&mut child)?;

    let client = ClientWindow {
        window,
        surface,
        child,
        command_tx,
        modifiers: ModifiersState::empty(),
        mouse_cell: Coord { line: 0, column: 0 },
        mouse_motion_state: MouseMotionState::default(),
        scroll_state: ScrollState::default(),
        did_force_startup_resize: false,
        state: AppState::default(),
        renderer,
    };
    client.send_resize(config);
    client.request_redraw();
    Ok(client)
}

#[allow(deprecated)]
fn create_initial_client_window(
    event_loop: &EventLoop<AppEvent>,
    args: &Args,
    proxy: EventLoopProxy<AppEvent>,
    config: &AppConfig,
    window_icon: Option<Icon>,
) -> Result<ClientWindow> {
    let window = Rc::new(event_loop.create_window(window_attributes(config, window_icon))?);
    create_client_window(window, args, proxy, config)
}

fn create_active_client_window(
    elwt: &ActiveEventLoop,
    args: &Args,
    proxy: EventLoopProxy<AppEvent>,
    config: &AppConfig,
    window_icon: Option<Icon>,
) -> Result<ClientWindow> {
    let window = Rc::new(elwt.create_window(window_attributes(config, window_icon))?);
    create_client_window(window, args, proxy, config)
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
    apply_launch_directory();

    let config = load_config()?;
    let user_keys = UserKeys::from_config(&config.keys)?;
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
    if let Err(error) = macos_open_files::register_open_file_handler(proxy.clone()) {
        log_error(format!("open file handler setup failed: {error:#}"));
    }

    let initial_client = create_initial_client_window(
        &event_loop,
        &args,
        proxy.clone(),
        &config,
        window_icon.clone(),
    )?;
    let kakoune_session = resolve_kakoune_session(&args.kak_args, initial_client.child.id());
    let kak_bin = args.kak_bin.clone();
    let mut clients = HashMap::new();
    clients.insert(initial_client.window_id(), initial_client);

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            Event::Resumed => {
                for client in clients.values() {
                    client.send_resize(&config);
                    client.request_redraw();
                }
            }
            Event::UserEvent(AppEvent::Rpc(window_id, notification)) => {
                if let Some(client) = clients.get_mut(&window_id) {
                    let should_force_resize =
                        matches!(notification.as_ref(), KakouneNotification::Draw { .. })
                            && !client.did_force_startup_resize;
                    apply_notification(&mut client.state, *notification);
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
            Event::UserEvent(AppEvent::OpenFiles(paths)) => {
                let open_args = connected_kakoune_args(&kak_bin, &kakoune_session, &paths);
                match create_active_client_window(
                    elwt,
                    &open_args,
                    proxy.clone(),
                    &config,
                    window_icon.clone(),
                ) {
                    Ok(client) => {
                        clients.insert(client.window_id(), client);
                    }
                    Err(error) => log_error(format!("open file window creation failed: {error:#}")),
                }
            }
            Event::WindowEvent { window_id, event } => {
                let mut remove_client = false;
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
                                    let changed = match action {
                                        FontSizeAction::Increase => {
                                            client.renderer.adjust_font_size(1.0)
                                        }
                                        FontSizeAction::Decrease => {
                                            client.renderer.adjust_font_size(-1.0)
                                        }
                                        FontSizeAction::Reset => client.renderer.reset_font_size(),
                                    };
                                    if changed {
                                        client.send_resize(&config);
                                        client.request_redraw();
                                    }
                                    return;
                                }
                                if let Some(keys) = key_event_to_kak(&event, client.modifiers) {
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
                if remove_client {
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

fn resolve_kakoune_session(kak_args: &[OsString], child_id: u32) -> OsString {
    explicit_kakoune_session(kak_args).unwrap_or_else(|| OsString::from(child_id.to_string()))
}

fn explicit_kakoune_session(kak_args: &[OsString]) -> Option<OsString> {
    let mut args = kak_args.iter();
    while let Some(arg) = args.next() {
        if matches!(arg.to_str(), Some("-c" | "-C" | "-s")) {
            return args.next().cloned();
        }
    }

    None
}

fn connected_kakoune_args(kak_bin: &str, kakoune_session: &OsStr, paths: &[PathBuf]) -> Args {
    let mut kak_args = vec![OsString::from("-c"), kakoune_session.to_os_string()];
    kak_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    Args {
        kak_bin: kak_bin.to_string(),
        kak_args,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;
    use std::path::PathBuf;

    use super::{
        connected_kakoune_args, default_launch_directory, extract_kak_bin, resolve_kakoune_session,
        should_show_combined_help,
    };

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
    fn session_resolution_uses_child_id_without_explicit_session() {
        assert_eq!(
            resolve_kakoune_session(&[OsString::from("file.txt")], 12345),
            OsString::from("12345")
        );
    }

    #[test]
    fn session_resolution_uses_explicit_server_session() {
        assert_eq!(
            resolve_kakoune_session(
                &[
                    OsString::from("-s"),
                    OsString::from("work"),
                    OsString::from("file.txt"),
                ],
                12345,
            ),
            OsString::from("work")
        );
    }

    #[test]
    fn session_resolution_uses_explicit_client_session() {
        assert_eq!(
            resolve_kakoune_session(
                &[
                    OsString::from("-c"),
                    OsString::from("work"),
                    OsString::from("file.txt"),
                ],
                12345,
            ),
            OsString::from("work")
        );

        assert_eq!(
            resolve_kakoune_session(
                &[
                    OsString::from("-C"),
                    OsString::from("maybe-work"),
                    OsString::from("file.txt"),
                ],
                12345,
            ),
            OsString::from("maybe-work")
        );
    }

    #[test]
    fn connected_kakoune_args_connect_to_session_and_append_paths() {
        let paths = vec![
            PathBuf::from("/tmp/file with spaces.md"),
            PathBuf::from("/tmp/alice's note.md"),
        ];

        let args = connected_kakoune_args("custom-kak", OsString::from("work").as_os_str(), &paths);

        assert_eq!(args.kak_bin, "custom-kak");
        assert_eq!(
            args.kak_args,
            vec![
                OsString::from("-c"),
                OsString::from("work"),
                OsString::from("/tmp/file with spaces.md"),
                OsString::from("/tmp/alice's note.md"),
            ]
        );
    }
}
