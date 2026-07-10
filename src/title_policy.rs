use winit::dpi::LogicalSize;
use winit::window::{WindowAttributes, WindowLevel};

use crate::app::{AppConfig, DEFAULT_WINDOW_TITLE};

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;

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
fn native_window_title(config: &AppConfig) -> &'static str {
    if config.transparent_menubar {
        ""
    } else {
        DEFAULT_WINDOW_TITLE
    }
}

#[cfg(not(target_os = "macos"))]
fn native_window_title(_config: &AppConfig) -> &'static str {
    DEFAULT_WINDOW_TITLE
}

pub fn window_attributes(config: &AppConfig) -> WindowAttributes {
    apply_platform_window_attributes(
        WindowAttributes::default()
            .with_title(native_window_title(config))
            .with_window_level(WindowLevel::Normal)
            .with_inner_size(LogicalSize::new(1200.0, 800.0)),
        config,
    )
}
