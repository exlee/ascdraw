use std::fs;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use crate::kakoune_messages::{
    Atom, Coord, Face, InfoStyle, KakouneNotification, MenuStyle, StatusStyle,
};

#[derive(Parser, Debug)]
pub struct Args {
    pub file: Option<String>,
    #[arg(long, default_value = "kak")]
    pub kak_bin: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppConfig {
    pub font_family: String,
    pub font_size: f32,
    pub transparent_menubar: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            font_family: "SF Mono".to_string(),
            font_size: 15.0,
            transparent_menubar: true,
        }
    }
}

#[derive(Debug)]
pub enum AppEvent {
    Rpc(KakouneNotification),
    KakouneExited,
}

#[derive(Debug, Clone)]
pub struct GridState {
    pub lines: Vec<Vec<Atom>>,
    pub cursor_pos: Coord,
    pub default_face: Face,
    pub padding_face: Face,
    pub widget_columns: usize,
}

impl Default for GridState {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            cursor_pos: Coord { line: 0, column: 0 },
            default_face: Face::default(),
            padding_face: Face::default(),
            widget_columns: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatusState {
    pub prompt: Vec<Atom>,
    pub content: Vec<Atom>,
    pub cursor_pos: isize,
    pub mode_line: Vec<Atom>,
    pub default_face: Face,
    pub style: StatusStyle,
}

impl Default for StatusState {
    fn default() -> Self {
        Self {
            prompt: Vec::new(),
            content: Vec::new(),
            cursor_pos: 0,
            mode_line: Vec::new(),
            default_face: Face::default(),
            style: StatusStyle::Status,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MenuState {
    pub items: Vec<Vec<Atom>>,
    pub anchor: Coord,
    pub selected: Option<usize>,
    pub selected_face: Face,
    pub menu_face: Face,
    pub style: MenuStyle,
}

#[derive(Debug, Clone)]
pub struct InfoState {
    pub title: Vec<Atom>,
    pub content: Vec<Vec<Atom>>,
    pub anchor: Coord,
    pub face: Face,
    pub style: InfoStyle,
}

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub grid: GridState,
    pub status: Option<StatusState>,
    pub menu: Option<MenuState>,
    pub info: Option<InfoState>,
}

pub fn load_config() -> Result<AppConfig> {
    let path = "kakvide.toml";
    match fs::read_to_string(path) {
        Ok(contents) => {
            toml::from_str(&contents).with_context(|| format!("failed to parse {path}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(error) => Err(error).with_context(|| format!("failed to read {path}")),
    }
}

pub fn apply_notification(state: &mut AppState, notification: KakouneNotification) {
    match notification {
        KakouneNotification::Draw {
            lines,
            cursor_pos,
            default_face,
            padding_face,
            widget_columns,
        } => {
            state.grid = GridState {
                lines,
                cursor_pos,
                default_face,
                padding_face,
                widget_columns,
            };
        }
        KakouneNotification::DrawStatus {
            prompt,
            content,
            cursor_pos,
            mode_line,
            default_face,
            style,
        } => {
            state.status = Some(StatusState {
                prompt,
                content,
                cursor_pos,
                mode_line,
                default_face,
                style,
            });
        }
        KakouneNotification::Refresh { force } => {
            let _ = force;
        }
        KakouneNotification::SetUiOptions { options } => {
            let _ = options;
        }
        KakouneNotification::MenuShow {
            items,
            anchor,
            selected_face,
            menu_face,
            style,
        } => {
            state.menu = Some(MenuState {
                items,
                anchor,
                selected: None,
                selected_face,
                menu_face,
                style,
            });
        }
        KakouneNotification::MenuSelect { selected } => {
            if let Some(menu) = state.menu.as_mut() {
                menu.selected = usize::try_from(selected).ok();
            }
        }
        KakouneNotification::MenuHide => {
            state.menu = None;
        }
        KakouneNotification::InfoShow {
            title,
            content,
            anchor,
            face,
            style,
        } => {
            state.info = Some(InfoState {
                title,
                content,
                anchor,
                face,
                style,
            });
        }
        KakouneNotification::InfoHide => {
            state.info = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_match_kakvide_toml_shape() {
        let config = AppConfig::default();
        assert_eq!(config.font_family, "SF Mono");
        assert_eq!(config.font_size, 15.0);
        assert!(config.transparent_menubar);
    }
}
