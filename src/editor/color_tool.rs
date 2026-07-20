use crate::model::Face;

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
}
