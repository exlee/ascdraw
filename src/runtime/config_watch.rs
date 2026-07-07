use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use winit::window::WindowId;

use crate::app::{AppConfig, load_config};
use crate::diagnostics::log_error;
#[cfg(target_os = "macos")]
use crate::macos;
use crate::runtime::client::ClientWindow;
use crate::title_policy::set_native_window_title;
use crate::user_keys::UserKeys;
use crate::{input::send_keys, input::send_paste};

pub const USER_CONFIG_POLL_INTERVAL: Duration = Duration::from_millis(500);
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserConfigWatchState {
    Missing,
    Present(SystemTime),
    Error(String),
}

pub struct UserConfigWatch {
    pub path: PathBuf,
    pub state: UserConfigWatchState,
}

impl UserConfigWatch {
    pub fn new(path: PathBuf) -> Self {
        let state = read_user_config_watch_state(&path);
        Self { path, state }
    }
}

pub fn read_user_config_watch_state(path: &Path) -> UserConfigWatchState {
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

pub fn config_error_command(message: &str) -> String {
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

pub fn event_loop_wait_duration(
    clients: &HashMap<WindowId, ClientWindow>,
    config: &AppConfig,
) -> Duration {
    let _ = (clients, config);
    USER_CONFIG_POLL_INTERVAL
}

pub fn poll_user_config_updates(
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
