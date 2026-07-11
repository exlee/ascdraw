use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use toml::Value;

use crate::model::Face;
use crate::user_keys::UserKeysConfig;

pub const DEFAULT_WINDOW_TITLE: &str = "ascdraw";

#[derive(Parser, Debug, Clone)]
pub struct Args {
    #[arg(long)]
    pub show_config: bool,
    pub document: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppConfig {
    pub font_family: String,
    pub font_size: f32,
    pub transparent_menubar: bool,
    pub cell: CellConfig,
    pub display: DisplayConfig,
    pub theme: ThemeConfig,
    pub macos: MacosConfig,
    pub keys: UserKeysConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        bundled_default_config()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct CellConfig {
    pub underline_offset: f32,
}

impl Default for CellConfig {
    fn default() -> Self {
        bundled_default_cell_config()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct DisplayConfig {
    pub cursor_shape: CursorShapeConfig,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        bundled_default_display_config()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct CursorShapeConfig {
    #[serde(alias = "normal")]
    pub move_draw: Option<CursorShape>,
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
#[allow(dead_code)]
pub enum CursorMode {
    #[default]
    MoveDraw,
    Text,
    Insert,
    Replace,
    Stamp,
    Shapes,
    Utilities,
}

impl CursorMode {
    pub fn accepts_text(self) -> bool {
        matches!(self, Self::Text | Self::Insert | Self::Replace)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct ThemeConfig {
    pub default: Face,
    pub cursor: Face,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        bundled_default_theme_config()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct MacosConfig {
    pub color_space: MacosColorSpace,
}

impl Default for MacosConfig {
    fn default() -> Self {
        bundled_default_macos_config()
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MacosColorSpace {
    #[default]
    P3,
    Srgb,
}

#[derive(Debug)]
pub enum AppEvent {
    Command(AppCommand),
}

#[derive(Debug, Clone, Copy)]
pub enum AppCommand {
    FontScaleUp,
    FontScaleDown,
    FontScaleReset,
    WindowNew,
    WindowClose,
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
                .join("ascdraw")
                .join("config.toml"),
        );
        return paths;
    }

    if let Some(home) = env_var("HOME").filter(|home| !home.is_empty()) {
        paths.push(
            PathBuf::from(home)
                .join(".config")
                .join("ascdraw")
                .join("config.toml"),
        );
    }

    paths
}

fn bundled_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ascdraw.toml")
}

fn bundled_default_value() -> Value {
    static VALUE: OnceLock<Value> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            toml::from_str(include_str!("../ascdraw.toml"))
                .expect("bundled ascdraw.toml should parse")
        })
        .clone()
}

fn bundled_default_config() -> AppConfig {
    bundled_default_value()
        .try_into()
        .expect("bundled ascdraw.toml should match AppConfig")
}

fn bundled_default_cell_config() -> CellConfig {
    bundled_default_value()
        .get("cell")
        .cloned()
        .expect("bundled ascdraw.toml should contain [cell]")
        .try_into()
        .expect("bundled [cell] should match CellConfig")
}

fn bundled_default_display_config() -> DisplayConfig {
    bundled_default_value()
        .get("display")
        .cloned()
        .expect("bundled ascdraw.toml should contain [display]")
        .try_into()
        .expect("bundled [display] should match DisplayConfig")
}

fn bundled_default_cursor_shape_config() -> CursorShapeConfig {
    bundled_default_value()
        .get("display")
        .and_then(|value| value.get("cursor-shape"))
        .cloned()
        .expect("bundled ascdraw.toml should contain [display.cursor-shape]")
        .try_into()
        .expect("bundled cursor shape should match CursorShapeConfig")
}

fn bundled_default_theme_config() -> ThemeConfig {
    bundled_default_value()
        .get("theme")
        .cloned()
        .expect("bundled ascdraw.toml should contain [theme]")
        .try_into()
        .expect("bundled [theme] should match ThemeConfig")
}

fn bundled_default_macos_config() -> MacosConfig {
    bundled_default_value()
        .get("macos")
        .cloned()
        .expect("bundled ascdraw.toml should contain [macos]")
        .try_into()
        .expect("bundled [macos] should match MacosConfig")
}

pub fn bundled_default_keys() -> UserKeysConfig {
    bundled_default_value()
        .get("keys")
        .cloned()
        .expect("bundled ascdraw.toml should contain [keys]")
        .try_into()
        .expect("bundled [keys] should match UserKeysConfig")
}

fn merge_toml_value(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Table(base), Value::Table(overlay)) => {
            for (key, value) in overlay {
                if let Some(base_value) = base.get_mut(&key) {
                    merge_toml_value(base_value, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_is_black_on_white() {
        let config = AppConfig::default();
        assert_eq!(config.theme.default.fg, "black");
        assert_eq!(config.theme.default.bg, "white");
        assert_eq!(config.theme.cursor.fg, "white");
        assert_eq!(config.theme.cursor.bg, "black");
    }

    #[test]
    fn config_paths_use_ascdraw_namespace() {
        let paths = checked_config_paths_with_env(|name| match name {
            "HOME" => Some(OsString::from("/Users/example")),
            _ => None,
        });
        assert_eq!(
            paths[1],
            PathBuf::from("/Users/example/.config/ascdraw/config.toml")
        );
    }

    #[test]
    fn partial_user_config_merges_over_defaults() {
        let mut value = bundled_default_value();
        merge_toml_value(
            &mut value,
            toml::from_str("font-size = 18.0\n[theme.default]\nfg = 'blue'\n").unwrap(),
        );
        let config: AppConfig = value.try_into().unwrap();
        assert_eq!(config.font_size, 18.0);
        assert_eq!(config.theme.default.fg, "blue");
        assert_eq!(config.theme.default.bg, "white");
    }
}
