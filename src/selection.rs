use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;

use crate::model::{Atom, Coord, Face};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct CanvasSelection {
    anchor: Coord,
    active: Coord,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct SelectionBounds {
    pub left: usize,
    pub right: usize,
    pub top: usize,
    pub bottom: usize,
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

    pub fn shift(&mut self, columns: usize, lines: usize) {
        self.anchor.column = self.anchor.column.saturating_add(columns);
        self.anchor.line = self.anchor.line.saturating_add(lines);
        self.active.column = self.active.column.saturating_add(columns);
        self.active.line = self.active.line.saturating_add(lines);
    }
}

impl SelectionBounds {
    pub fn width(self) -> usize {
        self.right.saturating_sub(self.left).saturating_add(1)
    }

    pub fn height(self) -> usize {
        self.bottom.saturating_sub(self.top).saturating_add(1)
    }

    pub fn contains(self, coord: Coord) -> bool {
        (self.left..=self.right).contains(&coord.column)
            && (self.top..=self.bottom).contains(&coord.line)
    }
}

pub fn selected_text(lines: &[Vec<Atom>], bounds: SelectionBounds) -> String {
    let mut rows = Vec::with_capacity(bounds.height());
    for line_index in bounds.top..=bounds.bottom {
        let line = lines.get(line_index).map(Vec::as_slice).unwrap_or_default();
        rows.push(selected_line_text(line, bounds.left, bounds.right));
    }
    rows.join("\n")
}

pub fn replace_range(
    lines: &mut Vec<Vec<Atom>>,
    bounds: SelectionBounds,
    replacement: Option<&str>,
) {
    while lines.len() <= bounds.bottom {
        lines.push(Vec::new());
    }
    for line in &mut lines[bounds.top..=bounds.bottom] {
        replace_line_range(line, bounds.left, bounds.right, replacement);
    }
}

fn selected_line_text(line: &[Atom], left: usize, right: usize) -> String {
    let width = SelectionBounds {
        left,
        right,
        top: 0,
        bottom: 0,
    }
    .width();
    let mut result = String::new();
    let mut result_width = 0;
    let mut column: usize = 0;
    for atom in line {
        let atom_width = atom_width(atom);
        let end = column.saturating_add(atom_width);
        if end <= left {
            column = end;
            continue;
        }
        if column > right {
            break;
        }
        let overlap_start = column.max(left);
        let overlap_end = end.min(right.saturating_add(1));
        if overlap_start > left.saturating_add(result_width) {
            push_spaces(&mut result, overlap_start - left - result_width);
            result_width = overlap_start - left;
        }
        let overlap_width = overlap_end.saturating_sub(overlap_start);
        if column >= left && end <= right.saturating_add(1) {
            result.push_str(&atom.contents);
        } else {
            push_spaces(&mut result, overlap_width);
        }
        result_width = result_width.saturating_add(overlap_width);
        column = end;
    }
    push_spaces(&mut result, width.saturating_sub(result_width));
    result
}

fn replace_line_range(line: &mut Vec<Atom>, left: usize, right: usize, replacement: Option<&str>) {
    let boundary = right.saturating_add(1);
    let mut prefix = Vec::new();
    let mut suffix = Vec::new();
    let mut column: usize = 0;
    for atom in line.iter() {
        let width = atom_width(atom);
        let end = column.saturating_add(width);
        if end <= left {
            prefix.push(atom.clone());
        } else if column < left {
            prefix.extend((column..left).map(|_| blank_atom()));
        }
        if column >= boundary {
            suffix.push(atom.clone());
        } else if end > boundary {
            suffix.extend((boundary..end).map(|_| blank_atom()));
        }
        column = end;
    }
    let prefix_width = display_width(&prefix);
    prefix.extend((prefix_width..left).map(|_| blank_atom()));
    prefix.extend(replacement_atoms(
        replacement,
        right.saturating_sub(left).saturating_add(1),
    ));
    prefix.extend(suffix);
    *line = prefix;
}

fn replacement_atoms(replacement: Option<&str>, width: usize) -> Vec<Atom> {
    let Some(replacement) = replacement else {
        return (0..width).map(|_| blank_atom()).collect();
    };
    let replacement_width = UnicodeWidthStr::width(replacement).max(1);
    let mut atoms = Vec::new();
    let mut remaining = width;
    while replacement_width <= remaining {
        atoms.push(Atom {
            face: Face::default(),
            contents: replacement.to_string(),
        });
        remaining -= replacement_width;
    }
    atoms.extend((0..remaining).map(|_| blank_atom()));
    atoms
}

fn atom_width(atom: &Atom) -> usize {
    UnicodeWidthStr::width(atom.contents.as_str()).max(usize::from(!atom.contents.is_empty()))
}

fn display_width(atoms: &[Atom]) -> usize {
    atoms.iter().map(atom_width).sum()
}

fn blank_atom() -> Atom {
    Atom {
        face: Face::default(),
        contents: " ".to_string(),
    }
}

fn push_spaces(target: &mut String, count: usize) {
    target.extend(std::iter::repeat_n(' ', count));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms(text: &[&str]) -> Vec<Atom> {
        text.iter()
            .map(|contents| Atom {
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
}
