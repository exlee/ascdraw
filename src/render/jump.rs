use skia_safe::{Canvas, Paint};

use super::{CellMetrics, FALLBACK_BG, FALLBACK_FG, PADDING};
use crate::app::CursorMode;
use crate::editor::Editor;
use crate::face_resolution::resolve_derived_face;
use crate::jump::JumpBounds;

const SELECTED_INSET: f32 = 2.0;
const SELECTED_STROKE_WIDTH: f32 = 2.0;

pub(super) fn render_jump_overlay(
    canvas: &Canvas,
    state: &Editor,
    metrics: &CellMetrics,
    grid_top: usize,
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
        .set_color(grid_color.to_color())
        .set_stroke_width(1.0);
    for sector in overlay.sectors {
        draw_outline(canvas, sector, metrics, grid_top, 0.0, &grid_paint);
    }
    if let Some(selected) = selected {
        let mut selected_paint = Paint::default();
        selected_paint
            .set_anti_alias(false)
            .set_color(cursor_color.to_color())
            .set_stroke_width(SELECTED_STROKE_WIDTH);
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
    grid_top: usize,
    inset: f32,
    paint: &Paint,
) {
    let left = PADDING as f32 + bounds.column as f32 * metrics.cell_width as f32 + inset;
    let top = grid_top as f32 + bounds.line as f32 * metrics.cell_height as f32 + inset;
    let right = left + bounds.columns as f32 * metrics.cell_width as f32 - 1.0 - inset * 2.0;
    let bottom = top + bounds.rows as f32 * metrics.cell_height as f32 - 1.0 - inset * 2.0;
    canvas.draw_line((left, top), (right, top), paint);
    canvas.draw_line((right, top), (right, bottom), paint);
    canvas.draw_line((right, bottom), (left, bottom), paint);
    canvas.draw_line((left, bottom), (left, top), paint);
}
