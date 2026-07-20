use crate::canvas::LineData;
use crate::drawing::{
    CornerStyle, LineEnding, LineStyle, glyph_for_connection_pair,
    glyph_with_connection_and_corner, is_line_glyph, line_ending_glyph,
};
use crate::model::{Atom, Coord, Direction};

use super::Editor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveStroke {
    pub(super) end: Coord,
    pub(super) end_base_glyph: String,
    pub(super) moving_ending: LineEnding,
    pub(super) incoming_connection: Direction,
    pub(super) end_was_existing_line: bool,
}

impl Editor {
    pub fn move_or_draw(&mut self, direction: Direction, draw: bool) -> bool {
        self.move_or_draw_with_endings(
            direction,
            draw,
            self.toolbar.line_start(),
            self.toolbar.line_end(),
        )
    }

    pub(super) fn move_or_draw_routed(&mut self, direction: Direction) -> bool {
        self.move_or_draw_with_endings(direction, true, LineEnding::None, LineEnding::None)
    }

    fn move_or_draw_with_endings(
        &mut self,
        direction: Direction,
        draw: bool,
        selected_start: LineEnding,
        selected_end: LineEnding,
    ) -> bool {
        let from = self.grid.cursor_pos;
        let Some(to) = self.prepared_adjacent_coord(direction) else {
            return false;
        };
        let line_style = self.toolbar.line_style();
        let corner_style = self.toolbar.line_corner();

        if !draw {
            self.end_stroke();
            self.move_to_without_ending_stroke(to);
            self.collapse_selection();
            return false;
        }

        let continuing_stroke = self
            .active_stroke
            .take()
            .filter(|stroke| stroke.end == from);
        let (from_was_existing_line, moving_ending, known_incoming_connection) =
            if let Some(stroke) = continuing_stroke.as_ref() {
                self.take_line_marker(from);
                self.set_cell_contents(from, stroke.end_base_glyph.clone());
                (
                    true,
                    stroke.moving_ending,
                    (!stroke.end_was_existing_line).then_some(stroke.incoming_connection),
                )
            } else if let Some(marker) = self.take_line_marker(from) {
                self.set_cell_contents(from, marker.base_glyph);
                (true, marker.ending, None)
            } else {
                (
                    self.cell_contents(from).is_some_and(is_line_glyph),
                    selected_end,
                    None,
                )
            };

        let continuing_stroke = continuing_stroke.is_some();
        if !continuing_stroke {
            self.active_stroke = None;
        }

        let from_base = if let Some(incoming) = known_incoming_connection {
            let glyph = glyph_for_connection_pair(incoming, direction, line_style, corner_style);
            self.set_cell_contents(from, glyph.to_string());
            Some(glyph.to_string())
        } else {
            self.add_connection(from, direction, line_style, corner_style)
        };
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
            self.apply_line_ending(from, selected_start, direction, line_style, &from_base);
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
            incoming_connection: direction.opposite(),
            end_was_existing_line: to_was_existing_line,
        });
        self.collapse_selection();
        true
    }

    pub(super) fn remove_connection(&mut self, coord: Coord, direction: Direction) {
        let foreground = self.write_face().fg;
        if let Some(marker) = self.take_line_marker(coord) {
            self.set_cell_contents(coord, marker.base_glyph);
        }
        let Some(data) = self.canvas.active_cell(coord) else {
            return;
        };
        let Some(glyph) = crate::drawing::glyph_without_connection(data.atom.contents(), direction)
        else {
            return;
        };
        let atom = Atom::new(glyph.to_string()).expect("line glyph is one cell");
        let mut face = data.face.as_ref().clone();
        if atom.contents().chars().all(char::is_whitespace) {
            face = crate::model::Face::default();
        } else {
            face.fg = foreground;
        }
        self.canvas
            .set_at(coord, atom, &face)
            .expect("line glyphs occupy one sparse cell");
    }

    pub(super) fn cell_contents(&self, coord: Coord) -> Option<&str> {
        self.canvas
            .active_cell(coord)
            .map(|data| data.atom.contents())
    }

    fn take_line_marker(&mut self, coord: Coord) -> Option<LineData> {
        self.canvas.take_line_at(coord)
    }

    pub(super) fn remove_line_marker(&mut self, coord: Coord) {
        self.canvas.remove_line_at(coord);
    }

    fn add_connection(
        &mut self,
        coord: Coord,
        direction: Direction,
        line_style: LineStyle,
        corner_style: CornerStyle,
    ) -> Option<String> {
        let foreground = self.write_face().fg;
        self.remove_line_marker(coord);
        let existing = self
            .canvas
            .active_cell(coord)
            .map(|data| (data.atom.contents().to_owned(), data.face.as_ref().clone()))
            .unwrap_or_else(|| (" ".to_owned(), crate::model::Face::default()));
        let glyph =
            glyph_with_connection_and_corner(&existing.0, direction, line_style, corner_style)?;
        let contents = glyph.to_string();
        let mut face = existing.1;
        face.fg = foreground;
        let atom = Atom::new(contents.clone()).expect("line glyph is one cell");
        self.canvas
            .set_at(coord, atom, &face)
            .expect("line glyphs occupy one sparse cell");
        Some(contents)
    }

    pub(super) fn apply_line_ending(
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
            self.commit_canvas();
            self.canvas.set_line_at(
                coord,
                LineData {
                    ending,
                    base_glyph: base_glyph.to_string(),
                },
            );
        }
    }

    fn set_cell_contents(&mut self, coord: Coord, contents: String) {
        let foreground = self.write_face().fg;
        let mut face = self
            .canvas
            .active_cell(coord)
            .map_or_else(crate::model::Face::default, |data| {
                data.face.as_ref().clone()
            });
        face.fg = foreground;
        let atom = Atom::new(contents).expect("line glyph is one cell");
        self.canvas
            .set_at(coord, atom, &face)
            .expect("line glyphs occupy one sparse cell");
    }

    pub(super) fn write_diagonal_cell(
        &mut self,
        coord: Coord,
        glyph: &str,
        overwrite_active_endpoint: bool,
        consume_marker: bool,
    ) -> bool {
        if consume_marker && let Some(marker) = self.take_line_marker(coord) {
            self.set_cell_contents(coord, marker.base_glyph);
        }
        let writable = self.cell_contents(coord).is_none_or(|contents| {
            contents.chars().all(char::is_whitespace)
                || matches!(contents, "·" | "╱" | "╲")
                || (overwrite_active_endpoint && is_line_glyph(contents))
        });
        if !writable {
            return false;
        }
        let face = self.write_face();
        let atom = Atom::new(glyph).expect("diagonal glyph is one cell");
        self.canvas
            .set_at(coord, atom, &face)
            .expect("diagonal glyphs occupy one sparse cell");
        true
    }

    pub fn end_stroke(&mut self) {
        self.active_stroke = None;
    }
}
