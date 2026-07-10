use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::AppConfig;
use crate::layout::{PADDING, content_top_padding};
use crate::model::Coord;
use crate::render::Renderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Home,
    End,
    Backspace,
    Delete,
    Newline,
    InsertTab,
}

pub fn edit_command(event: &KeyEvent, modifiers: ModifiersState) -> Option<EditCommand> {
    edit_command_for_key(&event.logical_key, modifiers)
}

fn edit_command_for_key(key: &Key, modifiers: ModifiersState) -> Option<EditCommand> {
    if modifiers.control_key() || modifiers.alt_key() || modifiers.super_key() {
        return None;
    }

    match key {
        Key::Named(NamedKey::ArrowLeft) => Some(EditCommand::MoveLeft),
        Key::Named(NamedKey::ArrowRight) => Some(EditCommand::MoveRight),
        Key::Named(NamedKey::ArrowUp) => Some(EditCommand::MoveUp),
        Key::Named(NamedKey::ArrowDown) => Some(EditCommand::MoveDown),
        Key::Named(NamedKey::Home) => Some(EditCommand::Home),
        Key::Named(NamedKey::End) => Some(EditCommand::End),
        Key::Named(NamedKey::Backspace) => Some(EditCommand::Backspace),
        Key::Named(NamedKey::Delete) => Some(EditCommand::Delete),
        Key::Named(NamedKey::Enter) => Some(EditCommand::Newline),
        Key::Named(NamedKey::Tab) => Some(EditCommand::InsertTab),
        _ => None,
    }
}

pub fn pointer_position_to_coord(
    x: f64,
    y: f64,
    renderer: &Renderer,
    scale_factor: f64,
    config: &AppConfig,
) -> Coord {
    let metrics = renderer.metrics(scale_factor);
    let top_padding = content_top_padding(scale_factor, config.transparent_menubar);
    let column =
        ((x - PADDING as f64).max(0.0) / metrics.cell_width.max(1) as f64).floor() as usize;
    let line =
        ((y - top_padding as f64).max(0.0) / metrics.cell_height.max(1) as f64).floor() as usize;
    Coord { line, column }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_editor_navigation_keys() {
        assert_eq!(
            edit_command_for_key(&Key::Named(NamedKey::ArrowLeft), ModifiersState::empty()),
            Some(EditCommand::MoveLeft)
        );
        assert_eq!(
            edit_command_for_key(&Key::Named(NamedKey::Backspace), ModifiersState::empty()),
            Some(EditCommand::Backspace)
        );
    }

    #[test]
    fn leaves_modified_keys_for_app_shortcuts() {
        assert_eq!(
            edit_command_for_key(&Key::Named(NamedKey::ArrowLeft), ModifiersState::SUPER),
            None
        );
    }
}
