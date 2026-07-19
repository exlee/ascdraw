use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::CursorMode;
use crate::model::{Atom, Coord, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH};

use super::{Editor, atom_width};

impl Editor {
    pub fn insert(&mut self, text: &str) {
        if !self.validate_text_cells(text) {
            return;
        }
        self.end_stroke();
        self.commit_canvas();
        for part in text.split_inclusive('\n') {
            let content = part.strip_suffix('\n').unwrap_or(part);
            let atoms = UnicodeSegmentation::graphemes(content, true)
                .map(|contents| Atom {
                    face: self.write_face(),
                    contents: contents.to_string(),
                })
                .collect::<Vec<_>>();
            let available = MAX_CANVAS_WIDTH.saturating_sub(
                self.canvas
                    .active_row_width(self.grid.cursor_pos.line)
                    .max(self.grid.cursor_pos.column),
            );
            let mut accepted_width: usize = 0;
            let atoms = atoms
                .into_iter()
                .take_while(|atom| {
                    let next = accepted_width.saturating_add(atom_width(atom));
                    if next > available {
                        return false;
                    }
                    accepted_width = next;
                    true
                })
                .collect::<Vec<_>>();
            let inserted_width = atoms.len();
            let line = self.grid.cursor_pos.line;
            let column = self.grid.cursor_pos.column;
            let cells = atoms
                .into_iter()
                .map(|atom| {
                    let face = atom.face.clone();
                    (atom, face)
                })
                .collect();
            self.canvas
                .insert_cells(line, column, cells)
                .expect("validated text fits the sparse canvas");
            self.grid.cursor_pos.column = self
                .grid
                .cursor_pos
                .column
                .saturating_add(inserted_width)
                .min(MAX_CANVAS_WIDTH - 1);
            if part.ends_with('\n')
                && self.canvas.layers()[self.canvas.active_index()]
                    .to_dense()
                    .len()
                    < MAX_CANVAS_HEIGHT
            {
                self.newline_sparse();
            }
        }
        self.refresh_active_dense_view();
        self.collapse_selection();
    }

    pub fn write_text(&mut self, text: &str) {
        if !self.validate_text_cells(text) {
            return;
        }
        if self.single_replace_pending {
            self.replace_once(text);
        } else if self.cursor_mode == CursorMode::Replace {
            self.replace(text);
        } else {
            self.insert(text);
        }
    }

    pub fn paste_text(&mut self, text: &str) -> bool {
        if !self.validate_text_cells(text) {
            return false;
        }
        if self.single_replace_pending {
            if UnicodeSegmentation::graphemes(text, true).next().is_none() {
                return false;
            }
            self.replace_once(text);
            true
        } else {
            self.paste_text_rectangle(text)
        }
    }

    fn validate_text_cells(&mut self, text: &str) -> bool {
        let valid = text.split('\n').all(|line| {
            UnicodeSegmentation::graphemes(line, true)
                .all(|grapheme| UnicodeWidthStr::width(grapheme) == 1)
        });
        if !valid {
            self.invalid_text_tip();
        }
        valid
    }

    fn replace_once(&mut self, text: &str) {
        let Some(grapheme) = UnicodeSegmentation::graphemes(text, true).next() else {
            return;
        };
        self.end_stroke();
        self.replace_selection_literal(Some(grapheme));
        self.sync_cursor_mode_with_toolbar();
        self.restore_active_cursor();
    }

    fn replace(&mut self, text: &str) {
        self.end_stroke();
        self.commit_canvas();
        for part in text.split_inclusive('\n') {
            let content = part.strip_suffix('\n').unwrap_or(part);
            for grapheme in UnicodeSegmentation::graphemes(content, true) {
                let atom = Atom {
                    face: self.write_face(),
                    contents: grapheme.to_string(),
                };
                let line = self.grid.cursor_pos.line;
                let column = self.grid.cursor_pos.column;
                if column >= MAX_CANVAS_WIDTH {
                    break;
                }
                let face = atom.face.clone();
                self.canvas.remove_line_at(Coord { line, column });
                self.canvas
                    .set_at(Coord { line, column }, atom, &face)
                    .expect("validated replacement fits the sparse canvas");
                self.canvas
                    .ensure_active_row_width(line, column.saturating_add(1));
                self.grid.cursor_pos.column = column.saturating_add(1);
            }
            if part.ends_with('\n')
                && self.canvas.layers()[self.canvas.active_index()]
                    .to_dense()
                    .len()
                    < MAX_CANVAS_HEIGHT
            {
                self.newline_sparse();
            }
        }
        self.refresh_active_dense_view();
        self.collapse_selection();
    }

    pub fn newline(&mut self) {
        self.end_stroke();
        self.commit_canvas();
        if self.grid.lines.len() >= MAX_CANVAS_HEIGHT {
            return;
        }
        self.newline_sparse();
        self.refresh_active_dense_view();
        self.collapse_selection();
    }

    fn newline_sparse(&mut self) {
        let line = self.grid.cursor_pos.line;
        let column = self.grid.cursor_pos.column;
        self.canvas
            .split_row(line, column)
            .expect("text newline fits the sparse canvas");
        self.grid.cursor_pos.line = line.saturating_add(1);
        self.grid.cursor_pos.column = 0;
    }

    pub fn backspace(&mut self) {
        self.end_stroke();
        self.commit_canvas();
        if self.grid.cursor_pos.column > 0 {
            self.grid.cursor_pos.column -= 1;
            self.canvas
                .remove_cells(self.grid.cursor_pos.line, self.grid.cursor_pos.column, 1)
                .expect("backspace remains inside the sparse canvas");
        } else if self.grid.cursor_pos.line > 0 {
            let removed_line = self.grid.cursor_pos.line;
            let previous = removed_line - 1;
            let join_column = self.canvas.active_row_width(previous);
            if join_column.saturating_add(self.canvas.active_row_width(removed_line))
                > MAX_CANVAS_WIDTH
            {
                return;
            }
            self.canvas
                .join_row_with_next(previous)
                .expect("joined text rows fit the sparse canvas");
            self.grid.cursor_pos.line = previous;
            self.grid.cursor_pos.column = join_column;
        }
        self.refresh_active_dense_view();
        self.collapse_selection();
    }

    pub fn delete(&mut self) {
        self.end_stroke();
        self.commit_canvas();
        let line = self.grid.cursor_pos.line;
        let column = self.grid.cursor_pos.column;
        let width = self.canvas.active_row_width(line);
        if column < width {
            self.canvas
                .remove_cells(line, column, 1)
                .expect("delete remains inside the sparse canvas");
        } else if line + 1 < self.grid.lines.len() {
            if width.saturating_add(self.canvas.active_row_width(line + 1)) > MAX_CANVAS_WIDTH {
                return;
            }
            self.canvas
                .join_row_with_next(line)
                .expect("joined text rows fit the sparse canvas");
        }
        self.refresh_active_dense_view();
        self.collapse_selection();
    }
}
