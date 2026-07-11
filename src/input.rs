use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::AppConfig;
use crate::app::CursorMode;
use crate::layout::{PADDING, ViewportOffset, content_top_padding};
use crate::model::{Coord, Direction};
use crate::render::Renderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    Move(Direction),
    Draw(Direction),
    Clear,
    ToggleTextEntry,
    PlaceStamp,
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
    if matches!(key, Key::Named(NamedKey::Enter)) {
        return Some(EditCommand::ToggleTextEntry);
    }

    if modifiers.control_key() || modifiers.alt_key() || modifiers.super_key() {
        return None;
    }

    if mode == CursorMode::MoveDraw {
        return match key {
            Key::Named(NamedKey::Backspace) => Some(EditCommand::Clear),
            _ if is_space_key(key) => Some(EditCommand::Clear),
            _ => direction_for_key(key).map(|direction| {
                if modifiers.shift_key() {
                    EditCommand::Draw(direction)
                } else {
                    EditCommand::Move(direction)
                }
            }),
        };
    }

    if mode == CursorMode::Text {
        return match key {
            Key::Named(NamedKey::Backspace) => Some(EditCommand::Backspace),
            Key::Named(NamedKey::Delete) => Some(EditCommand::Delete),
            Key::Named(NamedKey::Tab) => Some(EditCommand::InsertTab),
            _ => arrow_direction_for_key(key).map(EditCommand::Move),
        };
    }

    if mode == CursorMode::Stamp {
        return match key {
            _ if is_space_key(key) => Some(EditCommand::PlaceStamp),
            _ => direction_for_key(key).map(EditCommand::Move),
        };
    }

    if matches!(mode, CursorMode::Shapes | CursorMode::Utilities) {
        return direction_for_key(key).map(EditCommand::Move);
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

fn is_space_key(key: &Key) -> bool {
    match key {
        Key::Named(NamedKey::Space) => true,
        Key::Character(text) => text == " ",
        _ => false,
    }
}

fn direction_for_key(key: &Key) -> Option<Direction> {
    arrow_direction_for_key(key).or_else(|| match key {
        Key::Character(text) if text.eq_ignore_ascii_case("h") => Some(Direction::Left),
        Key::Character(text) if text.eq_ignore_ascii_case("j") => Some(Direction::Down),
        Key::Character(text) if text.eq_ignore_ascii_case("k") => Some(Direction::Up),
        Key::Character(text) if text.eq_ignore_ascii_case("l") => Some(Direction::Right),
        _ => None,
    })
}

fn arrow_direction_for_key(key: &Key) -> Option<Direction> {
    match key {
        Key::Named(NamedKey::ArrowLeft) => Some(Direction::Left),
        Key::Named(NamedKey::ArrowRight) => Some(Direction::Right),
        Key::Named(NamedKey::ArrowUp) => Some(Direction::Up),
        Key::Named(NamedKey::ArrowDown) => Some(Direction::Down),
        _ => None,
    }
}

pub fn pointer_position_to_coord(
    x: f64,
    y: f64,
    renderer: &Renderer,
    scale_factor: f64,
    config: &AppConfig,
    viewport: ViewportOffset,
) -> Option<Coord> {
    let metrics = renderer.metrics(scale_factor);
    let toolbar_metrics = renderer.title_metrics(scale_factor);
    let grid_top = content_top_padding(scale_factor, config.transparent_menubar)
        + crate::toolbar::toolbar_height(toolbar_metrics.cell_height);
    let grid_x = x - PADDING as f64 - viewport.x as f64;
    let grid_y = y - grid_top as f64 - viewport.y as f64;
    if grid_x < 0.0 || grid_y < 0.0 {
        return None;
    }
    let column = (grid_x / metrics.cell_width.max(1) as f64).floor() as usize;
    let line = (grid_y / metrics.cell_height.max(1) as f64).floor() as usize;
    Some(Coord { line, column })
}

pub fn pointer_position_to_toolbar_position(
    x: f64,
    y: f64,
    renderer: &Renderer,
    scale_factor: f64,
    config: &AppConfig,
) -> Option<(usize, usize)> {
    let metrics = renderer.title_metrics(scale_factor);
    let toolbar_x = x - PADDING as f64;
    let toolbar_y = y - content_top_padding(scale_factor, config.transparent_menubar) as f64;
    if toolbar_x < 0.0 || toolbar_y < 0.0 {
        return None;
    }
    let stride = metrics.cell_height + crate::toolbar::TOOLBAR_ROW_GAP;
    let row = (toolbar_y / stride as f64).floor() as usize;
    if row >= crate::toolbar::TOOLBAR_ROWS || toolbar_y as usize % stride >= metrics.cell_height {
        return None;
    }
    let column = (toolbar_x / metrics.cell_width.max(1) as f64).floor() as usize;
    Some((row, column))
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
                &Key::Named(NamedKey::Delete),
                ModifiersState::empty(),
                CursorMode::MoveDraw,
            ),
            None
        );
    }

    #[test]
    fn maps_backspace_and_space_to_clear_in_move_draw_mode() {
        for key in [Key::Named(NamedKey::Backspace), Key::Character(" ".into())] {
            assert_eq!(
                edit_command_for_key(&key, ModifiersState::empty(), CursorMode::MoveDraw),
                Some(EditCommand::Clear)
            );
        }
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

    #[test]
    fn return_toggles_text_mode_from_every_canvas_mode() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Text,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            assert_eq!(
                edit_command_for_key(&Key::Named(NamedKey::Enter), ModifiersState::empty(), mode,),
                Some(EditCommand::ToggleTextEntry)
            );
        }
    }

    #[test]
    fn text_mode_types_hjkl_and_moves_only_with_arrows() {
        assert_eq!(
            edit_command_for_key(
                &Key::Character("h".into()),
                ModifiersState::empty(),
                CursorMode::Text,
            ),
            None
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowLeft),
                ModifiersState::empty(),
                CursorMode::Text,
            ),
            Some(EditCommand::Move(Direction::Left))
        );
    }

    #[test]
    fn space_places_the_active_stamp() {
        for key in [Key::Character(" ".into()), Key::Named(NamedKey::Space)] {
            assert_eq!(
                edit_command_for_key(&key, ModifiersState::empty(), CursorMode::Stamp),
                Some(EditCommand::PlaceStamp)
            );
        }
    }
}
