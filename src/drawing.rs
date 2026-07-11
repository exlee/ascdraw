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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineStyle {
    #[default]
    Thin,
    Heavy,
    Double,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    None,
    Arrow,
    Diamond,
    Circle,
}

pub fn connection(direction: Direction) -> u8 {
    match direction {
        Direction::Up => UP,
        Direction::Right => RIGHT,
        Direction::Down => DOWN,
        Direction::Left => LEFT,
    }
}

pub fn glyph_with_connection(glyph: &str, direction: Direction, style: LineStyle) -> Option<char> {
    let connections = connections_for_glyph(glyph)? | connection(direction);
    Some(glyph_for_connections(connections, style))
}

pub fn glyph_without_connection(glyph: &str, direction: Direction) -> Option<char> {
    let connections = connections_for_glyph(glyph)? & !connection(direction);
    Some(glyph_for_connections(connections, style_for_glyph(glyph)))
}

pub fn is_line_glyph(glyph: &str) -> bool {
    connections_for_glyph(glyph).is_some_and(|connections| connections != 0)
}

pub fn line_ending_glyph(
    ending: LineEnding,
    connected_direction: Direction,
    style: LineStyle,
) -> char {
    match ending {
        LineEnding::None => match style {
            LineStyle::Double => match connected_direction {
                Direction::Up | Direction::Down => '║',
                Direction::Right | Direction::Left => '═',
            },
            LineStyle::Thin | LineStyle::Heavy => {
                glyph_for_connections(connection(connected_direction), style)
            }
        },
        LineEnding::Arrow => match connected_direction.opposite() {
            Direction::Up => '▲',
            Direction::Right => '▶',
            Direction::Down => '▼',
            Direction::Left => '◀',
        },
        LineEnding::Diamond => '◆',
        LineEnding::Circle => '●',
    }
}

fn connections_for_glyph(glyph: &str) -> Option<u8> {
    Some(match glyph {
        " " => 0,
        "╵" | "╹" => UP,
        "╶" | "╺" => RIGHT,
        "╷" | "╻" => DOWN,
        "╴" | "╸" => LEFT,
        "└" | "╰" | "┗" | "╚" => UP | RIGHT,
        "│" | "┃" | "║" => UP | DOWN,
        "┘" | "╯" | "┛" | "╝" => UP | LEFT,
        "┌" | "╭" | "┏" | "╔" => RIGHT | DOWN,
        "─" | "━" | "═" => RIGHT | LEFT,
        "┐" | "╮" | "┓" | "╗" => DOWN | LEFT,
        "├" | "┣" | "╠" => UP | RIGHT | DOWN,
        "┴" | "┻" | "╩" => UP | RIGHT | LEFT,
        "┤" | "┫" | "╣" => UP | DOWN | LEFT,
        "┬" | "┳" | "╦" => RIGHT | DOWN | LEFT,
        "┼" | "╋" | "╬" => UP | RIGHT | DOWN | LEFT,
        _ => return None,
    })
}

fn style_for_glyph(glyph: &str) -> LineStyle {
    match glyph {
        "╹" | "╺" | "╻" | "╸" | "┗" | "┃" | "┛" | "┏" | "━" | "┓" | "┣" | "┻" | "┫" | "┳" | "╋" => {
            LineStyle::Heavy
        }
        "╚" | "║" | "╝" | "╔" | "═" | "╗" | "╠" | "╩" | "╣" | "╦" | "╬" => {
            LineStyle::Double
        }
        _ => LineStyle::Thin,
    }
}

fn glyph_for_connections(connections: u8, style: LineStyle) -> char {
    match style {
        LineStyle::Thin => thin_glyph_for_connections(connections),
        LineStyle::Heavy => heavy_glyph_for_connections(connections),
        LineStyle::Double => double_glyph_for_connections(connections),
    }
}

fn thin_glyph_for_connections(connections: u8) -> char {
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

fn heavy_glyph_for_connections(connections: u8) -> char {
    match connections {
        0 => ' ',
        UP => '╹',
        RIGHT => '╺',
        DOWN => '╻',
        LEFT => '╸',
        UP_RIGHT => '┗',
        UP_DOWN => '┃',
        UP_LEFT => '┛',
        RIGHT_DOWN => '┏',
        RIGHT_LEFT => '━',
        DOWN_LEFT => '┓',
        UP_RIGHT_DOWN => '┣',
        UP_RIGHT_LEFT => '┻',
        UP_DOWN_LEFT => '┫',
        RIGHT_DOWN_LEFT => '┳',
        ALL => '╋',
        _ => unreachable!("connections only use four direction bits"),
    }
}

fn double_glyph_for_connections(connections: u8) -> char {
    match connections {
        0 => ' ',
        UP => '╵',
        RIGHT => '╶',
        DOWN => '╷',
        LEFT => '╴',
        UP_RIGHT => '╚',
        UP_DOWN => '║',
        UP_LEFT => '╝',
        RIGHT_DOWN => '╔',
        RIGHT_LEFT => '═',
        DOWN_LEFT => '╗',
        UP_RIGHT_DOWN => '╠',
        UP_RIGHT_LEFT => '╩',
        UP_DOWN_LEFT => '╣',
        RIGHT_DOWN_LEFT => '╦',
        ALL => '╬',
        _ => unreachable!("connections only use four direction bits"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_a_vertical_line_into_a_tee() {
        assert_eq!(
            glyph_with_connection("│", Direction::Left, LineStyle::Thin),
            Some('┤')
        );
    }

    #[test]
    fn prefers_uniline_rounded_corners() {
        assert_eq!(
            glyph_with_connection("╴", Direction::Down, LineStyle::Thin),
            Some('╮')
        );
        assert_eq!(
            glyph_with_connection("┘", Direction::Right, LineStyle::Thin),
            Some('┴')
        );
    }

    #[test]
    fn does_not_overwrite_text() {
        assert_eq!(
            glyph_with_connection("x", Direction::Right, LineStyle::Thin),
            None
        );
    }

    #[test]
    fn distinguishes_lines_from_blanks_and_text() {
        assert!(is_line_glyph("─"));
        assert!(!is_line_glyph(" "));
        assert!(!is_line_glyph("◆"));
    }

    #[test]
    fn selected_style_controls_the_connected_glyph() {
        assert_eq!(
            glyph_with_connection("━", Direction::Down, LineStyle::Heavy),
            Some('┳')
        );
        assert_eq!(
            glyph_with_connection("═", Direction::Down, LineStyle::Double),
            Some('╦')
        );
    }

    #[test]
    fn endings_follow_direction_and_selected_style() {
        assert_eq!(
            line_ending_glyph(LineEnding::Arrow, Direction::Right, LineStyle::Thin),
            '◀'
        );
        assert_eq!(
            line_ending_glyph(LineEnding::Arrow, Direction::Up, LineStyle::Thin),
            '▼'
        );
        assert_eq!(
            line_ending_glyph(LineEnding::None, Direction::Left, LineStyle::Heavy),
            '╸'
        );
        assert_eq!(
            line_ending_glyph(LineEnding::None, Direction::Up, LineStyle::Double),
            '║'
        );
    }

    #[test]
    fn erasing_one_connection_preserves_the_rest_and_the_style() {
        assert_eq!(glyph_without_connection("┴", Direction::Left), Some('╰'));
        assert_eq!(glyph_without_connection("╋", Direction::Left), Some('┣'));
        assert_eq!(glyph_without_connection("═", Direction::Left), Some('╶'));
        assert_eq!(glyph_without_connection("╴", Direction::Left), Some(' '));
    }
}
