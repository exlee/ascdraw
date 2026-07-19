use crate::canvas::LineData;
use crate::drawing::{
    CornerStyle, LineEnding, LineStyle, glyph_for_connection_pair,
    glyph_with_connection_and_corner, is_line_glyph, line_ending_glyph,
};
use crate::model::{Coord, Direction};

use super::{Editor, adjacent_coord, atom_width, grid, index_and_column_for_coord, replace_cell};

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
        let Some(prepended) = self.prepare_adjacent(direction) else {
            return false;
        };
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
            if atom.contents.chars().all(char::is_whitespace) {
                atom.face = crate::model::Face::default();
            } else {
                atom.face.fg = foreground;
            }
        }
    }

    pub(super) fn cell_contents(&self, coord: Coord) -> Option<&str> {
        let line = self.grid.lines.get(coord.line)?;
        let (index, column) = index_and_column_for_coord(line, coord.column);
        let atom = line.get(index)?;
        if column == coord.column {
            Some(atom.contents.as_str())
        } else if grid::is_blank_run(atom) {
            Some(" ")
        } else {
            None
        }
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
        let (index, column) =
            index_and_column_for_coord(&self.grid.lines[coord.line], coord.column);

        if column < coord.column && index == self.grid.lines[coord.line].len() {
            self.grid.lines[coord.line].extend(grid::blank_run(coord.column - column));
        }

        if let Some(atom) = self.grid.lines[coord.line].get_mut(index) {
            if atom_width(atom) == 1
                && let Some(glyph) = glyph_with_connection_and_corner(
                    &atom.contents,
                    direction,
                    line_style,
                    corner_style,
                )
            {
                atom.contents = glyph.to_string();
                atom.face.fg = foreground;
                return Some(atom.contents.clone());
            }
            if grid::is_blank_run(atom) {
                let contents =
                    glyph_with_connection_and_corner(" ", direction, line_style, corner_style)
                        .expect("blank cells accept line connections")
                        .to_string();
                replace_cell(&mut self.grid.lines, coord, contents.clone());
                self.color_written_cell(coord);
                return Some(contents);
            }
            None
        } else {
            let contents =
                glyph_with_connection_and_corner(" ", direction, line_style, corner_style)
                    .expect("blank cells accept line connections")
                    .to_string();
            replace_cell(&mut self.grid.lines, coord, contents.clone());
            self.color_written_cell(coord);
            Some(contents)
        }
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
        let line = &self.grid.lines[coord.line];
        let (index, column) = index_and_column_for_coord(line, coord.column);
        let blank_run = line.get(index).is_some_and(grid::is_blank_run);
        if column == coord.column
            && let Some(atom) = self.grid.lines[coord.line].get_mut(index)
            && atom_width(atom) == 1
        {
            atom.contents = contents;
            atom.face.fg = foreground;
        } else if blank_run {
            replace_cell(&mut self.grid.lines, coord, contents);
            self.color_written_cell(coord);
        }
    }

    pub(super) fn write_diagonal_cell(
        &mut self,
        coord: Coord,
        glyph: &str,
        overwrite_active_endpoint: bool,
        consume_marker: bool,
    ) -> bool {
        let cell_boundary_is_writable = self.grid.lines.get(coord.line).is_none_or(|line| {
            let (index, column) = index_and_column_for_coord(line, coord.column);
            column == coord.column
                || index == line.len()
                || line.get(index).is_some_and(grid::is_blank_run)
        });
        if !cell_boundary_is_writable {
            return false;
        }
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
        replace_cell(&mut self.grid.lines, coord, glyph.to_owned());
        self.color_written_cell(coord);
        true
    }

    pub fn end_stroke(&mut self) {
        self.active_stroke = None;
    }
}
