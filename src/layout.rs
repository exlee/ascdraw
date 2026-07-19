use serde::{Deserialize, Serialize};

use crate::model::Coord;
use crate::render::CellMetrics;
use crate::toolbar::ToolbarState;

pub const PADDING: usize = 20;
pub const SCROLL_MARGIN_CELLS: i64 = 3;
pub const TOOLTIP_GRID_GAP: usize = PADDING;
pub const TOOLTIP_BOTTOM_PAD: usize = 15;
pub const MINIMAP_COLUMNS: usize = 20;
const MINIMAP_ROWS: usize = 7;
const TRANSPARENT_MENUBAR_TOP_INSET_PT: f64 = 24.0;

#[derive(Clone, Copy, Debug)]
pub struct LayoutMetrics {
    pub top_padding: f32,
    pub grid_top: f32,
    pub cols: usize,
    pub rows: usize,
    pub grid_bottom: f32,
    pub tooltip_top: f32,
    pub tooltip_visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl ScreenRect {
    pub fn width(self) -> f32 {
        (self.right - self.left).max(0.0)
    }

    pub fn height(self) -> f32 {
        (self.bottom - self.top).max(0.0)
    }

    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.left as f64
            && x < self.right as f64
            && y >= self.top as f64
            && y < self.bottom as f64
    }
}

pub fn minimap_rect(
    viewport_width: usize,
    grid_top: f32,
    toolbar_cell_size: (f32, f32),
) -> ScreenRect {
    let cell_width = toolbar_cell_size.0.max(1.0);
    let cell_height = toolbar_cell_size.1.max(1.0);
    let toolbar_columns =
        ((viewport_width.saturating_sub(PADDING * 2) as f32) / cell_width) as usize;
    let width_in_cells = minimap_width_in_cells(toolbar_columns);
    let right_column = toolbar_columns.saturating_sub(2);
    let left_column = right_column.saturating_sub(width_in_cells.saturating_sub(1));
    let right = PADDING as f32 + right_column.saturating_add(1) as f32 * cell_width;
    ScreenRect {
        left: if width_in_cells == 0 {
            right
        } else {
            PADDING as f32 + left_column as f32 * cell_width
        },
        top: (grid_top - cell_height).max(0.0),
        right,
        bottom: (grid_top - cell_height).max(0.0) + MINIMAP_ROWS as f32 * cell_height,
    }
}

pub fn minimap_width_in_cells(toolbar_columns: usize) -> usize {
    let available = MINIMAP_COLUMNS.min(toolbar_columns.saturating_sub(1));
    available.saturating_sub(1 - available % 2)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleCanvasCells {
    pub origin: (i64, i64),
    pub columns: usize,
    pub rows: usize,
}

impl VisibleCanvasCells {
    pub fn from_layout(
        layout: LayoutMetrics,
        viewport: ViewportOffset,
        cell_size: (f32, f32),
    ) -> Self {
        let (left, columns) = visible_axis(viewport.x, cell_size.0, layout.cols);
        let (top, rows) = visible_axis(viewport.y, cell_size.1, layout.rows);
        Self {
            origin: (left, top),
            columns,
            rows,
        }
    }
}

fn visible_axis(offset: i64, cell_size: f32, full_cells: usize) -> (i64, usize) {
    let position = -(offset as f64) / cell_size.max(1.0) as f64;
    let origin = position.floor() as i64;
    let has_partial_cell = position.fract().abs() > f64::EPSILON;
    (
        origin,
        full_cells.saturating_add(usize::from(has_partial_cell)),
    )
}

/// Renderer-pixel translation applied to the canvas.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct ViewportOffset {
    pub x: i64,
    pub y: i64,
}

impl ViewportOffset {
    /// Keeps the canvas-to-screen transform stable when only the top edge of
    /// the grid moves (for example, because the toolbar changes height).
    pub fn reanchor_grid_top(&mut self, old_grid_top: f32, new_grid_top: f32) {
        self.y = self
            .y
            .saturating_add((old_grid_top - new_grid_top).round() as i64);
    }

