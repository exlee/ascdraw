use crate::app::CursorMode;
use crate::model::{Coord, Direction};

use super::EditorState;

impl EditorState {
    pub fn begin_pointer_drag(&mut self) -> bool {
        self.cancel_line_preview();
        self.cancel_move_lift();
        match self.cursor_mode {
            CursorMode::Stamp => {
                self.place_stamp();
                true
            }
            CursorMode::Shapes => self.start_shape_or_confirm(),
            _ => false,
        }
    }

    pub fn drag_pointer_to(&mut self, target: Coord) -> bool {
        if self.cursor_mode == CursorMode::Shapes {
            return self.move_to(target);
        }
        if !matches!(self.cursor_mode, CursorMode::MoveDraw | CursorMode::Stamp) {
            return self.move_to(target);
        }

        let mut changed = false;
        while self.grid.cursor_pos.column != target.column {
            let direction = if self.grid.cursor_pos.column < target.column {
                Direction::Right
            } else {
                Direction::Left
            };
            changed |= self.drag_pointer_step(direction);
        }
        while self.grid.cursor_pos.line != target.line {
            let direction = if self.grid.cursor_pos.line < target.line {
                Direction::Down
            } else {
                Direction::Up
            };
            changed |= self.drag_pointer_step(direction);
        }
        changed
    }

    pub fn finish_pointer_drag(&mut self) -> bool {
        if self.cursor_mode == CursorMode::Shapes && self.has_shape_preview() {
            return self.start_shape_or_confirm();
        }
        self.end_stroke();
        false
    }

    fn drag_pointer_step(&mut self, direction: Direction) -> bool {
        match self.cursor_mode {
            CursorMode::MoveDraw => self.move_or_draw(direction, true),
            CursorMode::Stamp => {
                self.draw_stamp(direction);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeConfig;
    use crate::toolbar::{MainMode, ToolbarAction};

    fn state(mode: MainMode) -> EditorState {
        let mut state = EditorState::new(&ThemeConfig::default(), "ascdraw");
        state.apply_toolbar_action(ToolbarAction::SelectMain(mode));
        state
    }

    fn plain_text(state: &EditorState) -> String {
        state
            .grid
            .lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|atom| atom.contents.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn line_drag_connects_every_traversed_cell() {
        let mut state = state(MainMode::Line);
        assert!(!state.begin_pointer_drag());
        assert!(state.drag_pointer_to(Coord { line: 0, column: 3 }));
        assert!(!state.finish_pointer_drag());

        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 3 });
        assert_eq!(plain_text(&state), "╶──╴");
    }

    #[test]
    fn stamp_drag_paints_the_start_and_every_traversed_cell() {
        let mut state = state(MainMode::Stamp);
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 0,
        });
        assert!(state.begin_pointer_drag());
        assert!(state.drag_pointer_to(Coord { line: 0, column: 3 }));
        assert!(!state.finish_pointer_drag());

        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 3 });
        assert_eq!(plain_text(&state), "░░░░");
    }

    #[test]
    fn shape_drag_previews_until_release_then_commits() {
        let mut state = state(MainMode::Shapes);
        assert!(!state.begin_pointer_drag());
        assert!(state.has_shape_preview());
        state.drag_pointer_to(Coord { line: 2, column: 3 });
        assert!(state.finish_pointer_drag());

        assert!(!state.has_shape_preview());
        assert_eq!(state.grid.cursor_pos, Coord { line: 2, column: 3 });
        assert_eq!(plain_text(&state), "┌──┐\n│  │\n└──┘");
    }
}
