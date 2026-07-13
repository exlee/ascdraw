use crate::drawing::{
    CornerStyle, LineEnding, LineStyle, glyph_with_connection_and_corner, is_line_glyph,
    line_ending_glyph,
};
use crate::model::{Atom, Coord, Direction, Face};

use super::{EditorState, adjacent_coord, atom_width, blank_atom, index_and_column_for_coord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveStroke {
    pub(super) end: Coord,
    pub(super) end_base_glyph: String,
    pub(super) moving_ending: LineEnding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlacedLineMarker {
    pub(super) coord: Coord,
    pub(super) ending: LineEnding,
    pub(super) base_glyph: String,
}

impl EditorState {
    pub fn move_or_draw(&mut self, direction: Direction, draw: bool) -> bool {
        let prepended = self.prepare_adjacent(direction);
        let from = self.grid.cursor_pos;
        let to = adjacent_coord(from, direction).expect("canvas edge was structurally extended");
        let line_style = self.toolbar.line_style();
        let corner_style = self.toolbar.line_corner();

        if !draw {
            self.end_stroke();
            self.move_to_without_ending_stroke(to);
            self.collapse_selection();
            return prepended;
        }

        let continuing_stroke = self
            .active_stroke
            .take()
            .filter(|stroke| stroke.end == from);
        let (from_was_existing_line, moving_ending) =
            if let Some(stroke) = continuing_stroke.as_ref() {
                self.take_line_marker(from);
                self.set_cell_contents(from, stroke.end_base_glyph.clone());
                (true, stroke.moving_ending)
            } else if let Some(marker) = self.take_line_marker(from) {
                self.set_cell_contents(from, marker.base_glyph);
                (true, marker.ending)
            } else {
                (
                    self.cell_contents(from).is_some_and(is_line_glyph),
                    self.toolbar.line_end(),
                )
            };

        let continuing_stroke = continuing_stroke.is_some();
        if !continuing_stroke {
            self.active_stroke = None;
        }

        let from_base = self.add_connection(from, direction, line_style, corner_style);
        self.move_to_without_ending_stroke(to);
        let to_was_existing_line = self.cell_contents(to).is_some_and(is_line_glyph);
        let Some(end_base_glyph) =
            self.add_connection(to, direction.opposite(), line_style, corner_style)
        else {
            self.active_stroke = None;
            self.collapse_selection();
            return true;
        };

        if !continuing_stroke
            && !from_was_existing_line
            && let Some(from_base) = from_base
        {
            self.apply_line_ending(
                from,
                self.toolbar.line_start(),
                direction,
                line_style,
                &from_base,
            );
        }
        if !to_was_existing_line {
            self.apply_line_ending(
                to,
                moving_ending,
                direction.opposite(),
                line_style,
                &end_base_glyph,
            );
        }
        self.active_stroke = Some(ActiveStroke {
            end: to,
            end_base_glyph,
            moving_ending,
        });
        self.collapse_selection();
        true
    }

    pub(super) fn remove_connection(&mut self, coord: Coord, direction: Direction) {
        if let Some(marker) = self.take_line_marker(coord) {
            self.set_cell_contents(coord, marker.base_glyph);
        }
        let Some(line) = self.grid.lines.get_mut(coord.line) else {
            return;
        };
        let (index, column) = index_and_column_for_coord(line, coord.column);
        if column != coord.column {
            return;
        }
        if let Some(atom) = line.get_mut(index)
            && atom_width(atom) == 1
            && let Some(glyph) = crate::drawing::glyph_without_connection(&atom.contents, direction)
        {
            atom.contents = glyph.to_string();
        }
    }

    pub(super) fn cell_contents(&self, coord: Coord) -> Option<&str> {
        let line = self.grid.lines.get(coord.line)?;
        let (index, column) = index_and_column_for_coord(line, coord.column);
        (column == coord.column)
            .then(|| line.get(index))
            .flatten()
            .map(|atom| atom.contents.as_str())
    }

    fn take_line_marker(&mut self, coord: Coord) -> Option<PlacedLineMarker> {
        let index = self
            .line_markers
            .iter()
            .position(|marker| marker.coord == coord)?;
        Some(self.line_markers.remove(index))
    }

    pub(super) fn remove_line_marker(&mut self, coord: Coord) {
        self.line_markers.retain(|marker| marker.coord != coord);
    }

    fn add_connection(
        &mut self,
        coord: Coord,
        direction: Direction,
        line_style: LineStyle,
        corner_style: CornerStyle,
    ) -> Option<String> {
        self.remove_line_marker(coord);
        let line = &mut self.grid.lines[coord.line];
        let (index, column) = index_and_column_for_coord(line, coord.column);

        if column < coord.column {
            line.extend((column..coord.column).map(|_| blank_atom()));
        }

        if let Some(atom) = line.get_mut(index) {
            if atom_width(atom) == 1
                && let Some(glyph) = glyph_with_connection_and_corner(
                    &atom.contents,
                    direction,
                    line_style,
                    corner_style,
                )
            {
                atom.contents = glyph.to_string();
                return Some(atom.contents.clone());
            }
            None
        } else {
            let contents =
                glyph_with_connection_and_corner(" ", direction, line_style, corner_style)
                    .expect("blank cells accept line connections")
                    .to_string();
            line.push(Atom {
                face: Face::default(),
                contents: contents.clone(),
            });
            Some(contents)
        }
    }

    fn apply_line_ending(
        &mut self,
        coord: Coord,
        ending: LineEnding,
        connected_direction: Direction,
        line_style: LineStyle,
        base_glyph: &str,
    ) {
        self.remove_line_marker(coord);
        self.set_cell_contents(
            coord,
            line_ending_glyph(ending, connected_direction, line_style).to_string(),
        );
        if ending != LineEnding::None {
            self.line_markers.push(PlacedLineMarker {
                coord,
                ending,
                base_glyph: base_glyph.to_string(),
            });
        }
    }

    fn set_cell_contents(&mut self, coord: Coord, contents: String) {
        let line = &mut self.grid.lines[coord.line];
        let (index, column) = index_and_column_for_coord(line, coord.column);
        if column == coord.column
            && let Some(atom) = line.get_mut(index)
            && atom_width(atom) == 1
        {
            atom.contents = contents;
        }
    }

    pub fn end_stroke(&mut self) {
        self.active_stroke = None;
    }
}
