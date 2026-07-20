use unicode_width::UnicodeWidthStr;

use crate::model::{Coord, StyledAtom};
use crate::selection::{SelectionBounds, TextRectangle, overwrite_rectangle};

pub(super) fn overlay_nonblank_atoms(
    target: &mut Vec<Vec<StyledAtom>>,
    source: &[Vec<StyledAtom>],
) -> Vec<SelectionBounds> {
    let mut covered = Vec::new();
    for (line, atoms) in source.iter().enumerate() {
        let mut column = 0usize;
        for atom in atoms {
            let width = UnicodeWidthStr::width(atom.contents.as_str()).max(1);
            if atom.contents.chars().all(char::is_whitespace) {
                column = column.saturating_add(width);
                continue;
            }
            let bounds = SelectionBounds {
                left: column,
                right: column.saturating_add(width.saturating_sub(1)),
                top: line,
                bottom: line,
            };
            overwrite_rectangle(
                target,
                Coord { line, column },
                &TextRectangle {
                    rows: vec![vec![atom.clone()]],
                    width,
                },
            );
            covered.push(bounds);
            column = column.saturating_add(width);
        }
    }
    covered
}
