use skia_safe::{Canvas, ClipOp, Paint, PathEffect, Rect, paint::Style};

use super::{
    CellMetrics, FALLBACK_BG, FALLBACK_FG, PADDING, font_for_face, font_for_text,
    outline_stroke_width,
};
use crate::editor::{AnchorSegment, Editor};
use crate::face_resolution::{Rgba, resolve_derived_face, resolve_root_face};
use crate::model::Coord;
use crate::objects::AnchorKind;
use crate::selection::SelectionBounds;

/// Opacity of the veil over everything outside the edited object.
const DIM_ALPHA: u8 = 150;
/// Opacity of the foreground-colored shade that darkens the dimmed area.
const SHADE_ALPHA: u8 = 40;
/// Anchor box size relative to its cell, scaled from the center.
const ANCHOR_SCALE: f32 = 0.33;
/// Opacity of the outline around the copy under the cursor.
const CURSOR_OBJECT_ALPHA: u8 = 110;
/// Opacity of the local copy drawn over the definition in DfnEdt.
const GHOST_ALPHA: u8 = 110;

fn cell_rect(bounds: SelectionBounds, metrics: &CellMetrics, grid_top: f32) -> Rect {
    Rect::new(
        PADDING as f32 + f32::from(bounds.left) * metrics.cell_width,
        grid_top + f32::from(bounds.top) * metrics.cell_height,
        PADDING as f32 + (f32::from(bounds.right) + 1.0) * metrics.cell_width,
        grid_top + (f32::from(bounds.bottom) + 1.0) * metrics.cell_height,
    )
}

fn cell_center(coord: Coord, metrics: &CellMetrics, grid_top: f32) -> (f32, f32) {
    (
        PADDING as f32 + (f32::from(coord.column) + 0.5) * metrics.cell_width,
        grid_top + (f32::from(coord.line) + 0.5) * metrics.cell_height,
    )
}

fn with_alpha(color: Rgba, a: u8) -> Rgba {
    Rgba { a, ..color }
}

