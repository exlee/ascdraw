use super::{Editor, atom_width};
use crate::model::{Coord, Direction, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH, StyledAtom};
use crate::toolbar::UtilityKind;

impl Editor {
    pub fn apply_utility(&mut self, direction: Direction) -> bool {
        self.commit_canvas();
        match self.toolbar.utility_kind() {
            UtilityKind::Push => self.push_blank(direction),
            UtilityKind::Pull => self.pull_all(direction),
            UtilityKind::View => false,
        }
    }

    fn push_blank(&mut self, direction: Direction) -> bool {
        match direction {
            Direction::Left if self.grid.cursor_pos.column == 0 => self.prepend_column(),
            Direction::Up if self.grid.cursor_pos.line == 0 => self.prepend_line(),
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
        if self.canvas_width() >= MAX_CANVAS_WIDTH {
            return false;
        }
        let height = self
            .layer_views()
            .into_iter()
            .map(|layer| layer.lines.len())
            .max()
            .unwrap_or(1);
        if self.layer_views().into_iter().any(|layer| {
            layer
                .lines
                .iter()
                .any(|line| boundary_index(line, column).is_none())
        }) {
            return false;
        }
        self.commit_canvas();
        self.canvas
            .insert_column_in_all_layers(column, height)
            .expect("inserted column fits the sparse canvas");
        self.map_global_coordinate_state(|mut coord| {
            if coord.column >= column {
                coord.column = coord.column.saturating_add(1);
            }
            coord
        });
        true
    }

    fn insert_blank_line(&mut self, line: usize) -> bool {
        if self.canvas_height() >= MAX_CANVAS_HEIGHT {
            return false;
        }
        self.commit_canvas();
        self.canvas
            .insert_row_in_all_layers(line)
            .expect("inserted row fits the sparse canvas");
        self.map_global_coordinate_state(|mut coord| {
            if coord.line >= line {
                coord.line = coord.line.saturating_add(1);
            }
            coord
        });
        true
    }

    fn pull_column_left(&mut self) -> bool {
        let column = self.grid.cursor_pos.column.saturating_add(1);
        let height = self
            .layer_views()
            .into_iter()
            .map(|layer| layer.lines.len())
            .max()
            .unwrap_or(0);
        let views = self.layer_views();
        if views.iter().any(|layer| {
            layer
                .lines
                .iter()
                .any(|line| cell_slot(line, column) == CellSlot::Interior)
        }) {
            return false;
        }
        let removed = (0..height)
            .map(|row| {
                views.iter().any(|layer| {
                    layer
                        .lines
                        .get(row)
                        .is_some_and(|line| matches!(cell_slot(line, column), CellSlot::Exact(_)))
                })
            })
            .collect::<Vec<_>>();
        if !removed.iter().any(|removed| *removed) {
            return false;
        }

        self.commit_canvas();
        self.canvas
            .pull_column_left_in_all_layers(column, &removed)
            .expect("pulled columns fit the sparse canvas");
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
        let views = self.layer_views();
        if views.iter().any(|layer| {
            layer
                .lines
                .iter()
                .any(|line| cell_slot(line, column) == CellSlot::Interior)
        }) {
            return false;
        }
        let height = views
            .iter()
            .map(|layer| layer.lines.len())
            .max()
            .unwrap_or(0);
        let affected = (0..height)
            .map(|row| {
                views
                    .iter()
                    .any(|layer| layer.lines.get(row).is_some_and(|line| !line.is_empty()))
            })
            .collect::<Vec<_>>();
        if !affected.iter().any(|affected| *affected) {
            return false;
        }

        self.commit_canvas();
        self.canvas
            .pull_column_right_in_all_layers(column, &affected)
            .expect("pulled columns fit the sparse canvas");
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
        if self
            .layer_views()
            .into_iter()
            .all(|layer| target >= layer.lines.len())
        {
            return false;
        }
        self.commit_canvas();
        self.canvas
            .remove_row_in_all_layers(target)
            .expect("pulled rows fit the sparse canvas");
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
            let has_content = self
                .layer_views()
                .into_iter()
                .any(|layer| layer.lines.iter().any(|line| !line.is_empty()));
            let has_markers = self
                .layer_contents()
                .into_iter()
                .any(|(_, _, markers)| !markers.is_empty());
            if !has_content && !has_markers && self.active_stroke.is_none() {
                return false;
            }
            if !self.prepend_line() {
                return false;
            }
            self.shape_preview = None;
            return true;
        }

        let target = cursor_line - 1;
        let has_content = self.layer_views().into_iter().any(|layer| {
            layer
                .lines
                .iter()
                .take(target.saturating_add(1))
                .any(|line| !line.is_empty())
        });
        let has_markers = self
            .layer_contents()
            .into_iter()
            .flat_map(|(_, _, markers)| markers)
            .any(|marker| marker.coord.line <= target);
        if !has_content && !has_markers {
            return false;
        }
        self.commit_canvas();
        self.canvas
            .remove_row_and_prepend_blank_in_all_layers(target)
            .expect("pulled rows fit the sparse canvas");
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
        self.shape_preview = None;
    }

    fn map_global_coordinate_state(&mut self, mut map: impl FnMut(Coord) -> Coord) {
        self.grid.cursor_pos = map(self.grid.cursor_pos);
        self.selection
            .select(map(self.selection.anchor()), map(self.selection.active()));
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end = map(stroke.end);
        }
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.anchor = map(preview.anchor);
            preview.end = map(preview.end);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellSlot {
    Exact(usize),
    Interior,
    Implicit,
}

fn boundary_index(line: &[StyledAtom], target: usize) -> Option<usize> {
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

fn cell_slot(line: &[StyledAtom], target: usize) -> CellSlot {
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
