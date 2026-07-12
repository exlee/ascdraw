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
    DrawStamp(Direction),
    Erase(Direction),
    Clear,
    ToggleTextEntry,
    ToggleReplaceMode,
    BeginSingleReplace,
    CancelTextEntry,
    PlaceStamp,
    ToggleShapePreview,
    ConfirmShape,
    Home,
    End,
    Backspace,
    Delete,
    Newline,
    InsertTab,
}

pub fn edit_command(
    key: &Key,
    repeat: bool,
    modifiers: ModifiersState,
    mode: CursorMode,
) -> Option<EditCommand> {
    if repeat && matches!(key, Key::Named(NamedKey::Escape)) {
        return None;
    }
    edit_command_for_key(key, modifiers, mode)
}

fn edit_command_for_key(
    key: &Key,
    modifiers: ModifiersState,
    mode: CursorMode,
) -> Option<EditCommand> {
    if mode.accepts_text()
        && (matches!(key, Key::Named(NamedKey::Escape))
            || (modifiers.control_key()
                && !modifiers.alt_key()
                && !modifiers.super_key()
                && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("c") || text.eq_ignore_ascii_case("g"))))
    {
        return Some(EditCommand::CancelTextEntry);
    }

    if matches!(key, Key::Named(NamedKey::Enter)) {
        return Some(if modifiers.shift_key() {
            EditCommand::ToggleReplaceMode
        } else {
            EditCommand::ToggleTextEntry
        });
    }

    if modifiers.control_key() || modifiers.super_key() {
        return None;
    }

    if !modifiers.shift_key()
        && !modifiers.alt_key()
        && matches!(key, Key::Character(text) if text == "r")
        && !matches!(
            mode,
            CursorMode::Text | CursorMode::Insert | CursorMode::Replace
        )
    {
        return Some(EditCommand::BeginSingleReplace);
    }

    if matches!(key, Key::Named(NamedKey::Backspace)) {
        return Some(EditCommand::Clear);
    }

    if modifiers.alt_key() {
        return direction_for_key(key).map(EditCommand::Erase);
    }

    if mode == CursorMode::MoveDraw {
        return match key {
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
            Key::Named(NamedKey::Delete) => Some(EditCommand::Delete),
            Key::Named(NamedKey::Tab) => Some(EditCommand::InsertTab),
            _ => arrow_direction_for_key(key).map(EditCommand::Move),
        };
    }

    if mode == CursorMode::Stamp {
        return match key {
            _ if is_space_key(key) => Some(EditCommand::PlaceStamp),
            _ => direction_for_key(key).map(|direction| {
                if modifiers.shift_key() {
                    EditCommand::DrawStamp(direction)
                } else {
                    EditCommand::Move(direction)
                }
            }),
        };
    }

    if mode == CursorMode::Shapes {
        return match key {
            Key::Named(NamedKey::Escape) => Some(EditCommand::ToggleShapePreview),
            _ if is_space_key(key) => Some(EditCommand::ConfirmShape),
            _ => direction_for_key(key).map(EditCommand::Move),
        };
    }

    if mode == CursorMode::Utilities {
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
            Some(EditCommand::Clear)
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
    fn maps_backspace_to_clear_in_every_mode() {
        for mode in [
            CursorMode::Insert,
            CursorMode::Replace,
            CursorMode::MoveDraw,
            CursorMode::Text,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            assert_eq!(
                edit_command_for_key(
                    &Key::Named(NamedKey::Backspace),
                    ModifiersState::empty(),
                    mode,
                ),
                Some(EditCommand::Clear)
            );
        }

        assert_eq!(
            edit_command_for_key(
                &Key::Character(" ".into()),
                ModifiersState::empty(),
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::Clear)
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
    fn maps_alt_directions_to_line_erasing_in_every_mode() {
        for mode in [
            CursorMode::Insert,
            CursorMode::Replace,
            CursorMode::MoveDraw,
            CursorMode::Text,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            for (key, direction) in [
                (Key::Character("h".into()), Direction::Left),
                (Key::Named(NamedKey::ArrowDown), Direction::Down),
            ] {
                assert_eq!(
                    edit_command_for_key(&key, ModifiersState::ALT, mode),
                    Some(EditCommand::Erase(direction))
                );
            }
        }
    }

    #[test]
    fn shape_escape_toggles_preview_and_space_confirms() {
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Escape),
                ModifiersState::empty(),
                CursorMode::Shapes,
            ),
            Some(EditCommand::ToggleShapePreview)
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Space),
                ModifiersState::empty(),
                CursorMode::Shapes,
            ),
            Some(EditCommand::ConfirmShape)
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
    fn shift_return_toggles_replace_mode() {
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Enter),
                ModifiersState::SHIFT,
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::ToggleReplaceMode)
        );
    }

    #[test]
    fn lowercase_r_starts_single_replace_only_outside_text_and_replace_modes() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            assert_eq!(
                edit_command_for_key(&Key::Character("r".into()), ModifiersState::empty(), mode,),
                Some(EditCommand::BeginSingleReplace)
            );
        }

        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            assert_eq!(
                edit_command_for_key(&Key::Character("r".into()), ModifiersState::empty(), mode,),
                None
            );
        }
    }

    #[test]
    fn cancel_keys_exit_every_text_accepting_mode() {
        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            for (key, modifiers) in [
                (Key::Named(NamedKey::Escape), ModifiersState::empty()),
                (Key::Character("c".into()), ModifiersState::CONTROL),
                (Key::Character("g".into()), ModifiersState::CONTROL),
            ] {
                assert_eq!(
                    edit_command_for_key(&key, modifiers, mode),
                    Some(EditCommand::CancelTextEntry),
                    "mode={mode:?}, key={key:?}"
                );
            }
        }
    }

    #[test]
    fn control_cancel_keys_do_nothing_in_ordinary_drawing_modes() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            for key in [Key::Character("c".into()), Key::Character("g".into())] {
                assert_eq!(
                    edit_command_for_key(&key, ModifiersState::CONTROL, mode),
                    None,
                    "mode={mode:?}, key={key:?}"
                );
            }
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

    #[test]
    fn shift_direction_draws_stamps() {
        for (key, direction) in [
            (Key::Character("l".into()), Direction::Right),
            (Key::Named(NamedKey::ArrowDown), Direction::Down),
        ] {
            assert_eq!(
                edit_command_for_key(&key, ModifiersState::SHIFT, CursorMode::Stamp),
                Some(EditCommand::DrawStamp(direction))
            );
        }
    }
}
