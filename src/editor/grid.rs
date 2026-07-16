use unicode_width::UnicodeWidthStr;

use crate::model::{Atom, Coord, Direction, Face};

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct ContentIndex {
    cells: Vec<Coord>,
    document_revision: u64,
    indexed_revision: u64,
    #[cfg(test)]
    rebuilds: usize,
}

#[cfg(test)]
impl ContentIndex {
    pub(crate) fn new(lines: &[Vec<Atom>]) -> Self {
        Self {
            cells: content_cells(lines),
            document_revision: 0,
            indexed_revision: 0,
            #[cfg(test)]
            rebuilds: 1,
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.document_revision = self.document_revision.wrapping_add(1);
    }

    pub(crate) fn cells<'a>(&'a mut self, lines: &[Vec<Atom>]) -> &'a [Coord] {
        if self.indexed_revision != self.document_revision {
            self.cells = content_cells(lines);
            self.indexed_revision = self.document_revision;
            #[cfg(test)]
            {
                self.rebuilds += 1;
            }
        }
        &self.cells
    }

    #[cfg(test)]
    pub(crate) fn rebuilds(&self) -> usize {
        self.rebuilds
    }
}

pub(crate) fn content_cells(lines: &[Vec<Atom>]) -> Vec<Coord> {
    let mut cells = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let mut column = 0usize;
        for atom in line {
            let width = UnicodeWidthStr::width(atom.contents.as_str());
            if !atom.contents.chars().all(char::is_whitespace) {
                cells.extend((column..column.saturating_add(width)).map(|column| Coord {
                    line: line_index,
                    column,
                }));
            }
            column = column.saturating_add(width);
        }
    }
    cells
}

pub(crate) fn edited_content_origin(lines: &[Vec<Atom>]) -> Option<Coord> {
    content_cells(lines)
        .into_iter()
        .reduce(|origin, coord| Coord {
            line: origin.line.min(coord.line),
            column: origin.column.min(coord.column),
        })
}

pub(crate) fn adjacent_coord(coord: Coord, direction: Direction) -> Option<Coord> {
    match direction {
        Direction::Up => Some(Coord {
            line: coord.line.checked_sub(1)?,
            column: coord.column,
        }),
        Direction::Right => Some(Coord {
            line: coord.line,
            column: coord.column.checked_add(1)?,
        }),
        Direction::Down => Some(Coord {
            line: coord.line.checked_add(1)?,
            column: coord.column,
        }),
        Direction::Left => Some(Coord {
            line: coord.line,
            column: coord.column.checked_sub(1)?,
        }),
    }
}

pub(crate) fn compact_blank_runs(lines: &mut [Vec<Atom>]) {
    for line in lines {
        compact_blank_line(line);
    }
}

pub(crate) fn compacted_blank_runs(lines: &[Vec<Atom>]) -> Vec<Vec<Atom>> {
    let mut lines = lines.to_vec();
    compact_blank_runs(&mut lines);
    lines
}

pub(crate) fn compact_blank_line(line: &mut Vec<Atom>) {
    let mut compacted: Vec<Atom> = Vec::with_capacity(line.len());
    for atom in line.drain(..) {
        if atom.contents.is_empty() {
            continue;
        }
        if is_blank_run(&atom)
            && let Some(previous) = compacted.last_mut()
            && is_blank_run(previous)
            && previous.face == atom.face
        {
            previous.contents.push_str(&atom.contents);
        } else {
            compacted.push(atom);
        }
    }
    *line = compacted;
}

pub(crate) fn blank_run(width: usize) -> Option<Atom> {
    blank_run_with_face(Face::default(), width)
}

pub(crate) fn blank_run_with_face(face: Face, width: usize) -> Option<Atom> {
    (width > 0).then(|| Atom {
        face,
        contents: " ".repeat(width),
    })
}

pub(crate) fn is_blank_run(atom: &Atom) -> bool {
    !atom.contents.is_empty() && atom.contents.bytes().all(|byte| byte == b' ')
}

