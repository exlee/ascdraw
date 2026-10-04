//! Stretching a definition to an instance size. Connected lines are mapped
//! onto the new grid and the gaps between connected neighbours are filled
//! with straight segments; implicit groups keep their shape and move as a
//! unit, either with the anchor they contain or proportionally.

use std::collections::BTreeMap;

use crate::drawing::{glyph_connects, glyph_with_directions, is_line_glyph, straight_glyph_like};
use crate::model::{Coord, Direction};

use super::{Anchor, AnchorTarget, Cells, ObjectCell, ObjectDefinition, Side};

/// Maps a local position on an axis of `from` cells onto `to` cells, keeping
/// both edges on the edges.
fn scale(value: i16, from: i16, to: i16) -> i16 {
    if from <= 1 || from == to {
        return value;
    }
    (f64::from(value) * f64::from(to - 1) / f64::from(from - 1)).round() as i16
}

/// Piecewise-linear mapping of one axis. Both edges and every anchor are
/// fixed points, so lines lengthen between anchors and anchored cells land
/// exactly where their anchors resolve.
struct AxisMap {
    points: Vec<(f64, f64)>,
}

impl AxisMap {
    fn new(from: i16, to: i16, anchors: impl Iterator<Item = (i16, i16)>) -> Self {
        let mut points = vec![(0, 0), ((from - 1).max(0), (to - 1).max(0))];
        points.extend(anchors);
        points.sort_by_key(|&(source, _)| source);
        points.dedup_by_key(|&mut (source, _)| source);
        let mut floor = i16::MIN;
        let points = points
            .into_iter()
            .map(|(source, target)| {
                // Crossing anchors would fold the axis; keep it monotonic.
                floor = floor.max(target);
                (f64::from(source), f64::from(floor))
            })
            .collect();
        Self { points }
    }

    fn map_f(&self, value: f64) -> f64 {
        let first = self.points[0];
        let last = self.points[self.points.len() - 1];
        if value <= first.0 {
            return first.1 + (value - first.0);
        }
        if value >= last.0 {
            return last.1 + (value - last.0);
        }
        let index = self.points.partition_point(|&(source, _)| source <= value);
        let (low, high) = (self.points[index - 1], self.points[index]);
        low.1 + (value - low.0) * (high.1 - low.1) / (high.0 - low.0)
    }

    fn map(&self, value: i16) -> i16 {
        self.map_f(f64::from(value)).round() as i16
    }

    /// Maps a span by its center, keeping its length.
    fn map_span(&self, low: i16, high: i16, to: i16) -> i16 {
        let length = high - low;
        let center = self.map_f(f64::from(low + high) / 2.0);
        let start = (center - f64::from(length) / 2.0).round() as i16;
        start.clamp(0, (to - 1 - length).max(0))
    }
}

fn is_group_member(cell: &ObjectCell) -> bool {
    !cell.atom.chars().all(char::is_whitespace) && !is_line_glyph(&cell.atom)
}

fn is_letter(cell: Option<&ObjectCell>) -> bool {
    cell.is_some_and(|cell| {
        let mut characters = cell.atom.chars();
        characters.next().is_some_and(|c| c.is_ascii_alphabetic()) && characters.next().is_none()
    })
}

/// Implicit groups: visible non-line cells touching horizontally or
/// vertically. A single space between two letters does not split a group.
pub fn implicit_groups(cells: &Cells) -> Vec<Vec<Coord>> {
    let members = cells
        .iter()
        .filter(|(_, cell)| is_group_member(cell))
        .map(|(local, _)| (local.line, local.column))
        .collect::<Vec<_>>();
    let index = members
        .iter()
        .enumerate()
        .map(|(index, key)| (*key, index))
        .collect::<BTreeMap<_, _>>();
    let mut parent = (0..members.len()).collect::<Vec<_>>();
    fn root(parent: &mut [usize], mut node: usize) -> usize {
        while parent[node] != node {
            parent[node] = parent[parent[node]];
            node = parent[node];
        }
        node
    }
    let union = |parent: &mut Vec<usize>, first: usize, second: usize| {
        let (first, second) = (root(parent, first), root(parent, second));
        if first != second {
            parent[second.max(first)] = second.min(first);
        }
    };
    for (position, &(line, column)) in members.iter().enumerate() {
        for neighbour in [(line, column + 1), (line + 1, column)] {
            if let Some(&other) = index.get(&neighbour) {
                union(&mut parent, position, other);
            }
        }
        let here = cells.get(Coord { line, column });
        let gap = cells.get(Coord {
            line,
            column: column + 1,
        });
        let after = Coord {
            line,
            column: column + 2,
        };
        if is_letter(here)
            && gap.is_none_or(|cell| cell.atom.chars().all(char::is_whitespace))
            && is_letter(cells.get(after))
            && let Some(&other) = index.get(&(after.line, after.column))
        {
            union(&mut parent, position, other);
        }
    }
    let mut groups = BTreeMap::<usize, Vec<Coord>>::new();
    for (position, &(line, column)) in members.iter().enumerate() {
        groups
            .entry(root(&mut parent, position))
            .or_default()
            .push(Coord { line, column });
    }
    groups.into_values().collect()
}

