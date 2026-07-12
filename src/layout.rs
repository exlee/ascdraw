use crate::model::Coord;
use crate::render::CellMetrics;
use crate::toolbar::ToolbarState;

pub const PADDING: usize = 20;
pub const SCROLL_MARGIN_CELLS: i64 = 3;
const TRANSPARENT_MENUBAR_TOP_INSET_PT: f64 = 24.0;

#[derive(Clone, Copy, Debug)]
pub struct LayoutMetrics {
    pub top_padding: usize,
    pub grid_top: usize,
    pub cols: usize,
    pub rows: usize,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentBounds {
    pub left: usize,
    pub right: usize,
    pub top: usize,
    pub bottom: usize,
}

pub fn legal_origin_range(min: usize, max: usize, viewport_cells: usize) -> (i64, i64) {
    let min = i64::try_from(min).unwrap_or(i64::MAX);
    let max = i64::try_from(max).unwrap_or(i64::MAX);
    let viewport_cells = i64::try_from(viewport_cells).unwrap_or(i64::MAX);
    let near_edge = min.saturating_sub(SCROLL_MARGIN_CELLS);
    let far_edge = max.saturating_sub(viewport_cells.saturating_sub(SCROLL_MARGIN_CELLS));
    (near_edge.min(far_edge), near_edge.max(far_edge))
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
    bounds: Option<ContentBounds>,
) -> Option<(i64, i64)> {
    let desired = (
        cursor_origin(current.0, cursor.column, viewport.0),
        cursor_origin(current.1, cursor.line, viewport.1),
    );
    let Some(bounds) = bounds else {
        return Some(desired);
    };
    let horizontal = legal_origin_range(bounds.left, bounds.right, viewport.0);
    let vertical = legal_origin_range(bounds.top, bounds.bottom, viewport.1);
    ((horizontal.0..=horizontal.1).contains(&desired.0)
        && (vertical.0..=vertical.1).contains(&desired.1))
    .then_some(desired)
}

pub fn clamped_navigation_origin(
    current: (i64, i64),
    cursor: Coord,
    viewport: (usize, usize),
    bounds: Option<ContentBounds>,
) -> (i64, i64) {
    let desired = (
        cursor_origin(current.0, cursor.column, viewport.0),
        cursor_origin(current.1, cursor.line, viewport.1),
    );
    let Some(bounds) = bounds else {
        return desired;
    };
    let horizontal = legal_origin_range(bounds.left, bounds.right, viewport.0);
    let vertical = legal_origin_range(bounds.top, bounds.bottom, viewport.1);
    (
        desired.0.clamp(horizontal.0, horizontal.1),
        desired.1.clamp(vertical.0, vertical.1),
    )
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
    let rows = height.saturating_sub(grid_top + PADDING) / metrics.cell_height.max(1);
    LayoutMetrics {
        top_padding,
        grid_top,
        cols,
        rows: rows.max(1),
    }
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
        assert_eq!(grid_top, 164);
    }

    #[test]
    fn scroll_margin_is_three_cells() {
        assert_eq!(SCROLL_MARGIN_CELLS, 3);
    }

    #[test]
    fn point_at_five_in_ten_cells_has_exact_requested_origin_range() {
        assert_eq!(legal_origin_range(5, 5, 10), (-2, 2));
        let bounds = ContentBounds {
            left: 5,
            right: 5,
            top: 5,
            bottom: 5,
        };
        for origin in -2..=2 {
            assert_eq!(
                navigation_origin(
                    (origin, origin),
                    Coord { line: 5, column: 5 },
                    (10, 10),
                    Some(bounds)
                ),
                Some((origin, origin))
            );
        }
    }

    #[test]
    fn multi_cell_bounds_allow_panning_between_both_margin_anchors() {
        assert_eq!(legal_origin_range(4, 17, 10), (1, 10));
        assert_eq!(legal_origin_range(8, 9, 10), (2, 5));
    }

    #[test]
    fn blank_canvas_is_unbounded_but_content_rejects_blank_escape() {
        let cursor = Coord {
            line: 50,
            column: 50,
        };
        assert_eq!(
            navigation_origin((0, 0), cursor, (10, 10), None),
            Some((41, 41))
        );

        let point = ContentBounds {
            left: 5,
            right: 5,
            top: 5,
            bottom: 5,
        };
        assert_eq!(
            navigation_origin(
                (2, 2),
                Coord {
                    line: 12,
                    column: 12
                },
                (10, 10),
                Some(point)
            ),
            None
        );
    }

    #[test]
    fn far_blank_cursor_is_allowed_while_visible_and_content_stays_in_margin() {
        let point = ContentBounds {
            left: 5,
            right: 5,
            top: 5,
            bottom: 5,
        };
        assert_eq!(
            navigation_origin(
                (0, 0),
                Coord {
                    line: 1,
                    column: 20
                },
                (24, 24),
                Some(point),
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
                Some(point),
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
}
