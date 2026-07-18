use skia_safe::{Canvas, Paint, Rect};

use super::{CellMetrics, FALLBACK_BG, FALLBACK_FG, PADDING, outline_stroke_width};
use crate::app::CursorMode;
use crate::editor::Editor;
use crate::face_resolution::resolve_derived_face;
use crate::jump::JumpBounds;

const SELECTED_INSET: f32 = 2.0;

pub(super) fn render_jump_overlay(
    canvas: &Canvas,
    state: &Editor,
    metrics: &CellMetrics,
    grid_top: f32,
) {
    let Some(overlay) = state.jump_overlay() else {
        return;
    };
    let selected = overlay.sectors.get(overlay.selected).copied();
    let grid_color = resolve_derived_face(
        &state.grid.default_face,
        &state.theme.jump_grid,
        FALLBACK_FG,
        FALLBACK_BG,
    )
    .fg;
    let cursor_color = if matches!(
        state.cursor_mode,
        CursorMode::MoveDraw | CursorMode::Stamp | CursorMode::Shapes | CursorMode::Utilities
    ) {
        resolve_derived_face(
            &state.grid.default_face,
            &state.theme.cursor_drawing,
            FALLBACK_FG,
            FALLBACK_BG,
        )
        .fg
    } else {
        resolve_derived_face(
            &state.grid.default_face,
            &state.grid.cursor_face,
            FALLBACK_FG,
            FALLBACK_BG,
        )
        .bg
    };

    let mut grid_paint = Paint::default();
    grid_paint
        .set_anti_alias(false)
        .set_style(skia_safe::paint::Style::Stroke)
        .set_stroke_join(skia_safe::paint::Join::Miter)
        .set_color(grid_color.to_color())
        .set_stroke_width(outline_stroke_width(metrics));
    for sector in overlay.sectors {
        draw_outline(canvas, sector, metrics, grid_top, 0.0, &grid_paint);
    }
    if let Some(selected) = selected {
        let mut selected_paint = Paint::default();
        selected_paint
            .set_anti_alias(false)
            .set_style(skia_safe::paint::Style::Stroke)
            .set_stroke_join(skia_safe::paint::Join::Miter)
            .set_color(cursor_color.to_color())
            .set_stroke_width(outline_stroke_width(metrics));
        draw_outline(
            canvas,
            selected,
            metrics,
            grid_top,
            SELECTED_INSET,
            &selected_paint,
        );
    }
}

fn draw_outline(
    canvas: &Canvas,
    bounds: JumpBounds,
    metrics: &CellMetrics,
    grid_top: f32,
    inset: f32,
    paint: &Paint,
) {
    let left = PADDING as f32 + bounds.column as f32 * metrics.cell_width + inset;
    let top = grid_top + bounds.line as f32 * metrics.cell_height + inset;
    let right = left + bounds.columns as f32 * metrics.cell_width - 1.0 - inset * 2.0;
    let bottom = top + bounds.rows as f32 * metrics.cell_height - 1.0 - inset * 2.0;
    canvas.draw_rect(Rect::new(left, top, right, bottom), paint);
}