/// Anchor positions after stretching the definition to `width`×`height`.
pub fn resolve_anchors(
    definition: &ObjectDefinition,
    width: i16,
    height: i16,
) -> BTreeMap<u32, Coord> {
    let mut resolved = BTreeMap::new();
    for anchor in &definition.anchors {
        resolve_anchor(
            definition,
            anchor.id,
            (width, height),
            &mut resolved,
            &mut Vec::new(),
        );
    }
    resolved
}

fn resolve_anchor(
    definition: &ObjectDefinition,
    id: u32,
    size: (i16, i16),
    resolved: &mut BTreeMap<u32, Coord>,
    visiting: &mut Vec<u32>,
) -> Option<Coord> {
    if let Some(coord) = resolved.get(&id) {
        return Some(*coord);
    }
    let anchor = definition.anchor(id)?;
    if visiting.contains(&id) {
        return None;
    }
    visiting.push(id);
    let column = resolve_axis(definition, anchor, true, size, resolved, visiting);
    let line = resolve_axis(definition, anchor, false, size, resolved, visiting);
    visiting.pop();
    let coord = Coord { line, column };
    resolved.insert(id, coord);
    Some(coord)
}

fn resolve_axis(
    definition: &ObjectDefinition,
    anchor: &Anchor,
    horizontal: bool,
    size: (i16, i16),
    resolved: &mut BTreeMap<u32, Coord>,
    visiting: &mut Vec<u32>,
) -> i16 {
    let (value, from, to, target, side) = if horizontal {
        (
            anchor.at.column,
            definition.width,
            size.0,
            anchor.horizontal,
            anchor.kind.horizontal_side(),
        )
    } else {
        (
            anchor.at.line,
            definition.height,
            size.1,
            anchor.vertical,
            anchor.kind.vertical_side(),
        )
    };
    let proportional = scale(value, from, to);
    match target {
        Some(AnchorTarget::Bounds) => match side {
            Some(Side::Min) => value,
            Some(Side::Max) => value + (to - from),
            None => proportional,
        },
        Some(AnchorTarget::Anchor(other)) => {
            let Some(reference) = definition.anchor(other).map(|reference| reference.at) else {
                return proportional;
            };
            match resolve_anchor(definition, other, size, resolved, visiting) {
                Some(moved) if horizontal => moved.column + (value - reference.column),
                Some(moved) => moved.line + (value - reference.line),
                None => proportional,
            }
        }
        None => proportional,
    }
}

