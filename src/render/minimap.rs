use std::collections::BTreeSet;

use skia_safe::{Canvas, Paint, Rect};

use crate::canvas::LayerStack;
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
    fn canvas(content: impl IntoIterator<Item = Coord>, viewport: VisibleCanvasCells) -> Self {
        let mut bounds = Self {
            left: 0,
            top: 0,
            right: i64::try_from(viewport.columns.max(1)).unwrap_or(i64::MAX),
            bottom: i64::try_from(viewport.rows.max(1)).unwrap_or(i64::MAX),
        };
        for coord in content {
            bounds.include(coord);
        }
        bounds
    }

    fn include(&mut self, coord: Coord) {
        let column = i64::from(coord.column);
        let line = i64::from(coord.line);
        self.left = self.left.min(column);
        self.top = self.top.min(line);
        self.right = self.right.max(column.saturating_add(1));
        self.bottom = self.bottom.max(line.saturating_add(1));
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
    columns: i64,
    rows: i64,
    world_left: f64,
    world_top: f64,
    canvas_cells_per_cell: f64,
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
        let canvas_cells_per_cell = (required_bounds.width() as f64 / columns as f64)
            .max(required_bounds.height() as f64 / rows as f64)
            .max(1.0);
        let world_width = columns as f64 * canvas_cells_per_cell;
        let world_height = rows as f64 * canvas_cells_per_cell;
        let world_left = centered_origin(
            canvas_bounds.left,
            canvas_bounds.right,
            required_bounds.left,
            required_bounds.right,
            world_width,
        );
        let world_top = centered_origin(
            canvas_bounds.top,
            canvas_bounds.bottom,
            required_bounds.top,
            required_bounds.bottom,
            world_height,
        );
        let map_width = columns as f32 * cell_width;
        let map_height = rows as f32 * cell_height;
        Some(Self {
            columns,
            rows,
            world_left,
            world_top,
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
                + ((column as f64 - self.world_left) / self.canvas_cells_per_cell) as f32
                    * self.cell_width,
            self.top
                + ((line as f64 - self.world_top) / self.canvas_cells_per_cell) as f32
                    * self.cell_height,
        )
    }

    fn rect(self, left: i64, top: i64, right: i64, bottom: i64) -> Rect {
        let (x, y) = self.point(left, top);
        let (right, bottom) = self.point(right, bottom);
        Rect::from_xywh(x, y, (right - x).max(1.0), (bottom - y).max(1.0))
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
    capacity: f64,
) -> f64 {
    let centered = (canvas_start as f64 + canvas_end as f64 - capacity) / 2.0;
    centered.clamp(required_end as f64 - capacity, required_start as f64)
}

fn density_level(coverage: f64) -> u8 {
    if coverage <= 0.0 {
        return 0;
    }
    ((coverage.clamp(0.0, 1.0) * 10.0 - 1e-9).ceil() as u8).clamp(1, 10)
}

fn empty_occupancy(geometry: MinimapGeometry) -> Option<Vec<f64>> {
    let len = usize::try_from(geometry.columns).ok().and_then(|columns| {
        usize::try_from(geometry.rows)
            .ok()
            .and_then(|rows| columns.checked_mul(rows))
    })?;
    Some(vec![0.0; len])
}

fn add_occupancy(occupancy: &mut [f64], geometry: MinimapGeometry, coord: Coord) {
    let scale = geometry.canvas_cells_per_cell;
    let tile_area = scale * scale;
    let source_left = coord.column as f64;
    let source_top = coord.line as f64;
    let source_right = source_left + 1.0;
    let source_bottom = source_top + 1.0;
    let first_column = ((source_left - geometry.world_left) / scale).floor() as i64;
    let last_column = ((source_right - geometry.world_left) / scale).ceil() as i64 - 1;
    let first_row = ((source_top - geometry.world_top) / scale).floor() as i64;
    let last_row = ((source_bottom - geometry.world_top) / scale).ceil() as i64 - 1;

    for row in first_row.max(0)..=last_row.min(geometry.rows - 1) {
        let tile_top = geometry.world_top + row as f64 * scale;
        let overlap_height = source_bottom.min(tile_top + scale) - source_top.max(tile_top);
        if overlap_height <= 0.0 {
            continue;
        }
        for column in first_column.max(0)..=last_column.min(geometry.columns - 1) {
            let tile_left = geometry.world_left + column as f64 * scale;
            let overlap_width = source_right.min(tile_left + scale) - source_left.max(tile_left);
            if overlap_width <= 0.0 {
                continue;
            }
            let index = row.saturating_mul(geometry.columns).saturating_add(column);
            if let Ok(index) = usize::try_from(index) {
                occupancy[index] += overlap_width * overlap_height / tile_area;
            }
        }
    }
}

pub(super) fn render(
    canvas: &Canvas,
    state: &Editor,
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
    let canvas_bounds = sparse_minimap_bounds(state.canvas(), viewport);
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
        border_metrics.cell_width / 2.0,
        border_metrics.cell_height / 2.0,
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
    let Some(mut occupancy) = empty_occupancy(geometry) else {
        canvas.restore();
        return;
    };
    for_each_visible_coord(state.canvas(), |coord| {
        add_occupancy(&mut occupancy, geometry, coord)
    });
    for (index, occupied) in occupancy
        .into_iter()
        .enumerate()
        .filter(|(_, coverage)| *coverage > 0.0)
    {
        let index = i64::try_from(index).unwrap_or(i64::MAX);
        let cell = (
            index.rem_euclid(geometry.columns),
            index.div_euclid(geometry.columns),
        );
        let mut density = foreground.clone();
        density.set_alpha_f(f32::from(density_level(occupied)) / 10.0);
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

fn sparse_minimap_bounds(layers: &LayerStack, viewport: VisibleCanvasCells) -> MinimapBounds {
    let mut bounds = MinimapBounds::canvas(std::iter::empty(), viewport);
    for layer in layers.layers() {
        for (&line, row) in layer.rows() {
            for (&column, data) in row {
                if !data.atom.contents().chars().all(char::is_whitespace) {
                    bounds.include(Coord { line, column });
                }
            }
        }
    }
    bounds
}

fn for_each_visible_coord(layers: &LayerStack, mut apply: impl FnMut(Coord)) {
    let mut rows = layers
        .effective_layers()
        .iter()
        .filter(|layer| layer.visible)
        .map(|layer| layer.rows().iter().peekable())
        .collect::<Vec<_>>();
    loop {
        let Some(line) = rows
            .iter_mut()
            .filter_map(|rows| rows.peek().map(|&(&line, _)| line))
            .min()
        else {
            break;
        };
        let mut columns = BTreeSet::new();
        for rows in &mut rows {
            if rows.peek().is_some_and(|&(&row_line, _)| row_line == line) {
                let (_, row) = rows.next().expect("peeked sparse row exists");
                columns.extend(row.iter().filter_map(|(&column, data)| {
                    (!data.atom.contents().chars().all(char::is_whitespace)).then_some(column)
                }));
            }
        }
        for column in columns {
            apply(Coord { line, column });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occupancy(
        content: impl IntoIterator<Item = Coord>,
        geometry: MinimapGeometry,
    ) -> Option<Vec<f64>> {
        let mut occupancy = empty_occupancy(geometry)?;
        for coord in content {
            add_occupancy(&mut occupancy, geometry, coord);
        }
        Some(occupancy)
    }

    #[test]
    fn keeps_micro_cells_fixed_and_scales_canvas_coverage_fluidly() {
        let viewport = VisibleCanvasCells {
            origin: (0, 0),
            columns: 20,
            rows: 10,
        };
        let content = [Coord {
            line: 9,
            column: 39,
        }];
        let canvas = MinimapBounds::canvas(content.iter().copied(), viewport);
        let panel = ScreenRect {
            left: 0.0,
            top: 0.0,
            right: 88.0,
            bottom: 48.0,
        };
        let nearby = MinimapGeometry::new(panel, canvas, canvas, 1.0, 2.0).unwrap();
        assert_eq!(nearby.canvas_cells_per_cell, 1.0);
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
        assert_eq!(panned.canvas_cells_per_cell, 1.0);

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
        assert!((zoomed.canvas_cells_per_cell - 1.0125).abs() < f64::EPSILON);
        assert_eq!((zoomed.cell_width, zoomed.cell_height), (1.0, 2.0));
    }

    #[test]
    fn uses_ten_ceiling_based_density_levels() {
        assert_eq!(density_level(0.0), 0);
        assert_eq!(density_level(0.001), 1);
        assert_eq!(density_level(0.1), 1);
        assert_eq!(density_level(0.1001), 2);
        assert_eq!(density_level(0.9), 9);
        assert_eq!(density_level(0.9001), 10);
        assert_eq!(density_level(1.0), 10);
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

        let occupancy = occupancy(content.iter().copied(), geometry).unwrap();

        assert_eq!(
            occupancy.len(),
            geometry.columns as usize * geometry.rows as usize
        );
        assert!((occupancy.iter().sum::<f64>() - 3.0 / 16.0).abs() < f64::EPSILON);
        assert_eq!(occupancy.iter().filter(|count| **count > 0.0).count(), 2);
    }

    #[test]
    fn projects_source_area_into_fixed_minimap_cells() {
        let panel = ScreenRect {
            left: 0.0,
            top: 0.0,
            right: 12.0,
            bottom: 12.0,
        };
        let bounds = MinimapBounds {
            left: 0,
            top: 0,
            right: 32,
            bottom: 32,
        };
        let geometry = MinimapGeometry::new(panel, bounds, bounds, 1.0, 1.0).unwrap();
        let content = (0..4)
            .flat_map(|line| (0..4).map(move |column| Coord { line, column }))
            .collect::<Vec<_>>();

        let occupancy = occupancy(content.iter().copied(), geometry).unwrap();

        assert_eq!(geometry.columns, 4);
        assert_eq!(geometry.rows, 4);
        assert_eq!(geometry.canvas_cells_per_cell, 8.0);
        assert!((occupancy[0] - 0.25).abs() < f64::EPSILON);
        assert_eq!(density_level(occupancy[0]), 3);
        assert_eq!(
            occupancy.iter().filter(|coverage| **coverage > 0.0).count(),
            1
        );
    }

    #[test]
    fn visible_sparse_scan_deduplicates_composed_layer_coordinates() {
        let mut base = crate::canvas::LayerMap::new(crate::model::LayerId(0), true);
        base.set_at(
            -2,
            -3,
            crate::model::Atom::new("a").unwrap(),
            &crate::model::Face::default(),
        )
        .unwrap();
        let mut top = crate::canvas::LayerMap::new(crate::model::LayerId(1), true);
        top.set_at(
            -2,
            -3,
            crate::model::Atom::new("b").unwrap(),
            &crate::model::Face::default(),
        )
        .unwrap();
        top.set_at(
            4,
            5,
            crate::model::Atom::new("c").unwrap(),
            &crate::model::Face::default(),
        )
        .unwrap();
        let stack = LayerStack::new(vec![base, top], true).unwrap();
        let mut coords = Vec::new();

        for_each_visible_coord(&stack, |coord| coords.push(coord));

        assert_eq!(
            coords,
            [
                Coord {
                    line: -3,
                    column: -2,
                },
                Coord { line: 5, column: 4 },
            ]
        );
    }
}