    pub fn compensate_for_prepend(&mut self, columns: usize, lines: usize, cell_size: (f32, f32)) {
        self.x = self.x.saturating_sub(cell_shift(columns, cell_size.0));
        self.y = self.y.saturating_sub(cell_shift(lines, cell_size.1));
    }

    pub fn origin(self, cell_size: (f32, f32)) -> (i64, i64) {
        (
            (-(self.x as f64) / cell_size.0.max(1.0) as f64).floor() as i64,
            (-(self.y as f64) / cell_size.1.max(1.0) as f64).floor() as i64,
        )
    }

    pub fn set_origin(&mut self, origin: (i64, i64), cell_size: (f32, f32)) {
        self.x = (-(origin.0 as f64) * cell_size.0.max(1.0) as f64).round() as i64;
        self.y = (-(origin.1 as f64) * cell_size.1.max(1.0) as f64).round() as i64;
    }

    pub fn reanchor_cursor(
        &mut self,
        cursor: Coord,
        old_cell_size: (f32, f32),
        new_cell_size: (f32, f32),
        old_grid_top: f32,
        new_grid_top: f32,
    ) {
        self.x = self
            .x
            .saturating_add(cell_delta(cursor.column, old_cell_size.0, new_cell_size.0));
        self.reanchor_grid_top(old_grid_top, new_grid_top);
        self.y = self
            .y
            .saturating_add(cell_delta(cursor.line, old_cell_size.1, new_cell_size.1));
    }
}

fn cell_shift(count: usize, size: f32) -> i64 {
    (count as f64 * size as f64).round() as i64
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
    let end = viewport_cells
        .saturating_sub(outer_margin)
        .min(viewport_cells.saturating_sub(1));
    (outer_margin.saturating_add(1).min(end), end)
}

