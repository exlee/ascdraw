use std::cell::RefCell;
use std::ffi::CString;

use anyhow::{Context, Result, anyhow};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSApplication, NSColorSpace, NSMenu, NSMenuItem, NSView};
use objc2_foundation::NSString;
use winit::event_loop::EventLoopProxy;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::app::{AppCommand, AppEvent, MacosColorSpace, MacosConfig};

thread_local! {
    static APP_PROXY: RefCell<Option<EventLoopProxy<AppEvent>>> = const { RefCell::new(None) };
}

pub fn apply_window_color_space(window: &Window, config: &MacosConfig) -> Result<()> {
    let handle = window
        .window_handle()
        .map_err(|error| anyhow!("failed to get raw window handle: {error}"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(anyhow!("expected AppKit window handle on macOS"));
    };

    let ns_view = unsafe { &*(handle.ns_view.as_ptr().cast::<NSView>()) };
    let ns_window = ns_view
        .window()
        .context("winit AppKit view was not attached to an NSWindow")?;
    let color_space = color_space_for_config(config.color_space);
    ns_window.setColorSpace(Some(color_space.as_ref()));
    Ok(())
}

pub fn install(proxy: EventLoopProxy<AppEvent>) -> Result<()> {
    APP_PROXY.with(|slot| *slot.borrow_mut() = Some(proxy));

    let mtm = MainThreadMarker::new().context("macOS integration must run on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let delegate = app.delegate().context("missing NSApplication delegate")?;
    install_delegate_methods(delegate.as_ref())?;
    install_menus()?;
    Ok(())
}

pub fn install_menus() -> Result<()> {
    let mtm =
        MainThreadMarker::new().context("macOS menus must be installed on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    let delegate = app.delegate().context("missing NSApplication delegate")?;
    install_main_menu(mtm, &app, delegate.as_ref());
    Ok(())
}

fn install_delegate_methods(delegate: &AnyObject) -> Result<()> {
    let delegate_class = AnyObject::class(delegate);
    let class_name = CString::new("AscdrawApplicationDelegate")?;
    let class = if let Some(mut builder) = ClassBuilder::new(&class_name, delegate_class) {
        unsafe {
            builder.add_method(
                sel!(newWindow:),
                handle_new_window as unsafe extern "C-unwind" fn(_, _, _),
            );
            builder.add_method(
                sel!(closeWindow:),
                handle_close_window as unsafe extern "C-unwind" fn(_, _, _),
            );
            builder.add_method(
                sel!(increaseFontSize:),
                handle_increase_font_size as unsafe extern "C-unwind" fn(_, _, _),
            );
            builder.add_method(
                sel!(decreaseFontSize:),
                handle_decrease_font_size as unsafe extern "C-unwind" fn(_, _, _),
            );
            builder.add_method(
                sel!(resetFontSize:),
                handle_reset_font_size as unsafe extern "C-unwind" fn(_, _, _),
            );
        }
        builder.register()
    } else {
        AnyClass::get(&class_name).context("failed to register AscdrawApplicationDelegate")?
    };

    unsafe { AnyObject::set_class(delegate, class) };
    Ok(())
}

fn install_main_menu(mtm: MainThreadMarker, app: &NSApplication, target: &AnyObject) {
    let main_menu = menu(mtm, "");
    let app_menu = menu(mtm, "ascdraw");
    let file_menu = menu(mtm, "File");
    let view_menu = menu(mtm, "View");
    let window_menu = menu(mtm, "Window");

    append_submenu(&main_menu, "ascdraw", &app_menu, mtm);
    append_submenu(&main_menu, "File", &file_menu, mtm);
    append_submenu(&main_menu, "View", &view_menu, mtm);
    append_submenu(&main_menu, "Window", &window_menu, mtm);

    add_item(&app_menu, "Hide ascdraw", "h", Some(sel!(hide:)), None);
    add_item(
        &app_menu,
        "Hide Others",
        "",
        Some(sel!(hideOtherApplications:)),
        None,
    );
    add_item(
        &app_menu,
        "Show All",
        "",
        Some(sel!(unhideAllApplications:)),
        None,
    );
    app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_item(&app_menu, "Quit ascdraw", "q", Some(sel!(terminate:)), None);

    add_item(
        &file_menu,
        "New Window",
        "n",
        Some(sel!(newWindow:)),
        Some(target),
    );
    add_item(
        &file_menu,
        "Close Window",
        "w",
        Some(sel!(closeWindow:)),
        Some(target),
    );
    add_item(
        &view_menu,
        "Increase Font Size",
        "=",
        Some(sel!(increaseFontSize:)),
        Some(target),
    );
    add_item(
        &view_menu,
        "Decrease Font Size",
        "-",
        Some(sel!(decreaseFontSize:)),
        Some(target),
    );
    add_item(
        &view_menu,
        "Reset Font Size",
        "0",
        Some(sel!(resetFontSize:)),
        Some(target),
    );

    add_item(
        &window_menu,
        "Minimize",
        "m",
        Some(sel!(performMiniaturize:)),
        None,
    );
    add_item(&window_menu, "Zoom", "", Some(sel!(performZoom:)), None);
    window_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_item(
        &window_menu,
        "Bring All to Front",
        "",
        Some(sel!(arrangeInFront:)),
        None,
    );

    app.setMainMenu(Some(&main_menu));
    app.setWindowsMenu(Some(&window_menu));
}

fn menu(mtm: MainThreadMarker, title: &str) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str(title));
    menu.setAutoenablesItems(false);
    menu
}

fn append_submenu(parent: &NSMenu, title: &str, submenu: &NSMenu, mtm: MainThreadMarker) {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            None,
            &NSString::from_str(""),
        )
    };
    item.setSubmenu(Some(submenu));
    parent.addItem(&item);
}

