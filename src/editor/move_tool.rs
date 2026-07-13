use crate::app::CursorMode;
use crate::model::{Coord, Direction};
use crate::selection::{
    CanvasSelection, SelectionBounds, TextRectangle, overwrite_rectangle, replace_range,
    selected_atoms,
};
use crate::toolbar::UtilityKind;

use super::{EditorState, PlacedLineMarker, index_for_column};

#[derive(Debug, Clone)]
pub(super) struct MoveLift {
    pub(super) source_selection: CanvasSelection,
    pub(super) source_cursor: Coord,
    pub(super) source_cursor_index: usize,
    source_bounds: SelectionBounds,
    origin: Coord,
    rectangle: TextRectangle,
    markers: Vec<PlacedLineMarker>,
    plain_direction_confirms: bool,
}

impl EditorState {
    pub fn move_lift_active(&self) -> bool {
        self.move_lift.is_some()
    }

    pub fn begin_move_lift(&mut self) -> bool {
        if self.cursor_mode != CursorMode::Utilities
            || self.toolbar.utility_kind() != UtilityKind::Move
        {
            return false;
        }
        self.begin_move_lift_inner(false)
    }

    pub fn begin_selected_move_lift(&mut self) -> bool {
        if self.selection.is_collapsed() {
            return false;
        }
        self.begin_move_lift_inner(true)
    }

    fn begin_move_lift_inner(&mut self, plain_direction_confirms: bool) -> bool {
        if self.move_lift.is_some() {
            return false;
        }
        self.end_stroke();
        self.shape_preview = None;
        self.toolbar.cancel_shortcut();
        let source_selection = self.selection;
        let source_bounds = source_selection.bounds();
        let rectangle = TextRectangle {
            rows: selected_atoms(&self.grid.lines, source_bounds),
            width: source_bounds.width(),
        };
        let markers = self
            .line_markers
            .iter()
            .filter(|marker| source_bounds.contains(marker.coord))
            .cloned()
            .map(|mut marker| {
                marker.coord.line -= source_bounds.top;
                marker.coord.column -= source_bounds.left;
                marker
            })
            .collect();
        self.move_lift = Some(MoveLift {
            source_selection,
            source_cursor: self.grid.cursor_pos,
            source_cursor_index: self.cursor_index,
            source_bounds,
            origin: Coord {
                line: source_bounds.top,
                column: source_bounds.left,
            },
            rectangle,
            markers,
            plain_direction_confirms,
        });
        true
    }

    pub fn move_lift_plain_direction_confirms(&self) -> bool {
        self.move_lift
            .as_ref()
            .is_some_and(|lift| lift.plain_direction_confirms)
    }

    pub fn move_lift(&mut self, direction: Direction) -> bool {
        let Some(lift) = self.move_lift.as_mut() else {
            return false;
        };
        let next = match direction {
            Direction::Up => lift.origin.line.checked_sub(1).map(|line| Coord {
                line,
                column: lift.origin.column,
            }),
            Direction::Left => lift.origin.column.checked_sub(1).map(|column| Coord {
                line: lift.origin.line,
                column,
            }),
            Direction::Down => Some(Coord {
                line: lift.origin.line.saturating_add(1),
                column: lift.origin.column,
            }),
            Direction::Right => Some(Coord {
                line: lift.origin.line,
                column: lift.origin.column.saturating_add(1),
            }),
        };
        let Some(next) = next else {
            return false;
        };
        lift.origin = next;
        let line_delta = next.line as i128 - lift.source_bounds.top as i128;
        let column_delta = next.column as i128 - lift.source_bounds.left as i128;
        self.selection.select(
            offset_coord(lift.source_selection.anchor(), line_delta, column_delta),
            offset_coord(lift.source_selection.active(), line_delta, column_delta),
        );
        self.grid.cursor_pos = offset_coord(lift.source_cursor, line_delta, column_delta);
        self.cursor_index = self
            .grid
            .lines
            .get(self.grid.cursor_pos.line)
            .map_or(0, |line| {
                index_for_column(line, self.grid.cursor_pos.column)
            });
        true
    }

    pub fn confirm_move_lift(&mut self) -> bool {
        let Some(lift) = self.move_lift.take() else {
            return false;
        };
        if lift.origin
            == (Coord {
                line: lift.source_bounds.top,
                column: lift.source_bounds.left,
            })
        {
            return false;
        }
        let before_lines = self.grid.lines.clone();
        let before_markers = self.line_markers.clone();
        let destination = lift.rectangle.bounds_at(lift.origin);
        replace_range(&mut self.grid.lines, lift.source_bounds, None);
        self.line_markers.retain(|marker| {
            !lift.source_bounds.contains(marker.coord) && !destination.contains(marker.coord)
        });
        overwrite_rectangle(&mut self.grid.lines, lift.origin, &lift.rectangle);
        self.line_markers
            .extend(lift.markers.into_iter().map(|mut marker| {
                marker.coord.line = marker.coord.line.saturating_add(lift.origin.line);
                marker.coord.column = marker.coord.column.saturating_add(lift.origin.column);
                marker
            }));
        self.cursor_index = index_for_column(
            &self.grid.lines[self.grid.cursor_pos.line],
            self.grid.cursor_pos.column,
        );
        self.grid.lines != before_lines || self.line_markers != before_markers
    }

    pub fn cancel_move_lift(&mut self) -> bool {
        let Some(lift) = self.move_lift.take() else {
            return false;
        };
        self.selection = lift.source_selection;
        self.grid.cursor_pos = lift.source_cursor;
        self.cursor_index = lift.source_cursor_index;
        true
    }

    pub fn move_lift_bounds(&self) -> Option<SelectionBounds> {
        self.move_lift
            .as_ref()
            .map(|lift| lift.rectangle.bounds_at(lift.origin))
    }

    pub(super) fn lines_with_move_lift_preview(&self) -> Option<Vec<Vec<crate::model::Atom>>> {
        let lift = self.move_lift.as_ref()?;
        let mut lines = self.grid.lines.clone();
        replace_range(&mut lines, lift.source_bounds, None);
        overwrite_rectangle(&mut lines, lift.origin, &lift.rectangle);
        Some(lines)
    }
}

fn offset_coord(coord: Coord, line_delta: i128, column_delta: i128) -> Coord {
    Coord {
        line: usize::try_from(coord.line as i128 + line_delta).unwrap_or(0),
        column: usize::try_from(coord.column as i128 + column_delta).unwrap_or(0),
    }
}
