use winit::dpi::LogicalSize;
use winit::window::{Icon, Window, WindowAttributes, WindowLevel};

use crate::app::{AppConfig, DEFAULT_WINDOW_TITLE, WINDOW_TITLE_UI_OPTION};

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleUpdate {
    pub window_title: String,
    pub client_name: Option<String>,
}

#[cfg(target_os = "macos")]
fn apply_platform_window_attributes(attrs: WindowAttributes, config: &AppConfig) -> WindowAttributes {
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
fn apply_platform_window_attributes(attrs: WindowAttributes, _config: &AppConfig) -> WindowAttributes {
    attrs
}

#[cfg(target_os = "macos")]
fn native_window_title<'a>(config: &AppConfig, title: &'a str) -> &'a str {
    if config.transparent_menubar { "" } else { title }
}

#[cfg(not(target_os = "macos"))]
fn native_window_title<'a>(_config: &AppConfig, title: &'a str) -> &'a str {
    title
}

pub fn window_attributes(config: &AppConfig, window_icon: Option<Icon>) -> WindowAttributes {
    apply_platform_window_attributes(
        WindowAttributes::default()
            .with_title(native_window_title(config, DEFAULT_WINDOW_TITLE))
            .with_window_level(WindowLevel::Normal)
            .with_inner_size(LogicalSize::new(1200.0, 800.0))
            .with_window_icon(window_icon),
        config,
    )
}

pub fn decode_title_update(
    options: &serde_json::Map<String, serde_json::Value>,
) -> Option<TitleUpdate> {
    let raw_title = options
        .get(WINDOW_TITLE_UI_OPTION)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)?;

    if raw_title.is_empty() {
        return Some(TitleUpdate {
            window_title: DEFAULT_WINDOW_TITLE.to_string(),
            client_name: None,
        });
    }

    let client_name = raw_title
        .rsplit_once(" - ")
        .map(|(_, client)| client.trim())
        .filter(|client| !client.is_empty())
        .map(str::to_string);

    Some(TitleUpdate {
        window_title: format_window_title(raw_title),
        client_name,
    })
}

fn format_window_title(title: &str) -> String {
    if let Some((pwd, client)) = title.rsplit_once(" - ") {
        format!(
            "{DEFAULT_WINDOW_TITLE} - {} - {}",
            pwd.trim(),
            display_client_name(client.trim())
        )
    } else {
        format!("{DEFAULT_WINDOW_TITLE} - {title}")
    }
}

fn display_client_name(client: &str) -> String {
    if let Some(suffix) = client.strip_prefix("client") {
        if suffix.is_empty() {
            "Client".to_string()
        } else {
            format!("Client {suffix}")
        }
    } else {
        client.to_string()
    }
}

#[cfg(target_os = "macos")]
pub fn should_update_native_window_title(config: &AppConfig) -> bool {
    !config.transparent_menubar
}

#[cfg(not(target_os = "macos"))]
pub fn should_update_native_window_title(_config: &AppConfig) -> bool {
    true
}

#[cfg(target_os = "macos")]
pub fn set_native_window_title(window: &Window, config: &AppConfig, title: &str) {
    if should_update_native_window_title(config) {
        window.set_title(title);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_native_window_title(window: &Window, _config: &AppConfig, title: &str) {
    window.set_title(title);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppConfig;

    #[test]
    fn title_update_formats_client_zero_name() {
        let mut options = serde_json::Map::new();
        options.insert(
            WINDOW_TITLE_UI_OPTION.to_string(),
            serde_json::Value::String("/tmp/project - client0".to_string()),
        );

        let update = decode_title_update(&options).expect("title update");
        assert_eq!(update.window_title, "kakvide - /tmp/project - Client 0");
        assert_eq!(update.client_name.as_deref(), Some("client0"));
    }

    #[test]
    fn title_update_preserves_non_default_client_name() {
        let mut options = serde_json::Map::new();
        options.insert(
            WINDOW_TITLE_UI_OPTION.to_string(),
            serde_json::Value::String("/tmp/project - main".to_string()),
        );

        let update = decode_title_update(&options).expect("title update");
        assert_eq!(update.window_title, "kakvide - /tmp/project - main");
        assert_eq!(update.client_name.as_deref(), Some("main"));
    }

    #[test]
    fn title_update_uses_default_for_empty_title() {
        let mut options = serde_json::Map::new();
        options.insert(
            WINDOW_TITLE_UI_OPTION.to_string(),
            serde_json::Value::String("  ".to_string()),
        );

        let update = decode_title_update(&options).expect("title update");
        assert_eq!(update.window_title, DEFAULT_WINDOW_TITLE);
        assert_eq!(update.client_name, None);
    }

    #[test]
    fn missing_title_returns_none() {
        assert_eq!(decode_title_update(&serde_json::Map::new()), None);
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
}
