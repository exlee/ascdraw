use std::sync::mpsc::Sender;

use winit::event::{KeyEvent, MouseButton, MouseScrollDelta};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::Window;

use crate::app::AppConfig;
use crate::kakoune_messages::{Coord, KakouneRequest, MouseButtonName};
use crate::layout::{PADDING, content_top_padding, layout_metrics};
use crate::render::Renderer;

#[derive(Debug, Default)]
pub struct ScrollState {
    pending_amount: f64,
}

pub fn send_resize(tx: &Sender<String>, window: &Window, renderer: &Renderer, config: &AppConfig) {
    let size = window.inner_size();
    let scale_factor = window.scale_factor();
    let metrics = renderer.metrics(scale_factor);
    let layout = layout_metrics(
        size.width as usize,
        size.height as usize,
        &metrics,
        config.transparent_menubar,
        scale_factor,
    );
    send_request(
        tx,
        KakouneRequest::Resize {
            rows: layout.rows,
            columns: layout.cols,
        },
    );
}

pub fn send_keys(tx: &Sender<String>, keys: &[String]) {
    send_request(
        tx,
        KakouneRequest::Keys {
            keys: keys.to_vec(),
        },
    );
}

pub fn send_mouse_move(tx: &Sender<String>, coord: Coord) {
    send_request(tx, KakouneRequest::MouseMove { coord });
}

pub fn send_mouse_button(tx: &Sender<String>, pressed: bool, button: MouseButton, coord: Coord) {
    if let Some(button) = mouse_button_to_kak(button) {
        let request = if pressed {
            KakouneRequest::MousePress { button, coord }
        } else {
            KakouneRequest::MouseRelease { button, coord }
        };
        send_request(tx, request);
    }
}

pub fn send_scroll(tx: &Sender<String>, amount: i32, coord: Coord) {
    send_request(tx, KakouneRequest::Scroll { amount, coord });
}

fn send_request(tx: &Sender<String>, request: KakouneRequest) {
    let _ = tx.send(request.to_json_line());
}

pub fn pointer_position_to_coord(
    x: f64,
    y: f64,
    renderer: &Renderer,
    window: &Window,
    config: &AppConfig,
) -> Coord {
    let scale_factor = window.scale_factor();
    let metrics = renderer.metrics(scale_factor);
    let top_padding = content_top_padding(scale_factor, config.transparent_menubar);
    let column =
        ((x - PADDING as f64).max(0.0) / metrics.cell_width.max(1) as f64).floor() as usize;
    let line =
        ((y - top_padding as f64).max(0.0) / metrics.cell_height.max(1) as f64).floor() as usize;
    Coord { line, column }
}

fn mouse_button_to_kak(button: MouseButton) -> Option<MouseButtonName> {
    match button {
        MouseButton::Left => Some(MouseButtonName::Left),
        MouseButton::Right => Some(MouseButtonName::Right),
        MouseButton::Middle => Some(MouseButtonName::Middle),
        _ => None,
    }
}

pub fn scroll_delta_to_kak(
    delta: MouseScrollDelta,
    scroll_rate: f64,
    state: &mut ScrollState,
) -> Option<i32> {
    let raw_amount = match delta {
        MouseScrollDelta::LineDelta(x, y) => dominant_scroll_component(x as f64, y as f64),
        MouseScrollDelta::PixelDelta(position) => dominant_scroll_component(position.x, position.y),
    };
    if raw_amount == 0.0 || scroll_rate <= 0.0 {
        return None;
    }

    state.pending_amount += raw_amount * scroll_rate;
    let amount = if state.pending_amount > 0.0 {
        state.pending_amount.floor()
    } else {
        state.pending_amount.ceil()
    };
    if amount == 0.0 {
        return None;
    }

    state.pending_amount -= amount;
    Some(amount as i32)
}

fn dominant_scroll_component(x: f64, y: f64) -> f64 {
    let dominant = if y.abs() >= x.abs() { y } else { x };
    -dominant
}

pub fn key_event_to_kak(event: &KeyEvent, modifiers: ModifiersState) -> Option<String> {
    match &event.logical_key {
        Key::Named(named) => named_key_to_kak(*named, modifiers),
        Key::Character(text) => {
            if modifiers.control_key() || modifiers.alt_key() {
                modified_character_key_to_kak(&event.key_without_modifiers(), modifiers)
            } else {
                Some(text.to_string())
            }
        }
        _ => None,
    }
}

fn modified_character_key_to_kak(key: &Key, modifiers: ModifiersState) -> Option<String> {
    let Key::Character(text) = key else {
        return None;
    };

    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    let mut result = String::from("<");
    if modifiers.shift_key() {
        result.push_str("s-");
    }
    if modifiers.alt_key() {
        result.push_str("a-");
    }
    if modifiers.control_key() {
        result.push_str("c-");
    }
    result.push(ch.to_ascii_lowercase());
    result.push('>');
    Some(result)
}

fn named_key_to_kak(key: NamedKey, modifiers: ModifiersState) -> Option<String> {
    let base = match key {
        NamedKey::Enter => "ret",
        NamedKey::Tab => "tab",
        NamedKey::Space => "space",
        NamedKey::Escape => "esc",
        NamedKey::ArrowUp => "up",
        NamedKey::ArrowDown => "down",
        NamedKey::ArrowLeft => "left",
        NamedKey::ArrowRight => "right",
        NamedKey::Backspace => "backspace",
        NamedKey::Delete => "del",
        NamedKey::Home => "home",
        NamedKey::End => "end",
        NamedKey::PageUp => "pageup",
        NamedKey::PageDown => "pagedown",
        _ => return None,
    };

    if modifiers == ModifiersState::empty() {
        return Some(format!("<{base}>"));
    }

    let mut result = String::from("<");
    if modifiers.shift_key() {
        result.push_str("s-");
    }
    if modifiers.alt_key() {
        result.push_str("a-");
    }
    if modifiers.control_key() {
        result.push_str("c-");
    }
    result.push_str(base);
    result.push('>');
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_alt_shift_character_keys_from_unmodified_character() {
        let modifiers = ModifiersState::ALT | ModifiersState::SHIFT;
        let key = Key::Character(";".into());
        assert_eq!(
            modified_character_key_to_kak(&key, modifiers).as_deref(),
            Some("<s-a-;>")
        );
    }

    #[test]
    fn formats_ctrl_shift_letter_keys_with_explicit_shift_modifier() {
        let modifiers = ModifiersState::CONTROL | ModifiersState::SHIFT;
        let key = Key::Character("p".into());
        assert_eq!(
            modified_character_key_to_kak(&key, modifiers).as_deref(),
            Some("<s-c-p>")
        );
    }

    #[test]
    fn scroll_delta_maps_wheel_up_to_negative_kak_scroll() {
        let mut state = ScrollState::default();
        assert_eq!(
            scroll_delta_to_kak(MouseScrollDelta::LineDelta(0.0, 1.0), 1.0, &mut state),
            Some(-1)
        );
        assert_eq!(
            scroll_delta_to_kak(MouseScrollDelta::LineDelta(0.0, -2.0), 1.0, &mut state),
            Some(2)
        );
    }

    #[test]
    fn scroll_rate_accumulates_fractional_line_scroll() {
        let mut state = ScrollState::default();
        assert_eq!(
            scroll_delta_to_kak(MouseScrollDelta::LineDelta(0.0, 1.0), 0.5, &mut state),
            None
        );
        assert_eq!(
            scroll_delta_to_kak(MouseScrollDelta::LineDelta(0.0, 1.0), 0.5, &mut state),
            Some(-1)
        );
    }
}
