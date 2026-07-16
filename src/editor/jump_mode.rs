use std::time::{Duration, Instant};

use winit::keyboard::{Key, ModifiersState};

use crate::jump::{JumpMode, JumpOverlay, JumpUpdate};
use crate::layout::VisibleCanvasCells;

use super::Editor;

impl Editor {
    pub fn begin_jump(&mut self, visible: VisibleCanvasCells, inactivity: Duration) -> bool {
        while self.cancel_current_state() {}
        let Some(jump_mode) = JumpMode::new(visible, self.grid.cursor_pos, inactivity) else {
            return false;
        };
        self.end_stroke();
        self.jump_mode = Some(jump_mode);
        true
    }

    pub fn handle_jump_key(&mut self, key: &Key, modifiers: ModifiersState, now: Instant) -> bool {
        let Some(jump_mode) = self.jump_mode.as_mut() else {
            return false;
        };
        match jump_mode.handle_key(key, modifiers, now) {
            JumpUpdate::Pending => false,
            JumpUpdate::Changed => true,
            JumpUpdate::MoveTo(active) => {
                self.jump_mode = None;
                self.move_to(active);
                true
            }
            JumpUpdate::Select { anchor, active } => {
                self.jump_mode = None;
                self.move_to(active);
                self.selection.select(anchor, self.grid.cursor_pos);
                true
            }
        }
    }

    pub fn advance_jump(&mut self, now: Instant) -> bool {
        let Some(jump_mode) = self.jump_mode.as_mut() else {
            return false;
        };
        match jump_mode.advance(now) {
            JumpUpdate::Pending => false,
            JumpUpdate::Changed => true,
            JumpUpdate::MoveTo(active) => {
                self.jump_mode = None;
                self.move_to(active);
                true
            }
            JumpUpdate::Select { anchor, active } => {
                self.jump_mode = None;
                self.move_to(active);
                self.selection.select(anchor, self.grid.cursor_pos);
                true
            }
        }
    }

    pub fn jump_deadline(&self) -> Option<Instant> {
        self.jump_mode.as_ref().and_then(JumpMode::deadline)
    }

    pub fn cancel_jump(&mut self) -> bool {
        self.jump_mode.take().is_some()
    }

    pub fn jump_active(&self) -> bool {
        self.jump_mode.is_some()
    }

    pub fn jump_overlay(&self) -> Option<JumpOverlay> {
        self.jump_mode.as_ref().map(JumpMode::overlay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{CursorMode, ThemeConfig};
    use crate::editor_event::EditorState;
    use crate::model::{Coord, Direction};

    #[test]
    fn landing_selects_from_the_start_and_shift_can_continue_extending() {
        let mut editor = Editor::new(&ThemeConfig::default(), "test");
        let start = Coord {
            line: 12,
            column: 40,
        };
        editor.move_to(start);
        assert!(editor.begin_jump(
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 80,
                rows: 24,
            },
            Duration::from_millis(10),
        ));

        let now = Instant::now();
        editor.handle_jump_key(&Key::Character("H".into()), ModifiersState::SHIFT, now);
        assert!(editor.advance_jump(now + Duration::from_millis(10)));
        assert!(editor.advance_jump(now + Duration::from_millis(20)));

        assert_eq!(
            editor.state(),
            EditorState::SelectionMode(CursorMode::Stamp)
        );
        assert_eq!(editor.selection.anchor(), start);
        let landed = editor.selection.active();
        assert_eq!(editor.grid.cursor_pos, landed);

        editor.extend_selection(Direction::Right);
        assert_eq!(editor.selection.anchor(), start);
        assert_eq!(
            editor.selection.active(),
            Coord {
                line: landed.line,
                column: landed.column + 1,
            }
        );
    }

    #[test]
    fn landing_without_shift_moves_only_the_cursor() {
        let mut editor = Editor::new(&ThemeConfig::default(), "test");
        let start = Coord {
            line: 12,
            column: 40,
        };
        editor.move_to(start);
        assert!(editor.begin_jump(
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 80,
                rows: 24,
            },
            Duration::from_millis(10),
        ));

        let now = Instant::now();
        editor.handle_jump_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        assert!(editor.advance_jump(now + Duration::from_millis(10)));
        assert!(editor.advance_jump(now + Duration::from_millis(20)));

        assert_eq!(editor.state(), EditorState::StampMode);
        assert!(editor.selection.is_collapsed());
        assert_ne!(editor.grid.cursor_pos, start);
    }

    #[test]
    fn second_level_movement_resets_the_editor_deadline_and_delays_landing() {
        let mut editor = Editor::new(&ThemeConfig::default(), "test");
        editor.move_to(Coord {
            line: 12,
            column: 40,
        });
        let inactivity = Duration::from_millis(10);
        assert!(editor.begin_jump(
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 80,
                rows: 24,
            },
            inactivity,
        ));

        let now = Instant::now();
        editor.handle_jump_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        let first_deadline = now + inactivity;
        assert_eq!(editor.jump_deadline(), Some(first_deadline));
        assert!(editor.advance_jump(first_deadline));

        let automatic_landing = first_deadline + inactivity;
        assert_eq!(editor.jump_deadline(), Some(automatic_landing));
        let moved_at = first_deadline + Duration::from_millis(3);
        assert!(editor.handle_jump_key(
            &Key::Character("k".into()),
            ModifiersState::empty(),
            moved_at,
        ));
        let reset_landing = moved_at + inactivity;
        assert_eq!(editor.jump_deadline(), Some(reset_landing));

        assert!(!editor.advance_jump(automatic_landing));
        assert_eq!(editor.state(), EditorState::JumpMode);
        assert!(editor.advance_jump(reset_landing));
        assert_eq!(editor.state(), EditorState::StampMode);
        assert!(editor.selection.is_collapsed());
    }
}
