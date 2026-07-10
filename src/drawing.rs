use crate::model::Direction;

const UP: u8 = 1 << 0;
const RIGHT: u8 = 1 << 1;
const DOWN: u8 = 1 << 2;
const LEFT: u8 = 1 << 3;
const UP_RIGHT: u8 = UP | RIGHT;
const UP_DOWN: u8 = UP | DOWN;
const UP_LEFT: u8 = UP | LEFT;
const RIGHT_DOWN: u8 = RIGHT | DOWN;
const RIGHT_LEFT: u8 = RIGHT | LEFT;
const DOWN_LEFT: u8 = DOWN | LEFT;
const UP_RIGHT_DOWN: u8 = UP | RIGHT | DOWN;
const UP_RIGHT_LEFT: u8 = UP | RIGHT | LEFT;
const UP_DOWN_LEFT: u8 = UP | DOWN | LEFT;
const RIGHT_DOWN_LEFT: u8 = RIGHT | DOWN | LEFT;
const ALL: u8 = UP | RIGHT | DOWN | LEFT;

pub fn connection(direction: Direction) -> u8 {
    match direction {
        Direction::Up => UP,
        Direction::Right => RIGHT,
        Direction::Down => DOWN,
        Direction::Left => LEFT,
    }
}

pub fn glyph_with_connection(glyph: &str, direction: Direction) -> Option<char> {
    let connections = connections_for_glyph(glyph)? | connection(direction);
    Some(glyph_for_connections(connections))
}

fn connections_for_glyph(glyph: &str) -> Option<u8> {
    Some(match glyph {
        " " => 0,
        "╵" => UP,
        "╶" => RIGHT,
        "╷" => DOWN,
        "╴" => LEFT,
        "└" | "╰" => UP | RIGHT,
        "│" => UP | DOWN,
        "┘" | "╯" => UP | LEFT,
        "┌" | "╭" => RIGHT | DOWN,
        "─" => RIGHT | LEFT,
        "┐" | "╮" => DOWN | LEFT,
        "├" => UP | RIGHT | DOWN,
        "┴" => UP | RIGHT | LEFT,
        "┤" => UP | DOWN | LEFT,
        "┬" => RIGHT | DOWN | LEFT,
        "┼" => UP | RIGHT | DOWN | LEFT,
        _ => return None,
    })
}

fn glyph_for_connections(connections: u8) -> char {
    match connections {
        0 => ' ',
        UP => '╵',
        RIGHT => '╶',
        DOWN => '╷',
        LEFT => '╴',
        UP_RIGHT => '╰',
        UP_DOWN => '│',
        UP_LEFT => '╯',
        RIGHT_DOWN => '╭',
        RIGHT_LEFT => '─',
        DOWN_LEFT => '╮',
        UP_RIGHT_DOWN => '├',
        UP_RIGHT_LEFT => '┴',
        UP_DOWN_LEFT => '┤',
        RIGHT_DOWN_LEFT => '┬',
        ALL => '┼',
        _ => unreachable!("connections only use four direction bits"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_a_vertical_line_into_a_tee() {
        assert_eq!(glyph_with_connection("│", Direction::Left), Some('┤'));
    }

    #[test]
    fn prefers_uniline_rounded_corners() {
        assert_eq!(glyph_with_connection("╴", Direction::Down), Some('╮'));
        assert_eq!(glyph_with_connection("┘", Direction::Right), Some('┴'));
    }

    #[test]
    fn does_not_overwrite_text() {
        assert_eq!(glyph_with_connection("x", Direction::Right), None);
    }
}
