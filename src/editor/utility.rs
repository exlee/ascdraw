use super::{EditorState, atom_width, blank_atom, display_width, index_for_column};
use crate::model::{Atom, Coord, Direction};
use crate::toolbar::UtilityKind;

impl EditorState {
    pub fn apply_utility(&mut self, direction: Direction) -> bool {
        match self.toolbar.utility_kind() {
            UtilityKind::Select => false,
            UtilityKind::Push => self.push_blank(direction),
            UtilityKind::Pull => self.pull_blank(direction),
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

    fn pull_blank(&mut self, direction: Direction) -> bool {
        match direction {
            Direction::Left => self.pull_column_left(),
            Direction::Right => self.pull_column_right(),
            Direction::Up => self.pull_columns_up(),
            Direction::Down if self.grid.cursor_pos.line == 0 => {
                self.prepend_line();
                true
            }
            Direction::Down => self.pull_columns_down(),
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
        let mut eligible = vec![false; self.grid.lines.len()];
        for (line_index, line) in self.grid.lines.iter_mut().enumerate() {
            if let Some(index) = explicit_blank_index(line, column) {
                line.remove(index);
                eligible[line_index] = true;
            }
        }
        if !eligible.iter().any(|eligible| *eligible) {
            return false;
        }
        self.map_coordinate_state(|mut coord| {
            if eligible.get(coord.line).copied().unwrap_or(false) && coord.column > column {
                coord.column -= 1;
            }
            coord
        });
        true
    }

    fn pull_column_right(&mut self) -> bool {
        let column = self.grid.cursor_pos.column.saturating_add(1);
        let mut eligible = vec![false; self.grid.lines.len()];
        for (line_index, line) in self.grid.lines.iter_mut().enumerate() {
            if let Some(index) = explicit_blank_index(line, column) {
                line.remove(index);
                line.insert(0, blank_atom());
                eligible[line_index] = true;
            }
        }
        if !eligible.iter().any(|eligible| *eligible) {
            return false;
        }
        self.map_coordinate_state(|mut coord| {
            if eligible.get(coord.line).copied().unwrap_or(false) && coord.column < column {
                coord.column = coord.column.saturating_add(1);
            }
            coord
        });
        true
    }

    fn pull_columns_up(&mut self) -> bool {
        let target = self.grid.cursor_pos.line.saturating_add(1);
        if target + 1 >= self.grid.lines.len() {
            return false;
        }
        let affected = self.eligible_vertical_columns(target, target + 1..self.grid.lines.len());
        if affected.is_empty() {
            return false;
        }
        let last = self.grid.lines.len() - 1;
        for column in &affected {
            for line in target..last {
                let atom = cell_atom(&self.grid.lines[line + 1], *column).cloned();
                set_cell_value(&mut self.grid.lines[line], *column, atom);
            }
            set_cell_value(&mut self.grid.lines[last], *column, None);
        }
        self.map_coordinate_state(|mut coord| {
            if affected.contains(&coord.column) && coord.line > target {
                coord.line -= 1;
            }
            coord
        });
        true
    }

    fn pull_columns_down(&mut self) -> bool {
        let target = self.grid.cursor_pos.line - 1;
        if target == 0 {
            return false;
        }
        let affected = self.eligible_vertical_columns(target, 0..target);
        if affected.is_empty() {
            return false;
        }
        for column in &affected {
            for line in (1..=target).rev() {
                let atom = cell_atom(&self.grid.lines[line - 1], *column).cloned();
                set_cell_value(&mut self.grid.lines[line], *column, atom);
            }
            set_cell_value(&mut self.grid.lines[0], *column, None);
        }
        self.map_coordinate_state(|mut coord| {
            if affected.contains(&coord.column) && coord.line < target {
                coord.line = coord.line.saturating_add(1);
            }
            coord
        });
        true
    }

    fn eligible_vertical_columns(
        &self,
        target: usize,
        source_lines: std::ops::Range<usize>,
    ) -> Vec<usize> {
        let Some(target_line) = self.grid.lines.get(target) else {
            return Vec::new();
        };
        let candidates = blank_columns(target_line);
        let validation_start = source_lines.start.min(target);
        let validation_end = source_lines.end.saturating_sub(1).max(target);
        candidates
            .into_iter()
            .filter(|column| {
                source_lines.clone().any(|line| {
                    self.grid
                        .lines
                        .get(line)
                        .and_then(|line| cell_atom(line, *column))
                        .is_some_and(|atom| !atom.contents.chars().all(char::is_whitespace))
                })
            })
            .filter(|column| {
                (validation_start..=validation_end).all(|line| {
                    self.grid
                        .lines
                        .get(line)
                        .map_or(CellSlot::Implicit, |line| cell_slot(line, *column))
                        != CellSlot::Interior
                })
            })
            .collect()
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

fn cell_atom(line: &[Atom], column: usize) -> Option<&Atom> {
    match cell_slot(line, column) {
        CellSlot::Exact(index) => line.get(index),
        CellSlot::Interior | CellSlot::Implicit => None,
    }
}

fn explicit_blank_index(line: &[Atom], column: usize) -> Option<usize> {
    match cell_slot(line, column) {
        CellSlot::Exact(index) if line[index].contents.chars().all(char::is_whitespace) => {
            Some(index)
        }
        CellSlot::Exact(_) | CellSlot::Interior | CellSlot::Implicit => None,
    }
}

fn blank_columns(line: &[Atom]) -> Vec<usize> {
    let mut columns = Vec::new();
    let mut column: usize = 0;
    for atom in line {
        let width = atom_width(atom);
        if width == 1 && atom.contents.chars().all(char::is_whitespace) {
            columns.push(column);
        }
        column = column.saturating_add(width);
    }
    columns
}

fn set_cell_value(line: &mut Vec<Atom>, column: usize, atom: Option<Atom>) {
    match (cell_slot(line, column), atom) {
        (CellSlot::Exact(index), Some(atom)) => line[index] = atom,
        (CellSlot::Exact(index), None) => line[index] = blank_atom(),
        (CellSlot::Implicit, Some(atom)) => {
            let width = display_width(line);
            line.extend((width..column).map(|_| blank_atom()));
            line.push(atom);
        }
        (CellSlot::Implicit, None) => {}
        (CellSlot::Interior, _) => {
            unreachable!("wide grapheme columns are filtered before transforms")
        }
    }
}
