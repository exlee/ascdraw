use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use toml::Value;

use crate::kakoune_messages::{
    Atom, Coord, Face, InfoStyle, KakouneNotification, MenuStyle, StatusStyle,
};
use crate::title_policy::decode_title_update;
use crate::user_keys::UserKeysConfig;
use winit::window::WindowId;

pub const DEFAULT_WINDOW_TITLE: &str = "kakvide";
pub const WINDOW_TITLE_UI_OPTION: &str = "kakvide_title";
pub const CURSOR_MODE_UI_OPTION: &str = "kakvide_cursor_mode";

#[derive(Parser, Debug, Clone)]
#[command(trailing_var_arg = true)]
pub struct Args {
    #[arg(long)]
    pub show_config: bool,
    #[arg(long, default_value = "kak")]
    pub kak_bin: String,
    #[arg(value_name = "KAK_ARG")]
    pub kak_args: Vec<OsString>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct AppConfig {
    pub font_family: String,
    pub font_size: f32,
    pub mouse_scroll_rate: f32,
    pub transparent_menubar: bool,
    pub cell: CellConfig,
    pub display: DisplayConfig,
    pub macos: MacosConfig,
    pub keys: UserKeysConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        bundled_default_config()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "kebab-case")]
pub struct CellConfig {
    pub underline_offset: f32,
}

impl Default for CellConfig {
    fn default() -> Self {
        bundled_default_cell_config()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "kebab-case")]
pub struct DisplayConfig {
    pub cursor_shape: CursorShapeConfig,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        bundled_default_display_config()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "kebab-case")]
pub struct CursorShapeConfig {
    pub normal: Option<CursorShape>,
    pub insert: Option<CursorShape>,
    pub replace: Option<CursorShape>,
}

