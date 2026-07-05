use std::ffi::OsString;
use std::fs;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use toml::Value;

use crate::kakoune_messages::{
    Atom, Coord, Face, InfoStyle, KakouneNotification, MenuStyle, StatusStyle,
};
use crate::user_keys::UserKeysConfig;

#[derive(Parser, Debug)]
#[command(trailing_var_arg = true)]
pub struct Args {
    #[arg(long, default_value = "kak")]
    pub kak_bin: String,
    #[arg(value_name = "KAK_ARG")]
    pub kak_args: Vec<OsString>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppConfig {
    pub font_family: String,
    pub font_size: f32,
    pub mouse_scroll_rate: f32,
    pub transparent_menubar: bool,
    pub keys: UserKeysConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        bundled_default_config()
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

fn bundled_default_config() -> AppConfig {
    let value = bundled_default_value();
    AppConfig {
        font_family: value
            .get("font-family")
            .and_then(Value::as_str)
            .expect("bundled kakvide.toml should set font-family")
            .to_string(),
        font_size: value
            .get("font-size")
            .and_then(Value::as_float)
            .expect("bundled kakvide.toml should set font-size") as f32,
        mouse_scroll_rate: value
            .get("mouse-scroll-rate")
            .and_then(Value::as_float)
            .expect("bundled kakvide.toml should set mouse-scroll-rate")
            as f32,
        transparent_menubar: value
            .get("transparent-menubar")
            .and_then(Value::as_bool)
            .expect("bundled kakvide.toml should set transparent-menubar"),
        keys: bundled_default_keys(),
    }
}

pub fn bundled_default_keys() -> UserKeysConfig {
    let value = bundled_default_value();
    let keys = value
        .get("keys")
        .and_then(Value::as_table)
        .expect("bundled kakvide.toml should contain a [keys] section");

    UserKeysConfig {
        font_scale_up: keys
            .get("font-scale-up")
            .and_then(Value::as_str)
            .expect("bundled [keys] should set font-scale-up")
            .to_string(),
        font_scale_down: keys
            .get("font-scale-down")
            .and_then(Value::as_str)
            .expect("bundled [keys] should set font-scale-down")
            .to_string(),
        font_scale_reset: keys
            .get("font-scale-reset")
            .and_then(Value::as_str)
            .expect("bundled [keys] should set font-scale-reset")
            .to_string(),
    }
}

fn bundled_default_value() -> Value {
    toml::from_str(include_str!("../kakvide.toml")).expect("bundled kakvide.toml should parse")
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
    use std::ffi::OsString;

    use clap::Parser;

    #[test]
    fn parses_without_forwarded_args() {
        let args = Args::try_parse_from(["kakvide"]).expect("args should parse");

        assert_eq!(args.kak_bin, "kak");
        assert!(args.kak_args.is_empty());
    }

    #[test]
    fn forwards_single_positional_arg() {
        let args = Args::try_parse_from(["kakvide", "file.txt"]).expect("args should parse");

        assert_eq!(args.kak_args, vec![OsString::from("file.txt")]);
    }

    #[test]
    fn forwards_multiple_positional_args() {
        let args =
            Args::try_parse_from(["kakvide", "file.txt", "other.txt"]).expect("args should parse");

        assert_eq!(
            args.kak_args,
            vec![OsString::from("file.txt"), OsString::from("other.txt")]
        );
    }

    #[test]
    fn parses_kakvide_flag_before_forwarded_positionals() {
        let args = Args::try_parse_from(["kakvide", "--kak-bin", "/tmp/kak", "file.txt"])
            .expect("args should parse");

        assert_eq!(args.kak_bin, "/tmp/kak");
        assert_eq!(args.kak_args, vec![OsString::from("file.txt")]);
    }

    #[test]
    fn forwards_option_args_after_double_dash() {
        let args = Args::try_parse_from([
            "kakvide",
            "--kak-bin",
            "/tmp/kak",
            "--",
            "-d",
            "-e",
            "echo hi",
            "file.txt",
        ])
        .expect("args should parse");

        assert_eq!(args.kak_bin, "/tmp/kak");
        assert_eq!(
            args.kak_args,
            vec![
                OsString::from("-d"),
                OsString::from("-e"),
                OsString::from("echo hi"),
                OsString::from("file.txt")
            ]
        );
    }

    #[test]
    fn config_defaults_match_kakvide_toml_shape() {
        let config = AppConfig::default();
        assert_eq!(config.font_family, "SF Mono");
        assert_eq!(config.font_size, 15.0);
        assert_eq!(config.mouse_scroll_rate, 0.25);
        assert!(config.transparent_menubar);
        assert_eq!(config.keys, UserKeysConfig::default());
    }
}
