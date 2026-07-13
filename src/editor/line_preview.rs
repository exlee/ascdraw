use crate::app::CursorMode;
use crate::model::{Atom, Coord, Direction};
use crate::selection::CanvasSelection;

use super::{EditorState, PlacedLineMarker, adjacent_coord, blank_atom, index_for_column};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LinePreview {
    anchors: Vec<Coord>,
    end: Coord,
    pub(super) source_lines: Vec<Vec<Atom>>,
    pub(super) source_markers: Vec<PlacedLineMarker>,
    pub(super) source_cursor: Coord,
    pub(super) source_cursor_index: usize,
    pub(super) source_selection: CanvasSelection,
    prepended_columns: usize,
    prepended_lines: usize,
}

impl LinePreview {
    pub(super) fn shift(&mut self, columns: usize, lines: usize) {
        for anchor in &mut self.anchors {
            anchor.column = anchor.column.saturating_add(columns);
            anchor.line = anchor.line.saturating_add(lines);
        }
        self.end.column = self.end.column.saturating_add(columns);
        self.end.line = self.end.line.saturating_add(lines);
        self.prepended_columns = self.prepended_columns.saturating_add(columns);
        self.prepended_lines = self.prepended_lines.saturating_add(lines);
    }
}

impl EditorState {
    pub fn has_line_preview(&self) -> bool {
        self.line_preview.is_some()
    }

    pub fn start_or_advance_line_preview(&mut self) -> bool {
        if self.cursor_mode != CursorMode::MoveDraw {
            return false;
        }
        let Some(preview) = self.line_preview.as_mut() else {
            self.end_stroke();
            self.shape_preview = None;
            self.cancel_move_lift();
            self.toolbar.cancel_shortcut();
            let source_selection = self.selection;
            self.selection.collapse(self.grid.cursor_pos);
            self.line_preview = Some(LinePreview {
                anchors: vec![self.grid.cursor_pos],
                end: self.grid.cursor_pos,
                source_lines: self.grid.lines.clone(),
                source_markers: self.line_markers.clone(),
                source_cursor: self.grid.cursor_pos,
                source_cursor_index: self.cursor_index,
                source_selection,
                prepended_columns: 0,
                prepended_lines: 0,
            });
            return false;
        };

        let anchor = *preview
            .anchors
            .last()
            .expect("line previews always have an initial anchor");
        if preview.end != anchor {
            preview.anchors.push(preview.end);
            return false;
        }
        self.confirm_line_preview()
    }

    pub fn move_line_preview(&mut self, direction: Direction) -> bool {
        let Some(preview) = self.line_preview.as_ref() else {
            return false;
        };
        let anchor = *preview
            .anchors
            .last()
            .expect("line previews always have an initial anchor");
        if !segment_accepts_direction(anchor, preview.end, direction) {
            return false;
        }

        let prepended = self.prepare_adjacent(direction);
        let to = adjacent_coord(self.grid.cursor_pos, direction)
            .expect("canvas edge was structurally extended");
        self.grid.cursor_pos = to;
        self.cursor_index = self
            .grid
            .lines
            .get(to.line)
            .map_or(0, |line| index_for_column(line, to.column));
        self.selection.collapse(self.grid.cursor_pos);
        self.line_preview
            .as_mut()
            .expect("preview remains active while moving")
            .end = self.grid.cursor_pos;
        prepended
    }

    pub fn remove_line_preview_anchor(&mut self) -> bool {
        let Some(preview) = self.line_preview.as_mut() else {
            return false;
        };
        if preview.anchors.len() == 1 {
            return false;
        }
        preview.anchors.pop();
        let target = *preview
            .anchors
            .last()
            .expect("the initial line anchor is retained");
        preview.end = target;
        self.grid.cursor_pos = target;
        self.cursor_index = self
            .grid
            .lines
            .get(target.line)
            .map_or(0, |line| index_for_column(line, target.column));
        self.selection.collapse(target);
        false
    }

    pub fn cancel_line_preview(&mut self) -> bool {
        let Some(preview) = self.line_preview.take() else {
            return false;
        };
        self.grid.lines = preview.source_lines;
        self.line_markers = preview.source_markers;
        self.grid.cursor_pos = preview.source_cursor;
        self.cursor_index = preview.source_cursor_index;
        self.selection = preview.source_selection;
        true
    }

    pub(super) fn shift_line_preview(&mut self, columns: usize, lines: usize) {
        if let Some(preview) = self.line_preview.as_mut() {
            preview.shift(columns, lines);
        }
    }

    pub(super) fn lines_with_line_preview(&self) -> Option<Vec<Vec<Atom>>> {
        Some(self.composed_line_preview_state()?.grid.lines)
    }

    fn confirm_line_preview(&mut self) -> bool {
        let Some(composed) = self.composed_line_preview_state() else {
            return false;
        };
        let preview = self
            .line_preview
            .as_ref()
            .expect("composed state requires an active preview");
        let changed = preview.source_lines != composed.grid.lines
            || preview.source_markers != composed.line_markers;
        self.grid.lines = composed.grid.lines;
        self.grid.cursor_pos = composed.grid.cursor_pos;
        self.cursor_index = composed.cursor_index;
        self.selection.collapse(self.grid.cursor_pos);
        self.line_markers = composed.line_markers;
        self.active_stroke = None;
        self.line_preview = None;
        changed
    }

    fn composed_line_preview_state(&self) -> Option<Self> {
        let preview = self.line_preview.as_ref()?;
        let start = *preview
            .anchors
            .first()
            .expect("line previews always have an initial anchor");
        let mut composed = self.clone();
        composed.line_preview = None;
        composed.shape_preview = None;
        composed.move_lift = None;
        composed.active_stroke = None;
        composed.grid.lines = preview.source_lines.clone();
        composed.line_markers = preview.source_markers.clone();
        for _ in 0..preview.prepended_lines {
            composed.grid.lines.insert(0, Vec::new());
        }
        for line in &mut composed.grid.lines {
            for _ in 0..preview.prepended_columns {
                line.insert(0, blank_atom());
            }
        }
        for marker in &mut composed.line_markers {
            marker.coord.line = marker.coord.line.saturating_add(preview.prepended_lines);
            marker.coord.column = marker
                .coord
                .column
                .saturating_add(preview.prepended_columns);
        }
        composed.grid.cursor_pos = start;
        composed.cursor_index = index_for_column(&composed.grid.lines[start.line], start.column);
        composed.selection.collapse(start);

        for end in preview
            .anchors
            .iter()
            .skip(1)
            .copied()
            .chain(std::iter::once(preview.end))
        {
            draw_orthogonal_segment(&mut composed, end);
        }
        composed.end_stroke();
        Some(composed)
    }
}

fn segment_accepts_direction(anchor: Coord, end: Coord, direction: Direction) -> bool {
    if end == anchor {
        return true;
    }
    match direction {
        Direction::Left | Direction::Right => end.line == anchor.line,
        Direction::Up | Direction::Down => end.column == anchor.column,
    }
}

fn draw_orthogonal_segment(state: &mut EditorState, end: Coord) {
    let start = state.grid.cursor_pos;
    let (direction, steps) = if end.line == start.line {
        if end.column >= start.column {
            (Direction::Right, end.column - start.column)
        } else {
            (Direction::Left, start.column - end.column)
        }
    } else {
        debug_assert_eq!(end.column, start.column);
        if end.line >= start.line {
            (Direction::Down, end.line - start.line)
        } else {
            (Direction::Up, start.line - end.line)
        }
    };
    for _ in 0..steps {
        state.move_or_draw(direction, true);
    }
}
