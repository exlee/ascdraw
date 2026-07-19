use unicode_width::UnicodeWidthStr;

use crate::canvas::{LayerMap, LayerStack, LineData};
use crate::model::{Atom, Coord, Direction, LayerId, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH};
use crate::selection::{
    CanvasSelection, SelectionBounds, TextRectangle, overwrite_rectangle, replace_range,
};

use super::{EditSnapshot, Editor, PlacedLineMarker};

#[derive(Debug, Clone)]
pub(super) struct MoveLift {
    pub(super) source_snapshot: EditSnapshot,
    source_selection: CanvasSelection,
    source_cursor: Coord,
    source_bounds: SelectionBounds,
    origin: Coord,
    prepended_columns: usize,
    prepended_lines: usize,
    clone_origins: Vec<Coord>,
    last_clone_press: Option<u64>,
    rectangle: TextRectangle,
    layers: Vec<LiftedLayer>,
    rendered_canvas: Option<LayerStack>,
}

#[derive(Debug, Clone)]
struct LiftedLayer {
    id: LayerId,
    edited_atoms: Vec<LiftedAtom>,
    markers: Vec<PlacedLineMarker>,
    #[cfg(test)]
    rendered_lines: Vec<Vec<Atom>>,
}

#[derive(Debug, Clone)]
struct LiftedAtom {
    offset: Coord,
    width: usize,
    atom: Atom,
}

impl Editor {
    pub fn move_lift_active(&self) -> bool {
        self.move_lift.is_some()
    }

    pub fn begin_selected_move_lift(&mut self) -> bool {
        if self.selection.is_collapsed() {
            return false;
        }
        self.begin_move_lift_inner()
    }

    fn begin_move_lift_inner(&mut self) -> bool {
        if self.move_lift.is_some() {
            return false;
        }
        self.end_stroke();
        self.shape_preview = None;
        self.toolbar.cancel_shortcut();
        let source_snapshot = self.edit_snapshot();
        self.canvas = source_snapshot.canvas.clone();
        let source_selection = self.selection;
        let source_bounds = source_selection.bounds();
        let source_origin = Coord {
            line: source_bounds.top,
            column: source_bounds.left,
        };
        let layers = source_snapshot
            .canvas
            .layers()
            .iter()
            .filter(|layer| layer.visible)
            .map(|layer| {
                lifted_layer(
                    layer.id,
                    layer.selected_atoms(source_bounds),
                    layer.line_markers(),
                    source_origin,
                    source_bounds,
                )
            })
            .collect();
        self.move_lift = Some(MoveLift {
            source_snapshot,
            source_selection,
            source_cursor: self.grid.cursor_pos,
            source_bounds,
            origin: Coord {
                line: source_bounds.top,
                column: source_bounds.left,
            },
            prepended_columns: 0,
            prepended_lines: 0,
            clone_origins: Vec::new(),
            last_clone_press: None,
            rectangle: TextRectangle {
                rows: vec![vec![super::blank_atom()]; source_bounds.height()],
                width: source_bounds.width(),
            },
            layers,
            rendered_canvas: None,
        });
        self.refresh_move_lift_render();
        true
    }

    pub fn move_lift(&mut self, direction: Direction) -> bool {
        let prepend = self.move_lift.as_ref().map(|lift| match direction {
            Direction::Up if lift.origin.line == 0 => (0, 1),
            Direction::Left if lift.origin.column == 0 => (1, 0),
            _ => (0, 0),
        });
        match prepend {
            Some((1, 0)) => {
                if !self.prepend_column() {
                    return false;
                }
                self.canvas_origin.column = self.canvas_origin.column.saturating_add(1);
                self.move_lift
                    .as_mut()
                    .expect("move lift remains active after prepend")
                    .shift(1, 0);
            }
            Some((0, 1)) => {
                if !self.prepend_line() {
                    return false;
                }
                self.canvas_origin.line = self.canvas_origin.line.saturating_add(1);
                self.move_lift
                    .as_mut()
                    .expect("move lift remains active after prepend")
                    .shift(0, 1);
            }
            Some((0, 0)) => {}
            Some(_) => unreachable!("move lift prepends at most one row or column"),
            None => return false,
        }
        let Some(lift) = self.move_lift.as_mut() else {
            return false;
        };
        let bounds = lift.rectangle.bounds_at(lift.origin);
        if direction == Direction::Right && bounds.right.saturating_add(1) >= MAX_CANVAS_WIDTH
            || direction == Direction::Down && bounds.bottom.saturating_add(1) >= MAX_CANVAS_HEIGHT
        {
            return false;
        }
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
        self.refresh_move_lift_render();
        true
    }

    pub fn clone_move_lift(&mut self, direction: Direction, shift_press: u64) -> bool {
        let cloned = if let Some(lift) = self.move_lift.as_mut()
            && lift.last_clone_press != Some(shift_press)
        {
            lift.last_clone_press = Some(shift_press);
            if lift.clone_origins.contains(&lift.origin) {
                false
            } else {
                lift.clone_origins.push(lift.origin);
                true
            }
        } else {
            false
        };
        let moved = self.move_lift(direction);
        if cloned && !moved {
            self.refresh_move_lift_render();
        }
        cloned || moved
    }

