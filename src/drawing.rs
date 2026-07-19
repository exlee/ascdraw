use crate::model::Direction;
use serde::{Deserialize, Serialize};

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
    Dashed,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    None,
    Directional(DirectionalEnding),
    Fixed(char),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum DirectionalEnding {
    WhiteTriangle,
    BlackTriangle,
    Arrow,
    WhiteSmallTriangle,
    BlackSmallTriangle,
    Bidirectional,
}

#[cfg(test)]
pub const DIRECTIONAL_ENDINGS: [DirectionalEnding; 6] = [
    DirectionalEnding::WhiteTriangle,
    DirectionalEnding::BlackTriangle,
    DirectionalEnding::Arrow,
    DirectionalEnding::WhiteSmallTriangle,
    DirectionalEnding::BlackSmallTriangle,
    DirectionalEnding::Bidirectional,
];

pub const ARROWS: [&str; 22] = [
    "△", "▽", "◁", "▷", "▲", "▼", "◀", "▶", "↑", "↓", "←", "→", "▵", "▿", "◃", "▹", "▴", "▾", "◂",
    "▸", "↕", "↔",
];

pub const LINE_ENDINGS: [LineEnding; 27] = [
    LineEnding::None,
    LineEnding::Directional(DirectionalEnding::WhiteTriangle),
    LineEnding::Directional(DirectionalEnding::BlackTriangle),
    LineEnding::Directional(DirectionalEnding::Arrow),
    LineEnding::Directional(DirectionalEnding::WhiteSmallTriangle),
    LineEnding::Directional(DirectionalEnding::BlackSmallTriangle),
    LineEnding::Directional(DirectionalEnding::Bidirectional),
    LineEnding::Fixed('□'),
    LineEnding::Fixed('■'),
    LineEnding::Fixed('▫'),
    LineEnding::Fixed('▪'),
    LineEnding::Fixed('◆'),
    LineEnding::Fixed('◊'),
    LineEnding::Fixed('·'),
    LineEnding::Fixed('∙'),
    LineEnding::Fixed('•'),
    LineEnding::Fixed('●'),
    LineEnding::Fixed('◦'),
    LineEnding::Fixed('Ø'),
    LineEnding::Fixed('ø'),
    LineEnding::Fixed('╳'),
    LineEnding::Fixed('╱'),
    LineEnding::Fixed('╲'),
    LineEnding::Fixed('÷'),
    LineEnding::Fixed('×'),
    LineEnding::Fixed('±'),
    LineEnding::Fixed('¤'),
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CornerStyle {
    #[default]
    Smooth,
    Sharp,
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
    glyph_with_connection_and_corner(glyph, direction, style, CornerStyle::Smooth)
}

pub fn glyph_with_connection_and_corner(
    glyph: &str,
    direction: Direction,
    style: LineStyle,
    corner_style: CornerStyle,
) -> Option<char> {
    let connections = connections_for_glyph(glyph)? | connection(direction);
    Some(glyph_for_connections(connections, style, corner_style))
}

pub fn glyph_for_connection_pair(
    first: Direction,
    second: Direction,
    style: LineStyle,
    corner_style: CornerStyle,
) -> char {
    glyph_for_connections(connection(first) | connection(second), style, corner_style)
}

pub fn glyph_without_connection(glyph: &str, direction: Direction) -> Option<char> {
    let connections = connections_for_glyph(glyph)? & !connection(direction);
    Some(glyph_for_connections(
        connections,
        style_for_glyph(glyph),
        corner_style_for_glyph(glyph),
    ))
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
            LineStyle::Thin | LineStyle::Heavy | LineStyle::Dashed => {
                glyph_for_connections(connection(connected_direction), style, CornerStyle::Smooth)
            }
        },
        LineEnding::Directional(style) => {
            directional_ending_glyph(style, connected_direction.opposite())
        }
        LineEnding::Fixed(glyph) => glyph,
    }
}

pub fn directional_ending_glyph(style: DirectionalEnding, outward: Direction) -> char {
    match style {
        DirectionalEnding::WhiteTriangle => match outward {
            Direction::Up => '△',
            Direction::Right => '▷',
            Direction::Down => '▽',
            Direction::Left => '◁',
        },
        DirectionalEnding::BlackTriangle => match outward {
            Direction::Up => '▲',
            Direction::Right => '▶',
            Direction::Down => '▼',
            Direction::Left => '◀',
        },
        DirectionalEnding::Arrow => match outward {
            Direction::Up => '↑',
            Direction::Right => '→',
            Direction::Down => '↓',
            Direction::Left => '←',
        },
        DirectionalEnding::WhiteSmallTriangle => match outward {
            Direction::Up => '▵',
            Direction::Right => '▹',
            Direction::Down => '▿',
            Direction::Left => '◃',
        },
        DirectionalEnding::BlackSmallTriangle => match outward {
            Direction::Up => '▴',
            Direction::Right => '▸',
            Direction::Down => '▾',
            Direction::Left => '◂',
        },
        DirectionalEnding::Bidirectional => match outward {
            Direction::Up | Direction::Down => '↕',
            Direction::Right | Direction::Left => '↔',
        },
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
        "│" | "┃" | "║" | "┆" => UP | DOWN,
        "┘" | "╯" | "┛" | "╝" => UP | LEFT,
        "┌" | "╭" | "┏" | "╔" => RIGHT | DOWN,
        "─" | "━" | "═" | "┄" => RIGHT | LEFT,
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
        "┄" | "┆" => LineStyle::Dashed,
        _ => LineStyle::Thin,
    }
}

