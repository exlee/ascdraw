use std::collections::VecDeque;

use crate::editor::EditSnapshot;
use crate::layout::ViewportOffset;

pub const HISTORY_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySnapshot {
    pub edit: EditSnapshot,
    pub viewport: ViewportOffset,
}

#[derive(Debug, Default)]
pub struct EditHistory {
    undo: VecDeque<HistorySnapshot>,
    redo: VecDeque<HistorySnapshot>,
    pending: Option<PendingChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryGroup {
    LineStroke,
    TextSession,
}

#[derive(Debug)]
struct PendingChange {
    group: HistoryGroup,
    previous: HistorySnapshot,
}

impl EditHistory {
    pub fn record_change(&mut self, previous: HistorySnapshot, current: &HistorySnapshot) -> bool {
        self.finish_transaction(&previous);
        if previous.edit.same_document(&current.edit) {
            return false;
        }
        push_bounded(&mut self.undo, previous);
        self.redo.clear();
        true
    }

    pub fn record_grouped_change(
        &mut self,
        group: HistoryGroup,
        previous: HistorySnapshot,
        current: &HistorySnapshot,
    ) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.group != group)
        {
            self.finish_transaction(&previous);
        }
        if previous.edit.same_document(&current.edit) {
            return false;
        }
        self.pending
            .get_or_insert(PendingChange { group, previous });
        true
    }

    pub fn finish_transaction(&mut self, current: &HistorySnapshot) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        if pending.previous.edit.same_document(&current.edit) {
            return false;
        }
        push_bounded(&mut self.undo, pending.previous);
        self.redo.clear();
        true
    }

    pub fn record_project_load(
        &mut self,
        previous: HistorySnapshot,
        current: &HistorySnapshot,
    ) -> bool {
        self.finish_transaction(&previous);
        if previous == *current {
            return false;
        }
        push_bounded(&mut self.undo, previous);
        self.redo.clear();
        true
    }

    pub fn undo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot> {
        self.finish_transaction(&current);
        let previous = self.undo.pop_back()?;
        push_bounded(&mut self.redo, current);
        Some(previous)
    }

    pub fn redo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot> {
        self.finish_transaction(&current);
        let next = self.redo.pop_back()?;
        push_bounded(&mut self.undo, current);
        Some(next)
    }

    #[cfg(test)]
    pub(crate) fn lengths(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    #[cfg(test)]
    pub(crate) fn has_pending_transaction(&self) -> bool {
        self.pending.is_some()
    }
}