pub fn content_intersects_inner_screen(
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

pub fn cursor_is_visible(origin: (i64, i64), cursor: Coord, viewport: (usize, usize)) -> bool {
    let x = i64::try_from(cursor.column)
        .unwrap_or(i64::MAX)
        .saturating_sub(origin.0);
    let y = i64::try_from(cursor.line)
        .unwrap_or(i64::MAX)
        .saturating_sub(origin.1);
    (0..i64::try_from(viewport.0.max(1)).unwrap_or(i64::MAX)).contains(&x)
        && (0..i64::try_from(viewport.1.max(1)).unwrap_or(i64::MAX)).contains(&y)
}

fn clamp_to_range(value: i64, range: (i64, i64)) -> Option<i64> {
    (range.0 <= range.1).then(|| value.clamp(range.0, range.1))
}

/// Finds the nearest origin which keeps the cursor visible and one actual
/// content cell inside the inner screen. Both axes must be satisfied by the
/// same content cell.
pub fn constrained_origin(
    desired: (i64, i64),
    cursor: Coord,
    viewport: (usize, usize),
    content: &[Coord],
) -> Option<(i64, i64)> {
    let cursor_x = i64::try_from(cursor.column).unwrap_or(i64::MAX);
    let cursor_y = i64::try_from(cursor.line).unwrap_or(i64::MAX);
    let cursor_ranges = (
        (
            cursor_x
                .saturating_sub(i64::try_from(viewport.0.saturating_sub(1)).unwrap_or(i64::MAX)),
            cursor_x,
        ),
        (
            cursor_y
                .saturating_sub(i64::try_from(viewport.1.saturating_sub(1)).unwrap_or(i64::MAX)),
            cursor_y,
        ),
    );
    if content.is_empty() {
        return Some((
            clamp_to_range(desired.0, cursor_ranges.0)?,
            clamp_to_range(desired.1, cursor_ranges.1)?,
        ));
    }

    let horizontal = inner_screen_offsets(viewport.0);
    let vertical = inner_screen_offsets(viewport.1);
    content
        .iter()
        .filter_map(|coord| {
            let x = i64::try_from(coord.column).unwrap_or(i64::MAX);
            let y = i64::try_from(coord.line).unwrap_or(i64::MAX);
            let x_range = (
                cursor_ranges.0.0.max(x.saturating_sub(horizontal.1)),
                cursor_ranges.0.1.min(x.saturating_sub(horizontal.0)),
            );
            let y_range = (
                cursor_ranges.1.0.max(y.saturating_sub(vertical.1)),
                cursor_ranges.1.1.min(y.saturating_sub(vertical.0)),
            );
            let origin = (
                clamp_to_range(desired.0, x_range)?,
                clamp_to_range(desired.1, y_range)?,
            );
            let distance = desired
                .0
                .abs_diff(origin.0)
                .saturating_add(desired.1.abs_diff(origin.1));
            Some((distance, origin))
        })
        .min_by_key(|candidate| *candidate)
        .map(|(_, origin)| origin)
        .filter(|origin| {
            cursor_is_visible(*origin, cursor, viewport)
                && content_intersects_inner_screen(*origin, viewport, content)
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
    constrained_origin(desired, cursor, viewport, content)
}

/// Resize/layout fallback. If the old cursor cannot coexist with content in
/// the reduced viewport, move it to the nearest actual content cell.
pub fn normalized_cursor_and_origin(
    desired: (i64, i64),
    cursor: Coord,
    viewport: (usize, usize),
    content: &[Coord],
) -> (Coord, (i64, i64)) {
    if let Some(origin) = constrained_origin(desired, cursor, viewport, content) {
        return (cursor, origin);
    }
    content
        .iter()
        .filter_map(|candidate| {
            let origin = constrained_origin(desired, *candidate, viewport, content)?;
            let cursor_distance = cursor
                .column
                .abs_diff(candidate.column)
                .saturating_add(cursor.line.abs_diff(candidate.line));
            let origin_distance = desired
                .0
                .abs_diff(origin.0)
                .saturating_add(desired.1.abs_diff(origin.1));
            Some((cursor_distance, origin_distance, *candidate, origin))
        })
        .min_by_key(|candidate| {
            (
                candidate.0,
                candidate.1,
                candidate.2.line,
                candidate.2.column,
                candidate.3,
            )
        })
        .map_or((cursor, desired), |(_, _, cursor, origin)| (cursor, origin))
}

fn cell_delta(index: usize, old_size: f32, new_size: f32) -> i64 {
    (index as f64 * (old_size - new_size) as f64).round() as i64
}

pub fn content_top_padding(scale_factor: f64, transparent_menubar: bool) -> f32 {
    content_top_padding_for_scale_factor(scale_factor, transparent_menubar)
}

pub fn content_top_padding_for_scale_factor(scale_factor: f64, transparent_menubar: bool) -> f32 {
    if transparent_menubar {
        PADDING as f32 + (TRANSPARENT_MENUBAR_TOP_INSET_PT * scale_factor) as f32
    } else {
        PADDING as f32
    }
}

pub fn layout_metrics(
    width: usize,
    height: usize,
    metrics: &CellMetrics,
    toolbar_cell_size: (f32, f32),
    toolbar: &ToolbarState,
    transparent_menubar: bool,
    scale_factor: f64,
) -> LayoutMetrics {
    let top_padding = content_top_padding(scale_factor, transparent_menubar);
    let toolbar_box_width =
        (width.saturating_sub(PADDING * 2) as f32 / toolbar_cell_size.0.max(1.0)) as usize;
    let grid_top = (top_padding
        + crate::toolbar::toolbar_height_for_width(
            toolbar,
            toolbar_box_width,
            toolbar_cell_size.1,
        ))
    .round();
    let cols = (width.saturating_sub(PADDING * 2) as f32 / metrics.cell_width.max(1.0)) as usize;
    let (rows, grid_bottom, tooltip_top, tooltip_visible) =
        vertical_geometry(height, grid_top, metrics.cell_height, toolbar_cell_size.1);
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
    grid_top: f32,
    grid_cell_height: f32,
    tooltip_cell_height: f32,
) -> (usize, f32, f32, bool) {
    let tooltip_top = (height as f32 - tooltip_cell_height - TOOLTIP_BOTTOM_PAD as f32).max(0.0);
    let tooltip_visible = tooltip_cell_height > 0.0
        && height as f32 >= tooltip_cell_height
        && tooltip_top >= grid_top + TOOLTIP_GRID_GAP as f32;
    let grid_bottom = if tooltip_visible {
        (tooltip_top - TOOLTIP_GRID_GAP as f32).max(0.0)
    } else {
        height.saturating_sub(PADDING) as f32
    };
    let rows = ((grid_bottom - grid_top).max(0.0) / grid_cell_height.max(1.0)) as usize;
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
    ((height as f32 - top_padding - PADDING as f32).max(0.0) / cell_height.max(1) as f32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::{MainMode, ToolbarAction};

    #[test]
    fn visible_canvas_cells_include_residual_pixels_and_signed_origins() {
        let layout = LayoutMetrics {
            top_padding: 20.0,
            grid_top: 100.0,
            cols: 12,
            rows: 7,
            grid_bottom: 212.0,
            tooltip_top: 240.0,
            tooltip_visible: true,
        };

        assert_eq!(
            VisibleCanvasCells::from_layout(layout, ViewportOffset { x: -16, y: 32 }, (8.0, 16.0),),
            VisibleCanvasCells {
                origin: (2, -2),
                columns: 12,
                rows: 7,
            }
        );
        assert_eq!(
            VisibleCanvasCells::from_layout(layout, ViewportOffset { x: 3, y: -17 }, (8.0, 16.0),),
            VisibleCanvasCells {
                origin: (-1, 1),
                columns: 13,
                rows: 8,
            }
        );
    }

    #[test]
    fn visible_export_rows_follow_dynamic_toolbar_height() {
        let menu_heights = [
            MainMode::Stamp,
            MainMode::Line,
            MainMode::Shapes,
            MainMode::Utilities,
        ]
        .map(|mode| {
            let mut toolbar = ToolbarState::default();
            toolbar.apply_action(ToolbarAction::SelectMain(mode));
            toolbar.rows()
        });
        let compact_rows = *menu_heights.iter().min().unwrap();
        let expanded_rows = *menu_heights.iter().max().unwrap();
        assert!(expanded_rows > compact_rows);

        let available_rows = 20usize;
        let compact_layout = LayoutMetrics {
            top_padding: 0.0,
            grid_top: compact_rows as f32,
            cols: 10,
            rows: available_rows.saturating_sub(compact_rows),
            grid_bottom: available_rows as f32,
            tooltip_top: available_rows as f32,
            tooltip_visible: false,
        };
        let expanded_layout = LayoutMetrics {
            top_padding: 0.0,
            grid_top: expanded_rows as f32,
            cols: 10,
            rows: available_rows.saturating_sub(expanded_rows),
            grid_bottom: available_rows as f32,
            tooltip_top: available_rows as f32,
            tooltip_visible: false,
        };

        assert!(
            VisibleCanvasCells::from_layout(compact_layout, ViewportOffset::default(), (1.0, 1.0),)
                .rows
                > VisibleCanvasCells::from_layout(
                    expanded_layout,
                    ViewportOffset::default(),
                    (1.0, 1.0),
                )
                .rows
        );
    }

    #[test]
    fn transparent_menubar_uses_fixed_point_top_inset() {
        assert_eq!(
            content_top_padding_for_scale_factor(1.0, false),
            PADDING as f32
        );
        assert_eq!(
            content_top_padding_for_scale_factor(1.0, true),
            (PADDING + 24) as f32
        );
        assert_eq!(
            content_top_padding_for_scale_factor(2.0, true),
            (PADDING + 48) as f32
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
        let before = cursor_top_left(cursor, (8.0, 16.0), 44.0, viewport);

        viewport.reanchor_cursor(cursor, (8.0, 16.0), (11.0, 20.0), 44.0, 44.0);

        assert_eq!(
            cursor_top_left(cursor, (11.0, 20.0), 44.0, viewport),
            before
        );
    }

    #[test]
    fn reanchoring_includes_changes_to_the_fixed_toolbar_height() {
        let cursor = Coord { line: 2, column: 0 };
        let mut viewport = ViewportOffset::default();
        let before = cursor_top_left(cursor, (8.0, 16.0), 44.0, viewport);

        viewport.reanchor_cursor(cursor, (8.0, 16.0), (8.0, 16.0), 44.0, 48.0);

        assert_eq!(cursor_top_left(cursor, (8.0, 16.0), 48.0, viewport), before);
    }

    #[test]
    fn boxed_toolbar_height_anchors_grid_below_both_borders() {
        let top_padding = content_top_padding_for_scale_factor(1.0, false);
        let cell_height = 18.0;
        let toolbar = ToolbarState::default();
        let grid_top = top_padding + crate::toolbar::toolbar_height(&toolbar, cell_height);

        assert_eq!(
            grid_top,
            PADDING as f32
                + toolbar.rows() as f32 * cell_height
                + toolbar.rows().saturating_sub(1) as f32 * crate::toolbar::TOOLBAR_ROW_GAP as f32
        );
        assert_eq!(grid_top, 218.0);
    }

    #[test]
    fn minimap_attaches_below_the_toolbar_and_has_an_inert_screen_region() {
        let rect = minimap_rect(1000, 200.0, (8.0, 16.0));

        assert_eq!(
            rect,
            ScreenRect {
                left: 820.0,
                top: 184.0,
                right: 972.0,
                bottom: 296.0,
            }
        );
        assert!(rect.contains(900.0, 250.0));
        assert!(!rect.contains(811.0, 250.0));
        assert!(!rect.contains(900.0, 296.0));

        assert_eq!(minimap_width_in_cells(120), 19);
        assert_eq!(minimap_width_in_cells(18), 17);
        assert_eq!(minimap_width_in_cells(4), 3);
        for columns in 0..40 {
            let width = minimap_width_in_cells(columns);
            assert!(width == 0 || width % 2 == 1);
        }
    }

    #[test]
    fn bottom_tooltip_reserves_its_row_and_gap_from_the_grid() {
        let (rows, grid_bottom, tooltip_top, visible) = vertical_geometry(400, 128.0, 18.0, 18.0);
        assert!(visible);
        assert_eq!(tooltip_top, (382 - TOOLTIP_BOTTOM_PAD) as f32);
        assert_eq!(grid_bottom, tooltip_top - TOOLTIP_GRID_GAP as f32);
        assert_eq!(rows, 12);
        assert!(grid_top_and_rows_fit_before(128.0, rows, 18.0, grid_bottom));
    }

    #[test]
    fn short_viewport_geometry_saturates_and_hides_overlapping_tooltip() {
        let (rows, grid_bottom, tooltip_top, visible) = vertical_geometry(40, 128.0, 18.0, 18.0);
        assert!(!visible);
        assert_eq!(tooltip_top, 7.0);
        assert_eq!(grid_bottom, 20.0);
        assert_eq!(rows, 1);
    }

    fn grid_top_and_rows_fit_before(
        grid_top: f32,
        rows: usize,
        cell_height: f32,
        grid_bottom: f32,
    ) -> bool {
        grid_top + rows as f32 * cell_height <= grid_bottom
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
    fn cursor_visibility_uses_all_four_inclusive_viewport_edges() {
        let viewport = (10, 10);
        for cursor in [
            Coord { line: 0, column: 0 },
            Coord { line: 0, column: 9 },
            Coord { line: 9, column: 0 },
            Coord { line: 9, column: 9 },
        ] {
            assert!(cursor_is_visible((0, 0), cursor, viewport));
        }
        assert!(!cursor_is_visible(
            (0, 0),
            Coord {
                line: 5,
                column: 10
            },
            viewport
        ));
        assert!(!cursor_is_visible(
            (0, 0),
            Coord {
                line: 10,
                column: 5
            },
            viewport
        ));
    }

    #[test]
    fn next_horizontal_and_vertical_blank_escape_steps_are_rejected() {
        let content = [Coord { line: 5, column: 5 }];
        let viewport = (10, 10);
        let horizontal_edge = Coord {
            line: 5,
            column: 11,
        };
        let vertical_edge = Coord {
            line: 11,
            column: 5,
        };
        let horizontal_origin = navigation_origin((0, 0), horizontal_edge, viewport, &content)
            .expect("last horizontal position remains legal");
        let vertical_origin = navigation_origin((0, 0), vertical_edge, viewport, &content)
            .expect("last vertical position remains legal");
        assert!(cursor_is_visible(
            horizontal_origin,
            horizontal_edge,
            viewport
        ));
        assert!(cursor_is_visible(vertical_origin, vertical_edge, viewport));
        assert_eq!(
            navigation_origin(
                horizontal_origin,
                Coord {
                    line: 5,
                    column: 12
                },
                viewport,
                &content
            ),
            None
        );
        assert_eq!(
            navigation_origin(
                vertical_origin,
                Coord {
                    line: 12,
                    column: 5
                },
                viewport,
                &content
            ),
            None
        );
    }

    #[test]
    fn blank_canvas_keeps_even_a_far_cursor_visible() {
        let cursor = Coord {
            line: 80,
            column: 90,
        };
        let origin = navigation_origin((0, 0), cursor, (10, 10), &[]).unwrap();
        assert!(cursor_is_visible(origin, cursor, (10, 10)));
    }

    #[test]
    fn prepend_compensation_remains_legal_and_visible() {
        let content = [Coord { line: 6, column: 6 }];
        let cursor = Coord { line: 0, column: 0 };
        let origin = navigation_origin((-1, -1), cursor, (10, 10), &content).unwrap();
        assert_eq!(origin, (-1, -1));
        assert!(cursor_is_visible(origin, cursor, (10, 10)));
        assert!(content_intersects_inner_screen(origin, (10, 10), &content));
    }

    #[test]
    fn smaller_layout_clamps_an_impossible_cross_axis_cursor_to_content() {
        let content = [
            Coord { line: 1, column: 1 },
            Coord {
                line: 10,
                column: 10,
            },
        ];
        let old_cursor = Coord {
            line: 1,
            column: 10,
        };
        assert_eq!(
            constrained_origin((0, 0), old_cursor, (10, 10), &content),
            None
        );
        let (cursor, origin) = normalized_cursor_and_origin((0, 0), old_cursor, (10, 10), &content);
        assert_eq!(cursor, content[0]);
        assert!(cursor_is_visible(origin, cursor, (10, 10)));
        assert!(content_intersects_inner_screen(origin, (10, 10), &content));
    }

    #[test]
    fn larger_layout_preserves_a_cursor_that_a_smaller_layout_must_clamp() {
        let content = [Coord { line: 5, column: 5 }];
        let cursor = Coord {
            line: 20,
            column: 20,
        };
        let (small_cursor, _) = normalized_cursor_and_origin((0, 0), cursor, (10, 10), &content);
        let (large_cursor, large_origin) =
            normalized_cursor_and_origin((0, 0), cursor, (24, 24), &content);
        assert_eq!(small_cursor, content[0]);
        assert_eq!(large_cursor, cursor);
        assert!(cursor_is_visible(large_origin, large_cursor, (24, 24)));
    }

    #[test]
    fn one_cell_layout_still_has_a_valid_inner_screen() {
        let point = Coord { line: 7, column: 9 };
        let origin = constrained_origin((0, 0), point, (1, 1), &[point]).unwrap();
        assert_eq!(origin, (9, 7));
        assert!(cursor_is_visible(origin, point, (1, 1)));
        assert!(content_intersects_inner_screen(origin, (1, 1), &[point]));
    }

    #[test]
    fn prepend_compensation_keeps_existing_cell_at_same_pixel() {
        let cell_size = (8.0, 16.0);
        let before = cursor_top_left(
            Coord { line: 2, column: 4 },
            cell_size,
            44.0,
            ViewportOffset::default(),
        );
        let mut viewport = ViewportOffset::default();
        viewport.compensate_for_prepend(1, 1, cell_size);
        let after = cursor_top_left(Coord { line: 3, column: 5 }, cell_size, 44.0, viewport);
        assert_eq!(after, before);
    }

    fn cursor_top_left(
        cursor: Coord,
        cell_size: (f32, f32),
        grid_top: f32,
        viewport: ViewportOffset,
    ) -> (f32, f32) {
        (
            PADDING as f32 + cursor.column as f32 * cell_size.0 + viewport.x as f32,
            grid_top + cursor.line as f32 * cell_size.1 + viewport.y as f32,
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
