use skia_safe::{Canvas, Paint, Rect};

use crate::editor::Editor;
use crate::face_resolution::ResolvedFace;
use crate::layout::{ScreenRect, VisibleCanvasCells};
use crate::model::Coord;

use super::render_marching_edge;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MinimapBounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl MinimapBounds {
    fn from_content_and_viewport(content: &[Coord], viewport: VisibleCanvasCells) -> Self {
        let mut bounds = Self {
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
        };
        for coord in content {
            let column = i64::try_from(coord.column).unwrap_or(i64::MAX);
            let line = i64::try_from(coord.line).unwrap_or(i64::MAX);
            bounds.left = bounds.left.min(column);
            bounds.top = bounds.top.min(line);
            bounds.right = bounds.right.max(column.saturating_add(1));
            bounds.bottom = bounds.bottom.max(line.saturating_add(1));
        }
        bounds
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
    panel: ScreenRect,
    world: MinimapBounds,
    scale_x: f32,
    scale_y: f32,
    left: f32,
    top: f32,
}

impl MinimapGeometry {
    fn new(panel: ScreenRect, world: MinimapBounds, cell_aspect: f32) -> Option<Self> {
        const INSET: f32 = 4.0;
        let inner_width = panel.width() as f32 - INSET * 2.0;
        let inner_height = panel.height() as f32 - INSET * 2.0;
        if inner_width <= 0.0 || inner_height <= 0.0 {
            return None;
        }
        let cell_aspect = cell_aspect.max(f32::EPSILON);
        let scale_y = (inner_width / (world.width() as f32 * cell_aspect))
            .min(inner_height / world.height() as f32)
            .max(f32::EPSILON);
        let scale_x = scale_y * cell_aspect;
        let map_width = world.width() as f32 * scale_x;
        let map_height = world.height() as f32 * scale_y;
        Some(Self {
            panel,
            world,
            scale_x,
            scale_y,
            left: panel.left as f32 + INSET + (inner_width - map_width) / 2.0,
            top: panel.top as f32 + INSET + (inner_height - map_height) / 2.0,
        })
    }

    fn point(self, column: i64, line: i64) -> (f32, f32) {
        (
            self.left + column.saturating_sub(self.world.left) as f32 * self.scale_x,
            self.top + line.saturating_sub(self.world.top) as f32 * self.scale_y,
        )
    }

    fn rect(self, left: i64, top: i64, right: i64, bottom: i64) -> Rect {
        let (x, y) = self.point(left, top);
        let (right, bottom) = self.point(right, bottom);
        Rect::from_xywh(x, y, (right - x).max(1.0), (bottom - y).max(1.0))
    }
}

pub(super) fn render(
    canvas: &Canvas,
    state: &Editor,
    viewport: VisibleCanvasCells,
    panel: ScreenRect,
    cell_aspect: f32,
    default_face: &ResolvedFace,
) {
    let content = state.content_cells();
    let world = MinimapBounds::from_content_and_viewport(&content, viewport);
    let Some(geometry) = MinimapGeometry::new(panel, world, cell_aspect) else {
        return;
    };
    let panel_rect = Rect::from_xywh(
        panel.left as f32,
        panel.top as f32,
        panel.width() as f32,
        panel.height() as f32,
    );
    let mut background = Paint::default();
    background
        .set_anti_alias(false)
        .set_color(default_face.bg.to_color());
    canvas.draw_rect(panel_rect, &background);

    canvas.save();
    canvas.clip_rect(panel_rect, None, false);
    let mut foreground = Paint::default();
    foreground
        .set_anti_alias(false)
        .set_color(default_face.fg.to_color());
    for coord in content {
        let column = i64::try_from(coord.column).unwrap_or(i64::MAX);
        let line = i64::try_from(coord.line).unwrap_or(i64::MAX);
        canvas.draw_rect(
            geometry.rect(
                column,
                line,
                column.saturating_add(1),
                line.saturating_add(1),
            ),
            &foreground,
        );
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
    let mut viewport_paints = [Paint::default(), Paint::default()];
    for (paint, color) in viewport_paints
        .iter_mut()
        .zip([default_face.fg, default_face.bg])
    {
        paint
            .set_anti_alias(false)
            .set_color(color.to_color())
            .set_stroke_width(1.0);
    }
    for (start, end) in [
        (
            (viewport_rect.left, viewport_rect.top),
            (viewport_rect.right, viewport_rect.top),
        ),
        (
            (viewport_rect.right, viewport_rect.top),
            (viewport_rect.right, viewport_rect.bottom),
        ),
        (
            (viewport_rect.right, viewport_rect.bottom),
            (viewport_rect.left, viewport_rect.bottom),
        ),
        (
            (viewport_rect.left, viewport_rect.bottom),
            (viewport_rect.left, viewport_rect.top),
        ),
    ] {
        render_marching_edge(canvas, start, end, 4.0, 0, &viewport_paints);
    }
    canvas.restore();

    foreground.set_stroke_width(1.0);
    for (start, end) in [
        (
            (panel_rect.left, panel_rect.top),
            (panel_rect.right, panel_rect.top),
        ),
        (
            (panel_rect.right, panel_rect.top),
            (panel_rect.right, panel_rect.bottom),
        ),
        (
            (panel_rect.right, panel_rect.bottom),
            (panel_rect.left, panel_rect.bottom),
        ),
        (
            (panel_rect.left, panel_rect.bottom),
            (panel_rect.left, panel_rect.top),
        ),
    ] {
        canvas.draw_line(start, end, &foreground);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_content_and_signed_viewport_with_adaptive_cell_size() {
        let viewport = VisibleCanvasCells {
            origin: (-3, 1),
            columns: 8,
            rows: 4,
        };
        let content = [Coord { line: 3, column: 2 }, Coord { line: 6, column: 9 }];
        let bounds = MinimapBounds::from_content_and_viewport(&content, viewport);
        assert_eq!(
            bounds,
            MinimapBounds {
                left: -3,
                top: 1,
                right: 10,
                bottom: 7,
            }
        );

        let panel = ScreenRect {
            left: 0,
            top: 0,
            right: 108,
            bottom: 58,
        };
        let nearby = MinimapGeometry::new(panel, bounds, 0.5).unwrap();
        let distant = MinimapGeometry::new(
            panel,
            MinimapBounds {
                left: -100,
                top: -50,
                right: 100,
                bottom: 50,
            },
            0.5,
        )
        .unwrap();
        assert!(distant.scale_y < nearby.scale_y);

        let viewport_rect = nearby.rect(-3, 1, 5, 5);
        assert!((viewport_rect.width() / viewport_rect.height() - 1.0).abs() < f32::EPSILON);
        assert!(viewport_rect.left >= panel.left as f32);
        assert!(viewport_rect.top >= panel.top as f32);
        assert!(viewport_rect.right <= panel.right as f32);
        assert!(viewport_rect.bottom <= panel.bottom as f32);
    }
}
