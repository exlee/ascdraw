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
mod macos;
mod render;
mod user_keys;

use app::{
    AppCommand, AppConfig, AppEvent, AppState, Args, WINDOW_TITLE_UI_OPTION, apply_notification,
    load_config,
};
use diagnostics::log_error;
use input::{
    MouseMotionState, ScrollState, key_event_to_kak, pointer_position_to_coord,
    scroll_delta_to_kak, send_keys, send_mouse_button, send_mouse_move, send_resize, send_scroll,
};
use kakoune_messages::{Coord, KakouneNotification};
#[cfg(unix)]
use kakoune_process::spawn_client_close_listener;
use kakoune_process::{build_kakoune_help_command, spawn_kakoune, spawn_stdin_writer};
use render::{Renderer, load_renderer, render, resize_surface};
use user_keys::{FontSizeAction, UserAction, UserKeys};

#[cfg(target_os = "macos")]
fn apply_platform_window_attributes(
    attrs: WindowAttributes,
    config: &AppConfig,
) -> WindowAttributes {
    if config.transparent_menubar {
        attrs
            .with_titlebar_transparent(true)
            .with_title_hidden(true)
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

#[cfg(target_os = "macos")]
fn native_window_title<'a>(config: &AppConfig, title: &'a str) -> &'a str {
    if config.transparent_menubar {
        ""
    } else {
        title
    }
}

#[cfg(not(target_os = "macos"))]
fn native_window_title<'a>(_config: &AppConfig, title: &'a str) -> &'a str {
    title
}

#[cfg(target_os = "macos")]
fn should_update_native_window_title(config: &AppConfig) -> bool {
    !config.transparent_menubar
}

#[cfg(not(target_os = "macos"))]
fn should_update_native_window_title(_config: &AppConfig) -> bool {
    true
}

