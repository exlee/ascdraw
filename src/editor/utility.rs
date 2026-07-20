use std::collections::BTreeSet;

use super::Editor;
use crate::model::{Coord, Direction, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH};
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

    fn insert_blank_column(&mut self, column: i16) -> bool {
        if self.canvas_width() >= MAX_CANVAS_WIDTH {
            return false;
        }
        self.commit_canvas();
        self.canvas
            .insert_column_in_all_layers(column)
            .expect("inserted column fits the sparse canvas");
        self.map_global_coordinate_state(|mut coord| {
            if coord.column >= column {
                coord.column = coord.column.saturating_add(1);
            }
            coord
        });
        true
    }

    fn insert_blank_line(&mut self, line: i16) -> bool {
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
        let removed =
            self.canvas
                .layers()
                .iter()
                .flat_map(|layer| {
                    layer.rows().iter().filter_map(|(&line, row)| {
                        row.range(column..).next().is_some().then_some(line)
                    })
                })
                .collect::<BTreeSet<_>>();
        if removed.is_empty() {
            return false;
        }

        self.commit_canvas();
        self.canvas
            .pull_column_left_in_all_layers(column, &removed)
            .expect("pulled columns fit the sparse canvas");
        self.remap_after_pull(
            |coord| removed.contains(&coord.line) && coord.column == column,
            |mut coord| {
                if removed.contains(&coord.line) && coord.column > column {
                    coord.column -= 1;
                }
                coord
            },
        );
        true
    }

    fn pull_column_right(&mut self) -> bool {
        let column = self.grid.cursor_pos.column.saturating_add(1);
        let affected = self
            .canvas
            .layers()
            .iter()
            .flat_map(|layer| layer.rows().keys().copied())
            .collect::<BTreeSet<_>>();
        if affected.is_empty() {
            return false;
        }

        self.commit_canvas();
        self.canvas
            .pull_column_right_in_all_layers(column, &affected)
            .expect("pulled columns fit the sparse canvas");
        self.remap_after_pull(
            |coord| affected.contains(&coord.line) && coord.column == column,
            |mut coord| {
                if affected.contains(&coord.line) && coord.column < column {
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
            .canvas
            .layers()
            .iter()
            .all(|layer| layer.rows().range(target..).next().is_none())
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
        let target = cursor_line - 1;
        if self
            .canvas
            .layers()
            .iter()
            .all(|layer| layer.rows().range(..=target).next_back().is_none())
        {
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
