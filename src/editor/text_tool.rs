use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::CursorMode;
use crate::model::{Atom, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH};

use super::{Editor, atom_width, display_width};

impl Editor {
    pub fn insert(&mut self, text: &str) {
        if !self.validate_text_cells(text) {
            return;
        }
        self.end_stroke();
        self.expose_cursor_cells();
        for part in text.split_inclusive('\n') {
            let content = part.strip_suffix('\n').unwrap_or(part);
            let atoms = UnicodeSegmentation::graphemes(content, true)
                .map(|contents| Atom {
                    face: self.write_face(),
                    contents: contents.to_string(),
                })
                .collect::<Vec<_>>();
            let available = MAX_CANVAS_WIDTH
                .saturating_sub(display_width(&self.grid.lines[self.grid.cursor_pos.line]));
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
            let inserted_count = atoms.len();
            let inserted_width = display_width(&atoms);
            let line_index = self.grid.cursor_pos.line;
            let insertion_column = display_width(&self.grid.lines[line_index][..self.cursor_index]);
            self.grid.lines[self.grid.cursor_pos.line]
                .splice(self.cursor_index..self.cursor_index, atoms);
            self.cursor_index = self.grid.lines[self.grid.cursor_pos.line]
                .len()
                .min(self.cursor_index + inserted_count);
            self.remap_line_data_after_edit(line_index, insertion_column, 0, inserted_width);
            if part.ends_with('\n') && self.grid.lines.len() < MAX_CANVAS_HEIGHT {
                self.newline();
            }
        }
        self.sync_cursor_column();
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
        self.restore_active_cursor_index();
    }

    fn replace(&mut self, text: &str) {
        self.end_stroke();
        self.expose_cursor_cells();
        for part in text.split_inclusive('\n') {
            let content = part.strip_suffix('\n').unwrap_or(part);
            for grapheme in UnicodeSegmentation::graphemes(content, true) {
                let atom = Atom {
                    face: self.write_face(),
                    contents: grapheme.to_string(),
                };
                let inserted_width = atom_width(&atom);
                let line_index = self.grid.cursor_pos.line;
                let line = &mut self.grid.lines[line_index];
                let replacement_column = display_width(&line[..self.cursor_index]);
                let removed_width = line.get(self.cursor_index).map_or(0, atom_width);
                if display_width(line)
                    .saturating_sub(removed_width)
                    .saturating_add(inserted_width)
                    > MAX_CANVAS_WIDTH
                {
                    break;
                }
                if self.cursor_index < line.len() {
                    line[self.cursor_index] = atom;
                } else {
                    line.push(atom);
                }
                self.cursor_index += 1;
                self.remap_line_data_after_edit(
                    line_index,
                    replacement_column,
                    removed_width,
                    inserted_width,
                );
            }
            if part.ends_with('\n') && self.grid.lines.len() < MAX_CANVAS_HEIGHT {
                self.newline();
            }
        }
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn newline(&mut self) {
        self.end_stroke();
        if self.grid.lines.len() >= MAX_CANVAS_HEIGHT {
            return;
        }
        let source_line = self.grid.cursor_pos.line;
        let split_column = display_width(&self.grid.lines[source_line][..self.cursor_index]);
        let remainder = self.grid.lines[source_line].split_off(self.cursor_index);
        self.grid.cursor_pos.line += 1;
        self.grid.lines.insert(self.grid.cursor_pos.line, remainder);
        self.split_line_data(source_line, split_column);
        self.cursor_index = 0;
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn backspace(&mut self) {
        self.end_stroke();
        self.expose_cursor_cells();
        if self.cursor_index > 0 {
            let line_index = self.grid.cursor_pos.line;
            self.cursor_index -= 1;
            let removal_column = display_width(&self.grid.lines[line_index][..self.cursor_index]);
            let removed_width = atom_width(&self.grid.lines[line_index][self.cursor_index]);
            self.grid.lines[line_index].remove(self.cursor_index);
            self.remap_line_data_after_edit(line_index, removal_column, removed_width, 0);
        } else if self.grid.cursor_pos.line > 0 {
            let removed_line = self.grid.cursor_pos.line;
            if display_width(&self.grid.lines[removed_line - 1])
                .saturating_add(display_width(&self.grid.lines[removed_line]))
                > MAX_CANVAS_WIDTH
            {
                return;
            }
            let current = self.grid.lines.remove(self.grid.cursor_pos.line);
            self.grid.cursor_pos.line -= 1;
            self.cursor_index = self.grid.lines[self.grid.cursor_pos.line].len();
            let join_column = display_width(&self.grid.lines[self.grid.cursor_pos.line]);
            self.grid.lines[self.grid.cursor_pos.line].extend(current);
            self.join_line_data(self.grid.cursor_pos.line, removed_line, join_column);
        }
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn delete(&mut self) {
        self.end_stroke();
        self.expose_cursor_cells();
        let line = self.grid.cursor_pos.line;
        if self.cursor_index < self.grid.lines[line].len() {
            let removal_column = display_width(&self.grid.lines[line][..self.cursor_index]);
            let removed_width = atom_width(&self.grid.lines[line][self.cursor_index]);
            self.grid.lines[line].remove(self.cursor_index);
            self.remap_line_data_after_edit(line, removal_column, removed_width, 0);
        } else if line + 1 < self.grid.lines.len() {
            let removed_line = line + 1;
            if display_width(&self.grid.lines[line])
                .saturating_add(display_width(&self.grid.lines[removed_line]))
                > MAX_CANVAS_WIDTH
            {
                return;
            }
            let join_column = display_width(&self.grid.lines[line]);
            let next = self.grid.lines.remove(line + 1);
            self.grid.lines[line].extend(next);
            self.join_line_data(line, removed_line, join_column);
        }
        self.sync_cursor_column();
        self.collapse_selection();
    }

    fn remap_line_data_after_edit(
        &mut self,
        line: usize,
        column: usize,
        removed_width: usize,
        inserted_width: usize,
    ) {
        let removed_end = column.saturating_add(removed_width);
        self.commit_canvas_with_remapped_line_data(|mut coord| {
            if coord.line != line {
                return Some(coord);
            }
            if coord.column >= column && coord.column < removed_end {
                return None;
            }
            if coord.column >= removed_end {
                coord.column = if inserted_width >= removed_width {
                    coord.column.saturating_add(inserted_width - removed_width)
                } else {
                    coord.column.saturating_sub(removed_width - inserted_width)
                };
            }
            Some(coord)
        });
    }

    fn split_line_data(&mut self, line: usize, column: usize) {
        self.commit_canvas_with_remapped_line_data(|mut coord| {
            if coord.line == line && coord.column >= column {
                coord.line = coord.line.saturating_add(1);
                coord.column -= column;
            } else if coord.line > line {
                coord.line = coord.line.saturating_add(1);
            }
            Some(coord)
        });
    }

    fn join_line_data(&mut self, line: usize, removed_line: usize, join_column: usize) {
        self.commit_canvas_with_remapped_line_data(|mut coord| {
            if coord.line == removed_line {
                coord.line = line;
                coord.column = coord.column.saturating_add(join_column);
            } else if coord.line > removed_line {
                coord.line -= 1;
            }
            Some(coord)
        });
    }
}
