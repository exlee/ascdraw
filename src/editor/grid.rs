use unicode_width::UnicodeWidthStr;

use crate::model::{Coord, Direction, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH, StyledAtom};

pub(crate) fn content_cells(lines: &[Vec<StyledAtom>]) -> Vec<Coord> {
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

pub(crate) fn edited_content_origin(lines: &[Vec<StyledAtom>]) -> Option<Coord> {
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
            column: coord
                .column
                .checked_add(1)
                .filter(|column| *column < MAX_CANVAS_WIDTH)?,
        }),
        Direction::Down => Some(Coord {
            line: coord
                .line
                .checked_add(1)
                .filter(|line| *line < MAX_CANVAS_HEIGHT)?,
            column: coord.column,
        }),
        Direction::Left => Some(Coord {
            line: coord.line,
            column: coord.column.checked_sub(1)?,
        }),
    }
}
