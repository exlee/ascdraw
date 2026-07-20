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
    use crate::app::ThemeConfig;

    fn atoms(text: &[&str]) -> Vec<StyledAtom> {
        text.iter()
            .map(|contents| StyledAtom {
                face: Face::default(),
                contents: (*contents).to_string(),
            })
            .collect()
    }

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
    fn extraction_preserves_rectangle_width_blank_rows_and_trailing_spaces() {
        let lines = vec![atoms(&["x", "a", "b", " ", "z"]), Vec::new(), atoms(&["q"])];
        let text = selected_text(
            &lines,
            SelectionBounds {
                left: 1,
                right: 3,
                top: 0,
                bottom: 2,
            },
        );
        assert_eq!(text, "ab \n   \n   ");
    }

    #[test]
    fn atom_extraction_normalizes_origin_and_preserves_faces_and_dimensions() {
        let special = Face {
            fg: "#ff0000".to_string(),
            ..Face::default()
        };
        let lines = vec![
            vec![
                StyledAtom {
                    face: Face::default(),
                    contents: "x".to_string(),
                },
                StyledAtom {
                    face: special.clone(),
                    contents: "a".to_string(),
                },
            ],
            Vec::new(),
        ];
        let selected = selected_atoms(
            &lines,
            SelectionBounds {
                left: 1,
                right: 2,
                top: 0,
                bottom: 1,
            },
        );
        assert_eq!(selected.len(), 2);
        assert!(selected.iter().all(|line| display_width(line) == 2));
        assert_eq!(selected[0][0].face, special);
        assert_eq!(
            selected_text(
                &selected,
                SelectionBounds {
                    left: 0,
                    right: 1,
                    top: 0,
                    bottom: 1
                }
            ),
            "a \n  "
        );
    }

    #[test]
    fn extraction_and_replacement_do_not_split_wide_graphemes() {
        let mut lines = vec![atoms(&["a", "😀", "z"])];
        assert_eq!(
            selected_text(
                &lines,
                SelectionBounds {
                    left: 1,
                    right: 2,
                    top: 0,
                    bottom: 0,
                },
            ),
            "😀"
        );
        replace_range(
            &mut lines,
            SelectionBounds {
                left: 2,
                right: 2,
                top: 0,
                bottom: 0,
            },
            Some("x"),
        );
        assert_eq!(
            selected_text(
                &lines,
                SelectionBounds {
                    left: 0,
                    right: 3,
                    top: 0,
                    bottom: 0
                }
            ),
            "a xz"
        );
    }

    #[test]
    fn signed_region_preserves_outside_blanks_rows_faces_and_trailing_width() {
        let special = ThemeConfig::default().selection;
        let lines = vec![
            vec![
                StyledAtom {
                    face: special.clone(),
                    contents: "a".to_string(),
                },
                StyledAtom {
                    face: Face::default(),
                    contents: "b".to_string(),
                },
            ],
            Vec::new(),
        ];
        let region = CanvasRegion {
            left: -1,
            top: -1,
            width: 4,
            height: 4,
        };

        let extracted = region_atoms(&lines, region);
        assert_eq!(region_text(&lines, region), "    \n ab \n    \n    ");
        assert!(extracted.iter().all(|row| display_width(row) == 4));
        assert_eq!(extracted[1][1].face, special);
    }

    #[test]
    fn signed_region_blanks_cells_that_cut_wide_grapheme_boundaries() {
        let lines = vec![atoms(&["a", "😀", "z"])];

        assert_eq!(
            region_text(
                &lines,
                CanvasRegion {
                    left: 2,
                    top: 0,
                    width: 2,
                    height: 1,
                },
            ),
            " z"
        );
        assert_eq!(
            region_text(
                &lines,
                CanvasRegion {
                    left: 0,
                    top: 0,
                    width: 2,
                    height: 1,
                },
            ),
            "a "
        );
    }

    #[test]
    fn text_rectangle_normalizes_crlf_unequal_rows_trailing_spaces_and_blank_rows() {
        let rectangle = TextRectangle::from_text("😀x\r\n \r\ny  ").unwrap();
        assert_eq!(rectangle.width, 3);
        assert_eq!(rectangle.rows.len(), 3);
        assert_eq!(
            selected_text(
                &rectangle.rows,
                SelectionBounds {
                    left: 0,
                    right: 2,
                    top: 0,
                    bottom: 2,
                }
            ),
            "😀x\n   \ny  "
        );
        assert!(TextRectangle::from_text("").is_none());
        assert!(TextRectangle::from_text("\n\r\n").is_none());
    }

    #[test]
    fn rectangular_overwrite_preserves_outside_atoms_and_blanks_wide_boundaries() {
        let outside_face = Face {
            fg: "#123456".to_string(),
            ..Face::default()
        };
        let left = StyledAtom {
            face: outside_face.clone(),
            contents: "L".to_string(),
        };
        let right = StyledAtom {
            face: outside_face,
            contents: "R".to_string(),
        };
        let mut lines = vec![vec![
            left.clone(),
            StyledAtom {
                face: Face::default(),
                contents: "😀".to_string(),
            },
            right.clone(),
        ]];
        overwrite_rectangle(
            &mut lines,
            Coord { line: 0, column: 2 },
            &TextRectangle::from_text("x").unwrap(),
        );
        assert_eq!(lines[0].first(), Some(&left));
        assert_eq!(lines[0].last(), Some(&right));
        assert_eq!(
            selected_text(
                &lines,
                SelectionBounds {
                    left: 0,
                    right: 3,
                    top: 0,
                    bottom: 0
                }
            ),
            "L xR"
        );
    }
}