/// The definition cells laid out at `width`×`height`.
pub fn render_definition(definition: &ObjectDefinition, width: i16, height: i16) -> Cells {
    if width == definition.width && height == definition.height {
        return definition.cells.clone();
    }
    let anchors = resolve_anchors(definition, width, height);
    let columns = AxisMap::new(
        definition.width,
        width,
        definition
            .anchors
            .iter()
            .filter_map(|anchor| Some((anchor.at.column, anchors.get(&anchor.id)?.column))),
    );
    let lines = AxisMap::new(
        definition.height,
        height,
        definition
            .anchors
            .iter()
            .filter_map(|anchor| Some((anchor.at.line, anchors.get(&anchor.id)?.line))),
    );
    let map = |local: Coord| Coord {
        line: lines.map(local.line),
        column: columns.map(local.column),
    };
    let mut stretched = Cells::default();
    // Line cells that shrink onto one cell keep only their outward
    // connections, so a corner stays a corner.
    let mut merged = BTreeMap::<(i16, i16), Vec<Direction>>::new();
    for (local, cell) in definition.cells.iter() {
        if is_group_member(cell) {
            continue;
        }
        let target = map(local);
        if is_line_glyph(&cell.atom) {
            let all = [
                Direction::Up,
                Direction::Right,
                Direction::Down,
                Direction::Left,
            ]
            .into_iter()
            .filter(|direction| glyph_connects(&cell.atom, *direction))
            .collect::<Vec<_>>();
            let outward = all
                .iter()
                .copied()
                .filter(|direction| {
                    crate::editor::adjacent_coord(local, *direction)
                        .is_none_or(|neighbour| map(neighbour) != target)
                })
                .collect::<Vec<_>>();
            let key = (target.line, target.column);
            let collided = merged.contains_key(&key);
            let directions = merged.entry(key).or_default();
            for direction in outward {
                if !directions.contains(&direction) {
                    directions.push(direction);
                }
            }
            let rebuilt = (collided || directions.len() != all.len())
                .then(|| glyph_with_directions(&cell.atom, directions))
                .flatten();
            let mut placed = cell.clone();
            if let Some(glyph) = rebuilt {
                placed.atom = glyph.to_string();
            }
            if collided && let Some(existing) = stretched.get(target) {
                placed.face = existing.face.clone();
                placed.line = existing.line.clone().or(placed.line);
            }
            stretched.insert(target, placed);
        } else {
            stretched.insert(target, cell.clone());
        }
        if !is_line_glyph(&cell.atom) {
            continue;
        }
        for (direction, horizontal) in [(Direction::Right, true), (Direction::Down, false)] {
            let next_local = if horizontal {
                Coord {
                    line: local.line,
                    column: local.column + 1,
                }
            } else {
                Coord {
                    line: local.line + 1,
                    column: local.column,
                }
            };
            let connected = glyph_connects(&cell.atom, direction)
                && definition
                    .cells
                    .get(next_local)
                    .is_some_and(|next| glyph_connects(&next.atom, direction.opposite()));
            if !connected {
                continue;
            }
            let next = map(next_local);
            // Mixed styles fill with the style of the run the gap continues.
            let previous_local = if horizontal {
                Coord {
                    line: local.line,
                    column: local.column - 1,
                }
            } else {
                Coord {
                    line: local.line - 1,
                    column: local.column,
                }
            };
            let after = definition.cells.get(next_local).unwrap_or(cell);
            let same_style = |first: &ObjectCell, second: &ObjectCell| {
                straight_glyph_like(&first.atom, horizontal)
                    == straight_glyph_like(&second.atom, horizontal)
            };
            let in_run = definition
                .cells
                .get(previous_local)
                .is_some_and(|previous| {
                    is_line_glyph(&previous.atom) && same_style(previous, cell)
                });
            let source = if same_style(cell, after) || in_run {
                cell
            } else {
                after
            };
            let fill = ObjectCell {
                atom: straight_glyph_like(&source.atom, horizontal).to_string(),
                face: source.face.clone(),
                line: None,
            };
            if horizontal {
                for column in target.column + 1..next.column {
                    stretched.insert(
                        Coord {
                            line: target.line,
                            column,
                        },
                        fill.clone(),
                    );
                }
            } else {
                for line in target.line + 1..next.line {
                    stretched.insert(
                        Coord {
                            line,
                            column: target.column,
                        },
                        fill.clone(),
                    );
                }
            }
        }
    }

    for group in implicit_groups(&definition.cells) {
        let anchored = definition
            .anchors
            .iter()
            .filter(|anchor| group.contains(&anchor.at))
            .min_by_key(|anchor| anchor.id)
            .and_then(|anchor| Some((anchor.at, *anchors.get(&anchor.id)?)));
        let (line_shift, column_shift) = match anchored {
            Some((at, moved)) => (moved.line - at.line, moved.column - at.column),
            None => {
                let (top, bottom) = span(group.iter().map(|local| local.line));
                let (left, right) = span(group.iter().map(|local| local.column));
                (
                    lines.map_span(top, bottom, height) - top,
                    columns.map_span(left, right, width) - left,
                )
            }
        };
        for &local in &group {
            let placed = Coord {
                line: local.line + line_shift,
                column: local.column + column_shift,
            };
            // A symbol sitting on a line stays joined to it.
            for direction in [
                Direction::Left,
                Direction::Right,
                Direction::Up,
                Direction::Down,
            ] {
                let Some(neighbour_local) = crate::editor::adjacent_coord(local, direction) else {
                    continue;
                };
                if group.contains(&neighbour_local) {
                    continue;
                }
                let Some(neighbour) = definition.cells.get(neighbour_local) else {
                    continue;
                };
                if !glyph_connects(&neighbour.atom, direction.opposite()) {
                    continue;
                }
                let end = map(neighbour_local);
                let horizontal = matches!(direction, Direction::Left | Direction::Right);
                let fill = ObjectCell {
                    atom: straight_glyph_like(&neighbour.atom, horizontal).to_string(),
                    face: neighbour.face.clone(),
                    line: None,
                };
                let gap: Vec<Coord> = match direction {
                    Direction::Left if end.line == placed.line => (end.column + 1..placed.column)
                        .map(|column| Coord {
                            line: placed.line,
                            column,
                        })
                        .collect(),
                    Direction::Right if end.line == placed.line => (placed.column + 1..end.column)
                        .map(|column| Coord {
                            line: placed.line,
                            column,
                        })
                        .collect(),
                    Direction::Up if end.column == placed.column => (end.line + 1..placed.line)
                        .map(|line| Coord {
                            line,
                            column: placed.column,
                        })
                        .collect(),
                    Direction::Down if end.column == placed.column => (placed.line + 1..end.line)
                        .map(|line| Coord {
                            line,
                            column: placed.column,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                for coord in gap {
                    stretched.insert(coord, fill.clone());
                }
            }
            if let Some(cell) = definition.cells.get(local) {
                stretched.insert(placed, cell.clone());
            }
        }
    }
    stretched
        .0
        .retain(|&(line, column), _| (0..height).contains(&line) && (0..width).contains(&column));
    stretched
}

fn span(values: impl Iterator<Item = i16>) -> (i16, i16) {
    values.fold((i16::MAX, i16::MIN), |(low, high), value| {
        (low.min(value), high.max(value))
    })
}
