use std::collections::VecDeque;

use crate::canvas::HistoryCanvasDelta;
use crate::editor::HistoryEditorState;
use crate::layout::ViewportOffset;

pub const HISTORY_LIMIT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySnapshot {
    pub edit: HistoryEditorState,
    pub viewport: ViewportOffset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRestore {
    pub edit: HistoryEditorState,
    pub viewport: ViewportOffset,
    pub canvas: HistoryCanvasDelta,
    pub forward: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HistoryChange {
    before: HistorySnapshot,
    after: HistorySnapshot,
    canvas: HistoryCanvasDelta,
}

#[derive(Debug, Default)]
pub struct EditHistory {
    undo: VecDeque<HistoryChange>,
    redo: VecDeque<HistoryChange>,
    pending: Option<PendingChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryGroup {
    ControlStroke,
    LineRoute,
    TextSession,
}

#[derive(Debug)]
struct PendingChange {
    group: HistoryGroup,
    change: HistoryChange,
}

impl EditHistory {
    pub fn record_change(
        &mut self,
        previous: HistorySnapshot,
        current: HistorySnapshot,
        canvas: HistoryCanvasDelta,
    ) -> bool {
        self.finish_transaction();
        if canvas.is_empty() {
            return false;
        }
        push_bounded(
            &mut self.undo,
            HistoryChange {
                before: previous,
                after: current,
                canvas,
            },
        );
        self.redo.clear();
        true
    }

    pub fn record_grouped_change(
        &mut self,
        group: HistoryGroup,
        previous: HistorySnapshot,
        current: HistorySnapshot,
        canvas: HistoryCanvasDelta,
    ) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.group != group)
        {
            self.finish_transaction();
        }
        if canvas.is_empty() {
            return false;
        }
        if let Some(pending) = self.pending.as_mut() {
            pending.change.after = current;
            pending.change.canvas.merge(canvas);
        } else {
            self.pending = Some(PendingChange {
                group,
                change: HistoryChange {
                    before: previous,
                    after: current,
                    canvas,
                },
            });
        }
        true
    }

    pub fn finish_transaction(&mut self) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        if pending.change.canvas.is_empty() {
            return false;
        }
        push_bounded(&mut self.undo, pending.change);
        self.redo.clear();
        true
    }

    pub fn record_project_load(
        &mut self,
        previous: HistorySnapshot,
        current: HistorySnapshot,
        canvas: HistoryCanvasDelta,
    ) -> bool {
        self.finish_transaction();
        if canvas.is_empty() && previous == current {
            return false;
        }
        push_bounded(
            &mut self.undo,
            HistoryChange {
                before: previous,
                after: current,
                canvas,
            },
        );
        self.redo.clear();
        true
    }

    pub fn undo(&mut self) -> Option<HistoryRestore> {
        self.finish_transaction();
        let change = self.undo.pop_back()?;
        let restore = HistoryRestore {
            edit: change.before.edit.clone(),
            viewport: change.before.viewport,
            canvas: change.canvas.clone(),
            forward: false,
        };
        push_bounded(&mut self.redo, change);
        Some(restore)
    }

    pub fn redo(&mut self) -> Option<HistoryRestore> {
        self.finish_transaction();
        let change = self.redo.pop_back()?;
        let restore = HistoryRestore {
            edit: change.after.edit.clone(),
            viewport: change.after.viewport,
            canvas: change.canvas.clone(),
            forward: true,
        };
        push_bounded(&mut self.undo, change);
        Some(restore)
    }

    #[cfg(test)]
    pub(crate) fn lengths(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}

fn push_bounded(stack: &mut VecDeque<HistoryChange>, change: HistoryChange) {
    if stack.len() == HISTORY_LIMIT {
        stack.pop_front();
    }
    stack.push_back(change);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppConfig;
    use crate::editor::Editor;
    use crate::model::Coord;

    fn snapshot(editor: &Editor, viewport: ViewportOffset) -> HistorySnapshot {
        HistorySnapshot {
            edit: editor.history_state(),
            viewport,
        }
    }

    fn captured_edit(
        editor: &mut Editor,
        viewport: ViewportOffset,
        edit: impl FnOnce(&mut Editor),
    ) -> (HistorySnapshot, HistorySnapshot, HistoryCanvasDelta) {
        let before = snapshot(editor, viewport);
        editor.begin_history_capture();
        edit(editor);
        let canvas = editor.finish_history_capture();
        let after = snapshot(editor, viewport);
        (before, after, canvas)
    }

    fn restore(editor: &mut Editor, restore: HistoryRestore) {
        editor.apply_history_delta(&restore.canvas, restore.forward);
        editor.restore_history_state(restore.edit);
    }

    #[test]
    fn undo_and_redo_apply_only_changed_cells() {
        let mut editor = Editor::new(&AppConfig::default().theme, "test");
        let (before, after, delta) =
            captured_edit(&mut editor, ViewportOffset::default(), |editor| {
                editor.insert("abc");
            });
        assert_eq!(delta.cells.len(), 3);
        let mut history = EditHistory::default();
        assert!(history.record_change(before, after, delta));

        restore(&mut editor, history.undo().unwrap());
        assert!(editor.content_cells().is_empty());
        restore(&mut editor, history.redo().unwrap());
        assert_eq!(editor.content_cells().len(), 3);
    }

    #[test]
    fn grouped_changes_merge_first_before_and_last_after_values() {
        let mut editor = Editor::new(&AppConfig::default().theme, "test");
        let mut history = EditHistory::default();
        for text in ["a", "b", "c"] {
            let (before, after, delta) =
                captured_edit(&mut editor, ViewportOffset::default(), |editor| {
                    editor.insert(text);
                });
            assert!(
                history.record_grouped_change(HistoryGroup::TextSession, before, after, delta,)
            );
        }
        assert!(history.finish_transaction());
        assert_eq!(history.lengths(), (1, 0));
        restore(&mut editor, history.undo().unwrap());
        assert!(editor.content_cells().is_empty());
        restore(&mut editor, history.redo().unwrap());
        assert_eq!(editor.content_cells().len(), 3);
    }

    #[test]
    fn grouped_change_that_returns_to_its_start_is_not_recorded() {
        let mut editor = Editor::new(&AppConfig::default().theme, "test");
        let mut history = EditHistory::default();
        let (before, after, delta) =
            captured_edit(&mut editor, ViewportOffset::default(), |editor| {
                editor.insert("x");
            });
        history.record_grouped_change(HistoryGroup::TextSession, before, after, delta);
        let (before, after, delta) =
            captured_edit(&mut editor, ViewportOffset::default(), |editor| {
                editor.clear_canvas();
            });
        history.record_grouped_change(HistoryGroup::TextSession, before, after, delta);
        assert!(!history.finish_transaction());
        assert_eq!(history.lengths(), (0, 0));
    }

    #[test]
    fn history_limit_discards_oldest_sparse_change() {
        let mut editor = Editor::new(&AppConfig::default().theme, "test");
        let mut history = EditHistory::default();
        for _ in 0..=HISTORY_LIMIT {
            let (before, after, delta) =
                captured_edit(&mut editor, ViewportOffset::default(), |editor| {
                    editor.insert("x");
                });
            history.record_change(before, after, delta);
        }
        assert_eq!(history.lengths(), (HISTORY_LIMIT, 0));
        for _ in 0..HISTORY_LIMIT {
            restore(&mut editor, history.undo().unwrap());
        }
        assert!(history.undo().is_none());
    }

    #[test]
    fn overwriting_one_cell_records_one_sparse_element() {
        let mut editor = Editor::new(&AppConfig::default().theme, "test");
        editor.insert(&"x".repeat(100));
        editor.move_to(Coord {
            line: 0,
            column: 50,
        });

        let (_, _, delta) =
            captured_edit(&mut editor, ViewportOffset::default(), Editor::place_stamp);

        assert_eq!(delta.cells.len(), 1);
        assert_eq!((delta.cells[0].line, delta.cells[0].column), (0, 50));
    }

    #[test]
    fn deleting_a_layer_restores_its_topology_and_cells() {
        let mut editor = Editor::new(&AppConfig::default().theme, "test");
        let base = editor.active_layer_id();
        assert!(editor.add_layer_above(base));
        let upper = editor.active_layer_id();
        editor.insert("top");
        let (before, after, delta) =
            captured_edit(&mut editor, ViewportOffset::default(), |editor| {
                assert!(editor.delete_layer(upper));
            });
        assert_eq!(delta.before.layers.len(), 2);
        assert_eq!(delta.after.layers.len(), 1);
        assert_eq!(delta.cells.len(), 3);

        let mut history = EditHistory::default();
        assert!(history.record_change(before, after, delta));
        restore(&mut editor, history.undo().unwrap());
        assert_eq!(editor.layer_summaries().len(), 2);
        assert_eq!(editor.active_layer_id(), upper);
        assert_eq!(
            editor.content_cells(),
            [
                Coord { line: 0, column: 0 },
                Coord { line: 0, column: 1 },
                Coord { line: 0, column: 2 },
            ]
        );
        restore(&mut editor, history.redo().unwrap());
        assert_eq!(editor.layer_summaries().len(), 1);
    }

    #[test]
    fn replacing_a_document_captures_old_and_new_sparse_cells() {
        let mut editor = Editor::new(&AppConfig::default().theme, "test");
        editor.insert("old");
        let (before, after, delta) =
            captured_edit(&mut editor, ViewportOffset::default(), |editor| {
                editor.replace_canvas(crate::export::lines_from_text("new"))
            });
        assert_eq!(delta.cells.len(), 3);

        let mut history = EditHistory::default();
        assert!(history.record_project_load(before, after, delta));
        restore(&mut editor, history.undo().unwrap());
        assert_eq!(
            editor.lines_for_test()[0]
                .iter()
                .map(|atom| atom.contents.as_str())
                .collect::<String>(),
            "old"
        );
        restore(&mut editor, history.redo().unwrap());
        assert_eq!(
            editor.lines_for_test()[0]
                .iter()
                .map(|atom| atom.contents.as_str())
                .collect::<String>(),
            "new"
        );
    }

    #[test]
    fn merging_a_layer_restores_new_target_coordinates_on_undo() {
        let mut editor = Editor::new(&AppConfig::default().theme, "test");
        editor.insert("A");
        let base = editor.active_layer_id();
        assert!(editor.add_layer_above(base));
        let upper = editor.active_layer_id();
        editor.move_to(Coord { line: 0, column: 2 });
        editor.insert("B");

        let (before, after, delta) =
            captured_edit(&mut editor, ViewportOffset::default(), |editor| {
                assert!(editor.merge_layer_up(upper));
            });
        assert!(
            delta
                .cells
                .iter()
                .any(|change| change.layer == base && change.column == 2)
        );
        let mut history = EditHistory::default();
        assert!(history.record_change(before, after, delta));

        restore(&mut editor, history.undo().unwrap());
        assert_eq!(editor.layer_summaries().len(), 2);
        assert!(editor.select_layer(base));
        assert_eq!(
            editor.lines_for_test()[0]
                .iter()
                .map(|atom| atom.contents.as_str())
                .collect::<String>(),
            "A"
        );
        assert!(editor.select_layer(upper));
        assert_eq!(
            editor.lines_for_test()[0]
                .iter()
                .map(|atom| atom.contents.as_str())
                .collect::<String>(),
            "  B"
        );
    }
}