fn corner_style_for_glyph(glyph: &str) -> CornerStyle {
    match glyph {
        "┌" | "┐" | "└" | "┘" => CornerStyle::Sharp,
        _ => CornerStyle::Smooth,
    }
}

fn glyph_for_connections(connections: u8, style: LineStyle, corner_style: CornerStyle) -> char {
    match style {
        LineStyle::Thin => thin_glyph_for_connections(connections, corner_style),
        LineStyle::Heavy => heavy_glyph_for_connections(connections),
        LineStyle::Double => double_glyph_for_connections(connections),
        LineStyle::Dashed => dashed_glyph_for_connections(connections, corner_style),
    }
}

fn dashed_glyph_for_connections(connections: u8, corner_style: CornerStyle) -> char {
    match connections {
        UP | DOWN | UP_DOWN => '╵',
        RIGHT | LEFT | RIGHT_LEFT => '╴',
        _ => thin_glyph_for_connections(connections, corner_style),
    }
}

fn thin_glyph_for_connections(connections: u8, corner_style: CornerStyle) -> char {
    match connections {
        0 => ' ',
        UP => '╵',
        RIGHT => '╶',
        DOWN => '╷',
        LEFT => '╴',
        UP_RIGHT => match corner_style {
            CornerStyle::Smooth => '╰',
            CornerStyle::Sharp => '└',
        },
        UP_DOWN => '│',
        UP_LEFT => match corner_style {
            CornerStyle::Smooth => '╯',
            CornerStyle::Sharp => '┘',
        },
        RIGHT_DOWN => match corner_style {
            CornerStyle::Smooth => '╭',
            CornerStyle::Sharp => '┌',
        },
        RIGHT_LEFT => '─',
        DOWN_LEFT => match corner_style {
            CornerStyle::Smooth => '╮',
            CornerStyle::Sharp => '┐',
        },
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
pub const DECORATORS: [&str; 20] = [
    "□", "■", "▫", "▪", "◆", "◊", "·", "∙", "•", "●", "◦", "Ø", "ø", "╳", "╱", "╲", "÷", "×", "±",
    "¤",
];

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
        assert_eq!(
            glyph_with_connection("╶", Direction::Left, LineStyle::Dashed),
            Some('╴')
        );
        assert_eq!(
            glyph_with_connection("╷", Direction::Up, LineStyle::Dashed),
            Some('╵')
        );
    }

    #[test]
    fn endings_follow_direction_and_selected_style() {
        assert_eq!(
            line_ending_glyph(
                LineEnding::Directional(DirectionalEnding::BlackTriangle),
                Direction::Right,
                LineStyle::Thin,
            ),
            '◀'
        );
        assert_eq!(
            line_ending_glyph(
                LineEnding::Directional(DirectionalEnding::BlackTriangle),
                Direction::Up,
                LineStyle::Thin,
            ),
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
    fn all_directional_endings_rotate_and_fixed_endings_do_not() {
        let expected = [
            ['△', '▷', '▽', '◁'],
            ['▲', '▶', '▼', '◀'],
            ['↑', '→', '↓', '←'],
            ['▵', '▹', '▿', '◃'],
            ['▴', '▸', '▾', '◂'],
            ['↕', '↔', '↕', '↔'],
        ];
        let directions = [
            Direction::Up,
            Direction::Right,
            Direction::Down,
            Direction::Left,
        ];
        for (style, glyphs) in DIRECTIONAL_ENDINGS.into_iter().zip(expected) {
            for (direction, glyph) in directions.into_iter().zip(glyphs) {
                assert_eq!(directional_ending_glyph(style, direction), glyph);
            }
        }
        for glyph in DECORATORS {
            let glyph = glyph.chars().next().unwrap();
            for connected_direction in directions {
                assert_eq!(
                    line_ending_glyph(
                        LineEnding::Fixed(glyph),
                        connected_direction,
                        LineStyle::Thin,
                    ),
                    glyph
                );
            }
        }
    }

    #[test]
    fn selected_corner_style_controls_thin_turns() {
        assert_eq!(
            glyph_with_connection_and_corner(
                "╴",
                Direction::Down,
                LineStyle::Thin,
                CornerStyle::Smooth,
            ),
            Some('╮')
        );
        assert_eq!(
            glyph_with_connection_and_corner(
                "╴",
                Direction::Down,
                LineStyle::Thin,
                CornerStyle::Sharp,
            ),
            Some('┐')
        );
    }

    #[test]
    fn erasing_one_connection_preserves_the_rest_and_the_style() {
        assert_eq!(glyph_without_connection("┴", Direction::Left), Some('╰'));
        assert_eq!(glyph_without_connection("╋", Direction::Left), Some('┣'));
        assert_eq!(glyph_without_connection("═", Direction::Left), Some('╶'));
        assert_eq!(glyph_without_connection("┄", Direction::Left), Some('╴'));
        assert_eq!(glyph_without_connection("╴", Direction::Left), Some(' '));
    }
}
