use super::{EditorState, atom_width, blank_atom, display_width, index_for_column};
use crate::model::{Atom, Coord, Direction};
use crate::toolbar::UtilityKind;

impl EditorState {
    pub fn apply_utility(&mut self, direction: Direction) -> bool {
        super::grid::expand_blank_runs(&mut self.grid.lines);
        match self.toolbar.utility_kind() {
            UtilityKind::Move => false,
            UtilityKind::Push => self.push_blank(direction),
            UtilityKind::Pull => self.pull_all(direction),
            UtilityKind::View => false,
        }
    }

    fn push_blank(&mut self, direction: Direction) -> bool {
        match direction {
            Direction::Left if self.grid.cursor_pos.column == 0 => {
                self.prepend_column();
                true
            }
            Direction::Up if self.grid.cursor_pos.line == 0 => {
                self.prepend_line();
                true
            }
            Direction::Left | Direction::Right => {
                let column = if direction == Direction::Left {
                    self.grid.cursor_pos.column - 1
                } else {
                    self.grid.cursor_pos.column.saturating_add(1)
                };
                self.insert_blank_column(column)
            }
            Direction::Up | Direction::Down => {
                let line = if direction == Direction::Up {
                    self.grid.cursor_pos.line - 1
                } else {
                    self.grid.cursor_pos.line.saturating_add(1)
                };
                self.insert_blank_line(line)
            }
        }
    }

    fn pull_all(&mut self, direction: Direction) -> bool {
        match direction {
            Direction::Left => self.pull_column_left(),
            Direction::Right => self.pull_column_right(),
            Direction::Up => self.pull_row_up(),
            Direction::Down => self.pull_row_down(),
        }
    }

    fn insert_blank_column(&mut self, column: usize) -> bool {
        let mut indices = Vec::with_capacity(self.grid.lines.len());
        for line in &self.grid.lines {
            let Some(index) = boundary_index(line, column) else {
                return false;
            };
            indices.push(index);
        }
        for (line, index) in self.grid.lines.iter_mut().zip(indices) {
            let width = display_width(line);
            line.extend((width..column).map(|_| blank_atom()));
            line.insert(index + column.saturating_sub(width), blank_atom());
        }
        self.map_coordinate_state(|mut coord| {
            if coord.column >= column {
                coord.column = coord.column.saturating_add(1);
            }
            coord
        });
        true
    }

    fn insert_blank_line(&mut self, line: usize) -> bool {
        while self.grid.lines.len() < line {
            self.grid.lines.push(Vec::new());
        }
        self.grid.lines.insert(line, Vec::new());
        self.map_coordinate_state(|mut coord| {
            if coord.line >= line {
                coord.line = coord.line.saturating_add(1);
            }
            coord
        });
        true
    }

    fn pull_column_left(&mut self) -> bool {
        let column = self.grid.cursor_pos.column.saturating_add(1);
        let slots = self
            .grid
            .lines
            .iter()
            .map(|line| cell_slot(line, column))
            .collect::<Vec<_>>();
        if slots.contains(&CellSlot::Interior) {
            return false;
        }
        let removed = slots
            .iter()
            .map(|slot| matches!(slot, CellSlot::Exact(_)))
            .collect::<Vec<_>>();
        if !removed.iter().any(|removed| *removed) {
            return false;
        }

        for (line, slot) in self.grid.lines.iter_mut().zip(slots) {
            if let CellSlot::Exact(index) = slot {
                line.remove(index);
            }
        }
        self.remap_after_pull(
            |coord| removed.get(coord.line).copied().unwrap_or(false) && coord.column == column,
            |mut coord| {
                if removed.get(coord.line).copied().unwrap_or(false) && coord.column > column {
                    coord.column -= 1;
                }
                coord
            },
        );
        true
    }

