use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::model::{Coord, Face, StyledAtom};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct CanvasSelection {
    anchor: Coord,
    active: Coord,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelectionBounds {
    pub left: i16,
    pub right: i16,
    pub top: i16,
    pub bottom: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasRegion {
    pub left: i64,
    pub top: i64,
    pub width: usize,
    pub height: usize,
}

impl CanvasRegion {
    pub fn from_selection(bounds: SelectionBounds) -> Self {
        Self {
            left: i64::from(bounds.left),
            top: i64::from(bounds.top),
            width: bounds.width(),
            height: bounds.height(),
        }
    }
}

impl CanvasSelection {
    pub fn collapsed_at(coord: Coord) -> Self {
        Self {
            anchor: coord,
            active: coord,
        }
    }

    pub fn anchor(self) -> Coord {
        self.anchor
    }

    pub fn active(self) -> Coord {
        self.active
    }

    pub fn bounds(self) -> SelectionBounds {
        SelectionBounds {
            left: self.anchor.column.min(self.active.column),
            right: self.anchor.column.max(self.active.column),
            top: self.anchor.line.min(self.active.line),
            bottom: self.anchor.line.max(self.active.line),
        }
    }

    pub fn is_collapsed(self) -> bool {
        self.anchor == self.active
    }

    pub fn collapse(&mut self, coord: Coord) {
        self.anchor = coord;
        self.active = coord;
    }

    pub fn set_active(&mut self, coord: Coord) {
        self.active = coord;
    }

    pub fn select(&mut self, anchor: Coord, active: Coord) {
        self.anchor = anchor;
        self.active = active;
    }

    pub fn shift(&mut self, columns: usize, lines: usize) {
        let columns = i16::try_from(columns).unwrap_or(i16::MAX);
        let lines = i16::try_from(lines).unwrap_or(i16::MAX);
        self.anchor.column = self.anchor.column.saturating_add(columns);
        self.anchor.line = self.anchor.line.saturating_add(lines);
        self.active.column = self.active.column.saturating_add(columns);
        self.active.line = self.active.line.saturating_add(lines);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRectangle {
    pub rows: Vec<Vec<StyledAtom>>,
    pub width: usize,
}

impl TextRectangle {
    pub fn from_rows(mut rows: Vec<Vec<StyledAtom>>) -> Option<Self> {
        let width = rows.iter().map(|row| display_width(row)).max().unwrap_or(0);
        if width == 0 {
            return None;
        }
        for row in &mut rows {
            let row_width = display_width(row);
            row.extend((row_width..width).map(|_| blank_atom()));
        }
        Some(Self { rows, width })
    }

    pub fn from_text(text: &str) -> Option<Self> {
        let rows: Vec<Vec<StyledAtom>> = text
            .split('\n')
            .map(|row| {
                let row = row.strip_suffix('\r').unwrap_or(row);
                UnicodeSegmentation::graphemes(row, true)
                    .map(|contents| StyledAtom {
                        face: Face::default(),
                        contents: contents.to_string(),
                    })
                    .collect()
            })
            .collect();
        Self::from_rows(rows)
    }

    pub fn bounds_at(&self, origin: Coord) -> SelectionBounds {
        let width = i16::try_from(self.width.saturating_sub(1)).unwrap_or(i16::MAX);
        let height = i16::try_from(self.rows.len().saturating_sub(1)).unwrap_or(i16::MAX);
        SelectionBounds {
            left: origin.column,
            right: origin.column.saturating_add(width),
            top: origin.line,
            bottom: origin.line.saturating_add(height),
        }
    }
}

impl SelectionBounds {
    pub fn width(self) -> usize {
        usize::try_from(i32::from(self.right) - i32::from(self.left) + 1).unwrap_or(0)
    }

    pub fn height(self) -> usize {
        usize::try_from(i32::from(self.bottom) - i32::from(self.top) + 1).unwrap_or(0)
    }

    pub fn contains(self, coord: Coord) -> bool {
        (self.left..=self.right).contains(&coord.column)
            && (self.top..=self.bottom).contains(&coord.line)
    }
}

fn atom_width(atom: &StyledAtom) -> usize {
    UnicodeWidthStr::width(atom.contents.as_str()).max(usize::from(!atom.contents.is_empty()))
}

fn display_width(atoms: &[StyledAtom]) -> usize {
    atoms.iter().map(atom_width).sum()
}

fn blank_atom() -> StyledAtom {
    StyledAtom {
        face: Face::default(),
        contents: " ".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_normalize_reversed_corners() {
        let selection = CanvasSelection {
            anchor: Coord { line: 4, column: 8 },
            active: Coord { line: 2, column: 3 },
        };
        assert_eq!(
            selection.bounds(),
            SelectionBounds {
                left: 3,
                right: 8,
                top: 2,
                bottom: 4,
            }
        );
    }

    #[test]
    fn text_rectangle_normalizes_crlf_unequal_rows_trailing_spaces_and_blank_rows() {
        let rectangle = TextRectangle::from_text("😀x\r\n \r\ny  ").unwrap();
        assert_eq!(rectangle.width, 3);
        assert_eq!(rectangle.rows.len(), 3);
        let rows: Vec<String> = rectangle
            .rows
            .iter()
            .map(|row| row.iter().map(|atom| atom.contents.as_str()).collect())
            .collect();
        assert_eq!(rows, ["😀x", "   ", "y  "]);
        assert!(rectangle.rows.iter().all(|row| display_width(row) == 3));
        assert!(TextRectangle::from_text("").is_none());
        assert!(TextRectangle::from_text("\n\r\n").is_none());
    }
}