    pub fn confirm_move_lift(&mut self) -> bool {
        let Some(lift) = self.move_lift.take() else {
            return false;
        };
        let source_origin = Coord {
            line: lift.source_bounds.top,
            column: lift.source_bounds.left,
        };
        let destinations = move_lift_destinations(&lift);
        if destinations.as_slice() == [source_origin] {
            self.restore_move_lift_source(lift);
            return false;
        }
        let mut changed = false;
        self.canvas.mutate_layers(|id, map| {
            let Some(layer) = lift.layers.iter().find(|layer| layer.id == id) else {
                return;
            };
            let before = map.clone();
            apply_sparse_move(map, source_origin, &destinations, layer);
            changed |= *map != before;
        });
        self.refresh_active_dense_view();
        changed
    }

    pub fn cancel_move_lift(&mut self) -> bool {
        let Some(lift) = self.move_lift.take() else {
            return false;
        };
        self.restore_move_lift_source(lift);
        true
    }

    pub fn move_lift_bounds(&self) -> Option<SelectionBounds> {
        self.move_lift
            .as_ref()
            .map(|lift| lift.rectangle.bounds_at(lift.origin))
    }

    pub(super) fn lines_with_move_lift_preview(&self) -> Option<Vec<Vec<crate::model::Atom>>> {
        #[cfg(not(test))]
        return None;
        #[cfg(test)]
        self.move_lift_render_lines().map(<[_]>::to_vec)
    }

    pub(super) fn move_lift_render_lines(&self) -> Option<&[Vec<crate::model::Atom>]> {
        #[cfg(not(test))]
        return None;
        #[cfg(test)]
        self.move_lift_render_lines_for_layer(self.active_layer_id())
    }

    pub(crate) fn move_lift_render_lines_for_layer(
        &self,
        id: LayerId,
    ) -> Option<&[Vec<crate::model::Atom>]> {
        #[cfg(not(test))]
        return None;
        #[cfg(test)]
        self.move_lift.as_ref().and_then(|lift| {
            lift.layers
                .iter()
                .find(|layer| layer.id == id)
                .map(|layer| layer.rendered_lines.as_slice())
        })
    }

    pub(crate) fn move_lift_render_canvas(&self) -> Option<&LayerStack> {
        self.move_lift
            .as_ref()
            .and_then(|lift| lift.rendered_canvas.as_ref())
    }

    fn refresh_move_lift_render(&mut self) {
        let Some(lift) = self.move_lift.as_ref() else {
            return;
        };
        let source_origin = Coord {
            line: lift.source_bounds.top,
            column: lift.source_bounds.left,
        };
        let destinations = move_lift_destinations(lift);
        let mut rendered_canvas = self.canvas.clone();
        rendered_canvas.mutate_layers(|id, map| {
            if let Some(layer) = lift.layers.iter().find(|layer| layer.id == id) {
                apply_sparse_move(map, source_origin, &destinations, layer);
            }
        });
        #[cfg(test)]
        let contents = self.layer_contents();
        let lift = self
            .move_lift
            .as_mut()
            .expect("move lift remains active while composing");
        #[cfg(test)]
        for layer in &mut lift.layers {
            let Some((_, lines, _)) = contents.iter().find(|(id, _, _)| *id == layer.id) else {
                continue;
            };
            layer.rendered_lines = lines.clone();
            compose_sparse_move(
                &mut layer.rendered_lines,
                source_origin,
                &destinations,
                &layer.edited_atoms,
            );
        }
        lift.rendered_canvas = Some(rendered_canvas);
    }

    fn restore_move_lift_source(&mut self, lift: MoveLift) {
        let prefix_shift = (
            -i64::try_from(lift.prepended_columns).unwrap_or(i64::MAX),
            -i64::try_from(lift.prepended_lines).unwrap_or(i64::MAX),
        );
        self.restore_edit_snapshot(lift.source_snapshot);
        self.pending_prepend = prefix_shift;
    }
}

fn lifted_layer(
    id: LayerId,
    rows: Vec<Vec<Atom>>,
    markers: Vec<PlacedLineMarker>,
    source_origin: Coord,
    source_bounds: SelectionBounds,
) -> LiftedLayer {
    let rectangle = TextRectangle {
        rows,
        width: source_bounds.width(),
    };
    let edited_atoms = lifted_edited_atoms(&rectangle);
    let markers = markers
        .into_iter()
        .filter(|marker| lifted_atoms_cover(&edited_atoms, source_origin, marker.coord))
        .map(|mut marker| {
            marker.coord.line -= source_bounds.top;
            marker.coord.column -= source_bounds.left;
            marker
        })
        .collect();
    LiftedLayer {
        id,
        edited_atoms,
        markers,
        #[cfg(test)]
        rendered_lines: Vec::new(),
    }
}