fn push_bounded(stack: &mut VecDeque<HistorySnapshot>, snapshot: HistorySnapshot) {
    if stack.len() == HISTORY_LIMIT {
        stack.pop_front();
    }
    stack.push_back(snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppConfig;
    use crate::editor::EditorState;
    use crate::model::Direction;
    use crate::toolbar::{MainMode, ToolbarAction};

    fn snapshot(text: &str, viewport: ViewportOffset) -> HistorySnapshot {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.insert(text);
        HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport,
        }
    }

    #[test]
    fn records_undo_redo_and_clears_redo_after_a_new_edit() {
        let blank = snapshot("", ViewportOffset::default());
        let one = snapshot("1", ViewportOffset { x: 1, y: 2 });
        let two = snapshot("12", ViewportOffset { x: 3, y: 4 });
        let three = snapshot("123", ViewportOffset { x: 5, y: 6 });
        let mut history = EditHistory::default();

        assert!(history.record_change(blank.clone(), &one));
        assert!(history.record_change(one.clone(), &two));
        assert_eq!(history.undo(two.clone()), Some(one.clone()));
        assert_eq!(history.undo(one.clone()), Some(blank.clone()));
        assert_eq!(history.redo(blank), Some(one.clone()));
        assert_eq!(history.redo(one.clone()), Some(two.clone()));

        assert_eq!(history.undo(two), Some(one.clone()));
        assert!(history.record_change(one, &three));
        assert_eq!(history.lengths(), (2, 0));
        assert!(history.redo(three).is_none());
    }

    #[test]
    fn multi_step_line_stroke_commits_once_and_reports_dirty_before_commit() {
        let blank = snapshot("", ViewportOffset::default());
        let one = snapshot("─", ViewportOffset { x: 1, y: 0 });
        let complete = snapshot("──", ViewportOffset { x: 2, y: 0 });
        let mut history = EditHistory::default();

        assert!(history.record_grouped_change(HistoryGroup::LineStroke, blank.clone(), &one));
        assert!(history.has_pending_transaction());
        assert_eq!(history.lengths(), (0, 0));
        assert!(history.record_grouped_change(HistoryGroup::LineStroke, one, &complete));
        assert_eq!(history.lengths(), (0, 0));

        assert!(history.finish_transaction(&complete));
        assert_eq!(history.lengths(), (1, 0));
        assert_eq!(history.undo(complete), Some(blank));
    }

    #[test]
    fn text_insert_replace_and_single_replace_sessions_each_commit_once_on_exit() {
        for mode in [
            crate::app::CursorMode::Text,
            crate::app::CursorMode::Insert,
            crate::app::CursorMode::Replace,
        ] {
            let mut state = EditorState::new(&AppConfig::default().theme, "test");
            state.cursor_mode = mode;
            let before = HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: ViewportOffset::default(),
            };
            state.write_text("a");
            let one = HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: ViewportOffset::default(),
            };
            state.write_text("b");
            let two = HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: ViewportOffset::default(),
            };
            let mut history = EditHistory::default();
            assert!(history.record_grouped_change(HistoryGroup::TextSession, before.clone(), &one));
            assert!(history.record_grouped_change(HistoryGroup::TextSession, one, &two));
            state.cancel_text_entry();
            let exited = HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: ViewportOffset::default(),
            };
            assert!(history.finish_transaction(&exited));
            assert_eq!(history.lengths(), (1, 0), "mode {mode:?}");
            assert_eq!(history.undo(exited), Some(before), "mode {mode:?}");
        }

        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.place_stamp();
        let before = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        assert!(state.begin_single_replace());
        state.write_text("x");
        assert_ne!(state.cursor_mode, crate::app::CursorMode::Replace);
        let replaced = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        let mut history = EditHistory::default();
        assert!(history.record_grouped_change(
            HistoryGroup::TextSession,
            before.clone(),
            &replaced
        ));
        assert!(history.finish_transaction(&replaced));
        assert_eq!(history.undo(replaced), Some(before));
    }

    #[test]
    fn no_op_grouped_sessions_and_interruptions_preserve_redo() {
        let blank = snapshot("", ViewportOffset::default());
        let edited = snapshot("x", ViewportOffset::default());
        let mut history = EditHistory::default();
        assert!(history.record_change(blank.clone(), &edited));
        assert_eq!(history.undo(edited.clone()), Some(blank.clone()));

        assert!(!history.record_grouped_change(HistoryGroup::TextSession, blank.clone(), &blank));
        assert!(!history.finish_transaction(&blank));
        assert_eq!(history.lengths(), (0, 1));
        assert_eq!(history.redo(blank.clone()), Some(edited.clone()));

        assert_eq!(history.undo(edited.clone()), Some(blank.clone()));
        let transient = snapshot("z", ViewportOffset::default());
        assert!(history.record_grouped_change(
            HistoryGroup::TextSession,
            blank.clone(),
            &transient
        ));
        assert!(history.record_grouped_change(HistoryGroup::TextSession, transient, &blank));
        assert!(!history.finish_transaction(&blank));
        assert_eq!(history.lengths(), (0, 1));

        let interim = snapshot("y", ViewportOffset { x: 4, y: 5 });
        assert!(history.record_grouped_change(HistoryGroup::TextSession, blank.clone(), &interim));
        assert_eq!(history.undo(interim), Some(blank));
        assert_eq!(history.lengths(), (0, 1));
    }

    #[test]
    fn suppresses_cursor_only_and_identical_document_changes() {
        let previous = snapshot("same", ViewportOffset::default());
        let mut current = previous.clone();
        current.viewport = ViewportOffset { x: 9, y: 8 };
        current.edit.set_cursor_for_test(0, 0);
        let mut history = EditHistory::default();

        assert!(!history.record_change(previous, &current));
        assert_eq!(history.lengths(), (0, 0));
    }

    #[test]
    fn project_load_records_cursor_selection_and_viewport_as_one_atomic_change() {
        let before = snapshot("same", ViewportOffset::default());
        let mut loaded = before.clone();
        loaded.viewport = ViewportOffset { x: -9, y: 14 };
        loaded.edit.set_cursor_for_test(0, 2);
        let mut history = EditHistory::default();

        assert!(history.record_project_load(before.clone(), &loaded));
        assert_eq!(history.undo(loaded.clone()), Some(before.clone()));
        assert_eq!(history.redo(before), Some(loaded));
    }

    #[test]
    fn both_stacks_are_bounded() {
        let mut history = EditHistory::default();
        let mut current = snapshot("0", ViewportOffset::default());
        for index in 1..=HISTORY_LIMIT + 10 {
            let next = snapshot(&index.to_string(), ViewportOffset::default());
            assert!(history.record_change(current, &next));
            current = next;
        }
        assert_eq!(history.lengths(), (HISTORY_LIMIT, 0));

        for _ in 0..HISTORY_LIMIT {
            current = history.undo(current).expect("bounded undo entry");
        }
        assert_eq!(history.lengths(), (0, HISTORY_LIMIT));
    }

    #[test]
    fn histories_are_isolated_per_owner() {
        let blank = snapshot("", ViewportOffset::default());
        let edited = snapshot("window one", ViewportOffset { x: 7, y: 8 });
        let mut first = EditHistory::default();
        let mut second = EditHistory::default();

        assert!(first.record_change(blank.clone(), &edited));
        assert_eq!(first.undo(edited), Some(blank.clone()));
        assert!(second.undo(blank).is_none());
    }

    #[test]
    fn utility_edits_round_trip_and_no_op_does_not_clear_redo() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.insert("ab");
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 1,
        });
        let before = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        assert!(state.apply_utility(Direction::Left));
        let after = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset { x: 3, y: 4 },
        };
        let mut history = EditHistory::default();
        assert!(history.record_change(before.clone(), &after));

        let restored = history.undo(after.clone()).unwrap();
        assert_eq!(restored, before);
        state.restore_edit_snapshot(restored.edit.clone());
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        });
        state.grid.cursor_pos.column = 99;
        state.selection.collapse(state.grid.cursor_pos);
        assert!(!state.apply_utility(Direction::Left));
        let no_op = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: restored.viewport,
        };
        assert!(!history.record_change(restored.clone(), &no_op));
        assert_eq!(history.redo(no_op), Some(after));
    }

    #[test]
    fn confirmed_move_lift_is_one_entry_and_stationary_confirmation_preserves_redo() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.insert("abcd");
        state.move_home();
        state.extend_selection(Direction::Right);
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 0,
        });
        let before = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        assert!(state.begin_move_lift());
        assert!(state.move_lift(Direction::Right));
        assert!(state.move_lift(Direction::Right));
        assert!(state.confirm_move_lift());
        let after = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        let mut history = EditHistory::default();
        assert!(history.record_change(before.clone(), &after));
        assert_eq!(history.lengths(), (1, 0));
        assert_eq!(history.undo(after.clone()), Some(before.clone()));
        assert_eq!(history.lengths(), (0, 1));

        state.restore_edit_snapshot(before.edit.clone());
        assert!(state.begin_move_lift());
        assert!(!state.confirm_move_lift());
        let stationary = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        assert!(!history.record_change(before.clone(), &stationary));
        assert_eq!(history.lengths(), (0, 1));
        assert_eq!(history.redo(before), Some(after));
    }

    #[test]
    fn clear_is_undoable_and_redoable_while_blank_clear_preserves_redo() {
        let mut blank_state = EditorState::new(&AppConfig::default().theme, "test");
        let blank = HistorySnapshot {
            edit: blank_state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        let mut edited_state = blank_state.clone();
        edited_state.insert("drawing");
        let edited = HistorySnapshot {
            edit: edited_state.edit_snapshot(),
            viewport: ViewportOffset { x: 8, y: 9 },
        };
        let mut history = EditHistory::default();
        assert!(history.record_change(blank.clone(), &edited));

        let restored_blank = history.undo(edited.clone()).unwrap();
        blank_state.restore_edit_snapshot(restored_blank.edit.clone());
        blank_state.clear_canvas();
        let blank_no_op = HistorySnapshot {
            edit: blank_state.edit_snapshot(),
            viewport: restored_blank.viewport,
        };
        assert!(!history.record_change(restored_blank, &blank_no_op));
        assert_eq!(history.redo(blank_no_op.clone()), Some(edited.clone()));

        edited_state.clear_canvas();
        let cleared = HistorySnapshot {
            edit: edited_state.edit_snapshot(),
            viewport: edited.viewport,
        };
        let mut clear_history = EditHistory::default();
        assert!(clear_history.record_change(edited.clone(), &cleared));
        assert_eq!(clear_history.undo(cleared.clone()), Some(edited.clone()));
        assert_eq!(clear_history.redo(edited), Some(cleared.clone()));
    }

    #[test]
    fn undo_and_redo_snapshots_do_not_change_durable_menu_selections() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 3,
        });
        let menu_selections = state.toolbar.durable_selections();
        let before = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        state.insert("x");
        let after = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        let mut history = EditHistory::default();
        assert!(history.record_change(before, &after));

        let undone = history.undo(after).unwrap();
        state.restore_edit_snapshot(undone.edit.clone());
        assert_eq!(state.toolbar.durable_selections(), menu_selections);
        let redone = history.redo(undone).unwrap();
        state.restore_edit_snapshot(redone.edit);
        assert_eq!(state.toolbar.durable_selections(), menu_selections);
    }

    #[test]
    fn literal_selection_clear_is_one_transaction_and_blank_clear_retains_redo() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.insert("x ");
        state.move_to(crate::model::Coord::default());
        let edited = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };

        state.clear_selection();
        let cleared = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        let mut history = EditHistory::default();
        assert!(history.record_change(edited.clone(), &cleared));

        let restored = history.undo(cleared.clone()).expect("clear is undoable");
        assert_eq!(restored, edited);
        state.restore_edit_snapshot(restored.edit.clone());
        state.move_to(crate::model::Coord { line: 0, column: 1 });
        let before_blank_clear = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: restored.viewport,
        };
        state.clear_selection();
        let after_blank_clear = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: restored.viewport,
        };
        assert!(!history.record_change(before_blank_clear, &after_blank_clear));
        assert_eq!(history.redo(after_blank_clear), Some(cleared));
    }
}
