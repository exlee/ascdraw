use std::time::{Duration, Instant};

use winit::keyboard::{Key, ModifiersState};

use crate::input::direction_for_key;
use crate::layout::VisibleCanvasCells;
use crate::model::{Coord, Direction};

const INITIAL_SECTOR_COLUMNS: usize = 21;
const INITIAL_SECTOR_ROWS: usize = 15;
const REFINED_GRID_COLUMNS: usize = 5;
const REFINED_GRID_ROWS: usize = 5;
const REFINED_SECTOR_COLUMNS: usize = 5;
const REFINED_SECTOR_ROWS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JumpBounds {
    pub line: i64,
    pub column: i64,
    pub columns: usize,
    pub rows: usize,
}

impl JumpBounds {
    fn center(self) -> Coord {
        Coord {
            line: usize::try_from(self.line + i64::try_from(self.rows / 2).unwrap_or(i64::MAX))
                .unwrap_or_default(),
            column: usize::try_from(
                self.column + i64::try_from(self.columns / 2).unwrap_or(i64::MAX),
            )
            .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpOverlay {
    pub sectors: Vec<JumpBounds>,
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JumpViewportPan {
    pub columns: i64,
    pub rows: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpUpdate {
    Pending,
    Changed,
    MoveTo(Coord),
    Select { anchor: Coord, active: Coord },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JumpLevel {
    Initial,
    Refined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpMode {
    sectors: Vec<JumpBounds>,
    columns: usize,
    rows: usize,
    selected: usize,
    start: Coord,
    select_on_land: bool,
    pending_viewport_pan: JumpViewportPan,
    level: JumpLevel,
    inactivity: Duration,
    deadline: Option<Instant>,
}

impl JumpMode {
    pub fn new(visible: VisibleCanvasCells, cursor: Coord, inactivity: Duration) -> Option<Self> {
        let bounds = visible_bounds(visible)?;
        let (sectors, columns, rows, selected) = initial_grid(bounds, cursor);
        Some(Self {
            sectors,
            columns,
            rows,
            selected,
            start: cursor,
            select_on_land: false,
            pending_viewport_pan: JumpViewportPan::default(),
            level: JumpLevel::Initial,
            inactivity,
            deadline: None,
        })
    }

    pub fn overlay(&self) -> JumpOverlay {
        JumpOverlay {
            sectors: self.sectors.clone(),
            selected: self.selected,
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub fn take_viewport_pan(&mut self) -> JumpViewportPan {
        std::mem::take(&mut self.pending_viewport_pan)
    }

    pub fn handle_key(&mut self, key: &Key, modifiers: ModifiersState, now: Instant) -> JumpUpdate {
        if modifiers != ModifiersState::empty() && modifiers != ModifiersState::SHIFT {
            return JumpUpdate::Pending;
        }
        let Some(direction) = direction_for_key(key) else {
            return JumpUpdate::Pending;
        };
        let moved = self.move_selection(direction);
        if moved || self.level == JumpLevel::Refined {
            self.select_on_land = modifiers == ModifiersState::SHIFT;
            self.deadline = Some(now + self.inactivity);
            JumpUpdate::Changed
        } else {
            JumpUpdate::Pending
        }
    }

    pub fn advance(&mut self, now: Instant) -> JumpUpdate {
        let Some(deadline) = self.deadline else {
            return JumpUpdate::Pending;
        };
        if now < deadline {
            return JumpUpdate::Pending;
        }

        let selected = self.sectors[self.selected];
        match self.level {
            JumpLevel::Initial => {
                let refined_columns = REFINED_GRID_COLUMNS * REFINED_SECTOR_COLUMNS;
                let refined_rows = REFINED_GRID_ROWS * REFINED_SECTOR_ROWS;
                let refined_column =
                    selected.column + centered_offset(selected.columns, refined_columns);
                let refined_line = selected.line + centered_offset(selected.rows, refined_rows);
                self.sectors = fixed_grid(
                    refined_line,
                    refined_column,
                    REFINED_GRID_COLUMNS,
                    REFINED_GRID_ROWS,
                    REFINED_SECTOR_COLUMNS,
                    REFINED_SECTOR_ROWS,
                );
                self.columns = REFINED_GRID_COLUMNS;
                self.rows = REFINED_GRID_ROWS;
                self.selected = middle_index(REFINED_GRID_COLUMNS, REFINED_GRID_ROWS);
                self.level = JumpLevel::Refined;
                self.deadline = Some(now + self.inactivity);
                JumpUpdate::Changed
            }
            JumpLevel::Refined if self.select_on_land => JumpUpdate::Select {
                anchor: self.start,
                active: selected.center(),
            },
            JumpLevel::Refined => JumpUpdate::MoveTo(selected.center()),
        }
    }

    fn move_selection(&mut self, direction: Direction) -> bool {
        let row = self.selected / self.columns;
        let column = self.selected % self.columns;
        let next = match direction {
            Direction::Left if column > 0 => self.selected - 1,
            Direction::Right if column + 1 < self.columns => self.selected + 1,
            Direction::Up if row > 0 => self.selected - self.columns,
            Direction::Down if row + 1 < self.rows => self.selected + self.columns,
            _ => return self.pan_at_edge(direction),
        };
        self.selected = next;
        true
    }

    fn pan_at_edge(&mut self, direction: Direction) -> bool {
        let selected = self.sectors[self.selected];
        let sector_columns = i64::try_from(selected.columns).unwrap_or(i64::MAX);
        let sector_rows = i64::try_from(selected.rows).unwrap_or(i64::MAX);
        let (column_delta, line_delta) = match direction {
            Direction::Left => (-sector_columns, 0),
            Direction::Right => (sector_columns, 0),
            Direction::Up => (0, -sector_rows),
            Direction::Down => (0, sector_rows),
        };
        let next_center_column = selected
            .column
            .saturating_add(column_delta)
            .saturating_add(sector_columns / 2);
        let next_center_line = selected
            .line
            .saturating_add(line_delta)
            .saturating_add(sector_rows / 2);
        if next_center_column < 0 || next_center_line < 0 {
            return false;
        }

        for sector in &mut self.sectors {
            sector.column = sector.column.saturating_add(column_delta);
            sector.line = sector.line.saturating_add(line_delta);
        }
        self.pending_viewport_pan.columns = self
            .pending_viewport_pan
            .columns
            .saturating_add(column_delta);
        self.pending_viewport_pan.rows = self.pending_viewport_pan.rows.saturating_add(line_delta);
        true
    }
}

fn middle_index(columns: usize, rows: usize) -> usize {
    (rows / 2) * columns + columns / 2
}

fn centered_offset(container: usize, content: usize) -> i64 {
    (i64::try_from(container).unwrap_or(i64::MAX) - i64::try_from(content).unwrap_or(i64::MAX)) / 2
}

fn initial_grid(bounds: JumpBounds, cursor: Coord) -> (Vec<JumpBounds>, usize, usize, usize) {
    let selected_line = i64::try_from(cursor.line).unwrap_or(i64::MAX)
        - i64::try_from(INITIAL_SECTOR_ROWS / 2).unwrap_or(i64::MAX);
    let selected_column = i64::try_from(cursor.column).unwrap_or(i64::MAX)
        - i64::try_from(INITIAL_SECTOR_COLUMNS / 2).unwrap_or(i64::MAX);
    let sector_columns = i64::try_from(INITIAL_SECTOR_COLUMNS).unwrap_or(i64::MAX);
    let sector_rows = i64::try_from(INITIAL_SECTOR_ROWS).unwrap_or(i64::MAX);
    let mut grid_column = selected_column;
    let mut selected_grid_column = 0;
    while grid_column > bounds.column {
        grid_column -= sector_columns;
        selected_grid_column += 1;
    }
    let mut grid_line = selected_line;
    let mut selected_grid_row = 0;
    while grid_line > bounds.line {
        grid_line -= sector_rows;
        selected_grid_row += 1;
    }
    let right = bounds.column + i64::try_from(bounds.columns).unwrap_or(i64::MAX);
    let bottom = bounds.line + i64::try_from(bounds.rows).unwrap_or(i64::MAX);
    let columns = usize::try_from((right - grid_column + sector_columns - 1) / sector_columns)
        .unwrap_or(1)
        .max(1);
    let rows = usize::try_from((bottom - grid_line + sector_rows - 1) / sector_rows)
        .unwrap_or(1)
        .max(1);
    (
        fixed_grid(
            grid_line,
            grid_column,
            columns,
            rows,
            INITIAL_SECTOR_COLUMNS,
            INITIAL_SECTOR_ROWS,
        ),
        columns,
        rows,
        selected_grid_row * columns + selected_grid_column,
    )
}

fn fixed_grid(
    origin_line: i64,
    origin_column: i64,
    columns: usize,
    rows: usize,
    sector_columns: usize,
    sector_rows: usize,
) -> Vec<JumpBounds> {
    let mut sectors = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            sectors.push(JumpBounds {
                line: origin_line + i64::try_from(row * sector_rows).unwrap_or(i64::MAX),
                column: origin_column + i64::try_from(column * sector_columns).unwrap_or(i64::MAX),
                columns: sector_columns,
                rows: sector_rows,
            });
        }
    }
    sectors
}

fn visible_bounds(visible: VisibleCanvasCells) -> Option<JumpBounds> {
    let right = visible
        .origin
        .0
        .saturating_add(i64::try_from(visible.columns).unwrap_or(i64::MAX))
        .max(0);
    let bottom = visible
        .origin
        .1
        .saturating_add(i64::try_from(visible.rows).unwrap_or(i64::MAX))
        .max(0);
    let left = visible.origin.0.max(0);
    let top = visible.origin.1.max(0);
    if right <= left || bottom <= top {
        return None;
    }
    Some(JumpBounds {
        line: top,
        column: left,
        columns: usize::try_from(right - left).ok()?,
        rows: usize::try_from(bottom - top).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(columns: usize, rows: usize) -> VisibleCanvasCells {
        VisibleCanvasCells {
            origin: (0, 0),
            columns,
            rows,
        }
    }

    fn start(columns: usize, rows: usize) -> JumpMode {
        JumpMode::new(
            visible(columns, rows),
            Coord {
                line: rows / 2,
                column: columns / 2,
            },
            Duration::from_millis(10),
        )
        .expect("visible canvas")
    }

    #[test]
    fn initial_grid_uses_fixed_twenty_one_by_fifteen_sectors_covering_the_viewport() {
        let jump = start(80, 24);
        let overlay = jump.overlay();
        assert_eq!(jump.columns, 5);
        assert_eq!(jump.rows, 3);
        assert_eq!(overlay.sectors.len(), 15);
        assert_eq!(overlay.selected, 7);
        assert!(
            overlay
                .sectors
                .iter()
                .all(|sector| sector.columns == 21 && sector.rows == 15)
        );
        let first = overlay.sectors[0];
        let last = overlay.sectors[14];
        assert!(first.column <= 0);
        assert!(first.line <= 0);
        assert!(last.column + i64::try_from(last.columns).unwrap() >= 80);
        assert!(last.line + i64::try_from(last.rows).unwrap() >= 24);
    }

    #[test]
    fn selected_initial_sector_is_centered_exactly_on_the_cursor() {
        let cursor = Coord {
            line: 12,
            column: 40,
        };
        let jump = JumpMode::new(visible(80, 24), cursor, Duration::from_millis(10))
            .expect("visible canvas");
        assert_eq!(jump.sectors[jump.selected].center(), cursor);
    }

    #[test]
    fn movement_starts_and_restarts_the_deadline() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        assert_eq!(jump.deadline(), None);
        assert_eq!(
            jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now),
            JumpUpdate::Changed
        );
        assert_eq!(jump.overlay().selected, 6);
        assert_eq!(jump.deadline(), Some(now + Duration::from_millis(10)));

        let moved_at = now + Duration::from_millis(3);
        assert_eq!(
            jump.handle_key(
                &Key::Named(winit::keyboard::NamedKey::ArrowUp),
                ModifiersState::empty(),
                moved_at
            ),
            JumpUpdate::Changed
        );
        assert_eq!(jump.overlay().selected, 1);
        assert_eq!(jump.deadline(), Some(moved_at + Duration::from_millis(10)));
    }

    #[test]
    fn shift_hjkl_and_shift_arrows_move_in_jump_mode() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        assert_eq!(
            jump.handle_key(&Key::Character("H".into()), ModifiersState::SHIFT, now),
            JumpUpdate::Changed
        );
        assert_eq!(jump.overlay().selected, 6);

        assert_eq!(
            jump.handle_key(
                &Key::Named(winit::keyboard::NamedKey::ArrowUp),
                ModifiersState::SHIFT,
                now
            ),
            JumpUpdate::Changed
        );
        assert_eq!(jump.overlay().selected, 1);
    }

    #[test]
    fn moving_past_the_initial_grid_edge_pans_by_one_initial_sector() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        for _ in 0..2 {
            jump.handle_key(&Key::Character("l".into()), ModifiersState::empty(), now);
        }
        let before = jump.sectors[jump.selected].center();

        assert_eq!(
            jump.handle_key(&Key::Character("l".into()), ModifiersState::empty(), now),
            JumpUpdate::Changed
        );
        assert_eq!(
            jump.take_viewport_pan(),
            JumpViewportPan {
                columns: 21,
                rows: 0,
            }
        );
        assert_eq!(
            jump.sectors[jump.selected].center(),
            Coord {
                line: before.line,
                column: before.column + 21,
            }
        );
        assert_eq!(jump.take_viewport_pan(), JumpViewportPan::default());
    }

    #[test]
    fn moving_past_the_refined_grid_edge_pans_by_one_refined_sector() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("l".into()), ModifiersState::empty(), now);
        jump.advance(now + Duration::from_millis(10));
        for _ in 0..2 {
            jump.handle_key(&Key::Character("l".into()), ModifiersState::empty(), now);
        }
        let before = jump.sectors[jump.selected].center();

        assert_eq!(
            jump.handle_key(&Key::Character("l".into()), ModifiersState::empty(), now),
            JumpUpdate::Changed
        );
        assert_eq!(
            jump.take_viewport_pan(),
            JumpViewportPan {
                columns: 5,
                rows: 0,
            }
        );
        assert_eq!(
            jump.sectors[jump.selected].center(),
            Coord {
                line: before.line,
                column: before.column + 5,
            }
        );
    }

    #[test]
    fn edge_panning_accumulates_until_the_runtime_applies_it() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        for _ in 0..4 {
            jump.handle_key(&Key::Character("l".into()), ModifiersState::empty(), now);
        }
        assert_eq!(
            jump.take_viewport_pan(),
            JumpViewportPan {
                columns: 42,
                rows: 0,
            }
        );
    }

    #[test]
    fn first_transition_centers_twenty_five_five_by_five_sectors_and_starts_its_timer() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        let selected = jump.overlay().sectors[jump.overlay().selected];
        assert_eq!(
            jump.advance(now + Duration::from_millis(10)),
            JumpUpdate::Changed
        );

        let overlay = jump.overlay();
        assert_eq!(overlay.sectors.len(), 25);
        assert_eq!(overlay.selected, 12);
        assert_eq!(jump.deadline(), Some(now + Duration::from_millis(20)));
        assert_eq!(overlay.sectors[0].column, selected.column - 2);
        assert_eq!(overlay.sectors[0].line, selected.line - 5);
        assert_eq!(overlay.sectors[24].column + 5, selected.column + 23);
        assert_eq!(overlay.sectors[24].line + 5, selected.line + 20);
        assert!(
            overlay
                .sectors
                .iter()
                .all(|sector| { sector.columns == 5 && sector.rows == 5 })
        );
        assert_eq!(
            overlay.sectors[overlay.selected].center(),
            selected.center()
        );
    }

    #[test]
    fn second_level_timer_lands_without_requiring_another_move() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        jump.advance(now + Duration::from_millis(10));
        let expected = jump.sectors[jump.selected].center();
        assert_eq!(
            jump.advance(now + Duration::from_millis(20)),
            JumpUpdate::MoveTo(expected)
        );
    }

