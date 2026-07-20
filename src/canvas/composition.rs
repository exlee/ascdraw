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
            let line = i16::try_from(line).expect("dense layer line fits signed canvas range");
            let column_i16 =
                i16::try_from(column).expect("dense layer column fits signed canvas range");
            let right = column_i16.saturating_add(
                i16::try_from(width.saturating_sub(1))
                    .expect("dense atom width fits signed canvas range"),
            );
            let bounds = SelectionBounds {
                left: column_i16,
                right,
                top: line,
                bottom: line,
            };
            overwrite_rectangle(
                target,
                Coord {
                    line,
                    column: column_i16,
                },
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
