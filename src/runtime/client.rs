use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use anyhow::{Result, anyhow};
use softbuffer::{Context as SoftContext, Surface};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Icon, Window, WindowId};

use crate::app::{AppCommand, AppConfig, AppEvent, AppState, Args};
use crate::input::{MouseMotionState, ScrollState, send_resize};
use crate::kakoune_integration::post_boot_command;
use crate::kakoune_messages::Coord;
#[cfg(target_os = "macos")]
use crate::macos;
use crate::render::{Renderer, load_renderer, resize_surface};
use crate::title_policy::window_attributes;
use crate::user_keys::FontSizeAction;
use crate::{
    diagnostics::log_error, kakoune_process::spawn_kakoune, kakoune_process::spawn_stdin_writer,
};

pub struct ClientWindow {
    pub window: Rc<Window>,
    pub surface: Surface<Rc<Window>, Rc<Window>>,
    pub child: Child,
    pub session: OsString,
    pub client_id: Option<String>,
    pub command_tx: Sender<String>,
    pub modifiers: ModifiersState,
    pub mouse_cell: Coord,
    pub mouse_motion_state: MouseMotionState,
    pub scroll_state: ScrollState,
    pub did_force_startup_resize: bool,
    pub kakvide_hook_install_state: KakvideHookInstallState,
    pub kakvide_post_boot_command: String,
    pub state: AppState,
    pub renderer: Renderer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KakvideHookInstallState {
    NotStarted,
    WaitingForCommandPrompt,
    Installed,
}

impl ClientWindow {
    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn send_resize(&self, config: &AppConfig) {
        send_resize(&self.command_tx, &self.window, &self.renderer, config);
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn multi_cursor_indicator_active(&self, config: &AppConfig) -> bool {
        let _ = config;
        false
    }
}

pub fn create_client_window(
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
        kakvide_post_boot_command: post_boot_command(client_close_socket),
        state: AppState::default(),
        renderer,
    };
    client.send_resize(config);
    client.request_redraw();
    Ok(client)
}

pub fn create_active_client_window(
    elwt: &ActiveEventLoop,
    args: &Args,
    proxy: EventLoopProxy<AppEvent>,
    config: &AppConfig,
    window_icon: Option<Icon>,
    client_close_socket: Option<&Path>,
) -> Result<ClientWindow> {
    let window = Rc::new(elwt.create_window(window_attributes(config, window_icon))?);
    #[cfg(target_os = "macos")]
    {
        if let Err(error) = macos::apply_window_color_space(window.as_ref(), &config.macos) {
            log_error(format!("macOS color space setup failed: {error:#}"));
        }
        window.focus_window();
    }
    create_client_window(window, args, proxy, config, client_close_socket)
}

pub fn focused_window_id(clients: &HashMap<WindowId, ClientWindow>) -> Option<WindowId> {
    clients
        .iter()
        .find_map(|(window_id, client)| client.window.has_focus().then_some(*window_id))
        .or_else(|| (clients.len() == 1).then(|| *clients.keys().next().expect("one client")))
}

pub fn command_window_id(
    clients: &HashMap<WindowId, ClientWindow>,
    source_window_id: Option<WindowId>,
) -> Option<WindowId> {
    source_window_id
        .filter(|window_id| clients.contains_key(window_id))
        .or_else(|| focused_window_id(clients))
}

pub fn close_client(
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

pub fn remove_closed_client(
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

pub fn adjust_client_font_size(
    client: &mut ClientWindow,
    action: FontSizeAction,
    config: &AppConfig,
) {
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

pub struct RuntimeContext<'a> {
    pub elwt: &'a ActiveEventLoop,
    pub clients: &'a mut HashMap<WindowId, ClientWindow>,
    pub proxy: EventLoopProxy<AppEvent>,
    pub config: &'a AppConfig,
    pub window_icon: Option<Icon>,
    pub kak_bin: &'a str,
    pub kakoune_session: &'a OsStr,
    pub client_close_socket: Option<&'a Path>,
}

impl RuntimeContext<'_> {
    pub fn open_session_window(&mut self, session: &OsStr, paths: &[PathBuf], log_label: &str) {
        let open_args = crate::runtime::startup::connected_kakoune_args(self.kak_bin, session, paths);
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

    pub fn handle_command(&mut self, command: AppCommand, source_window_id: Option<WindowId>) {
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
