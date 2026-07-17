use crate::model::{Coord, Direction};
use crate::toolbar::RoutingMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RouteStep {
    Orthogonal(Direction),
    Diagonal {
        horizontal: Direction,
        vertical: Direction,
    },
}

pub(super) fn route_steps(start: Coord, end: Coord, mode: RoutingMode) -> Vec<RouteStep> {
    let horizontal = if end.column >= start.column {
        Direction::Right
    } else {
        Direction::Left
    };
    let vertical = if end.line >= start.line {
        Direction::Down
    } else {
        Direction::Up
    };
    let columns = start.column.abs_diff(end.column);
    let lines = start.line.abs_diff(end.line);
    let mut steps = Vec::with_capacity(columns.saturating_add(lines));
    match mode {
        RoutingMode::HorizontalVertical => {
            orthogonal(&mut steps, horizontal, columns);
            orthogonal(&mut steps, vertical, lines);
        }
        RoutingMode::VerticalHorizontal => {
            orthogonal(&mut steps, vertical, lines);
            orthogonal(&mut steps, horizontal, columns);
        }
        RoutingMode::HorizontalDiagonal => {
            orthogonal(&mut steps, horizontal, columns.saturating_sub(lines));
            diagonal(&mut steps, horizontal, vertical, columns.min(lines));
            orthogonal(&mut steps, vertical, lines.saturating_sub(columns));
        }
        RoutingMode::VerticalDiagonal => {
            orthogonal(&mut steps, vertical, lines.saturating_sub(columns));
            diagonal(&mut steps, horizontal, vertical, columns.min(lines));
            orthogonal(&mut steps, horizontal, columns.saturating_sub(lines));
        }
        RoutingMode::Stairs => stairs(&mut steps, horizontal, vertical, columns, lines),
    }
    steps
}

fn orthogonal(steps: &mut Vec<RouteStep>, direction: Direction, count: usize) {
    steps.extend(std::iter::repeat_n(RouteStep::Orthogonal(direction), count));
}

fn diagonal(steps: &mut Vec<RouteStep>, horizontal: Direction, vertical: Direction, count: usize) {
    steps.extend(std::iter::repeat_n(
        RouteStep::Diagonal {
            horizontal,
            vertical,
        },
        count,
    ));
}

fn stairs(
    steps: &mut Vec<RouteStep>,
    horizontal: Direction,
    vertical: Direction,
    columns: usize,
    lines: usize,
) {
    let total = columns.saturating_add(lines);
    let mut used_columns = 0;
    let mut used_lines = 0;
    for index in 1..=total {
        let expected_columns = index.saturating_mul(columns).saturating_add(total / 2) / total;
        if used_columns < expected_columns {
            steps.push(RouteStep::Orthogonal(horizontal));
            used_columns += 1;
        } else if used_lines < lines {
            steps.push(RouteStep::Orthogonal(vertical));
            used_lines += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destination(start: Coord, steps: &[RouteStep]) -> Coord {
        steps.iter().fold(start, |mut coord, step| {
            let directions: &[Direction] = match step {
                RouteStep::Orthogonal(direction) => std::slice::from_ref(direction),
                RouteStep::Diagonal {
                    horizontal,
                    vertical,
                } => &[*horizontal, *vertical],
            };
            for direction in directions {
                match direction {
                    Direction::Up => coord.line -= 1,
                    Direction::Right => coord.column += 1,
                    Direction::Down => coord.line += 1,
                    Direction::Left => coord.column -= 1,
                }
            }
            coord
        })
    }

    #[test]
    fn every_routing_mode_reaches_targets_in_all_quadrants_and_axis_dominance() {
        let start = Coord { line: 6, column: 6 };
        let targets = [
            Coord { line: 4, column: 1 },
            Coord { line: 1, column: 4 },
            Coord {
                line: 8,
                column: 11,
            },
            Coord {
                line: 11,
                column: 8,
            },
        ];
        for mode in [
            RoutingMode::HorizontalVertical,
            RoutingMode::VerticalHorizontal,
            RoutingMode::HorizontalDiagonal,
            RoutingMode::VerticalDiagonal,
            RoutingMode::Stairs,
        ] {
            for target in targets {
                assert_eq!(
                    destination(start, &route_steps(start, target, mode)),
                    target
                );
            }
        }
    }
}
