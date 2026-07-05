use anyhow::{Context, Result, anyhow};
use image::ImageFormat;
use winit::window::Icon;

const APP_ICON_PNG: &[u8] = include_bytes!("../assets/kakvide.png");

pub fn load_window_icon() -> Result<Icon> {
    let image = image::load_from_memory_with_format(APP_ICON_PNG, ImageFormat::Png)
        .context("failed to decode bundled app icon PNG")?
        .into_rgba8();
    let (width, height) = image.dimensions();

    Icon::from_rgba(image.into_raw(), width, height)
        .map_err(|error| anyhow!("failed to build window icon: {error}"))
}

#[cfg(target_os = "macos")]
pub fn apply_app_icon() -> Result<()> {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let mtm = MainThreadMarker::new().context("app icon setup must run on the main thread")?;
    let png_data =
        unsafe { NSData::dataWithBytes_length(APP_ICON_PNG.as_ptr().cast(), APP_ICON_PNG.len()) };
    let image = NSImage::initWithData(NSImage::alloc(), &png_data)
        .ok_or_else(|| anyhow!("failed to create NSImage from bundled app icon PNG"))?;
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&image)) };

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn apply_app_icon() -> Result<()> {
    Ok(())
}