    #[test]
    fn shift_movement_selects_from_the_jump_start_on_landing() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("H".into()), ModifiersState::SHIFT, now);
        jump.advance(now + Duration::from_millis(10));
        let expected = jump.sectors[jump.selected].center();
        assert_eq!(
            jump.advance(now + Duration::from_millis(20)),
            JumpUpdate::Select {
                anchor: jump.start,
                active: expected,
            }
        );
    }

    #[test]
    fn most_recent_accepted_direction_controls_whether_landing_selects() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("H".into()), ModifiersState::SHIFT, now);
        jump.advance(now + Duration::from_millis(10));

        let moved_at = now + Duration::from_millis(12);
        jump.handle_key(
            &Key::Character("k".into()),
            ModifiersState::empty(),
            moved_at,
        );
        let expected = jump.sectors[jump.selected].center();
        assert_eq!(
            jump.advance(moved_at + Duration::from_millis(10)),
            JumpUpdate::MoveTo(expected)
        );

        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        jump.advance(now + Duration::from_millis(10));
        let moved_at = now + Duration::from_millis(12);
        jump.handle_key(&Key::Character("K".into()), ModifiersState::SHIFT, moved_at);
        let expected = jump.sectors[jump.selected].center();
        assert_eq!(
            jump.advance(moved_at + Duration::from_millis(10)),
            JumpUpdate::Select {
                anchor: jump.start,
                active: expected,
            }
        );
    }

    #[test]
    fn refined_movement_restarts_the_automatic_landing_timer() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        jump.advance(now + Duration::from_millis(10));
        let moved_at = now + Duration::from_millis(12);
        jump.handle_key(
            &Key::Character("k".into()),
            ModifiersState::empty(),
            moved_at,
        );
        assert_eq!(jump.deadline(), Some(moved_at + Duration::from_millis(10)));
        assert_eq!(
            jump.advance(now + Duration::from_millis(20)),
            JumpUpdate::Pending
        );
    }

    #[test]
    fn refined_direction_at_an_edge_also_restarts_the_landing_timer() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        jump.advance(now + Duration::from_millis(10));
        for offset in 0..2 {
            jump.handle_key(
                &Key::Character("k".into()),
                ModifiersState::empty(),
                now + Duration::from_millis(11 + offset),
            );
        }
        let edge_attempt = now + Duration::from_millis(15);
        assert_eq!(
            jump.handle_key(
                &Key::Character("k".into()),
                ModifiersState::empty(),
                edge_attempt,
            ),
            JumpUpdate::Changed
        );
        assert_eq!(
            jump.deadline(),
            Some(edge_attempt + Duration::from_millis(10))
        );
    }

    #[test]
    fn blocked_initial_movement_does_not_start_the_timer() {
        let now = Instant::now();
        let mut jump = start(80, 24);
        jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now);
        jump.deadline = None;
        assert_eq!(
            jump.handle_key(&Key::Character("h".into()), ModifiersState::empty(), now),
            JumpUpdate::Pending
        );
        assert_eq!(jump.deadline(), None);
    }

    #[test]
    fn negative_visible_coordinates_are_clipped_before_fixed_grid_layout() {
        let jump = JumpMode::new(
            VisibleCanvasCells {
                origin: (-2, -1),
                columns: 10,
                rows: 6,
            },
            Coord::default(),
            Duration::from_millis(10),
        )
        .expect("visible nonnegative canvas");
        let overlay = jump.overlay();
        assert_eq!(overlay.sectors.len(), 1);
        assert_eq!(overlay.sectors[0].line, -7);
        assert_eq!(overlay.sectors[0].column, -10);
        assert_eq!(overlay.sectors[0].columns, 21);
        assert_eq!(overlay.sectors[0].rows, 15);
    }
}
