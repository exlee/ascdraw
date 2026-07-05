use std::cell::RefCell;
use std::ffi::{CString, OsString};
use std::path::PathBuf;

use anyhow::{Context, Result};
use objc2::rc::Retained;
use objc2::rc::autoreleasepool;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{MainThreadMarker, ProtocolType, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuDelegate, NSMenuItem};
use objc2_foundation::{NSArray, NSString};
use winit::event_loop::EventLoopProxy;

use crate::app::{AppCommand, AppEvent};
use crate::diagnostics::log_error;
use crate::kakoune_process::list_kakoune_sessions;

const CONNECT_SESSIONS_MENU: &str = "Connect to Session";
const SWITCH_SESSIONS_MENU: &str = "Switch to Session";

thread_local! {
    static APP_PROXY: RefCell<Option<EventLoopProxy<AppEvent>>> = const { RefCell::new(None) };
    static KAK_BIN: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn install(proxy: EventLoopProxy<AppEvent>, kak_bin: String) -> Result<()> {
    APP_PROXY.with(|slot| {
        *slot.borrow_mut() = Some(proxy);
    });
    KAK_BIN.with(|slot| {
        *slot.borrow_mut() = Some(kak_bin);
    });

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
    let class_name = CString::new("KakvideApplicationDelegate")?;

    let class = if let Some(mut builder) = ClassBuilder::new(&class_name, delegate_class) {
        builder.add_protocol(
            <dyn NSMenuDelegate>::protocol().context("missing NSMenuDelegate protocol")?,
        );
        unsafe {
            builder.add_method(
                sel!(application:openFiles:),
                handle_open_files as unsafe extern "C-unwind" fn(_, _, _, _),
            );
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
            builder.add_method(
                sel!(connectToSession:),
                handle_connect_to_session as unsafe extern "C-unwind" fn(_, _, _),
            );
            builder.add_method(
                sel!(switchToSession:),
                handle_switch_to_session as unsafe extern "C-unwind" fn(_, _, _),
            );
            builder.add_method(
                sel!(menuNeedsUpdate:),
                handle_menu_needs_update as unsafe extern "C-unwind" fn(_, _, _),
            );
        }
        builder.register()
    } else {
        AnyClass::get(&class_name).context("failed to register KakvideApplicationDelegate")?
    };

    unsafe {
        AnyObject::set_class(delegate, class);
    }

    Ok(())
}

fn install_main_menu(mtm: MainThreadMarker, app: &NSApplication, target: &AnyObject) {
    let main_menu = menu(mtm, "");
    let app_menu = menu(mtm, "Kakvide");
    let file_menu = menu(mtm, "File");
    let view_menu = menu(mtm, "View");
    let sessions_menu = menu(mtm, "Sessions");
    let window_menu = menu(mtm, "Window");

    append_submenu(&main_menu, "Kakvide", &app_menu, mtm);
    append_submenu(&main_menu, "File", &file_menu, mtm);
    append_submenu(&main_menu, "View", &view_menu, mtm);
    append_submenu(&main_menu, "Sessions", &sessions_menu, mtm);
    append_submenu(&main_menu, "Window", &window_menu, mtm);

    add_item(&app_menu, "Hide Kakvide", "h", Some(sel!(hide:)), None);
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
    add_item(&app_menu, "Quit Kakvide", "q", Some(sel!(terminate:)), None);

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

    let connect_menu = menu(mtm, CONNECT_SESSIONS_MENU);
    let switch_menu = menu(mtm, SWITCH_SESSIONS_MENU);
    unsafe {
        let _: () = msg_send![&*connect_menu, setDelegate: target];
        let _: () = msg_send![&*switch_menu, setDelegate: target];
    }
    append_submenu(&sessions_menu, CONNECT_SESSIONS_MENU, &connect_menu, mtm);
    append_submenu(&sessions_menu, SWITCH_SESSIONS_MENU, &switch_menu, mtm);

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

fn menu(mtm: MainThreadMarker, title: &str) -> objc2::rc::Retained<NSMenu> {
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
) -> objc2::rc::Retained<NSMenuItem> {
    let item = unsafe {
        menu.addItemWithTitle_action_keyEquivalent(
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    if let Some(target) = target {
        unsafe {
            item.setTarget(Some(target));
        }
    }
    item
}

fn add_disabled_item(menu: &NSMenu, title: &str) {
    let item = add_item(menu, title, "", None, None);
    item.setEnabled(false);
}

unsafe extern "C-unwind" fn handle_open_files(
    _this: &mut AnyObject,
    _sel: Sel,
    _sender: &AnyObject,
    filenames: &NSArray<NSString>,
) {
    let paths = filenames_to_paths(filenames);
    if paths.is_empty() {
        return;
    }
    send_event(AppEvent::OpenFiles(paths));
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

unsafe extern "C-unwind" fn handle_connect_to_session(
    _this: &mut AnyObject,
    _sel: Sel,
    sender: &NSMenuItem,
) {
    send_command(AppCommand::ConnectToSession(session_from_item(sender)));
}

unsafe extern "C-unwind" fn handle_switch_to_session(
    _this: &mut AnyObject,
    _sel: Sel,
    sender: &NSMenuItem,
) {
    send_command(AppCommand::SwitchToSession(session_from_item(sender)));
}

unsafe extern "C-unwind" fn handle_menu_needs_update(
    _this: &mut AnyObject,
    _sel: Sel,
    menu: &NSMenu,
) {
    let title = menu.title().to_string();
    match title.as_str() {
        CONNECT_SESSIONS_MENU => refresh_session_menu(menu, sel!(connectToSession:)),
        SWITCH_SESSIONS_MENU => refresh_session_menu(menu, sel!(switchToSession:)),
        _ => {}
    }
}

fn refresh_session_menu(menu: &NSMenu, action: Sel) {
    menu.removeAllItems();

    let sessions = KAK_BIN.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|kak_bin| list_kakoune_sessions(kak_bin))
    });

    match sessions {
        Some(Ok(sessions)) if !sessions.is_empty() => {
            let target = app_delegate_target();
            for session in sessions {
                let title = session.to_string_lossy();
                add_item(menu, &title, "", Some(action), target.as_deref());
            }
        }
        Some(Ok(_)) => add_disabled_item(menu, "No Sessions Available"),
        Some(Err(error)) => {
            log_error(format!("session list failed: {error:#}"));
            add_disabled_item(menu, "Unable to List Sessions");
        }
        None => add_disabled_item(menu, "Unable to List Sessions"),
    }
}

fn app_delegate_target() -> Option<Retained<AnyObject>> {
    let mtm = MainThreadMarker::new()?;
    NSApplication::sharedApplication(mtm)
        .delegate()
        .map(|delegate| unsafe { Retained::cast_unchecked(delegate) })
}

fn session_from_item(item: &NSMenuItem) -> OsString {
    OsString::from(item.title().to_string())
}

fn send_command(command: AppCommand) {
    send_event(AppEvent::Command(command));
}

fn send_event(event: AppEvent) {
    APP_PROXY.with(|slot| {
        if let Some(proxy) = slot.borrow().as_ref() {
            let _ = proxy.send_event(event);
        }
    });
}

fn filenames_to_paths(filenames: &NSArray<NSString>) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(filenames.count());
    for index in 0..filenames.count() {
        let filename = unsafe { filenames.objectAtIndex_unchecked(index) };
        let path = autoreleasepool(|pool| unsafe { filename.to_str(pool).to_owned() });
        paths.push(PathBuf::from(path));
    }
    paths
}