    fn pull_column_right(&mut self) -> bool {
        let column = self.grid.cursor_pos.column.saturating_add(1);
        let slots = self
            .grid
            .lines
            .iter()
            .map(|line| cell_slot(line, column))
            .collect::<Vec<_>>();
        if slots.contains(&CellSlot::Interior) {
            return false;
        }
        let affected = self
            .grid
            .lines
            .iter()
            .map(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if !affected.iter().any(|affected| *affected) {
            return false;
        }

        let old_lines = self.grid.lines.clone();
        for ((line, slot), affected) in self.grid.lines.iter_mut().zip(slots).zip(&affected) {
            if !affected {
                continue;
            }
            if let CellSlot::Exact(index) = slot {
                line.remove(index);
            }
            line.insert(0, blank_atom());
        }
        if self.grid.lines == old_lines {
            return false;
        }
        self.remap_after_pull(
            |coord| affected.get(coord.line).copied().unwrap_or(false) && coord.column == column,
            |mut coord| {
                if affected.get(coord.line).copied().unwrap_or(false) && coord.column < column {
                    coord.column = coord.column.saturating_add(1);
                }
                coord
            },
        );
        true
    }

    fn pull_row_up(&mut self) -> bool {
        let target = self.grid.cursor_pos.line.saturating_add(1);
        if target >= self.grid.lines.len() {
            return false;
        }
        self.grid.lines.remove(target);
        self.remap_after_pull(
            |coord| coord.line == target,
            |mut coord| {
                if coord.line > target {
                    coord.line -= 1;
                }
                coord
            },
        );
        true
    }

    fn pull_row_down(&mut self) -> bool {
        let cursor_line = self.grid.cursor_pos.line;
        if cursor_line == 0 {
            if self.grid.lines.iter().all(Vec::is_empty)
                && self.line_markers.is_empty()
                && self.active_stroke.is_none()
            {
                return false;
            }
            self.prepend_line();
            self.shape_preview = None;
            return true;
        }

        let target = cursor_line - 1;
        if target >= self.grid.lines.len() {
            return false;
        }
        if self.grid.lines[..=target].iter().all(Vec::is_empty)
            && !self
                .line_markers
                .iter()
                .any(|marker| marker.coord.line <= target)
        {
            return false;
        }
        self.grid.lines.remove(target);
        self.grid.lines.insert(0, Vec::new());
        self.remap_after_pull(
            |coord| coord.line == target,
            |mut coord| {
                if coord.line < target {
                    coord.line = coord.line.saturating_add(1);
                }
                coord
            },
        );
        true
    }

    fn remap_after_pull(
        &mut self,
        deleted: impl Fn(Coord) -> bool,
        mut map: impl FnMut(Coord) -> Coord,
    ) {
        self.grid.cursor_pos = map(self.grid.cursor_pos);
        self.selection
            .select(map(self.selection.anchor()), map(self.selection.active()));
        self.active_stroke = self.active_stroke.take().and_then(|mut stroke| {
            if deleted(stroke.end) {
                None
            } else {
                stroke.end = map(stroke.end);
                Some(stroke)
            }
        });
        self.line_markers.retain(|marker| !deleted(marker.coord));
        for marker in &mut self.line_markers {
            marker.coord = map(marker.coord);
        }
        self.shape_preview = None;
        self.cursor_index = index_for_column(
            &self.grid.lines[self.grid.cursor_pos.line],
            self.grid.cursor_pos.column,
        );
    }

    fn map_coordinate_state(&mut self, mut map: impl FnMut(Coord) -> Coord) {
        self.grid.cursor_pos = map(self.grid.cursor_pos);
        self.selection
            .select(map(self.selection.anchor()), map(self.selection.active()));
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end = map(stroke.end);
        }
        for marker in &mut self.line_markers {
            marker.coord = map(marker.coord);
        }
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.anchor = map(preview.anchor);
            preview.end = map(preview.end);
        }
        self.cursor_index = index_for_column(
            &self.grid.lines[self.grid.cursor_pos.line],
            self.grid.cursor_pos.column,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellSlot {
    Exact(usize),
    Interior,
    Implicit,
}

fn boundary_index(line: &[Atom], target: usize) -> Option<usize> {
    let mut column: usize = 0;
    for (index, atom) in line.iter().enumerate() {
        if column == target {
            return Some(index);
        }
        let end = column.saturating_add(atom_width(atom));
        if target < end {
            return None;
        }
        column = end;
    }
    (target >= column).then_some(line.len())
}

fn cell_slot(line: &[Atom], target: usize) -> CellSlot {
    let mut column: usize = 0;
    for (index, atom) in line.iter().enumerate() {
        let width = atom_width(atom);
        let end = column.saturating_add(width);
        if target == column {
            return if width == 1 {
                CellSlot::Exact(index)
            } else {
                CellSlot::Interior
            };
        }
        if target < end {
            return CellSlot::Interior;
        }
        column = end;
    }
    CellSlot::Implicit
}
