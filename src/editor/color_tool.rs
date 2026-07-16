use crate::model::{Atom, Coord, Face};
use crate::selection::SelectionBounds;

use super::{EditorState, index_and_column_for_coord};

impl EditorState {
    pub(super) fn write_face(&self) -> Face {
        let mut face = Face::default();
        if self.toolbar.multi_color_mode()
            && let Some(color) = self.toolbar.active_color().hex()
        {
            face.fg = color.to_owned();
        }
        face
    }

    pub(super) fn color_written_cell(&mut self, coord: Coord) {
        let foreground = self.write_face().fg;
        color_atom_at(&mut self.grid.lines, coord, &foreground);
    }

    pub(super) fn color_written_bounds(&mut self, bounds: SelectionBounds) {
        for line in bounds.top..=bounds.bottom {
            for column in bounds.left..=bounds.right {
                self.color_written_cell(Coord { line, column });
            }
        }
    }
}

pub(super) fn color_atom_at(lines: &mut [Vec<Atom>], coord: Coord, foreground: &str) {
    let Some(line) = lines.get_mut(coord.line) else {
        return;
    };
    let (index, column) = index_and_column_for_coord(line, coord.column);
    if column != coord.column {
        return;
    }
    let Some(atom) = line.get_mut(index) else {
        return;
    };
    if atom.contents.chars().all(char::is_whitespace) {
        atom.face = Face::default();
    } else {
        atom.face.fg = foreground.to_owned();
    }
}
