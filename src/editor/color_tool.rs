#[cfg(test)]
use crate::model::Coord;
use crate::model::Face;
#[cfg(test)]
use crate::selection::SelectionBounds;

use super::Editor;

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

    #[cfg(test)]
    pub(super) fn color_written_cell(&mut self, coord: Coord) {
        let Some(data) = self.canvas.active_cell(coord) else {
            return;
        };
        let face = if data.atom.contents().chars().all(char::is_whitespace) {
            Face::default()
        } else {
            self.write_face()
        };
        self.canvas.set_face_at(coord, face);
    }

    #[cfg(test)]
    pub(super) fn color_written_bounds(&mut self, bounds: SelectionBounds) {
        self.commit_canvas();
        for line in bounds.top..=bounds.bottom {
            for column in bounds.left..=bounds.right {
                self.color_written_cell(Coord { line, column });
            }
        }
    }
}