fn add_item(
    menu: &NSMenu,
    title: &str,
    key: &str,
    action: Option<Sel>,
    target: Option<&AnyObject>,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        menu.addItemWithTitle_action_keyEquivalent(
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    if let Some(target) = target {
        unsafe { item.setTarget(Some(target)) };
    }
    item
}

unsafe extern "C-unwind" fn handle_new_window(
    _this: &mut AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
) {
    send_command(AppCommand::WindowNew);
}

unsafe extern "C-unwind" fn handle_close_window(
    _this: &mut AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
) {
    send_command(AppCommand::WindowClose);
}

unsafe extern "C-unwind" fn handle_increase_font_size(
    _this: &mut AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
) {
    send_command(AppCommand::FontScaleUp);
}

unsafe extern "C-unwind" fn handle_decrease_font_size(
    _this: &mut AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
) {
    send_command(AppCommand::FontScaleDown);
}

unsafe extern "C-unwind" fn handle_reset_font_size(
    _this: &mut AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
) {
    send_command(AppCommand::FontScaleReset);
}

fn send_command(command: AppCommand) {
    APP_PROXY.with(|slot| {
        if let Some(proxy) = slot.borrow().as_ref() {
            let _ = proxy.send_event(AppEvent::Command(command));
        }
    });
}

pub(crate) fn color_space_for_config(color_space: MacosColorSpace) -> Retained<NSColorSpace> {
    match color_space {
        MacosColorSpace::P3 => NSColorSpace::displayP3ColorSpace(),
        MacosColorSpace::Srgb => NSColorSpace::sRGBColorSpace(),
    }
}

#[cfg(test)]
mod tests {
    use super::color_space_for_config;
    use crate::app::MacosColorSpace;

    #[test]
    fn p3_color_space_maps_to_display_p3() {
        let actual = color_space_for_config(MacosColorSpace::P3);
        let expected = objc2_app_kit::NSColorSpace::displayP3ColorSpace();
        assert_eq!(actual.localizedName(), expected.localizedName());
    }

    #[test]
    fn srgb_color_space_maps_to_srgb() {
        let actual = color_space_for_config(MacosColorSpace::Srgb);
        let expected = objc2_app_kit::NSColorSpace::sRGBColorSpace();
        assert_eq!(actual.localizedName(), expected.localizedName());
    }
}
