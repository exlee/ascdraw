#[cfg(test)]
use crate::model::Atom;
use crate::model::{Coord, Face};
use crate::selection::SelectionBounds;

use super::{Editor, index_and_column_for_coord};

impl Editor {
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
        let Some(data) = self.canvas.active_cell(coord) else {
            return;
        };
        let face = if data.atom.contents.chars().all(char::is_whitespace) {
            Face::default()
        } else {
            self.write_face()
        };
        self.canvas.set_face_at(coord, face);
    }

    pub(super) fn color_written_bounds(&mut self, bounds: SelectionBounds) {
        self.commit_canvas();
        for line in bounds.top..=bounds.bottom {
            for column in bounds.left..=bounds.right {
                self.color_written_cell(Coord { line, column });
            }
        }
        self.refresh_active_dense_view();
    }
}

#[cfg(test)]
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
