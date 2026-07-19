use skia_safe::{Canvas, Paint, Rect};

use crate::editor::Editor;
use crate::face_resolution::{ResolvedFace, resolve_derived_face};
use crate::layout::{ScreenRect, VisibleCanvasCells};
use crate::model::Coord;

use super::{CellMetrics, FALLBACK_BG, FALLBACK_FG, draw_text_cluster, is_drawing_mode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MinimapBounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl MinimapBounds {
    fn canvas(content: &[Coord], viewport: VisibleCanvasCells) -> Self {
        let mut bounds = Self {
            left: 0,
            top: 0,
            right: i64::try_from(viewport.columns.max(1)).unwrap_or(i64::MAX),
            bottom: i64::try_from(viewport.rows.max(1)).unwrap_or(i64::MAX),
        };
        for coord in content {
            let column = i64::try_from(coord.column).unwrap_or(i64::MAX);
            let line = i64::try_from(coord.line).unwrap_or(i64::MAX);
            bounds.right = bounds.right.max(column.saturating_add(1));
            bounds.bottom = bounds.bottom.max(line.saturating_add(1));
        }
        bounds
    }

    fn viewport(viewport: VisibleCanvasCells) -> Self {
        Self {
            left: viewport.origin.0,
            top: viewport.origin.1,
            right: viewport
                .origin
                .0
                .saturating_add(i64::try_from(viewport.columns.max(1)).unwrap_or(i64::MAX)),
            bottom: viewport
                .origin
                .1
                .saturating_add(i64::try_from(viewport.rows.max(1)).unwrap_or(i64::MAX)),
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    fn width(self) -> i64 {
        self.right.saturating_sub(self.left).max(1)
    }

    fn height(self) -> i64 {
        self.bottom.saturating_sub(self.top).max(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MinimapGeometry {
    world: MinimapBounds,
    columns: i64,
    rows: i64,
    canvas_cells_per_cell: i64,
    cell_width: f32,
    cell_height: f32,
    left: f32,
    top: f32,
}

impl MinimapGeometry {
    fn new(
        panel: ScreenRect,
        canvas_bounds: MinimapBounds,
        required_bounds: MinimapBounds,
        cell_width: f32,
        cell_height: f32,
    ) -> Option<Self> {
        const INSET: f32 = 4.0;
        let inner_width = panel.width() - INSET * 2.0;
        let inner_height = panel.height() - INSET * 2.0;
        if inner_width <= 0.0 || inner_height <= 0.0 || cell_width <= 0.0 || cell_height <= 0.0 {
            return None;
        }

        let columns = (inner_width / cell_width).floor().max(1.0) as i64;
        let rows = (inner_height / cell_height).floor().max(1.0) as i64;
        let mut canvas_cells_per_cell = 1_i64;
        while required_bounds.width() > columns.saturating_mul(canvas_cells_per_cell)
            || required_bounds.height() > rows.saturating_mul(canvas_cells_per_cell)
        {
            let doubled = canvas_cells_per_cell.saturating_mul(2);
            if doubled == canvas_cells_per_cell {
                break;
            }
            canvas_cells_per_cell = doubled;
        }

        let world_width = columns.saturating_mul(canvas_cells_per_cell);
        let world_height = rows.saturating_mul(canvas_cells_per_cell);
        let world = MinimapBounds {
            left: centered_origin(
                canvas_bounds.left,
                canvas_bounds.right,
                required_bounds.left,
                required_bounds.right,
                world_width,
            ),
            top: centered_origin(
                canvas_bounds.top,
                canvas_bounds.bottom,
                required_bounds.top,
                required_bounds.bottom,
                world_height,
            ),
            right: 0,
            bottom: 0,
        };
        let world = MinimapBounds {
            right: world.left.saturating_add(world_width),
            bottom: world.top.saturating_add(world_height),
            ..world
        };
        let map_width = columns as f32 * cell_width;
        let map_height = rows as f32 * cell_height;
        Some(Self {
            world,
            columns,
            rows,
            canvas_cells_per_cell,
            cell_width,
            cell_height,
            left: panel.left + INSET + (inner_width - map_width) / 2.0,
            top: panel.top + INSET + (inner_height - map_height) / 2.0,
        })
    }

    fn point(self, column: i64, line: i64) -> (f32, f32) {
        (
            self.left
                + column.saturating_sub(self.world.left) as f32 / self.canvas_cells_per_cell as f32
                    * self.cell_width,
            self.top
                + line.saturating_sub(self.world.top) as f32 / self.canvas_cells_per_cell as f32
                    * self.cell_height,
        )
    }

    fn rect(self, left: i64, top: i64, right: i64, bottom: i64) -> Rect {
        let (x, y) = self.point(left, top);
        let (right, bottom) = self.point(right, bottom);
        Rect::from_xywh(x, y, (right - x).max(1.0), (bottom - y).max(1.0))
    }

    fn content_cell(self, column: i64, line: i64) -> Option<(i64, i64)> {
        let column = column
            .saturating_sub(self.world.left)
            .div_euclid(self.canvas_cells_per_cell);
        let row = line
            .saturating_sub(self.world.top)
            .div_euclid(self.canvas_cells_per_cell);
        (column >= 0 && column < self.columns && row >= 0 && row < self.rows)
            .then_some((column, row))
    }

    fn cell_rect(self, column: i64, row: i64) -> Rect {
        Rect::from_xywh(
            self.left + column as f32 * self.cell_width,
            self.top + row as f32 * self.cell_height,
            self.cell_width,
            self.cell_height,
        )
    }
}

fn centered_origin(
    canvas_start: i64,
    canvas_end: i64,
    required_start: i64,
    required_end: i64,
    capacity: i64,
) -> i64 {
    let centered = canvas_start
        .saturating_add(canvas_end)
        .saturating_sub(capacity)
        .div_euclid(2);
    centered.clamp(required_end.saturating_sub(capacity), required_start)
}

fn density_level(occupied: usize, canvas_cells_per_cell: i64) -> u8 {
    if occupied == 0 {
        return 0;
    }
    let side = canvas_cells_per_cell as u128;
    let capacity = side.saturating_mul(side).max(1);
    let occupied = occupied as u128;
    occupied
        .saturating_mul(4)
        .saturating_add(capacity - 1)
        .checked_div(capacity)
        .unwrap_or(4)
        .clamp(1, 4) as u8
}

fn occupancy(content: &[Coord], geometry: MinimapGeometry) -> Option<Vec<usize>> {
    let len = usize::try_from(geometry.columns).ok().and_then(|columns| {
        usize::try_from(geometry.rows)
            .ok()
            .and_then(|rows| columns.checked_mul(rows))
    })?;
    let mut occupancy = vec![0usize; len];
    for coord in content {
        let column = i64::try_from(coord.column).unwrap_or(i64::MAX);
        let line = i64::try_from(coord.line).unwrap_or(i64::MAX);
        if let Some((column, row)) = geometry.content_cell(column, line) {
            let index = row.saturating_mul(geometry.columns).saturating_add(column);
            if let Ok(index) = usize::try_from(index) {
                occupancy[index] = occupancy[index].saturating_add(1);
            }
        }
    }
    Some(occupancy)
}

pub(super) fn render(
    canvas: &Canvas,
    state: &Editor,
    content: &[Coord],
    viewport: VisibleCanvasCells,
    panel: ScreenRect,
    border_metrics: &CellMetrics,
    default_face: &ResolvedFace,
) {
    let drawing_cursor = is_drawing_mode(state.cursor_mode);
    let cursor_face = resolve_derived_face(
        &state.grid.default_face,
        if drawing_cursor {
            &state.theme.cursor_drawing
        } else {
            &state.grid.cursor_face
        },
        FALLBACK_FG,
        FALLBACK_BG,
    );
    let cursor_color = if drawing_cursor {
        cursor_face.fg
    } else {
        cursor_face.bg
    };
    let canvas_bounds = MinimapBounds::canvas(content, viewport);
    let required_bounds = canvas_bounds.union(MinimapBounds::viewport(viewport));
    let content_panel = ScreenRect {
        left: panel.left + border_metrics.cell_width,
        top: panel.top + border_metrics.cell_height,
        right: panel.right - border_metrics.cell_width,
        bottom: panel.bottom - border_metrics.cell_height,
    };
    let Some(geometry) = MinimapGeometry::new(
        content_panel,
        canvas_bounds,
        required_bounds,
        border_metrics.cell_width / 4.0,
        border_metrics.cell_height / 4.0,
    ) else {
        return;
    };
    let body_rect = Rect::from_xywh(
        panel.left,
        panel.top + border_metrics.cell_height,
        panel.width(),
        (panel.height() - border_metrics.cell_height).max(0.0),
    );
    let mut background = Paint::default();
    background
        .set_anti_alias(false)
        .set_color(default_face.bg.to_color());
    canvas.draw_rect(body_rect, &background);

    canvas.save();
    canvas.clip_rect(body_rect, None, false);
    let mut foreground = Paint::default();
    foreground
        .set_anti_alias(false)
        .set_color(default_face.fg.to_color());
    let Some(occupancy) = occupancy(content, geometry) else {
        canvas.restore();
        return;
    };
    for (index, occupied) in occupancy
        .into_iter()
        .enumerate()
        .filter(|(_, count)| *count > 0)
    {
        let index = i64::try_from(index).unwrap_or(i64::MAX);
        let cell = (
            index.rem_euclid(geometry.columns),
            index.div_euclid(geometry.columns),
        );
        let mut density = foreground.clone();
        density
            .set_alpha_f(f32::from(density_level(occupied, geometry.canvas_cells_per_cell)) / 4.0);
        canvas.draw_rect(geometry.cell_rect(cell.0, cell.1), &density);
    }

    let viewport_right = viewport
        .origin
        .0
        .saturating_add(i64::try_from(viewport.columns.max(1)).unwrap_or(i64::MAX));
    let viewport_bottom = viewport
        .origin
        .1
        .saturating_add(i64::try_from(viewport.rows.max(1)).unwrap_or(i64::MAX));
    let viewport_rect = geometry.rect(
        viewport.origin.0,
        viewport.origin.1,
        viewport_right,
        viewport_bottom,
    );
    let mut viewport_foreground = Paint::default();
    viewport_foreground
        .set_anti_alias(false)
        .set_style(skia_safe::paint::Style::Stroke)
        .set_stroke_join(skia_safe::paint::Join::Miter)
        .set_color(cursor_color.to_color())
        .set_stroke_width(1.0);
    canvas.draw_rect(viewport_rect, &viewport_foreground);
    canvas.restore();

    let left_column = ((panel.left - crate::layout::PADDING as f32).max(0.0)
        / border_metrics.cell_width.max(1.0)) as usize;
    let right_column = (((panel.right - crate::layout::PADDING as f32).max(0.0)
        / border_metrics.cell_width.max(1.0)) as usize)
        .saturating_sub(1);
    let rows = (panel.height() / border_metrics.cell_height.max(1.0)) as usize;
    let font = border_metrics.font.clone();
    foreground.set_anti_alias(true);
    for row in 1..rows.saturating_sub(1) {
        let top = panel.top + row as f32 * border_metrics.cell_height;
        draw_text_cluster(
            canvas,
            left_column,
            top,
            "│",
            &font,
            border_metrics,
            &foreground,
        );
        draw_text_cluster(
            canvas,
            right_column,
            top,
            "│",
            &font,
            border_metrics,
            &foreground,
        );
    }
    if rows >= 2 {
        let top = panel.top + rows.saturating_sub(1) as f32 * border_metrics.cell_height;
        for column in left_column..=right_column {
            let glyph = if column == left_column {
                "└"
            } else if column == right_column {
                "┘"
            } else {
                "─"
            };
            draw_text_cluster(
                canvas,
                column,
                top,
                glyph,
                &font,
                border_metrics,
                &foreground,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_micro_cells_fixed_and_doubles_their_canvas_coverage() {
        let viewport = VisibleCanvasCells {
            origin: (0, 0),
            columns: 20,
            rows: 10,
        };
        let content = [Coord {
            line: 9,
            column: 39,
        }];
        let canvas = MinimapBounds::canvas(&content, viewport);
        let panel = ScreenRect {
            left: 0.0,
            top: 0.0,
            right: 88.0,
            bottom: 48.0,
        };
        let nearby = MinimapGeometry::new(panel, canvas, canvas, 1.0, 2.0).unwrap();
        assert_eq!(nearby.canvas_cells_per_cell, 1);
        assert_eq!((nearby.cell_width, nearby.cell_height), (1.0, 2.0));

        let panned_viewport = VisibleCanvasCells {
            origin: (50, 0),
            ..viewport
        };
        let panned = MinimapGeometry::new(
            panel,
            canvas,
            canvas.union(MinimapBounds::viewport(panned_viewport)),
            1.0,
            2.0,
        )
        .unwrap();
        assert_eq!(panned.canvas_cells_per_cell, 1);

        let beyond_range = VisibleCanvasCells {
            origin: (61, 0),
            ..viewport
        };
        let zoomed = MinimapGeometry::new(
            panel,
            canvas,
            canvas.union(MinimapBounds::viewport(beyond_range)),
            1.0,
            2.0,
        )
        .unwrap();
        assert_eq!(zoomed.canvas_cells_per_cell, 2);
        assert_eq!((zoomed.cell_width, zoomed.cell_height), (1.0, 2.0));
    }

    #[test]
    fn uses_five_cumulative_density_levels() {
        assert_eq!(density_level(0, 2), 0);
        assert_eq!(density_level(1, 2), 1);
        assert_eq!(density_level(2, 2), 2);
        assert_eq!(density_level(3, 2), 3);
        assert_eq!(density_level(4, 2), 4);
    }

    #[test]
    fn aggregates_content_into_a_bounded_dense_grid() {
        let panel = ScreenRect {
            left: 0.0,
            top: 0.0,
            right: 10.0,
            bottom: 10.0,
        };
        let bounds = MinimapBounds {
            left: 0,
            top: 0,
            right: 8,
            bottom: 8,
        };
        let geometry = MinimapGeometry::new(panel, bounds, bounds, 1.0, 1.0).unwrap();
        let content = [
            Coord { line: 0, column: 0 },
            Coord { line: 0, column: 1 },
            Coord { line: 7, column: 7 },
        ];

        let occupancy = occupancy(&content, geometry).unwrap();

        assert_eq!(
            occupancy.len(),
            geometry.columns as usize * geometry.rows as usize
        );
        assert_eq!(occupancy.iter().sum::<usize>(), content.len());
        assert_eq!(occupancy.iter().filter(|count| **count > 0).count(), 2);
    }
}
