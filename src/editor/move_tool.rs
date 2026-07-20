use unicode_width::UnicodeWidthStr;

use crate::canvas::{LayerMap, LayerStack, LineData};
use crate::model::{Atom, Coord, Direction, Face, LayerId, StyledAtom};
use crate::selection::{CanvasSelection, SelectionBounds, TextRectangle};

use super::{EditSnapshot, Editor, PlacedLineMarker};

#[derive(Debug, Clone)]
pub(super) struct MoveLift {
    pub(super) source_snapshot: EditSnapshot,
    source_selection: CanvasSelection,
    source_cursor: Coord,
    source_bounds: SelectionBounds,
    origin: Coord,
    clone_origins: Vec<Coord>,
    last_clone_press: Option<u64>,
    width: usize,
    height: usize,
    layers: Vec<LiftedLayer>,
    rendered_canvas: Option<LayerStack>,
}

#[derive(Debug, Clone)]
struct LiftedLayer {
    id: LayerId,
    edited_atoms: Vec<LiftedAtom>,
    markers: Vec<PlacedLineMarker>,
}

#[derive(Debug, Clone)]
struct LiftedAtom {
    offset: Coord,
    width: usize,
    atom: Atom,
    face: Face,
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
            clone_origins: Vec::new(),
            last_clone_press: None,
            width: source_bounds.width(),
            height: source_bounds.height(),
            layers,
            rendered_canvas: None,
        });
        self.refresh_move_lift_render();
        true
    }

    pub fn move_lift(&mut self, direction: Direction) -> bool {
        let Some(lift) = self.move_lift.as_mut() else {
            return false;
        };
        let next = super::adjacent_coord(lift.origin, direction);
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
        self.move_lift.as_ref().map(MoveLift::bounds)
    }

    #[cfg(test)]
    pub(super) fn lines_with_move_lift_preview(&self) -> Option<Vec<Vec<StyledAtom>>> {
        self.move_lift_render_canvas()
            .map(LayerStack::active_dense_lines)
    }

    #[cfg(test)]
    pub(crate) fn move_lift_render_lines_for_layer(
        &self,
        id: LayerId,
    ) -> Option<Vec<Vec<StyledAtom>>> {
        self.move_lift_render_canvas().and_then(|canvas| {
            canvas
                .layers()
                .iter()
                .find(|layer| layer.id == id && layer.visible)
                .map(LayerMap::to_dense)
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
        let lift = self
            .move_lift
            .as_mut()
            .expect("move lift remains active while composing");
        lift.rendered_canvas = Some(rendered_canvas);
    }

    fn restore_move_lift_source(&mut self, lift: MoveLift) {
        self.restore_edit_snapshot(lift.source_snapshot);
    }
}

fn lifted_layer(
    id: LayerId,
    rows: Vec<Vec<StyledAtom>>,
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
            map.set_at(coord.column, coord.line, atom.atom.clone(), &atom.face)
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
    fn bounds(&self) -> SelectionBounds {
        let width = i16::try_from(self.width.saturating_sub(1)).unwrap_or(i16::MAX);
        let height = i16::try_from(self.height.saturating_sub(1)).unwrap_or(i16::MAX);
        SelectionBounds {
            left: self.origin.column,
            right: self.origin.column.saturating_add(width),
            top: self.origin.line,
            bottom: self.origin.line.saturating_add(height),
        }
    }
}

fn lifted_edited_atoms(rectangle: &TextRectangle) -> Vec<LiftedAtom> {
    let mut lifted = Vec::new();
    for (line, row) in rectangle.rows.iter().enumerate() {
        let mut column: usize = 0;
        for atom in row {
            let width = UnicodeWidthStr::width(atom.contents.as_str()).max(1);
            if !atom.contents.chars().all(char::is_whitespace) {
                let line = i16::try_from(line).expect("lifted line fits signed canvas range");
                let column = i16::try_from(column).expect("lifted column fits signed canvas range");
                lifted.push(LiftedAtom {
                    offset: Coord { line, column },
                    width,
                    atom: Atom::new(atom.contents.clone())
                        .expect("lifted atoms are validated one-cell graphemes"),
                    face: atom.face.clone(),
                });
            }
            column = column.saturating_add(width);
        }
    }
    lifted
}

fn lifted_atoms_cover(atoms: &[LiftedAtom], origin: Coord, coord: Coord) -> bool {
    atoms.iter().any(|atom| {
        let width = i16::try_from(atom.width).unwrap_or(i16::MAX);
        coord.line == origin.line.saturating_add(atom.offset.line)
            && (origin.column.saturating_add(atom.offset.column)
                ..origin
                    .column
                    .saturating_add(atom.offset.column)
                    .saturating_add(width))
                .contains(&coord.column)
    })
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
        right: origin
            .column
            .saturating_add(i16::try_from(atom.width.saturating_sub(1)).unwrap_or(i16::MAX)),
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
        line: i16::try_from(coord.line as i128 + line_delta).unwrap_or_else(|_| {
            if line_delta.is_negative() {
                i16::MIN
            } else {
                i16::MAX
            }
        }),
        column: i16::try_from(coord.column as i128 + column_delta).unwrap_or_else(|_| {
            if column_delta.is_negative() {
                i16::MIN
            } else {
                i16::MAX
            }
        }),
    }
}