pub(crate) fn split_blank_cell(line: &mut Vec<Atom>, target: usize) {
    let mut column = 0usize;
    for index in 0..line.len() {
        let width = UnicodeWidthStr::width(line[index].contents.as_str());
        let end = column.saturating_add(width);
        if target < end {
            if is_blank_run(&line[index]) && width > 1 {
                let face = line[index].face.clone();
                let offset = target - column;
                let mut replacement = Vec::with_capacity(3);
                replacement.extend(blank_run_with_face(face.clone(), offset));
                replacement.push(Atom {
                    face: face.clone(),
                    contents: " ".to_owned(),
                });
                replacement.extend(blank_run_with_face(face, width - offset - 1));
                line.splice(index..=index, replacement);
            }
            return;
        }
        column = end;
    }
}

pub(crate) fn expose_cursor_cells(line: &mut Vec<Atom>, column: usize) {
    if column > 0 {
        split_blank_cell(line, column - 1);
    }
    split_blank_cell(line, column);
}

pub(crate) fn expand_blank_runs(lines: &mut [Vec<Atom>]) {
    for line in lines {
        let mut expanded = Vec::new();
        for atom in line.drain(..) {
            if is_blank_run(&atom) {
                expanded.extend((0..atom.contents.len()).map(|_| Atom {
                    face: atom.face.clone(),
                    contents: " ".to_owned(),
                }));
            } else {
                expanded.push(atom);
            }
        }
        *line = expanded;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(face: Face, width: usize) -> Atom {
        Atom {
            face,
            contents: " ".repeat(width),
        }
    }

    #[test]
    fn compaction_is_idempotent_and_keeps_differing_faces_separate() {
        let plain = Face::default();
        let styled = Face {
            bg: "selection".to_owned(),
            ..Face::default()
        };
        let mut lines = vec![vec![
            blank(plain.clone(), 1),
            blank(plain, 3),
            blank(styled.clone(), 2),
            Atom {
                face: styled.clone(),
                contents: "x".to_owned(),
            },
            blank(styled, 1),
        ]];

        compact_blank_runs(&mut lines);
        let once = lines.clone();
        compact_blank_runs(&mut lines);

        assert_eq!(lines, once);
        assert_eq!(lines[0].len(), 4);
        assert_eq!(lines[0][0].contents, "    ");
        assert_eq!(lines[0][1].contents, "  ");
        assert_eq!(lines[0][2].contents, "x");
    }

    #[test]
    fn compaction_does_not_merge_nonblank_or_wide_graphemes() {
        let face = Face::default();
        let mut lines = vec![vec![
            Atom {
                face: face.clone(),
                contents: "a".to_owned(),
            },
            Atom {
                face: face.clone(),
                contents: "界".to_owned(),
            },
            blank(face, 2),
        ]];

        compact_blank_runs(&mut lines);

        assert_eq!(lines[0].len(), 3);
        assert_eq!(lines[0][1].contents, "界");
    }

    #[test]
    fn content_index_reuses_cells_until_invalidated() {
        let mut lines = vec![vec![Atom {
            face: Face::default(),
            contents: "x".to_owned(),
        }]];
        let mut index = ContentIndex::new(&lines);

        assert_eq!(index.cells(&lines), &[Coord::default()]);
        assert_eq!(index.cells(&lines), &[Coord::default()]);
        assert_eq!(index.rebuilds(), 1);

        lines[0][0].contents = "y".to_owned();
        index.invalidate();
        assert_eq!(index.cells(&lines), &[Coord::default()]);
        assert_eq!(index.rebuilds(), 2);
    }

    #[test]
    fn sparse_sixty_one_by_one_seventy_fixture_compacts_blank_cells() {
        let mut lines = vec![
            vec![
                Atom {
                    face: Face::default(),
                    contents: " ".to_owned(),
                };
                170
            ];
            61
        ];
        for cell in 0..520 {
            let line = cell % 61;
            let column = (cell * 19) % 170;
            lines[line][column].contents = "x".to_owned();
        }
        let before = lines.iter().map(Vec::len).sum::<usize>();

        compact_blank_runs(&mut lines);

        let after = lines.iter().map(Vec::len).sum::<usize>();
        assert_eq!(before, 61 * 170);
        assert!(after < before / 5, "expected sparse compaction: {after}");
        assert_eq!(content_cells(&lines).len(), 520);
    }
}
