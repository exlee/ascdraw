use unicode_width::UnicodeWidthStr;

use crate::model::{Atom, Coord, Direction, LayerId};
use crate::selection::{
    CanvasSelection, SelectionBounds, TextRectangle, overwrite_rectangle, replace_range,
    selected_atoms,
};

use super::{Editor, PlacedLineMarker, index_for_column};

#[derive(Debug, Clone)]
pub(super) struct MoveLift {
    pub(super) source_selection: CanvasSelection,
    pub(super) source_cursor: Coord,
    pub(super) source_cursor_index: usize,
    source_bounds: SelectionBounds,
    origin: Coord,
    clone_origins: Vec<Coord>,
    last_clone_press: Option<u64>,
    rectangle: TextRectangle,
    layers: Vec<LiftedLayer>,
}

#[derive(Debug, Clone)]
struct LiftedLayer {
    id: LayerId,
    edited_atoms: Vec<LiftedAtom>,
    markers: Vec<PlacedLineMarker>,
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
        let source_selection = self.selection;
        let source_bounds = source_selection.bounds();
        let source_origin = Coord {
            line: source_bounds.top,
            column: source_bounds.left,
        };
        let layers = self
            .layers
            .layer_contents(&self.grid.lines, &self.line_markers)
            .into_iter()
            .map(|(id, lines, markers)| {
                let rectangle = TextRectangle {
                    rows: selected_atoms(&lines, source_bounds),
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
                    rendered_lines: lines,
                }
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
            clone_origins: Vec::new(),
            last_clone_press: None,
            rectangle: TextRectangle {
                rows: vec![vec![super::blank_atom()]; source_bounds.height()],
                width: source_bounds.width(),
            },
            layers,
        });
        self.refresh_move_lift_render();
        true
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
            return false;
        }
        let mut changed = false;
        self.layers.for_each_layer_mut(
            &mut self.grid.lines,
            &mut self.line_markers,
            |id, lines, markers| {
                let Some(layer) = lift.layers.iter().find(|layer| layer.id == id) else {
                    return;
                };
                let before_lines = lines.clone();
                let before_markers = markers.clone();
                markers.retain(|marker| {
                    !lifted_atoms_cover(&layer.edited_atoms, source_origin, marker.coord)
                        && !destinations.iter().any(|origin| {
                            lifted_atoms_cover(&layer.edited_atoms, *origin, marker.coord)
                        })
                });
                compose_sparse_move(lines, source_origin, &destinations, &layer.edited_atoms);
                for origin in &destinations {
                    for mut marker in layer.markers.iter().cloned() {
                        marker.coord.line = marker.coord.line.saturating_add(origin.line);
                        marker.coord.column = marker.coord.column.saturating_add(origin.column);
                        markers.retain(|existing| existing.coord != marker.coord);
                        markers.push(marker);
                    }
                }
                changed |= *lines != before_lines || *markers != before_markers;
            },
        );
        self.cursor_index = index_for_column(
            &self.grid.lines[self.grid.cursor_pos.line],
            self.grid.cursor_pos.column,
        );
        changed
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
        self.move_lift_render_lines().map(<[_]>::to_vec)
    }

    pub(super) fn move_lift_render_lines(&self) -> Option<&[Vec<crate::model::Atom>]> {
        self.move_lift_render_lines_for_layer(self.active_layer_id())
    }

    pub(crate) fn move_lift_render_lines_for_layer(
        &self,
        id: LayerId,
    ) -> Option<&[Vec<crate::model::Atom>]> {
        self.move_lift.as_ref().and_then(|lift| {
            lift.layers
                .iter()
                .find(|layer| layer.id == id)
                .map(|layer| layer.rendered_lines.as_slice())
        })
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
        let contents = self
            .layers
            .layer_contents(&self.grid.lines, &self.line_markers);
        let lift = self
            .move_lift
            .as_mut()
            .expect("move lift remains active while composing");
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
