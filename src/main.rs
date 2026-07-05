use std::rc::Rc;

use anyhow::{Result, anyhow};
use clap::Parser;
use softbuffer::{Context as SoftContext, Surface};
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, Ime, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
#[cfg(target_os = "macos")]
use winit::platform::macos::WindowAttributesExtMacOS;
use winit::window::WindowAttributes;
use winit::window::WindowLevel;

mod app;
mod input;
mod kakoune_messages;
mod kakoune_process;
mod layout;
mod render;
mod user_keys;

use app::{AppConfig, AppEvent, AppState, Args, apply_notification, load_config};
use input::{
    ScrollState, key_event_to_kak, pointer_position_to_coord, scroll_delta_to_kak, send_keys,
    send_mouse_button, send_mouse_move, send_resize, send_scroll,
};
use kakoune_messages::{Coord, KakouneNotification};
use kakoune_process::{spawn_kakoune, spawn_stdin_writer};
use render::{load_renderer, render, resize_surface};
use user_keys::{UserAction, UserKeys};

#[cfg(target_os = "macos")]
fn apply_platform_window_attributes(
    attrs: WindowAttributes,
    config: &AppConfig,
) -> WindowAttributes {
    if config.transparent_menubar {
        attrs
            .with_titlebar_transparent(true)
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

fn main() -> Result<()> {
    let args = Args::parse();
    let config = load_config()?;
    let user_keys = UserKeys::from_config(&config.keys)?;

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let attrs = apply_platform_window_attributes(
        WindowAttributes::default()
            .with_title("kakvide")
            .with_window_level(WindowLevel::Normal)
            .with_inner_size(LogicalSize::new(1200.0, 800.0)),
        &config,
    );
    let window = Rc::new(event_loop.create_window(attrs)?);
    let renderer = load_renderer(&config);
    let context = SoftContext::new(window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    let mut surface =
        Surface::new(&context, window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    resize_surface(&mut surface, window.inner_size())?;

    let mut child = spawn_kakoune(&args, event_loop.create_proxy())?;
    let command_tx = spawn_stdin_writer(&mut child)?;

    let mut modifiers = ModifiersState::empty();
    let mut mouse_cell = Coord { line: 0, column: 0 };
    let mut scroll_state = ScrollState::default();
    let mut did_force_startup_resize = false;
    let mut state = AppState::default();

    send_resize(&command_tx, &window, &renderer, &config);
    window.request_redraw();

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            Event::Resumed => {
                send_resize(&command_tx, &window, &renderer, &config);
                window.request_redraw();
            }
            Event::UserEvent(AppEvent::Rpc(notification)) => {
                let should_force_resize = matches!(notification, KakouneNotification::Draw { .. })
                    && !did_force_startup_resize;
                apply_notification(&mut state, notification);
                if should_force_resize {
                    send_resize(&command_tx, &window, &renderer, &config);
                    did_force_startup_resize = true;
                }
                window.request_redraw();
            }
            Event::UserEvent(AppEvent::KakouneExited) => {
                elwt.exit();
            }
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    let _ = child.kill();
                    elwt.exit();
                }
                WindowEvent::Resized(size) => {
                    if let Err(error) = resize_surface(&mut surface, size) {
                        eprintln!("surface resize failed: {error:#}");
                    }
                    send_resize(&command_tx, &window, &renderer, &config);
                    window.request_redraw();
                }
                WindowEvent::RedrawRequested => {
                    if let Err(error) = render(&window, &mut surface, &state, &renderer, &config) {
                        eprintln!("render failed: {error:#}");
                        let _ = child.kill();
                        elwt.exit();
                    }
                }
                WindowEvent::ModifiersChanged(new_modifiers) => {
                    modifiers = new_modifiers.state();
                }
                WindowEvent::Ime(Ime::Commit(text)) => {
                    if !text.is_empty()
                        && !modifiers.control_key()
                        && !modifiers.alt_key()
                        && !modifiers.super_key()
                    {
                        send_keys(&command_tx, &[text.to_string()]);
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed {
                        if let Some(action) = user_keys.action_for_event(&event, modifiers) {
                            let changed = match action {
                                UserAction::FontScaleUp => renderer.adjust_font_size(1.0),
                                UserAction::FontScaleDown => renderer.adjust_font_size(-1.0),
                                UserAction::FontScaleReset => renderer.reset_font_size(),
                            };
                            if changed {
                                send_resize(&command_tx, &window, &renderer, &config);
                                window.request_redraw();
                            }
                            return;
                        }
                        if let Some(keys) = key_event_to_kak(&event, modifiers) {
                            send_keys(&command_tx, &[keys]);
                        }
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    mouse_cell = pointer_position_to_coord(
                        position.x, position.y, &renderer, &window, &config,
                    );
                    send_mouse_move(&command_tx, mouse_cell);
                }
                WindowEvent::MouseInput { state, button, .. } => match state {
                    ElementState::Pressed => {
                        send_mouse_button(&command_tx, true, button, mouse_cell)
                    }
                    ElementState::Released => {
                        send_mouse_button(&command_tx, false, button, mouse_cell)
                    }
                },
                WindowEvent::MouseWheel { delta, .. } => {
                    if let Some(amount) = scroll_delta_to_kak(
                        delta,
                        config.mouse_scroll_rate.max(0.0) as f64,
                        &mut scroll_state,
                    ) {
                        send_scroll(&command_tx, amount, mouse_cell);
                    }
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    send_resize(&command_tx, &window, &renderer, &config);
                    window.request_redraw();
                }
                _ => {}
            },
            _ => {}
        }
    })?;

    Ok(())
}