#[cfg(target_os = "macos")]
fn set_native_window_title(window: &Window, config: &AppConfig, title: &str) {
    if should_update_native_window_title(config) {
        window.set_title(title);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_native_window_title(window: &Window, _config: &AppConfig, title: &str) {
    window.set_title(title);
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
    session: OsString,
    client_id: Option<String>,
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
            .with_title(native_window_title(config, "kakvide"))
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
    client_close_socket: Option<&Path>,
) -> Result<ClientWindow> {
    let context = SoftContext::new(window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    let mut surface =
        Surface::new(&context, window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    resize_surface(&mut surface, window.inner_size())?;

    let renderer = load_renderer(config);
    let mut child = spawn_kakoune(args, proxy, window.id(), client_close_socket)?;
    let command_tx = spawn_stdin_writer(&mut child)?;

    let client = ClientWindow {
        window,
        surface,
        child,
        session: OsString::new(),
        client_id: None,
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
    client_close_socket: Option<&Path>,
) -> Result<ClientWindow> {
    let window = Rc::new(event_loop.create_window(window_attributes(config, window_icon))?);
    create_client_window(window, args, proxy, config, client_close_socket)
}

fn create_active_client_window(
    elwt: &ActiveEventLoop,
    args: &Args,
    proxy: EventLoopProxy<AppEvent>,
    config: &AppConfig,
    window_icon: Option<Icon>,
    client_close_socket: Option<&Path>,
) -> Result<ClientWindow> {
    let window = Rc::new(elwt.create_window(window_attributes(config, window_icon))?);
    create_client_window(window, args, proxy, config, client_close_socket)
}

fn focused_window_id(clients: &HashMap<WindowId, ClientWindow>) -> Option<WindowId> {
    clients
        .iter()
        .find_map(|(window_id, client)| client.window.has_focus().then_some(*window_id))
        .or_else(|| (clients.len() == 1).then(|| *clients.keys().next().expect("one client")))
}

fn command_window_id(
    clients: &HashMap<WindowId, ClientWindow>,
    source_window_id: Option<WindowId>,
) -> Option<WindowId> {
    source_window_id
        .filter(|window_id| clients.contains_key(window_id))
        .or_else(|| focused_window_id(clients))
}

fn close_client(
    clients: &mut HashMap<WindowId, ClientWindow>,
    window_id: WindowId,
    elwt: &ActiveEventLoop,
    exit_if_empty: bool,
) {
    if let Some(mut client) = clients.remove(&window_id) {
        let _ = client.child.kill();
    }
    if exit_if_empty && clients.is_empty() {
        elwt.exit();
    }
}

fn remove_closed_client(
    clients: &mut HashMap<WindowId, ClientWindow>,
    session: &OsStr,
    client_id: &str,
    elwt: &ActiveEventLoop,
) {
    let window_id = clients.iter().find_map(|(window_id, client)| {
        (client.session == session && client.client_id.as_deref() == Some(client_id))
            .then_some(*window_id)
    });
    if let Some(window_id) = window_id {
        clients.remove(&window_id);
    }
    if clients.is_empty() {
        elwt.exit();
    }
}

fn client_name_from_ui_options(
    options: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    options
        .get(WINDOW_TITLE_UI_OPTION)
        .and_then(serde_json::Value::as_str)
        .and_then(|title| title.rsplit_once(" - ").map(|(_, client)| client.trim()))
        .filter(|client| !client.is_empty())
        .map(str::to_string)
}

fn adjust_client_font_size(client: &mut ClientWindow, action: FontSizeAction, config: &AppConfig) {
    let changed = match action {
        FontSizeAction::Increase => client.renderer.adjust_font_size(1.0),
        FontSizeAction::Decrease => client.renderer.adjust_font_size(-1.0),
        FontSizeAction::Reset => client.renderer.reset_font_size(),
    };
    if changed {
        client.send_resize(config);
        client.request_redraw();
    }
}

struct RuntimeContext<'a> {
    elwt: &'a ActiveEventLoop,
    clients: &'a mut HashMap<WindowId, ClientWindow>,
    proxy: EventLoopProxy<AppEvent>,
    config: &'a AppConfig,
    window_icon: Option<Icon>,
    kak_bin: &'a str,
    kakoune_session: &'a OsStr,
    client_close_socket: Option<&'a Path>,
}

impl RuntimeContext<'_> {
    fn open_session_window(&mut self, session: &OsStr, paths: &[PathBuf], log_label: &str) {
        let open_args = connected_kakoune_args(self.kak_bin, session, paths);
        match create_active_client_window(
            self.elwt,
            &open_args,
            self.proxy.clone(),
            self.config,
            self.window_icon.clone(),
            self.client_close_socket,
        ) {
            Ok(client) => {
                let mut client = client;
                client.session = session.to_os_string();
                self.clients.insert(client.window_id(), client);
            }
            Err(error) => log_error(format!("{log_label} window creation failed: {error:#}")),
        }
    }

    fn handle_command(&mut self, command: AppCommand, source_window_id: Option<WindowId>) {
        match command {
            AppCommand::FontScaleUp => {
                self.adjust_font_size_for_window(source_window_id, FontSizeAction::Increase)
            }
            AppCommand::FontScaleDown => {
                self.adjust_font_size_for_window(source_window_id, FontSizeAction::Decrease)
            }
            AppCommand::FontScaleReset => {
                self.adjust_font_size_for_window(source_window_id, FontSizeAction::Reset)
            }
            AppCommand::WindowNew => {
                let session = command_window_id(self.clients, source_window_id)
                    .and_then(|window_id| self.clients.get(&window_id))
                    .map(|client| client.session.clone())
                    .filter(|session| !session.is_empty())
                    .unwrap_or_else(|| self.kakoune_session.to_os_string());
                self.open_session_window(&session, &[], "new");
            }
            AppCommand::WindowClose => {
                if let Some(window_id) = command_window_id(self.clients, source_window_id) {
                    close_client(self.clients, window_id, self.elwt, true);
                }
            }
            AppCommand::ConnectToSession(session) => {
                self.open_session_window(&session, &[], "connect to session");
            }
            AppCommand::SwitchToSession(session) => {
                if let Some(window_id) = command_window_id(self.clients, source_window_id) {
                    close_client(self.clients, window_id, self.elwt, false);
                }
                self.open_session_window(&session, &[], "switch to session");
                if self.clients.is_empty() {
                    self.elwt.exit();
                }
            }
        }
    }

    fn adjust_font_size_for_window(
        &mut self,
        source_window_id: Option<WindowId>,
        action: FontSizeAction,
    ) {
        if let Some(window_id) = command_window_id(self.clients, source_window_id)
            && let Some(client) = self.clients.get_mut(&window_id)
        {
            adjust_client_font_size(client, action, self.config);
        }
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

    let mut initial_client = create_initial_client_window(
        &event_loop,
        &args,
        proxy.clone(),
        &config,
        window_icon.clone(),
        client_close_socket.as_deref(),
    )?;
    let kakoune_session = resolve_kakoune_session(&args.kak_args, initial_client.child.id());
    initial_client.session = kakoune_session.clone();
    let kak_bin = args.kak_bin.clone();
    let mut clients = HashMap::new();
    clients.insert(initial_client.window_id(), initial_client);
    #[cfg(target_os = "macos")]
    let mut did_install_macos_menus_after_winit = false;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

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
                    let should_force_resize =
                        matches!(notification.as_ref(), KakouneNotification::Draw { .. })
                            && !client.did_force_startup_resize;
                    let old_window_title = client.state.window_title.clone();
                    if client.client_id.is_none()
                        && let KakouneNotification::SetUiOptions { options } = notification.as_ref()
                    {
                        client.client_id = client_name_from_ui_options(options);
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
                RuntimeContext {
                    elwt,
                    clients: &mut clients,
                    proxy: proxy.clone(),
                    config: &config,
                    window_icon: window_icon.clone(),
                    kak_bin: &kak_bin,
                    kakoune_session: &kakoune_session,
                    client_close_socket: client_close_socket.as_deref(),
                }
                .open_session_window(&kakoune_session, &paths, "open file");
            }
            Event::UserEvent(AppEvent::Command(command)) => {
                RuntimeContext {
                    elwt,
                    clients: &mut clients,
                    proxy: proxy.clone(),
                    config: &config,
                    window_icon: window_icon.clone(),
                    kak_bin: &kak_bin,
                    kakoune_session: &kakoune_session,
                    client_close_socket: client_close_socket.as_deref(),
                }
                .handle_command(command, None);
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
                    RuntimeContext {
                        elwt,
                        clients: &mut clients,
                        proxy: proxy.clone(),
                        config: &config,
                        window_icon: window_icon.clone(),
                        kak_bin: &kak_bin,
                        kakoune_session: &kakoune_session,
                        client_close_socket: client_close_socket.as_deref(),
                    }
                    .handle_command(command, Some(window_id));
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
        client_name_from_ui_options, connected_kakoune_args, default_launch_directory,
        extract_kak_bin, resolve_kakoune_session, should_show_combined_help,
        should_update_native_window_title,
    };
    use crate::app::AppConfig;

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
    fn client_name_is_read_from_kakvide_title_ui_option() {
        let mut options = serde_json::Map::new();
        options.insert(
            crate::app::WINDOW_TITLE_UI_OPTION.to_string(),
            serde_json::Value::String("/tmp/project - client0".to_string()),
        );

        assert_eq!(
            client_name_from_ui_options(&options),
            Some("client0".to_string())
        );
    }

    #[test]
    fn transparent_menubar_skips_native_window_title_updates_on_macos() {
        let transparent_config = AppConfig {
            transparent_menubar: true,
            ..AppConfig::default()
        };
        let standard_config = AppConfig {
            transparent_menubar: false,
            ..AppConfig::default()
        };

        assert_eq!(
            should_update_native_window_title(&transparent_config),
            !cfg!(target_os = "macos")
        );
        assert!(should_update_native_window_title(&standard_config));
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

    #[test]
    fn connected_kakoune_args_connect_to_session_without_paths() {
        let args = connected_kakoune_args("custom-kak", OsString::from("work").as_os_str(), &[]);

        assert_eq!(args.kak_bin, "custom-kak");
        assert_eq!(
            args.kak_args,
            vec![OsString::from("-c"), OsString::from("work")]
        );
    }
}
