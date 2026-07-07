use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, ExitCode};
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

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
    checked_config_paths, load_config, show_config_toml, user_config_path,
};
use diagnostics::log_error;
use input::{
    MouseMotionState, ScrollState, key_event_to_kak, pointer_position_to_coord,
    scroll_delta_to_kak, send_keys, send_mouse_button, send_mouse_move, send_paste, send_resize,
    send_scroll,
};
use kakoune_messages::{Coord, KakouneNotification, StatusStyle};
#[cfg(unix)]
use kakoune_process::spawn_client_close_listener;
use kakoune_process::{
    build_kakoune_help_command, kakvide_post_boot_command, spawn_kakoune, spawn_stdin_writer,
};
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
    kakvide_hook_install_state: KakvideHookInstallState,
    kakvide_post_boot_command: String,
    state: AppState,
    renderer: Renderer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KakvideHookInstallState {
    NotStarted,
    WaitingForCommandPrompt,
    Installed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupOpenState {
    NoPendingStartupOpen,
    WaitingForStartupOpenFiles,
    StartupOpenHandled,
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
    let mut child = spawn_kakoune(args, proxy, window.id())?;
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
        kakvide_hook_install_state: KakvideHookInstallState::NotStarted,
        kakvide_post_boot_command: kakvide_post_boot_command(client_close_socket),
        state: AppState::default(),
        renderer,
    };
    client.send_resize(config);
    client.request_redraw();
    Ok(client)
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
    #[cfg(target_os = "macos")]
    if let Err(error) = macos::apply_window_color_space(window.as_ref(), &config.macos) {
        log_error(format!("macOS color space setup failed: {error:#}"));
    }
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

fn kakvide_hook_prompt_keys() -> Vec<String> {
    vec![":".to_string()]
}

fn install_kakvide_hooks_once(
    client: &mut ClientWindow,
    notification: &KakouneNotification,
) -> bool {
    match client.kakvide_hook_install_state {
        KakvideHookInstallState::NotStarted => {
            send_keys(&client.command_tx, &kakvide_hook_prompt_keys());
            client.kakvide_hook_install_state = KakvideHookInstallState::WaitingForCommandPrompt;
            true
        }
        KakvideHookInstallState::WaitingForCommandPrompt => {
            if matches!(
                notification,
                KakouneNotification::DrawStatus {
                    style: StatusStyle::Command,
                    ..
                }
            ) {
                send_paste(
                    &client.command_tx,
                    &format!(
                        " evaluate-commands -draft %{{{}}}",
                        client.kakvide_post_boot_command
                    ),
                );
                send_keys(&client.command_tx, &[String::from("<ret>")]);
                client.kakvide_hook_install_state = KakvideHookInstallState::Installed;
                true
            } else {
                false
            }
        }
        KakvideHookInstallState::Installed => false,
    }
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

const USER_CONFIG_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
enum UserConfigWatchState {
    Missing,
    Present(SystemTime),
    Error(String),
}

struct UserConfigWatch {
    path: PathBuf,
    state: UserConfigWatchState,
}

impl UserConfigWatch {
    fn new(path: PathBuf) -> Self {
        let state = read_user_config_watch_state(&path);
        Self { path, state }
    }
}

fn read_user_config_watch_state(path: &Path) -> UserConfigWatchState {
    match fs::metadata(path) {
        Ok(metadata) => match metadata.modified() {
            Ok(modified) => UserConfigWatchState::Present(modified),
            Err(error) => UserConfigWatchState::Error(format!(
                "failed to read metadata for {}: {error}",
                path.display()
            )),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => UserConfigWatchState::Missing,
        Err(error) => UserConfigWatchState::Error(format!(
            "failed to read metadata for {}: {error}",
            path.display()
        )),
    }
}

fn escaped_kakoune_double_quoted_string(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
        .replace('\r', " ")
}

fn config_error_command(message: &str) -> String {
    let escaped = escaped_kakoune_double_quoted_string(message.trim());
    format!(
        "echo -markup \"{{Error}}Error parsing config{{Default}}: {}\"",
        escaped
    )
}

fn send_kakoune_command(tx: &Sender<String>, command: &str) {
    send_keys(tx, &[String::from(":")]);
    send_paste(tx, command);
    send_keys(tx, &[String::from("<ret>")]);
}

fn show_user_config_error(clients: &HashMap<WindowId, ClientWindow>, message: &str) {
    let command = config_error_command(message);
    for client in clients.values() {
        send_kakoune_command(&client.command_tx, &command);
    }
}

fn apply_reloaded_config(clients: &mut HashMap<WindowId, ClientWindow>, config: &AppConfig) {
    for client in clients.values_mut() {
        client.renderer.apply_config(config);
        #[cfg(target_os = "macos")]
        if let Err(error) = macos::apply_window_color_space(client.window.as_ref(), &config.macos) {
            log_error(format!("macOS color space setup failed: {error:#}"));
        }
        set_native_window_title(&client.window, config, &client.state.window_title);
        client.send_resize(config);
        client.request_redraw();
    }
}

fn poll_user_config_updates(
    watch: &mut UserConfigWatch,
    config: &mut AppConfig,
    user_keys: &mut UserKeys,
    clients: &mut HashMap<WindowId, ClientWindow>,
) {
    let current_state = read_user_config_watch_state(&watch.path);
    if current_state == watch.state {
        return;
    }

    match &current_state {
        UserConfigWatchState::Error(message) => {
            show_user_config_error(clients, message);
            watch.state = current_state;
        }
        UserConfigWatchState::Missing | UserConfigWatchState::Present(_) => match load_config() {
            Ok(next_config) => match UserKeys::from_config(&next_config.keys) {
                Ok(next_user_keys) => {
                    *config = next_config;
                    *user_keys = next_user_keys;
                    apply_reloaded_config(clients, config);
                    watch.state = current_state;
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    show_user_config_error(clients, &message);
                    watch.state = UserConfigWatchState::Error(message);
                }
            },
            Err(error) => {
                let message = format!("{error:#}");
                show_user_config_error(clients, &message);
                watch.state = UserConfigWatchState::Error(message);
            }
        },
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

fn create_startup_client_window(
    elwt: &ActiveEventLoop,
    startup_args: &Args,
    proxy: EventLoopProxy<AppEvent>,
    config: &AppConfig,
    window_icon: Option<Icon>,
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

fn startup_args(base_args: &Args, paths: &[PathBuf]) -> Args {
    if paths.is_empty() {
        return base_args.clone();
    }

    Args {
        show_config: base_args.show_config,
        kak_bin: base_args.kak_bin.clone(),
        kak_args: paths
            .iter()
            .map(|path| path.as_os_str().to_os_string())
            .collect(),
    }
}

fn startup_open_state_for_launch(is_macos: bool, kak_args: &[OsString]) -> StartupOpenState {
    if is_macos && kak_args.is_empty() {
        StartupOpenState::WaitingForStartupOpenFiles
    } else {
        StartupOpenState::NoPendingStartupOpen
    }
}

fn should_handle_startup_open_with_files(
    state: StartupOpenState,
    has_clients: bool,
    has_session: bool,
) -> bool {
    state == StartupOpenState::WaitingForStartupOpenFiles && !has_clients && !has_session
}

fn should_create_fallback_startup_client(
    state: StartupOpenState,
    has_clients: bool,
    has_session: bool,
) -> bool {
    state != StartupOpenState::StartupOpenHandled && !has_clients && !has_session
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
            std::time::Instant::now() + USER_CONFIG_POLL_INTERVAL,
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
                    install_kakvide_hooks_once(client, notification.as_ref());
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

fn resolve_kakoune_session(kak_args: &[OsString], child_id: u32) -> OsString {
    explicit_kakoune_session(kak_args).unwrap_or_else(|| OsString::from(child_id.to_string()))
}

fn startup_open_files(kak_args: &[OsString]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut args = kak_args.iter();

    while let Some(arg) = args.next() {
        if arg == "--" {
            files.extend(args.map(PathBuf::from));
            break;
        }

        if matches!(arg.to_str(), Some("-c" | "-C" | "-s" | "-e")) {
            let _ = args.next();
            continue;
        }

        if arg.to_string_lossy().starts_with('-') {
            continue;
        }

        files.push(PathBuf::from(arg));
    }

    files
}

fn should_ignore_startup_open_files(
    pending_startup_open_files: &mut Option<Vec<PathBuf>>,
    paths: &[PathBuf],
) -> bool {
    if pending_startup_open_files.as_deref() == Some(paths) {
        *pending_startup_open_files = None;
        true
    } else {
        false
    }
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
        show_config: false,
        kak_bin: kak_bin.to_string(),
        kak_args,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    use super::{
        KakvideHookInstallState, StartupOpenState, UserConfigWatchState,
        client_name_from_ui_options, config_error_command, connected_kakoune_args,
        default_launch_directory, early_exit_output, extract_kak_bin, kakvide_hook_prompt_keys,
        read_user_config_watch_state, resolve_kakoune_session,
        should_create_fallback_startup_client, should_handle_startup_open_with_files,
        should_ignore_startup_open_files, should_show_combined_help,
        should_update_native_window_title, startup_open_files, startup_open_state_for_launch,
    };
    use crate::app::{AppConfig, Args};

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
    fn kakvide_hook_prompt_keys_open_command_prompt() {
        assert_eq!(kakvide_hook_prompt_keys(), vec![":".to_string()]);
    }

    #[test]
    fn kakvide_hooks_wait_for_command_prompt_before_installing() {
        assert_eq!(
            KakvideHookInstallState::WaitingForCommandPrompt,
            KakvideHookInstallState::WaitingForCommandPrompt
        );
        assert_ne!(
            KakvideHookInstallState::WaitingForCommandPrompt,
            KakvideHookInstallState::Installed
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

    #[test]
    fn config_error_command_uses_error_markup() {
        assert_eq!(
            config_error_command("failed to parse \"font-size\"\nextra"),
            "echo -markup \"{Error}Error parsing config{Default}: failed to parse \\\"font-size\\\" extra\""
        );
    }

    #[test]
    fn watch_state_reports_missing_file() {
        let path = std::env::temp_dir().join(format!(
            "kakvide-missing-config-{}-missing.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        assert_eq!(
            read_user_config_watch_state(&path),
            UserConfigWatchState::Missing
        );
    }

    #[test]
    fn watch_state_reports_present_file() {
        let path = std::env::temp_dir().join(format!(
            "kakvide-present-config-{}.toml",
            std::process::id()
        ));
        fs::write(&path, "font-size = 14.0\n").expect("temp config should be written");

        let state = read_user_config_watch_state(&path);
        assert!(matches!(state, UserConfigWatchState::Present(_)));

        let _ = fs::remove_file(&path);
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
    fn startup_open_files_skips_known_option_values() {
        assert_eq!(
            startup_open_files(&[
                OsString::from("-e"),
                OsString::from("echo hi"),
                OsString::from("-s"),
                OsString::from("work"),
                OsString::from("file.txt"),
                OsString::from("--"),
                OsString::from("-literal"),
            ]),
            vec![PathBuf::from("file.txt"), PathBuf::from("-literal")]
        );
    }

    #[test]
    fn startup_open_files_are_ignored_only_once() {
        let mut pending = Some(vec![PathBuf::from("file.txt")]);

        assert!(should_ignore_startup_open_files(
            &mut pending,
            &[PathBuf::from("file.txt")]
        ));
        assert!(!should_ignore_startup_open_files(
            &mut pending,
            &[PathBuf::from("file.txt")]
        ));
    }

    #[test]
    fn startup_open_state_waits_for_open_files_on_macos_without_args() {
        assert_eq!(
            startup_open_state_for_launch(true, &[]),
            StartupOpenState::WaitingForStartupOpenFiles
        );
        assert_eq!(
            startup_open_state_for_launch(true, &[OsString::from("file.txt")]),
            StartupOpenState::NoPendingStartupOpen
        );
    }

    #[test]
    fn startup_open_is_handled_only_while_waiting_and_uninitialized() {
        assert!(should_handle_startup_open_with_files(
            StartupOpenState::WaitingForStartupOpenFiles,
            false,
            false
        ));
        assert!(!should_handle_startup_open_with_files(
            StartupOpenState::NoPendingStartupOpen,
            false,
            false
        ));
        assert!(!should_handle_startup_open_with_files(
            StartupOpenState::WaitingForStartupOpenFiles,
            true,
            false
        ));
    }

    #[test]
    fn fallback_startup_client_is_skipped_after_startup_open_is_handled() {
        assert!(!should_create_fallback_startup_client(
            StartupOpenState::StartupOpenHandled,
            false,
            false
        ));
        assert!(should_create_fallback_startup_client(
            StartupOpenState::NoPendingStartupOpen,
            false,
            false
        ));
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
