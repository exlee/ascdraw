use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::AppConfig;
use crate::app::CursorMode;
use crate::layout::{PADDING, content_top_padding};
use crate::model::{Coord, Direction};
use crate::render::Renderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    Move(Direction),
    Draw(Direction),
    Home,
    End,
    Backspace,
    Delete,
    Newline,
    InsertTab,
}

pub fn edit_command(
    event: &KeyEvent,
    modifiers: ModifiersState,
    mode: CursorMode,
) -> Option<EditCommand> {
    edit_command_for_key(&event.logical_key, modifiers, mode)
}

fn edit_command_for_key(
    key: &Key,
    modifiers: ModifiersState,
    mode: CursorMode,
) -> Option<EditCommand> {
    if modifiers.control_key() || modifiers.alt_key() || modifiers.super_key() {
        return None;
    }

    if mode == CursorMode::MoveDraw {
        return direction_for_key(key).map(|direction| {
            if modifiers.shift_key() {
                EditCommand::Draw(direction)
            } else {
                EditCommand::Move(direction)
            }
        });
    }

    match key {
        Key::Named(NamedKey::ArrowLeft) => Some(EditCommand::Move(Direction::Left)),
        Key::Named(NamedKey::ArrowRight) => Some(EditCommand::Move(Direction::Right)),
        Key::Named(NamedKey::ArrowUp) => Some(EditCommand::Move(Direction::Up)),
        Key::Named(NamedKey::ArrowDown) => Some(EditCommand::Move(Direction::Down)),
        Key::Named(NamedKey::Home) => Some(EditCommand::Home),
        Key::Named(NamedKey::End) => Some(EditCommand::End),
        Key::Named(NamedKey::Backspace) => Some(EditCommand::Backspace),
        Key::Named(NamedKey::Delete) => Some(EditCommand::Delete),
        Key::Named(NamedKey::Enter) => Some(EditCommand::Newline),
        Key::Named(NamedKey::Tab) => Some(EditCommand::InsertTab),
        _ => None,
    }
}

fn direction_for_key(key: &Key) -> Option<Direction> {
    match key {
        Key::Named(NamedKey::ArrowLeft) => Some(Direction::Left),
        Key::Named(NamedKey::ArrowRight) => Some(Direction::Right),
        Key::Named(NamedKey::ArrowUp) => Some(Direction::Up),
        Key::Named(NamedKey::ArrowDown) => Some(Direction::Down),
        Key::Character(text) if text.eq_ignore_ascii_case("h") => Some(Direction::Left),
        Key::Character(text) if text.eq_ignore_ascii_case("j") => Some(Direction::Down),
        Key::Character(text) if text.eq_ignore_ascii_case("k") => Some(Direction::Up),
        Key::Character(text) if text.eq_ignore_ascii_case("l") => Some(Direction::Right),
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
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowLeft),
                ModifiersState::empty(),
                CursorMode::Insert,
            ),
            Some(EditCommand::Move(Direction::Left))
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Backspace),
                ModifiersState::empty(),
                CursorMode::Insert,
            ),
            Some(EditCommand::Backspace)
        );
    }

    #[test]
    fn move_draw_ignores_non_directional_editing_keys() {
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Backspace),
                ModifiersState::empty(),
                CursorMode::MoveDraw,
            ),
            None
        );
    }

    #[test]
    fn maps_hjkl_and_shifted_movement_to_move_draw_commands() {
        assert_eq!(
            edit_command_for_key(
                &Key::Character("h".into()),
                ModifiersState::empty(),
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::Move(Direction::Left))
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowDown),
                ModifiersState::SHIFT,
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::Draw(Direction::Down))
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Character("L".into()),
                ModifiersState::SHIFT,
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::Draw(Direction::Right))
        );
    }

    #[test]
    fn leaves_modified_keys_for_app_shortcuts() {
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowLeft),
                ModifiersState::SUPER,
                CursorMode::MoveDraw,
            ),
            None
        );
    }
}