impl Default for CursorShapeConfig {
    fn default() -> Self {
        bundled_default_cursor_shape_config()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CursorShape {
    Block,
    Beam,
    Underline,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorMode {
    Normal,
    Insert,
    Replace,
    #[default]
    Unknown,
}

impl CursorMode {
    fn from_ui_option(value: &str) -> Self {
        match value.trim() {
            "normal" => Self::Normal,
            "insert" => Self::Insert,
            "replace" => Self::Replace,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "kebab-case")]
pub struct MacosConfig {
    pub color_space: MacosColorSpace,
}

impl Default for MacosConfig {
    fn default() -> Self {
        bundled_default_macos_config()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MacosColorSpace {
    P3,
    Srgb,
}

impl Default for MacosColorSpace {
    fn default() -> Self {
        Self::P3
    }
}

#[derive(Debug)]
pub enum AppEvent {
    Rpc(WindowId, Box<KakouneNotification>),
    KakouneExited(WindowId),
    ClientClosed {
        session: OsString,
        client_id: String,
    },
    OpenFiles(Vec<PathBuf>),
    Command(AppCommand),
}

#[derive(Debug)]
pub enum AppCommand {
    FontScaleUp,
    FontScaleDown,
    FontScaleReset,
    WindowNew,
    WindowClose,
    ConnectToSession(OsString),
    SwitchToSession(OsString),
}

#[derive(Debug, Clone)]
pub struct GridState {
    pub lines: Vec<Vec<Atom>>,
    pub cursor_pos: Coord,
    pub default_face: Face,
    #[allow(dead_code)]
    pub padding_face: Face,
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub cursor_pos: isize,
    pub mode_line: Vec<Atom>,
    pub default_face: Face,
    #[allow(dead_code)]
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

#[derive(Debug, Clone)]
pub struct AppState {
    pub grid: GridState,
    pub status: Option<StatusState>,
    pub menu: Option<MenuState>,
    pub info: Option<InfoState>,
    pub window_title: String,
    pub cursor_mode: CursorMode,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            grid: GridState::default(),
            status: None,
            menu: None,
            info: None,
            window_title: DEFAULT_WINDOW_TITLE.to_string(),
            cursor_mode: CursorMode::default(),
        }
    }
}

pub fn load_config() -> Result<AppConfig> {
    load_config_with_env(|name| std::env::var_os(name))
}

pub fn checked_config_paths() -> Vec<PathBuf> {
    checked_config_paths_with_env(|name| std::env::var_os(name))
}

pub fn user_config_path() -> Option<PathBuf> {
    user_config_path_with_env(|name| std::env::var_os(name))
}

pub fn show_config_toml(config: &AppConfig) -> Result<String> {
    toml::to_string_pretty(config).context("failed to serialize effective config")
}

fn load_config_with_env(env_var: impl Fn(&str) -> Option<OsString>) -> Result<AppConfig> {
    let mut value = bundled_default_value();
    if let Some(path) = user_config_path_with_env(&env_var) {
        match fs::read_to_string(&path) {
            Ok(contents) => {
                let user_value = toml::from_str::<Value>(&contents)
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                merge_toml_value(&mut value, user_value);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        }
    }

    value.try_into().context("failed to parse effective config")
}

fn user_config_path_with_env(env_var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    checked_config_paths_with_env(env_var).into_iter().nth(1)
}

fn checked_config_paths_with_env(env_var: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let mut paths = vec![bundled_config_path()];
    if let Some(xdg_config_home) = env_var("XDG_CONFIG_HOME")
        && !xdg_config_home.is_empty()
    {
        paths.push(
            PathBuf::from(xdg_config_home)
                .join("kakvide")
                .join("config.toml"),
        );
        return paths;
    }

    if let Some(home) = env_var("HOME").filter(|home| !home.is_empty()) {
        paths.push(
            PathBuf::from(home)
                .join(".config")
                .join("kakvide")
                .join("config.toml"),
        );
    }

    paths
}

fn bundled_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("kakvide.toml")
}

fn merge_toml_value(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Table(base_table), Value::Table(overlay_table)) => {
            for (key, overlay_value) in overlay_table {
                match base_table.get_mut(&key) {
                    Some(base_value) => merge_toml_value(base_value, overlay_value),
                    None => {
                        base_table.insert(key, overlay_value);
                    }
                }
            }
        }
        (base_value, overlay_value) => *base_value = overlay_value,
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
        cell: bundled_default_cell_config(),
        display: bundled_default_display_config(),
        macos: bundled_default_macos_config(),
        keys: bundled_default_keys(),
    }
}

pub fn bundled_default_cell_config() -> CellConfig {
    let value = bundled_default_value();
    let cell = value
        .get("cell")
        .and_then(Value::as_table)
        .expect("bundled kakvide.toml should contain a [cell] section");

    CellConfig {
        underline_offset: cell
            .get("underline-offset")
            .and_then(Value::as_float)
            .expect("bundled [cell] should set underline-offset") as f32,
    }
}

pub fn bundled_default_display_config() -> DisplayConfig {
    DisplayConfig {
        cursor_shape: bundled_default_cursor_shape_config(),
    }
}

pub fn bundled_default_cursor_shape_config() -> CursorShapeConfig {
    let value = bundled_default_value();
    let cursor_shape = value
        .get("display")
        .and_then(Value::as_table)
        .and_then(|display| display.get("cursor-shape"))
        .and_then(Value::as_table)
        .expect("bundled kakvide.toml should contain a [display.cursor-shape] section");

    CursorShapeConfig {
        normal: cursor_shape
            .get("normal")
            .and_then(Value::as_str)
            .map(|value| match value {
                "block" => CursorShape::Block,
                "beam" => CursorShape::Beam,
                "underline" => CursorShape::Underline,
                other => panic!("unsupported bundled [display.cursor-shape] value {other}"),
            }),
        insert: cursor_shape
            .get("insert")
            .and_then(Value::as_str)
            .map(|value| match value {
                "block" => CursorShape::Block,
                "beam" => CursorShape::Beam,
                "underline" => CursorShape::Underline,
                other => panic!("unsupported bundled [display.cursor-shape] value {other}"),
            }),
        replace: cursor_shape
            .get("replace")
            .and_then(Value::as_str)
            .map(|value| match value {
                "block" => CursorShape::Block,
                "beam" => CursorShape::Beam,
                "underline" => CursorShape::Underline,
                other => panic!("unsupported bundled [display.cursor-shape] value {other}"),
            }),
    }
}

pub fn bundled_default_macos_config() -> MacosConfig {
    let value = bundled_default_value();
    let macos = value
        .get("macos")
        .and_then(Value::as_table)
        .expect("bundled kakvide.toml should contain a [macos] section");

    MacosConfig {
        color_space: match macos
            .get("color-space")
            .and_then(Value::as_str)
            .expect("bundled [macos] should set color-space")
        {
            "p3" => MacosColorSpace::P3,
            "srgb" => MacosColorSpace::Srgb,
            other => panic!("unsupported bundled [macos].color-space value {other}"),
        },
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
        window_new: keys
            .get("window-new")
            .and_then(Value::as_str)
            .expect("bundled [keys] should set window-new")
            .to_string(),
        window_close: keys
            .get("window-close")
            .and_then(Value::as_str)
            .expect("bundled [keys] should set window-close")
            .to_string(),
    }
}

fn bundled_default_value() -> Value {
    static BUNDLED_DEFAULT_VALUE: OnceLock<Value> = OnceLock::new();
    BUNDLED_DEFAULT_VALUE
        .get_or_init(|| {
            toml::from_str(include_str!("../kakvide.toml"))
                .expect("bundled kakvide.toml should parse")
        })
        .clone()
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
            if let Some(title_update) = decode_title_update(&options) {
                state.window_title = title_update.window_title;
            }
            if let Some(cursor_mode) = cursor_mode_from_ui_options(&options) {
                state.cursor_mode = cursor_mode;
            }
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

fn cursor_mode_from_ui_options(
    options: &serde_json::Map<String, serde_json::Value>,
) -> Option<CursorMode> {
    options
        .get(CURSOR_MODE_UI_OPTION)
        .and_then(serde_json::Value::as_str)
        .map(CursorMode::from_ui_option)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    use clap::Parser;

    #[test]
    fn parses_without_forwarded_args() {
        let args = Args::try_parse_from(["kakvide"]).expect("args should parse");

        assert!(!args.show_config);
        assert_eq!(args.kak_bin, "kak");
        assert!(args.kak_args.is_empty());
    }

    #[test]
    fn parses_show_config_flag() {
        let args = Args::try_parse_from(["kakvide", "--show-config"]).expect("args should parse");

        assert!(args.show_config);
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
            "--show-config",
            "--kak-bin",
            "/tmp/kak",
            "--",
            "-d",
            "-e",
            "echo hi",
            "file.txt",
        ])
        .expect("args should parse");

        assert!(args.show_config);
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
        assert_eq!(config.font_size, 12.0);
        assert_eq!(config.mouse_scroll_rate, 0.25);
        assert!(config.transparent_menubar);
        assert_eq!(config.cell, CellConfig::default());
        assert_eq!(config.cell.underline_offset, 0.0);
        assert_eq!(config.display, DisplayConfig::default());
        assert_eq!(config.display.cursor_shape.normal, None);
        assert_eq!(config.display.cursor_shape.insert, None);
        assert_eq!(config.display.cursor_shape.replace, None);
        assert_eq!(config.macos.color_space, MacosColorSpace::P3);
        assert_eq!(config.keys, UserKeysConfig::default());
    }

    #[test]
    fn config_path_uses_xdg_config_home_when_set() {
        let paths = checked_config_paths_with_env(|name| match name {
            "XDG_CONFIG_HOME" => Some(OsString::from("/tmp/xdg")),
            "HOME" => Some(OsString::from("/Users/example")),
            _ => None,
        });

        assert_eq!(
            paths,
            vec![
                bundled_config_path(),
                PathBuf::from("/tmp/xdg/kakvide/config.toml")
            ]
        );
    }

    #[test]
    fn config_path_falls_back_to_home_when_xdg_unset() {
        let paths = checked_config_paths_with_env(|name| match name {
            "HOME" => Some(OsString::from("/Users/example")),
            _ => None,
        });

        assert_eq!(
            paths,
            vec![
                bundled_config_path(),
                PathBuf::from("/Users/example/.config/kakvide/config.toml")
            ]
        );
    }

    #[test]
    fn config_path_falls_back_to_home_when_xdg_empty() {
        let paths = checked_config_paths_with_env(|name| match name {
            "XDG_CONFIG_HOME" => Some(OsString::from("")),
            "HOME" => Some(OsString::from("/Users/example")),
            _ => None,
        });

        assert_eq!(
            paths,
            vec![
                bundled_config_path(),
                PathBuf::from("/Users/example/.config/kakvide/config.toml")
            ]
        );
    }

    #[test]
    fn config_path_is_missing_without_xdg_or_home() {
        assert_eq!(
            checked_config_paths_with_env(|_| None),
            vec![bundled_config_path()]
        );
    }

    #[test]
    fn load_config_uses_defaults_without_user_file() {
        let config = load_config_with_env(|_| None).expect("config should load");

        assert_eq!(config.font_family, AppConfig::default().font_family);
        assert_eq!(config.font_size, AppConfig::default().font_size);
        assert_eq!(
            config.mouse_scroll_rate,
            AppConfig::default().mouse_scroll_rate
        );
        assert_eq!(
            config.transparent_menubar,
            AppConfig::default().transparent_menubar
        );
        assert_eq!(config.cell, AppConfig::default().cell);
        assert_eq!(config.display, AppConfig::default().display);
        assert_eq!(config.macos, AppConfig::default().macos);
        assert_eq!(config.keys, AppConfig::default().keys);
    }

    #[test]
    fn config_merges_user_overrides() {
        let mut value = bundled_default_value();
        let user_value = toml::from_str::<Value>(
            r#"
font-size = 18.0
transparent-menubar = false

[cell]
underline-offset = 1.5

[display.cursor-shape]
insert = "beam"

[macos]
color-space = "srgb"

[keys]
window-new = "Cmd-Shift-N"
"#,
        )
        .expect("user config should parse");

        merge_toml_value(&mut value, user_value);
        let config: AppConfig = value.try_into().expect("merged config should parse");

        assert_eq!(config.font_family, "SF Mono");
        assert_eq!(config.font_size, 18.0);
        assert!(!config.transparent_menubar);
        assert_eq!(config.cell.underline_offset, 1.5);
        assert_eq!(config.display.cursor_shape.normal, None);
        assert_eq!(config.display.cursor_shape.insert, Some(CursorShape::Beam));
        assert_eq!(config.display.cursor_shape.replace, None);
        assert_eq!(config.macos.color_space, MacosColorSpace::Srgb);
        assert_eq!(config.keys.window_new, "Cmd-Shift-N");
        assert_eq!(config.keys.window_close, "Cmd-W");
    }

    #[test]
    fn show_config_prints_all_effective_settings() {
        let config = AppConfig {
            font_size: 18.0,
            transparent_menubar: false,
            macos: MacosConfig {
                color_space: MacosColorSpace::Srgb,
            },
            ..AppConfig::default()
        };
        let output = show_config_toml(&config).expect("config should serialize");

        assert!(output.contains("font-family = \"SF Mono\""));
        assert!(output.contains("font-size = 18.0"));
        assert!(output.contains("transparent-menubar = false"));
        assert!(output.contains("[cell]"));
        assert!(output.contains("[macos]"));
        assert!(output.contains("color-space = \"srgb\""));
        assert!(output.contains("[display.cursor-shape]"));
        assert!(output.contains("[keys]"));
        assert!(output.contains("window-close = \"Cmd-W\""));
    }

    #[test]
    fn user_config_path_skips_bundled_entry() {
        let path = user_config_path_with_env(|name| match name {
            "HOME" => Some(OsString::from("/Users/example")),
            _ => None,
        });

        assert_eq!(
            path,
            Some(PathBuf::from("/Users/example/.config/kakvide/config.toml"))
        );
    }

    #[test]
    fn config_parses_fractional_cell_underline_offset() {
        let config: AppConfig = toml::from_str(
            r#"
font-family = "SF Mono"
font-size = 12.0
mouse-scroll-rate = 0.25
transparent-menubar = true

[cell]
underline-offset = 1.5

[macos]
color-space = "p3"
"#,
        )
        .expect("config should parse");

        assert_eq!(config.cell.underline_offset, 1.5);
    }

    #[test]
    fn config_parses_cursor_shape_overrides() {
        let config: AppConfig = toml::from_str(
            r#"
font-family = "SF Mono"
font-size = 12.0
mouse-scroll-rate = 0.25
transparent-menubar = true

[display.cursor-shape]
normal = "beam"
insert = "underline"
replace = "block"

[macos]
color-space = "p3"
"#,
        )
        .expect("config should parse");

        assert_eq!(config.display.cursor_shape.normal, Some(CursorShape::Beam));
        assert_eq!(
            config.display.cursor_shape.insert,
            Some(CursorShape::Underline)
        );
        assert_eq!(
            config.display.cursor_shape.replace,
            Some(CursorShape::Block)
        );
    }

    #[test]
    fn config_rejects_unknown_cursor_shape() {
        let error = toml::from_str::<AppConfig>(
            r#"
font-family = "SF Mono"
font-size = 12.0
mouse-scroll-rate = 0.25
transparent-menubar = true

[display.cursor-shape]
insert = "triangle"
"#,
        )
        .expect_err("config should reject unknown cursor shape")
        .to_string();

        assert!(error.contains("unknown variant"));
        assert!(error.contains("triangle"));
    }

    #[test]
    fn config_parses_explicit_macos_srgb_color_space() {
        let config: AppConfig = toml::from_str(
            r#"
font-family = "SF Mono"
font-size = 12.0
mouse-scroll-rate = 0.25
transparent-menubar = true

[macos]
color-space = "srgb"
"#,
        )
        .expect("config should parse");

        assert_eq!(config.macos.color_space, MacosColorSpace::Srgb);
    }

    #[test]
    fn config_rejects_unknown_macos_color_space() {
        let error = toml::from_str::<AppConfig>(
            r#"
font-family = "SF Mono"
font-size = 12.0
mouse-scroll-rate = 0.25
transparent-menubar = true

[macos]
color-space = "adobe-rgb"
"#,
        )
        .expect_err("config should reject unknown color space")
        .to_string();

        assert!(error.contains("unknown variant"));
        assert!(error.contains("adobe-rgb"));
    }

    #[test]
    fn set_ui_options_preserves_window_title_for_missing_title() {
        let mut state = AppState {
            window_title: "kakvide - /tmp/project - Client 0".to_string(),
            ..AppState::default()
        };

        apply_notification(
            &mut state,
            KakouneNotification::SetUiOptions {
                options: serde_json::Map::new(),
            },
        );

        assert_eq!(state.window_title, "kakvide - /tmp/project - Client 0");
    }

    #[test]
    fn set_ui_options_uses_default_window_title_for_empty_title() {
        let mut state = AppState {
            window_title: "kakvide - /tmp/project - Client 0".to_string(),
            ..AppState::default()
        };
        let mut options = serde_json::Map::new();
        options.insert(
            WINDOW_TITLE_UI_OPTION.to_string(),
            serde_json::Value::String("  ".to_string()),
        );

        apply_notification(&mut state, KakouneNotification::SetUiOptions { options });

        assert_eq!(state.window_title, DEFAULT_WINDOW_TITLE);
    }

    #[test]
    fn set_ui_options_updates_cursor_mode() {
        let mut state = AppState::default();
        let mut options = serde_json::Map::new();
        options.insert(
            CURSOR_MODE_UI_OPTION.to_string(),
            serde_json::Value::String("insert".to_string()),
        );

        apply_notification(&mut state, KakouneNotification::SetUiOptions { options });

        assert_eq!(state.cursor_mode, CursorMode::Insert);
    }

    #[test]
    fn set_ui_options_falls_back_to_unknown_for_unrecognized_cursor_mode() {
        let mut state = AppState {
            cursor_mode: CursorMode::Insert,
            ..AppState::default()
        };
        let mut options = serde_json::Map::new();
        options.insert(
            CURSOR_MODE_UI_OPTION.to_string(),
            serde_json::Value::String("prompt".to_string()),
        );

        apply_notification(&mut state, KakouneNotification::SetUiOptions { options });

        assert_eq!(state.cursor_mode, CursorMode::Unknown);
    }
}
