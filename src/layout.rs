use crate::model::Coord;
use crate::render::CellMetrics;
use crate::toolbar::ToolbarState;

pub const PADDING: usize = 20;
pub const SCROLL_MARGIN_CELLS: i64 = 3;
pub const TOOLTIP_GRID_GAP: usize = PADDING;
const TRANSPARENT_MENUBAR_TOP_INSET_PT: f64 = 24.0;

#[derive(Clone, Copy, Debug)]
pub struct LayoutMetrics {
    pub top_padding: usize,
    pub grid_top: usize,
    pub cols: usize,
    pub rows: usize,
    pub grid_bottom: usize,
    pub tooltip_top: usize,
    pub tooltip_visible: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewportOffset {
    pub x: i64,
    pub y: i64,
}

impl ViewportOffset {
    pub fn compensate_for_prepend(
        &mut self,
        columns: usize,
        lines: usize,
        cell_size: (usize, usize),
    ) {
        self.x = self.x.saturating_sub(cell_shift(columns, cell_size.0));
        self.y = self.y.saturating_sub(cell_shift(lines, cell_size.1));
    }

    pub fn origin(self, cell_size: (usize, usize)) -> (i64, i64) {
        (
            self.x
                .saturating_neg()
                .div_euclid(cell_size.0.max(1) as i64),
            self.y
                .saturating_neg()
                .div_euclid(cell_size.1.max(1) as i64),
        )
    }

    pub fn set_origin(&mut self, origin: (i64, i64), cell_size: (usize, usize)) {
        self.x = origin
            .0
            .saturating_mul(cell_size.0.max(1) as i64)
            .saturating_neg();
        self.y = origin
            .1
            .saturating_mul(cell_size.1.max(1) as i64)
            .saturating_neg();
    }

