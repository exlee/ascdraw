use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use winit::window::WindowId;

use crate::app::{AppConfig, load_config};
use crate::diagnostics::log_error;
use crate::runtime::window::EditorWindow;
use crate::user_keys::UserKeys;

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

pub fn poll_user_config_updates(
    watch: &mut UserConfigWatch,
    config: &mut AppConfig,
    user_keys: &mut UserKeys,
    windows: &mut HashMap<WindowId, EditorWindow>,
) {
    let current_state = read_user_config_watch_state(&watch.path);
    if current_state == watch.state {
        return;
    }

    match &current_state {
        UserConfigWatchState::Error(message) => log_error(message.clone()),
        UserConfigWatchState::Missing | UserConfigWatchState::Present(_) => match load_config() {
            Ok(next_config) => match UserKeys::from_config(&next_config.keys) {
                Ok(next_user_keys) => {
                    *config = next_config;
                    *user_keys = next_user_keys;
                    for editor in windows.values_mut() {
                        editor.apply_config(config);
                    }
                }
                Err(error) => log_error(format!("invalid key configuration: {error:#}")),
            },
            Err(error) => log_error(format!("configuration reload failed: {error:#}")),
        },
    }
    watch.state = current_state;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_state_reports_missing_file() {
        let path = std::env::temp_dir().join(format!(
            "ascdraw-missing-config-{}-missing.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        assert_eq!(
            read_user_config_watch_state(&path),
            UserConfigWatchState::Missing
        );
    }
}