fn apply_sparse_move(
    map: &mut LayerMap,
    source_origin: Coord,
    destinations: &[Coord],
    layer: &LiftedLayer,
) {
    for atom in &layer.edited_atoms {
        map.replace_bounds(lifted_atom_bounds(source_origin, atom), None)
            .expect("move source fits the sparse canvas");
        for origin in destinations {
            map.replace_bounds(lifted_atom_bounds(*origin, atom), None)
                .expect("move destination fits the sparse canvas");
        }
    }
    for origin in destinations {
        for atom in &layer.edited_atoms {
            let coord = offset_origin(*origin, atom.offset);
            map.set_at(
                i16::try_from(coord.column).expect("validated move column"),
                i16::try_from(coord.line).expect("validated move line"),
                atom.atom.clone(),
                &atom.atom.face,
            )
            .expect("moved atom occupies one sparse cell");
        }
        for marker in &layer.markers {
            let coord = offset_origin(*origin, marker.coord);
            map.set_line_at(
                coord,
                LineData {
                    ending: marker.ending,
                    base_glyph: marker.base_glyph.clone(),
                },
            );
        }
    }
}

impl MoveLift {
    fn shift(&mut self, columns: usize, lines: usize) {
        self.prepended_columns = self.prepended_columns.saturating_add(columns);
        self.prepended_lines = self.prepended_lines.saturating_add(lines);
        self.source_selection.shift(columns, lines);
        self.source_cursor.column = self.source_cursor.column.saturating_add(columns);
        self.source_cursor.line = self.source_cursor.line.saturating_add(lines);
        self.source_bounds.left = self.source_bounds.left.saturating_add(columns);
        self.source_bounds.right = self.source_bounds.right.saturating_add(columns);
        self.source_bounds.top = self.source_bounds.top.saturating_add(lines);
        self.source_bounds.bottom = self.source_bounds.bottom.saturating_add(lines);
        self.origin.column = self.origin.column.saturating_add(columns);
        self.origin.line = self.origin.line.saturating_add(lines);
        for origin in &mut self.clone_origins {
            origin.column = origin.column.saturating_add(columns);
            origin.line = origin.line.saturating_add(lines);
        }
    }
}

fn lifted_edited_atoms(rectangle: &TextRectangle) -> Vec<LiftedAtom> {
    let mut lifted = Vec::new();
    for (line, row) in rectangle.rows.iter().enumerate() {
        let mut column = 0;
        for atom in row {
            let width = UnicodeWidthStr::width(atom.contents.as_str()).max(1);
            if !atom.contents.chars().all(char::is_whitespace) {
                lifted.push(LiftedAtom {
                    offset: Coord { line, column },
                    width,
                    atom: atom.clone(),
                });
            }
            column = column.saturating_add(width);
        }
    }
    lifted
}

fn lifted_atoms_cover(atoms: &[LiftedAtom], origin: Coord, coord: Coord) -> bool {
    atoms.iter().any(|atom| {
        coord.line == origin.line.saturating_add(atom.offset.line)
            && (origin.column.saturating_add(atom.offset.column)
                ..origin
                    .column
                    .saturating_add(atom.offset.column)
                    .saturating_add(atom.width))
                .contains(&coord.column)
    })
}

fn compose_sparse_move(
    lines: &mut Vec<Vec<Atom>>,
    source_origin: Coord,
    destination_origins: &[Coord],
    atoms: &[LiftedAtom],
) {
    for atom in atoms {
        replace_range(lines, lifted_atom_bounds(source_origin, atom), None);
    }
    for destination_origin in destination_origins {
        for atom in atoms {
            overwrite_rectangle(
                lines,
                offset_origin(*destination_origin, atom.offset),
                &TextRectangle {
                    rows: vec![vec![atom.atom.clone()]],
                    width: atom.width,
                },
            );
        }
    }
}

fn move_lift_destinations(lift: &MoveLift) -> Vec<Coord> {
    let mut destinations = lift.clone_origins.clone();
    if !destinations.contains(&lift.origin) {
        destinations.push(lift.origin);
    }
    destinations
}

fn lifted_atom_bounds(origin: Coord, atom: &LiftedAtom) -> SelectionBounds {
    let origin = offset_origin(origin, atom.offset);
    SelectionBounds {
        left: origin.column,
        right: origin.column.saturating_add(atom.width.saturating_sub(1)),
        top: origin.line,
        bottom: origin.line,
    }
}

fn offset_origin(origin: Coord, offset: Coord) -> Coord {
    Coord {
        line: origin.line.saturating_add(offset.line),
        column: origin.column.saturating_add(offset.column),
    }
}

fn offset_coord(coord: Coord, line_delta: i128, column_delta: i128) -> Coord {
    Coord {
        line: usize::try_from(coord.line as i128 + line_delta).unwrap_or(0),
        column: usize::try_from(coord.column as i128 + column_delta).unwrap_or(0),
    }
}
