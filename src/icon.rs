use anyhow::{Context, Result, anyhow};

const APP_ICON_PNG: &[u8] = include_bytes!("../assets/ascdraw.png");

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