    pub fn reanchor_cursor(
        &mut self,
        cursor: Coord,
        old_cell_size: (usize, usize),
        new_cell_size: (usize, usize),
        old_grid_top: usize,
        new_grid_top: usize,
    ) {
        self.x = self
            .x
            .saturating_add(cell_delta(cursor.column, old_cell_size.0, new_cell_size.0));
        self.y = self
            .y
            .saturating_add(old_grid_top as i64 - new_grid_top as i64)
            .saturating_add(cell_delta(cursor.line, old_cell_size.1, new_cell_size.1));
    }
}

fn cell_shift(count: usize, size: usize) -> i64 {
    i64::try_from(count)
        .unwrap_or(i64::MAX)
        .saturating_mul(i64::try_from(size).unwrap_or(i64::MAX))
}

#[cfg(test)]
pub fn legal_origin_range(min: usize, max: usize, viewport_cells: usize) -> (i64, i64) {
    let min = i64::try_from(min).unwrap_or(i64::MAX);
    let max = i64::try_from(max).unwrap_or(i64::MAX);
    let (inner_start, inner_end) = inner_screen_offsets(viewport_cells);
    (
        min.saturating_sub(inner_end),
        max.saturating_sub(inner_start),
    )
}

fn inner_screen_offsets(viewport_cells: usize) -> (i64, i64) {
    let viewport_cells = i64::try_from(viewport_cells.max(1)).unwrap_or(i64::MAX);
    let outer_margin = SCROLL_MARGIN_CELLS
        .saturating_sub(1)
        .min(viewport_cells.saturating_sub(1) / 2);
    (
        outer_margin.saturating_add(1),
        viewport_cells.saturating_sub(outer_margin),
    )
}

fn content_intersects_inner_screen(
    origin: (i64, i64),
    viewport: (usize, usize),
    content: &[Coord],
) -> bool {
    let horizontal = inner_screen_offsets(viewport.0);
    let vertical = inner_screen_offsets(viewport.1);
    content.iter().any(|coord| {
        let x = i64::try_from(coord.column)
            .unwrap_or(i64::MAX)
            .saturating_sub(origin.0);
        let y = i64::try_from(coord.line)
            .unwrap_or(i64::MAX)
            .saturating_sub(origin.1);
        (horizontal.0..=horizontal.1).contains(&x) && (vertical.0..=vertical.1).contains(&y)
    })
}

pub fn cursor_origin(current: i64, cursor: usize, viewport_cells: usize) -> i64 {
    let cursor = i64::try_from(cursor).unwrap_or(i64::MAX);
    let last_visible = i64::try_from(viewport_cells.saturating_sub(1)).unwrap_or(i64::MAX);
    if cursor < current {
        cursor
    } else if cursor > current.saturating_add(last_visible) {
        cursor.saturating_sub(last_visible)
    } else {
        current
    }
}

pub fn navigation_origin(
    current: (i64, i64),
    cursor: Coord,
    viewport: (usize, usize),
    content: &[Coord],
) -> Option<(i64, i64)> {
    let desired = (
        cursor_origin(current.0, cursor.column, viewport.0),
        cursor_origin(current.1, cursor.line, viewport.1),
    );
    if content.is_empty() {
        return Some(desired);
    }
    content_intersects_inner_screen(desired, viewport, content).then_some(desired)
}

pub fn clamped_navigation_origin(
    current: (i64, i64),
    cursor: Coord,
    viewport: (usize, usize),
    content: &[Coord],
) -> (i64, i64) {
    let desired = (
        cursor_origin(current.0, cursor.column, viewport.0),
        cursor_origin(current.1, cursor.line, viewport.1),
    );
    if content.is_empty() {
        return desired;
    }
    let horizontal = inner_screen_offsets(viewport.0);
    let vertical = inner_screen_offsets(viewport.1);
    content
        .iter()
        .map(|coord| {
            let x = i64::try_from(coord.column).unwrap_or(i64::MAX);
            let y = i64::try_from(coord.line).unwrap_or(i64::MAX);
            let origin = (
                desired.0.clamp(
                    x.saturating_sub(horizontal.1),
                    x.saturating_sub(horizontal.0),
                ),
                desired
                    .1
                    .clamp(y.saturating_sub(vertical.1), y.saturating_sub(vertical.0)),
            );
            let distance = desired
                .0
                .abs_diff(origin.0)
                .saturating_add(desired.1.abs_diff(origin.1));
            (distance, origin)
        })
        .min_by_key(|candidate| *candidate)
        .map_or(desired, |(_, origin)| origin)
}

fn cell_delta(index: usize, old_size: usize, new_size: usize) -> i64 {
    let index = i64::try_from(index).unwrap_or(i64::MAX);
    let old_size = i64::try_from(old_size).unwrap_or(i64::MAX);
    let new_size = i64::try_from(new_size).unwrap_or(i64::MAX);
    index
        .saturating_mul(old_size)
        .saturating_sub(index.saturating_mul(new_size))
}

pub fn content_top_padding(scale_factor: f64, transparent_menubar: bool) -> usize {
    content_top_padding_for_scale_factor(scale_factor, transparent_menubar)
}

pub fn content_top_padding_for_scale_factor(scale_factor: f64, transparent_menubar: bool) -> usize {
    if transparent_menubar {
        PADDING + (TRANSPARENT_MENUBAR_TOP_INSET_PT * scale_factor).round() as usize
    } else {
        PADDING
    }
}

pub fn layout_metrics(
    width: usize,
    height: usize,
    metrics: &CellMetrics,
    toolbar_cell_height: usize,
    toolbar: &ToolbarState,
    transparent_menubar: bool,
    scale_factor: f64,
) -> LayoutMetrics {
    let top_padding = content_top_padding(scale_factor, transparent_menubar);
    let grid_top = top_padding + crate::toolbar::toolbar_height(toolbar, toolbar_cell_height);
    let cols = width.saturating_sub(PADDING * 2) / metrics.cell_width.max(1);
    let (rows, grid_bottom, tooltip_top, tooltip_visible) =
        vertical_geometry(height, grid_top, metrics.cell_height, toolbar_cell_height);
    LayoutMetrics {
        top_padding,
        grid_top,
        cols,
        rows: rows.max(1),
        grid_bottom,
        tooltip_top,
        tooltip_visible,
    }
}

fn vertical_geometry(
    height: usize,
    grid_top: usize,
    grid_cell_height: usize,
    tooltip_cell_height: usize,
) -> (usize, usize, usize, bool) {
    let tooltip_top = height.saturating_sub(tooltip_cell_height);
    let tooltip_visible = tooltip_cell_height > 0
        && height >= tooltip_cell_height
        && tooltip_top >= grid_top.saturating_add(TOOLTIP_GRID_GAP);
    let grid_bottom = if tooltip_visible {
        tooltip_top.saturating_sub(TOOLTIP_GRID_GAP)
    } else {
        height.saturating_sub(PADDING)
    };
    let rows = grid_bottom.saturating_sub(grid_top) / grid_cell_height.max(1);
    (rows.max(1), grid_bottom, tooltip_top, tooltip_visible)
}

#[cfg(test)]
fn layout_rows(
    height: usize,
    cell_height: usize,
    transparent_menubar: bool,
    scale_factor: f64,
) -> usize {
    let top_padding = content_top_padding_for_scale_factor(scale_factor, transparent_menubar);
    height.saturating_sub(top_padding + PADDING) / cell_height.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_menubar_uses_fixed_point_top_inset() {
        assert_eq!(content_top_padding_for_scale_factor(1.0, false), PADDING);
        assert_eq!(
            content_top_padding_for_scale_factor(1.0, true),
            PADDING + 24
        );
        assert_eq!(
            content_top_padding_for_scale_factor(2.0, true),
            PADDING + 48
        );
    }

    #[test]
    fn transparent_menubar_reduces_available_rows_by_fixed_inset() {
        let height = PADDING * 2 + 10 * 18;
        assert_eq!(layout_rows(height, 18, false, 1.0), 10);
        assert_eq!(layout_rows(height, 18, true, 1.0), 8);
    }

    #[test]
    fn zoom_reanchors_the_cursor_top_left_exactly() {
        let cursor = Coord {
            line: 7,
            column: 11,
        };
        let mut viewport = ViewportOffset { x: 3, y: -5 };
        let before = cursor_top_left(cursor, (8, 16), 44, viewport);

        viewport.reanchor_cursor(cursor, (8, 16), (11, 20), 44, 44);

        assert_eq!(cursor_top_left(cursor, (11, 20), 44, viewport), before);
    }

    #[test]
    fn reanchoring_includes_changes_to_the_fixed_toolbar_height() {
        let cursor = Coord { line: 2, column: 0 };
        let mut viewport = ViewportOffset::default();
        let before = cursor_top_left(cursor, (8, 16), 44, viewport);

        viewport.reanchor_cursor(cursor, (8, 16), (8, 16), 44, 48);

        assert_eq!(cursor_top_left(cursor, (8, 16), 48, viewport), before);
    }

    #[test]
    fn boxed_toolbar_height_anchors_grid_below_both_borders() {
        let top_padding = content_top_padding_for_scale_factor(1.0, false);
        let cell_height = 18;
        let toolbar = ToolbarState::default();
        let grid_top = top_padding + crate::toolbar::toolbar_height(&toolbar, cell_height);

        assert_eq!(grid_top, PADDING + toolbar.rows() * cell_height);
        assert_eq!(grid_top, 128);
    }

    #[test]
    fn bottom_tooltip_reserves_its_row_and_gap_from_the_grid() {
        let (rows, grid_bottom, tooltip_top, visible) = vertical_geometry(400, 128, 18, 18);
        assert!(visible);
        assert_eq!(tooltip_top, 382);
        assert_eq!(grid_bottom, tooltip_top - TOOLTIP_GRID_GAP);
        assert_eq!(rows, 13);
        assert!(grid_top_and_rows_fit_before(128, rows, 18, grid_bottom));
    }

    #[test]
    fn short_viewport_geometry_saturates_and_hides_overlapping_tooltip() {
        let (rows, grid_bottom, tooltip_top, visible) = vertical_geometry(40, 128, 18, 18);
        assert!(!visible);
        assert_eq!(tooltip_top, 22);
        assert_eq!(grid_bottom, 20);
        assert_eq!(rows, 1);
    }

    fn grid_top_and_rows_fit_before(
        grid_top: usize,
        rows: usize,
        cell_height: usize,
        grid_bottom: usize,
    ) -> bool {
        grid_top.saturating_add(rows.saturating_mul(cell_height)) <= grid_bottom
    }

    #[test]
    fn scroll_margin_is_three_cells() {
        assert_eq!(SCROLL_MARGIN_CELLS, 3);
    }

    #[test]
    fn ten_cell_viewport_uses_the_requested_six_cell_inner_screen() {
        assert_eq!(inner_screen_offsets(10), (3, 8));
        assert_eq!(legal_origin_range(1, 10, 10), (-7, 7));
    }

    #[test]
    fn requested_extreme_viewports_keep_a_real_point_in_the_inner_screen() {
        let content = [
            Coord { line: 1, column: 1 },
            Coord {
                line: 10,
                column: 10,
            },
        ];
        let viewport = (10, 10);

        assert_eq!(viewport_rectangle((-7, -7), viewport), ((-7, -7), (3, 3)));
        assert_eq!(inner_rectangle((-7, -7), viewport), ((-4, -4), (1, 1)));
        assert_eq!(screen_position((-7, -7), content[0]), (8, 8));
        assert!(content_intersects_inner_screen(
            (-7, -7),
            viewport,
            &content
        ));
        assert!(!content_intersects_inner_screen(
            (-8, -8),
            viewport,
            &content
        ));

        assert_eq!(viewport_rectangle((7, 7), viewport), ((7, 7), (17, 17)));
        assert_eq!(inner_rectangle((7, 7), viewport), ((10, 10), (15, 15)));
        assert_eq!(screen_position((7, 7), content[1]), (3, 3));
        assert!(content_intersects_inner_screen((7, 7), viewport, &content));
        assert!(!content_intersects_inner_screen((8, 8), viewport, &content));

        assert_eq!(
            navigation_origin((-7, -7), content[0], viewport, &content),
            Some((-7, -7))
        );
        assert_eq!(
            navigation_origin((7, 7), content[1], viewport, &content),
            Some((7, 7))
        );
    }

    #[test]
    fn constraint_requires_one_point_to_match_both_axes() {
        let content = [
            Coord {
                line: 10,
                column: 1,
            },
            Coord {
                line: 1,
                column: 10,
            },
        ];
        assert!(!content_intersects_inner_screen(
            (-7, -7),
            (10, 10),
            &content
        ));
    }

    #[test]
    fn blank_canvas_is_unbounded_but_content_rejects_blank_escape() {
        let cursor = Coord {
            line: 50,
            column: 50,
        };
        assert_eq!(
            navigation_origin((0, 0), cursor, (10, 10), &[]),
            Some((41, 41))
        );

        let point = [Coord { line: 5, column: 5 }];
        assert_eq!(
            navigation_origin(
                (2, 2),
                Coord {
                    line: 12,
                    column: 12
                },
                (10, 10),
                &point
            ),
            None
        );
    }

    #[test]
    fn far_blank_cursor_is_allowed_while_visible_and_content_stays_in_margin() {
        let point = [Coord { line: 5, column: 5 }];
        assert_eq!(
            navigation_origin(
                (0, 0),
                Coord {
                    line: 1,
                    column: 20
                },
                (24, 24),
                &point,
            ),
            Some((0, 0))
        );
        assert_eq!(
            navigation_origin(
                (0, 0),
                Coord {
                    line: 20,
                    column: 1
                },
                (24, 24),
                &point,
            ),
            Some((0, 0))
        );
    }

    #[test]
    fn prepend_compensation_keeps_existing_cell_at_same_pixel() {
        let cell_size = (8, 16);
        let before = cursor_top_left(
            Coord { line: 2, column: 4 },
            cell_size,
            44,
            ViewportOffset::default(),
        );
        let mut viewport = ViewportOffset::default();
        viewport.compensate_for_prepend(1, 1, cell_size);
        let after = cursor_top_left(Coord { line: 3, column: 5 }, cell_size, 44, viewport);
        assert_eq!(after, before);
    }

    fn cursor_top_left(
        cursor: Coord,
        cell_size: (usize, usize),
        grid_top: usize,
        viewport: ViewportOffset,
    ) -> (i64, i64) {
        (
            PADDING as i64 + cursor.column as i64 * cell_size.0 as i64 + viewport.x,
            grid_top as i64 + cursor.line as i64 * cell_size.1 as i64 + viewport.y,
        )
    }

    fn viewport_rectangle(
        origin: (i64, i64),
        viewport: (usize, usize),
    ) -> ((i64, i64), (i64, i64)) {
        (
            origin,
            (
                origin.0 + i64::try_from(viewport.0).unwrap(),
                origin.1 + i64::try_from(viewport.1).unwrap(),
            ),
        )
    }

    fn inner_rectangle(origin: (i64, i64), viewport: (usize, usize)) -> ((i64, i64), (i64, i64)) {
        let horizontal = inner_screen_offsets(viewport.0);
        let vertical = inner_screen_offsets(viewport.1);
        (
            (origin.0 + horizontal.0, origin.1 + vertical.0),
            (origin.0 + horizontal.1, origin.1 + vertical.1),
        )
    }

    fn screen_position(origin: (i64, i64), coord: Coord) -> (i64, i64) {
        (
            i64::try_from(coord.column).unwrap() - origin.0,
            i64::try_from(coord.line).unwrap() - origin.1,
        )
    }
}
