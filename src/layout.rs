use crate::model::Coord;
use crate::render::CellMetrics;

pub const PADDING: usize = 20;
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
    transparent_menubar: bool,
    scale_factor: f64,
) -> LayoutMetrics {
    let top_padding = content_top_padding(scale_factor, transparent_menubar);
    let grid_top = top_padding + crate::toolbar::toolbar_height(toolbar_cell_height);
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
        let grid_top = top_padding + crate::toolbar::toolbar_height(cell_height);

        assert_eq!(
            grid_top,
            PADDING + crate::toolbar::TOOLBAR_ROWS * cell_height
        );
        assert_eq!(grid_top, 182);
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