pub(super) fn render_object_overlay(
    canvas: &Canvas,
    state: &Editor,
    metrics: &CellMetrics,
    grid_top: f32,
) {
    let overlay = state.object_overlay();
    let default_face = &state.grid.default_face;
    let stroke = outline_stroke_width(metrics);

    if let Some(focus) = overlay.focus {
        let background = resolve_root_face(default_face, FALLBACK_FG, FALLBACK_BG).bg;
        let mut veil = Paint::default();
        veil.set_color(with_alpha(background, DIM_ALPHA).to_color());
        canvas.save();
        canvas.clip_rect(
            cell_rect(focus, metrics, grid_top),
            ClipOp::Difference,
            false,
        );
        canvas.draw_paint(&veil);
        // Darken as well, so the background recedes along with the lines.
        let shade = resolve_root_face(default_face, FALLBACK_FG, FALLBACK_BG).fg;
        veil.set_color(with_alpha(shade, SHADE_ALPHA).to_color());
        canvas.draw_paint(&veil);
        canvas.restore();
    }

    if !overlay.definition_cells.is_empty() {
        let background = resolve_root_face(default_face, FALLBACK_FG, FALLBACK_BG).bg;
        let mut veil = Paint::default();
        veil.set_color(with_alpha(background, DIM_ALPHA).to_color());
        for coord in &overlay.definition_cells {
            let cell = SelectionBounds {
                left: coord.column,
                right: coord.column,
                top: coord.line,
                bottom: coord.line,
            };
            canvas.draw_rect(cell_rect(cell, metrics, grid_top), &veil);
        }
    }

    if !overlay.blank_cells.is_empty() {
        let shade = resolve_root_face(default_face, FALLBACK_FG, FALLBACK_BG).fg;
        let mut paint = Paint::default();
        paint.set_color(with_alpha(shade, GHOST_ALPHA / 3).to_color());
        for coord in &overlay.blank_cells {
            let cell = SelectionBounds {
                left: coord.column,
                right: coord.column,
                top: coord.line,
                bottom: coord.line,
            };
            canvas.draw_rect(cell_rect(cell, metrics, grid_top), &paint);
        }
    }

    if !overlay.outlines.is_empty() {
        let color = resolve_derived_face(
            default_face,
            &state.theme.object_outline,
            FALLBACK_FG,
            FALLBACK_BG,
        )
        .fg;
        let dash = (metrics.cell_width.min(metrics.cell_height) / 3.0).max(2.0);
        let mut paint = Paint::default();
        paint
            .set_anti_alias(false)
            .set_style(Style::Stroke)
            .set_stroke_width(stroke)
            .set_color(color.to_color())
            .set_path_effect(PathEffect::dash(&[dash, dash], 0.0));
        for bounds in &overlay.outlines {
            canvas.draw_rect(cell_rect(*bounds, metrics, grid_top), &paint);
        }
    }

    if let Some(bounds) = overlay.cursor_object {
        let color = resolve_derived_face(
            default_face,
            &state.theme.cursor_drawing,
            FALLBACK_FG,
            FALLBACK_BG,
        )
        .fg;
        let mut paint = Paint::default();
        paint
            .set_anti_alias(false)
            .set_style(Style::Stroke)
            .set_stroke_width(stroke * 2.0)
            .set_color(with_alpha(color, CURSOR_OBJECT_ALPHA).to_color())
            .set_path_effect(PathEffect::dash(&[stroke * 4.0, stroke * 3.0], 0.0));
        canvas.draw_rect(cell_rect(bounds, metrics, grid_top), &paint);
    }

    for (coord, cell) in &overlay.ghost {
        let resolved = resolve_derived_face(default_face, &cell.face, FALLBACK_FG, FALLBACK_BG);
        let rect = cell_rect(
            SelectionBounds {
                left: coord.column,
                right: coord.column,
                top: coord.line,
                bottom: coord.line,
            },
            metrics,
            grid_top,
        );
        let mut paint = Paint::default();
        paint
            .set_anti_alias(true)
            .set_color(with_alpha(resolved.fg, GHOST_ALPHA).to_color());
        if cell.is_blank() {
            // A blank local cell clears the definition underneath.
            paint.set_color(with_alpha(resolved.fg, GHOST_ALPHA / 3).to_color());
            canvas.draw_rect(rect, &paint);
            continue;
        }
        let font = font_for_text(metrics, &font_for_face(metrics, &resolved), &cell.atom);
        canvas.draw_str(
            &cell.atom,
            (rect.left, rect.top + metrics.baseline_offset),
            &font,
            &paint,
        );
    }

    if overlay.anchors.is_empty() {
        return;
    }
    let color = resolve_derived_face(
        default_face,
        &state.theme.object_anchor,
        FALLBACK_FG,
        FALLBACK_BG,
    )
    .fg;
    let half = (
        metrics.cell_width * ANCHOR_SCALE / 2.0,
        metrics.cell_height * ANCHOR_SCALE / 2.0,
    );
    let dash = (half.0.min(half.1) / 2.0).max(1.0);
    for anchor in &overlay.anchors {
        let mut line = Paint::default();
        line.set_anti_alias(true)
            .set_style(Style::Stroke)
            .set_stroke_width(stroke)
            .set_color(color.to_color());
        if anchor.kind == AnchorKind::Extension {
            line.set_path_effect(PathEffect::dash(&[stroke, dash], 0.0));
        }
        let end = cell_center(anchor.at, metrics, grid_top);
        for segment in &anchor.segments {
            // Lines run between anchor outlines, or from one cell past the
            // object boundary.
            let start = match *segment {
                AnchorSegment::Anchor(from) => {
                    box_edge(cell_center(from, metrics, grid_top), end, half)
                }
                AnchorSegment::Edge { horizontal, min } => {
                    let Some(focus) = overlay.focus else {
                        continue;
                    };
                    edge_point(
                        end,
                        horizontal,
                        min,
                        cell_rect(focus, metrics, grid_top),
                        (metrics.cell_width, metrics.cell_height),
                    )
                }
            };
            canvas.draw_line(start, box_edge(end, start, half), &line);
        }
        let (x, y) = cell_center(anchor.at, metrics, grid_top);
        let mut fill = Paint::default();
        fill.set_anti_alias(true).set_color(color.to_color());
        canvas.draw_rect(
            Rect::new(x - half.0, y - half.1, x + half.0, y + half.1),
            &fill,
        );
    }
}

/// The start of a line from one cell past a side of `boundary` to an
/// anchor centered at `anchor`.
fn edge_point(
    anchor: (f32, f32),
    horizontal: bool,
    min: bool,
    boundary: Rect,
    cell: (f32, f32),
) -> (f32, f32) {
    match (horizontal, min) {
        (true, true) => (boundary.left - cell.0, anchor.1),
        (true, false) => (boundary.right + cell.0, anchor.1),
        (false, true) => (anchor.0, boundary.top - cell.1),
        (false, false) => (anchor.0, boundary.bottom + cell.1),
    }
}

/// Where the segment from `center` toward `toward` leaves a box of `half`
/// extents around `center`.
fn box_edge(center: (f32, f32), toward: (f32, f32), half: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (toward.0 - center.0, toward.1 - center.1);
    let reach = (dx.abs() / half.0).max(dy.abs() / half.1);
    if reach <= 1.0 {
        return toward;
    }
    (center.0 + dx / reach, center.1 + dy / reach)
}

#[cfg(test)]
#[path = "../inline_tests/render_objects_tests.rs"]
mod tests;
