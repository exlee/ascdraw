use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow};
use skia_safe::{
    AlphaType, Canvas, ColorType, Font, FontHinting, FontMgr, FontStyle, ImageInfo, Paint,
    PixelGeometry, Rect, SurfaceProps, SurfacePropsFlags, font::Edging, surfaces,
};
use softbuffer::Surface;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use winit::window::Window;

use crate::app::{AppConfig, CursorMode, CursorShape, CursorShapeConfig};
use crate::editor::Editor;
use crate::face_resolution::{
    ResolvedFace, Rgba, UnderlineStyle, resolve_derived_face, resolve_root_face,
};
use crate::layout::{
    LayoutMetrics, PADDING, ViewportOffset, VisibleCanvasCells, layout_metrics, minimap_rect,
};
use crate::model::{Atom, Face};
use crate::perf::FrameTiming;
use crate::selection::SelectionBounds;
use crate::toolbar_stamp::toolbar_atoms;

mod cell_graphics;
mod export_png;
mod jump;
#[cfg(target_os = "macos")]
mod metal;
mod minimap;
mod window_surface;
pub use export_png::{CanvasImage, render_canvas_image, render_canvas_layers_image};
pub use window_surface::WindowSurface;

pub(crate) const FALLBACK_BG: Rgba = Rgba::rgb(0xff, 0xff, 0xff);
pub(crate) const FALLBACK_FG: Rgba = Rgba::rgb(0x00, 0x00, 0x00);
const TOOLBAR_SELECTION_PADDING: f32 = 1.0;
const TOOLBAR_SELECTION_STROKE_WIDTH: f32 = 2.0;
const CANVAS_SELECTION_STROKE_WIDTH: f32 = 2.0;
const DRAWING_CURSOR_INSET_RATIO: f32 = 0.12;
const DRAWING_CURSOR_WIDTH_RATIO: f32 = 0.06;

#[derive(Clone)]
pub struct Renderer {
    font_mgr: FontMgr,
    preferred_font_family: String,
    default_logical_font_size: f32,
    underline_offset: f32,
    logical_font_size: Cell<f32>,
    content_metrics_cache: RefCell<Option<(u64, CellMetrics)>>,
    fixed_metrics_cache: RefCell<Option<(u64, CellMetrics)>>,
}

#[derive(Clone)]
pub struct CellMetrics {
    pub font: Font,
    pub cell_width: f32,
    pub cell_height: f32,
    pub baseline_offset: f32,
    pub underline_offset: f32,
    font_mgr: FontMgr,
    fallback_fonts: Rc<RefCell<HashMap<FallbackFontKey, Font>>>,
}

#[derive(Clone)]
struct CursorCell {
    face: Face,
    text: Option<String>,
}

const CURSOR_BEAM_WIDTH_RATIO: f32 = 0.14;
const CURSOR_UNDERLINE_HEIGHT_RATIO: f32 = 0.12;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct FallbackFontKey {
    character: u32,
    size_bits: u32,
    bold: bool,
    italic: bool,
}

#[derive(Clone, Copy)]
struct LineRenderPosition {
    pub row: usize,
    pub first_column: usize,
    pub max_column: usize,
}

#[derive(Clone, Copy)]
enum DrawOrigin {
    Grid { top_padding: f32 },
}

struct RenderFrame<'a> {
    metrics: &'a CellMetrics,
    toolbar_metrics: &'a CellMetrics,
    layout: LayoutMetrics,
    width: usize,
    viewport: ViewportOffset,
    toolbar_hotspot_hovered: bool,
}

pub fn render(
    window: &Window,
    surface: &mut Surface<Rc<Window>, Rc<Window>>,
    state: &Editor,
    renderer: &Renderer,
    config: &AppConfig,
    viewport: ViewportOffset,
    toolbar_hotspot_hovered: bool,
) -> Result<FrameTiming> {
    let buffer_started = std::time::Instant::now();
    let size = window.inner_size();
    let width = size.width.max(1) as usize;
    let height = size.height.max(1) as usize;
    let metrics = renderer.metrics(window.scale_factor());
    let title_metrics = renderer.title_metrics(window.scale_factor());

    let mut buffer = surface
        .buffer_mut()
        .map_err(|error| anyhow!(error.to_string()))?;
    let buffer_acquisition = buffer_started.elapsed();
    let raster_started = std::time::Instant::now();
    let pixels = unsafe { buffer_as_u8_mut(buffer.as_mut()) };

    let image_info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::BGRA8888,
        AlphaType::Premul,
        None,
    );
    let props = SurfaceProps::new(
        SurfacePropsFlags::USE_DEVICE_INDEPENDENT_FONTS,
        PixelGeometry::Unknown,
    );
    let mut skia_surface = surfaces::wrap_pixels(&image_info, pixels, width * 4, Some(&props))
        .context("failed to wrap Skia surface around window buffer")?;
    let canvas = skia_surface.canvas();

    render_canvas(
        canvas,
        state,
        config,
        RenderFrame {
            metrics: &metrics,
            toolbar_metrics: &title_metrics,
            layout: layout_metrics(
                width,
                height,
                &metrics,
                (title_metrics.cell_width, title_metrics.cell_height),
                &state.toolbar,
                config.transparent_menubar,
                window.scale_factor(),
            ),
            width,
            viewport,
            toolbar_hotspot_hovered,
        },
    );
    let rasterization = raster_started.elapsed();

    let presentation_started = std::time::Instant::now();
    buffer
        .present()
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(FrameTiming {
        buffer_acquisition,
        rasterization,
        presentation: presentation_started.elapsed(),
    })
}

fn render_canvas(canvas: &Canvas, state: &Editor, config: &AppConfig, frame: RenderFrame<'_>) {
    let RenderFrame {
        metrics,
        toolbar_metrics,
        layout,
        width,
        viewport,
        toolbar_hotspot_hovered,
    } = frame;
    let default_face = resolve_root_face(&state.grid.default_face, FALLBACK_FG, FALLBACK_BG);
    canvas.clear(default_face.bg.to_color());
    render_window_title(
        canvas,
        &state.window_title,
        &state.grid.default_face,
        toolbar_metrics,
        layout,
        width,
        config.transparent_menubar,
    );
    render_toolbar(
        canvas,
        state,
        toolbar_metrics,
        layout.top_padding,
        width,
        toolbar_hotspot_hovered,
    );

    let grid_layout = visible_grid_layout(layout, metrics, viewport);
    let visible_cells = VisibleCanvasCells::from_layout(
        layout,
        viewport,
        (metrics.cell_width, metrics.cell_height),
    );
    let first_row = usize::try_from(visible_cells.origin.1.max(0)).unwrap_or(usize::MAX);
    let first_column = usize::try_from(visible_cells.origin.0.max(0)).unwrap_or(usize::MAX);
    let max_row = first_row
        .saturating_add(visible_cells.rows)
        .saturating_add(2);
    let max_column = first_column
        .saturating_add(visible_cells.columns)
        .saturating_add(2);
    canvas.save();
    canvas.clip_rect(
        Rect::from_xywh(
            0.0,
            layout.grid_top,
            width as f32,
            (layout.grid_bottom - layout.grid_top).max(0.0),
        ),
        None,
        false,
    );
    canvas.translate((viewport.x as f32, viewport.y as f32));

    let preview_lines = state
        .preview_render_lines()
        .is_none()
        .then(|| state.lines_with_shape_preview())
        .flatten();
    let active_lines = state
        .preview_render_lines()
        .or(preview_lines.as_deref())
        .unwrap_or(&state.grid.lines);
    let active_layer = state.active_layer_id();
    for layer in state
        .layer_views()
        .into_iter()
        .filter(|layer| layer.visible)
    {
        let lines = state
            .move_lift_render_lines_for_layer(layer.id)
            .unwrap_or_else(|| {
                if layer.id == active_layer {
                    active_lines
                } else {
                    layer.lines
                }
            });
        for (row_index, line) in lines
            .iter()
            .enumerate()
            .skip(first_row)
            .take(max_row.saturating_sub(first_row))
        {
            render_overlay_line(
                canvas,
                row_index,
                line,
                &state.grid.default_face,
                first_column..max_column,
                metrics,
                DrawOrigin::Grid {
                    top_padding: layout.grid_top,
                },
            );
        }
    }

    render_canvas_selection(canvas, state, metrics, layout.grid_top);
    jump::render_jump_overlay(canvas, state, metrics, layout.grid_top);
    if grid_cursor_is_visible(state) {
        render_grid_cursor(
            canvas,
            state,
            active_lines,
            &config.display.cursor_shape,
            grid_layout,
            metrics,
        );
    }
    canvas.restore();
    let minimap_panel = minimap_rect(
        width,
        layout.grid_top,
        (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
    );
    minimap::render(
        canvas,
        state,
        visible_cells,
        minimap_panel,
        metrics.cell_width / metrics.cell_height.max(1.0),
        toolbar_metrics,
        &default_face,
    );
    render_bottom_tooltip(canvas, state, toolbar_metrics, layout, width);
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CanvasSelectionOutline {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn canvas_selection_outline(
    bounds: SelectionBounds,
    metrics: &CellMetrics,
    grid_top: f32,
) -> CanvasSelectionOutline {
    CanvasSelectionOutline {
        left: PADDING as f32 + bounds.left as f32 * metrics.cell_width,
        top: row_top(bounds.top, metrics, grid_top),
        right: PADDING as f32 + (bounds.left + bounds.width()) as f32 * metrics.cell_width - 1.0,
        bottom: row_top(bounds.top + bounds.height(), metrics, grid_top) - 1.0,
    }
}

fn render_canvas_selection(canvas: &Canvas, state: &Editor, metrics: &CellMetrics, grid_top: f32) {
    if !canvas_selection_is_visible(state) {
        return;
    }
    let outline = canvas_selection_outline(
        state
            .move_lift_bounds()
            .unwrap_or_else(|| state.selection_bounds()),
        metrics,
        grid_top,
    );
    let color = resolve_derived_face(
        &state.grid.default_face,
        &state.theme.selection,
        FALLBACK_FG,
        FALLBACK_BG,
    )
    .fg;
    if state.move_lift_active() {
        let alternate = resolve_derived_face(
            &state.grid.default_face,
            &state.theme.selection_highlight,
            FALLBACK_FG,
            FALLBACK_BG,
        )
        .fg;
        render_marching_ants(canvas, outline, color, alternate, metrics);
        return;
    }
    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_color(color.to_color())
        .set_stroke_width(CANVAS_SELECTION_STROKE_WIDTH);
    canvas.draw_line(
        (outline.left, outline.top),
        (outline.right, outline.top),
        &paint,
    );
    canvas.draw_line(
        (outline.left, outline.bottom),
        (outline.right, outline.bottom),
        &paint,
    );
    canvas.draw_line(
        (outline.left, outline.top),
        (outline.left, outline.bottom),
        &paint,
    );
    canvas.draw_line(
        (outline.right, outline.top),
        (outline.right, outline.bottom),
        &paint,
    );
}

fn canvas_selection_is_visible(state: &Editor) -> bool {
    state.move_lift_active() || !state.selection.is_collapsed()
}

fn render_marching_ants(
    canvas: &Canvas,
    outline: CanvasSelectionOutline,
    primary: Rgba,
    alternate: Rgba,
    metrics: &CellMetrics,
) {
    let segment = (metrics.cell_width.min(metrics.cell_height) / 3.0).max(2.0);
    let phase = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as usize
        / 125;
    let mut paints = [Paint::default(), Paint::default()];
    for (paint, color) in paints.iter_mut().zip([primary, alternate]) {
        paint
            .set_anti_alias(false)
            .set_color(color.to_color())
            .set_stroke_width(CANVAS_SELECTION_STROKE_WIDTH);
    }
    render_marching_edge(
        canvas,
        (outline.left, outline.top),
        (outline.right, outline.top),
        segment,
        phase,
        &paints,
    );
    render_marching_edge(
        canvas,
        (outline.right, outline.top),
        (outline.right, outline.bottom),
        segment,
        phase,
        &paints,
    );
    render_marching_edge(
        canvas,
        (outline.right, outline.bottom),
        (outline.left, outline.bottom),
        segment,
        phase,
        &paints,
    );
    render_marching_edge(
        canvas,
        (outline.left, outline.bottom),
        (outline.left, outline.top),
        segment,
        phase,
        &paints,
    );
}

fn render_marching_edge(
    canvas: &Canvas,
    start: (f32, f32),
    end: (f32, f32),
    segment: f32,
    phase: usize,
    paints: &[Paint; 2],
) {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let length = dx.abs() + dy.abs();
    if length == 0.0 {
        return;
    }
    let unit = (dx / length, dy / length);
    let offset = (phase as f32 % segment) - segment;
    let mut position = offset;
    let mut index = phase / segment as usize;
    while position < length {
        let from = position.max(0.0);
        let to = (position + segment).min(length);
        if to > from {
            canvas.draw_line(
                (start.0 + unit.0 * from, start.1 + unit.1 * from),
                (start.0 + unit.0 * to, start.1 + unit.1 * to),
                &paints[index % paints.len()],
            );
        }
        position += segment;
        index += 1;
    }
}

fn render_toolbar(
    canvas: &Canvas,
    state: &Editor,
    metrics: &CellMetrics,
    top_padding: f32,
    width: usize,
    hotspot_hovered: bool,
) {
    let max_columns =
        (width.saturating_sub(PADDING * 2) as f32 / metrics.cell_width.max(1.0)) as usize;
    let mut rows = vec![(0, crate::toolbar::toolbar_border_spans(max_columns, true))];
    for row in 0..state.toolbar.content_rows_for_width(max_columns) {
        let physical_row = crate::toolbar::toolbar_content_row(row);
        rows.push((
            physical_row,
            crate::toolbar::boxed_toolbar_spans(
                &state.toolbar_spans_for_width(row, max_columns),
                max_columns,
            ),
        ));
    }

    rows.push((
        state.toolbar.rows_for_width(max_columns) - 1,
        crate::toolbar::toolbar_minimap_border_spans(
            max_columns,
            crate::layout::minimap_width_in_cells(max_columns),
            state.cursor_coordinates(),
        ),
    ));

    for (row, spans) in &rows {
        render_toolbar_span_contents(
            canvas,
            *row,
            spans,
            state,
            max_columns,
            metrics,
            top_padding,
        );
    }
    for (row, spans) in &rows {
        render_toolbar_span_outlines(canvas, *row, spans, state, metrics, top_padding);
    }
    render_toolbar_hotspot(canvas, hotspot_hovered, max_columns, metrics, top_padding);
}

fn render_toolbar_hotspot(
    canvas: &Canvas,
    hovered: bool,
    box_width: usize,
    metrics: &CellMetrics,
    top_padding: f32,
) {
    if !hovered || box_width < 2 {
        return;
    }
    let mut paint = Paint::default();
    paint.set_anti_alias(false);
    paint.set_color(Rgba::rgb(0, 0, 0).to_color());
    canvas.draw_rect(
        Rect::from_xywh(
            PADDING as f32 + (box_width - 1) as f32 * metrics.cell_width.max(1.0),
            top_padding,
            metrics.cell_width.max(1.0),
            metrics.cell_height,
        ),
        &paint,
    );
}

fn render_bottom_tooltip(
    canvas: &Canvas,
    state: &Editor,
    metrics: &CellMetrics,
    layout: LayoutMetrics,
    width: usize,
) {
    if !layout.tooltip_visible {
        return;
    }
    let max_columns =
        (width.saturating_sub(PADDING * 2) as f32 / metrics.cell_width.max(1.0)) as usize;
    let spans = crate::toolbar::tooltip_spans(state.tooltip(), max_columns);
    render_toolbar_span_contents(
        canvas,
        0,
        &spans,
        state,
        max_columns,
        metrics,
        layout.tooltip_top,
    );
}

fn render_toolbar_span_contents(
    canvas: &Canvas,
    row: usize,
    spans: &[crate::toolbar::ToolbarSpan],
    state: &Editor,
    max_columns: usize,
    metrics: &CellMetrics,
    top_padding: f32,
) {
    let atoms = toolbar_atoms(spans, state);
    render_line(
        canvas,
        row,
        &atoms,
        &state.grid.default_face,
        0..max_columns,
        metrics,
        DrawOrigin::Grid {
            top_padding: top_padding + crate::toolbar::toolbar_row_offset(row, metrics.cell_height),
        },
    );
}

fn render_toolbar_span_outlines(
    canvas: &Canvas,
    row: usize,
    spans: &[crate::toolbar::ToolbarSpan],
    state: &Editor,
    metrics: &CellMetrics,
    top_padding: f32,
) {
    for outline in toolbar_span_outlines(row, spans, state, metrics, top_padding) {
        let mut paint = Paint::default();
        paint
            .set_anti_alias(false)
            .set_color(outline.color.to_color())
            .set_stroke_width(TOOLBAR_SELECTION_STROKE_WIDTH);
        canvas.draw_line(
            (outline.left, outline.top),
            (outline.right, outline.top),
            &paint,
        );
        canvas.draw_line(
            (outline.left, outline.bottom),
            (outline.right, outline.bottom),
            &paint,
        );
        canvas.draw_line(
            (outline.left, outline.top),
            (outline.left, outline.bottom),
            &paint,
        );
        canvas.draw_line(
            (outline.right, outline.top),
            (outline.right, outline.bottom),
            &paint,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ToolbarSpanOutline {
    color: Rgba,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn toolbar_span_outlines(
    row: usize,
    spans: &[crate::toolbar::ToolbarSpan],
    state: &Editor,
    metrics: &CellMetrics,
    top_padding: f32,
) -> Vec<ToolbarSpanOutline> {
    let top = row_top(row, metrics, top_padding)
        + crate::toolbar::toolbar_row_offset(row, metrics.cell_height)
        - TOOLBAR_SELECTION_PADDING;
    let bottom = top + metrics.cell_height - 1.0 + TOOLBAR_SELECTION_PADDING * 2.0;
    let mut outlines = Vec::new();
    let mut column = 0;
    for span in spans {
        let span_width = UnicodeWidthStr::width(span.contents.as_str());
        if let Some(color) = toolbar_span_outline_color(state, span)
            && span_width > 0
        {
            let left =
                PADDING as f32 + column as f32 * metrics.cell_width - TOOLBAR_SELECTION_PADDING;
            let right = left + span_width as f32 * metrics.cell_width - 1.0
                + TOOLBAR_SELECTION_PADDING * 2.0;
            outlines.push(ToolbarSpanOutline {
                color,
                left,
                top,
                right,
                bottom,
            });
        }
        column += span_width;
    }
    outlines
}

fn toolbar_span_outline_color(state: &Editor, span: &crate::toolbar::ToolbarSpan) -> Option<Rgba> {
    let face = if span.highlighted {
        &state.theme.selection_highlight
    } else if span.selected
        && matches!(
            span.action,
            Some(crate::toolbar::ToolbarAction::SelectColor(_))
        )
    {
        &state.theme.color_selection
    } else if span.selected {
        &state.theme.selection
    } else {
        return None;
    };
    Some(resolve_derived_face(&state.grid.default_face, face, FALLBACK_FG, FALLBACK_BG).fg)
}

fn visible_grid_layout(
    mut layout: LayoutMetrics,
    metrics: &CellMetrics,
    viewport: ViewportOffset,
) -> LayoutMetrics {
    layout.cols = layout
        .cols
        .saturating_add(hidden_leading_cells(viewport.x, metrics.cell_width));
    layout.rows = layout
        .rows
        .saturating_add(hidden_leading_cells(viewport.y, metrics.cell_height));
    layout
}

fn hidden_leading_cells(offset: i64, cell_size: f32) -> usize {
    if offset >= 0 {
        return 0;
    }
    ((offset.saturating_abs() as f64 / cell_size.max(1.0) as f64).floor() as usize)
        .saturating_add(2)
}

fn render_window_title(
    canvas: &Canvas,
    title: &str,
    default_face: &Face,
    metrics: &CellMetrics,
    layout: LayoutMetrics,
    window_width: usize,
    transparent_menubar: bool,
) {
    if !transparent_menubar || layout.top_padding <= PADDING as f32 || title.is_empty() {
        return;
    }

    let max_columns =
        (window_width.saturating_sub(PADDING * 2) as f32 / metrics.cell_width.max(1.0)) as usize;
    let title = truncate_title(title, max_columns);
    if title.is_empty() {
        return;
    }

    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(
        resolve_root_face(default_face, FALLBACK_FG, FALLBACK_BG)
            .fg
            .to_color(),
    );

    let text_width = metrics.font.measure_str(&title, Some(&paint)).0;
    let left = ((window_width as f32 - text_width) / 2.0).max(PADDING as f32);
    let top = (layout.top_padding - metrics.cell_height).max(0.0) / 2.0;
    let baseline = top + metrics.baseline_offset;
    canvas.draw_str(title, (left, baseline), &metrics.font, &paint);
}

fn render_line(
    canvas: &Canvas,
    row: usize,
    line: &[Atom],
    default_face: &Face,
    columns: std::ops::Range<usize>,
    metrics: &CellMetrics,
    origin: DrawOrigin,
) {
    render_line_at(
        canvas,
        LineRenderPosition {
            row,
            first_column: columns.start,
            max_column: columns.end,
        },
        line,
        default_face,
        metrics,
        origin,
        false,
    );
}

fn render_overlay_line(
    canvas: &Canvas,
    row: usize,
    line: &[Atom],
    default_face: &Face,
    columns: std::ops::Range<usize>,
    metrics: &CellMetrics,
    origin: DrawOrigin,
) {
    render_line_at(
        canvas,
        LineRenderPosition {
            row,
            first_column: columns.start,
            max_column: columns.end,
        },
        line,
        default_face,
        metrics,
        origin,
        true,
    );
}

fn render_line_at(
    canvas: &Canvas,
    position: LineRenderPosition,
    line: &[Atom],
    default_face: &Face,
    metrics: &CellMetrics,
    origin: DrawOrigin,
    transparent_default_background: bool,
) {
    let top = line_top(origin, position.row, metrics);
    let root_face = resolve_root_face(default_face, FALLBACK_FG, FALLBACK_BG);
    let mut column = 0usize;
    let mut bg_paint = Paint::default();
    bg_paint.set_anti_alias(false);
    let mut fg_paint = Paint::default();
    fg_paint.set_anti_alias(true);

    for atom in line {
        let full_width = atom_display_width(&atom.contents);
        if full_width == 0 {
            continue;
        }

        let atom_start = column;
        let atom_end = atom_start.saturating_add(full_width);
        column = atom_end;
        if atom_end <= position.first_column {
            continue;
        }
        if atom_start >= position.max_column {
            return;
        }
        let visible_start = atom_start.max(position.first_column);
        let visible_end = atom_end.min(position.max_column);
        let visible_width = visible_end.saturating_sub(visible_start);
        let resolved = if face_is_default(&atom.face) {
            root_face
        } else {
            resolve_derived_face(default_face, &atom.face, FALLBACK_FG, FALLBACK_BG)
        };
        bg_paint.set_color(resolved.bg.to_color());
        fg_paint.set_color(resolved.fg.to_color());
        let is_plain_blank = atom.contents.bytes().all(|byte| byte == b' ')
            && resolved.bg == root_face.bg
            && resolved.underline_style.is_none()
            && !resolved.strikethrough;
        let paints_background = !transparent_default_background
            || atom.face.bg != "default"
            || atom.face.attributes.iter().any(|attribute| {
                attribute
                    .trim_start_matches("final:")
                    .eq_ignore_ascii_case("reverse")
            });
        if !is_plain_blank && paints_background {
            fill_cells(
                canvas,
                visible_start,
                top,
                visible_width,
                metrics,
                &bg_paint,
            );
        }
        let font = (!is_plain_blank).then(|| font_for_face(metrics, &resolved));

        let mut cluster_column = atom_start;
        for cluster in text_clusters(&atom.contents) {
            if cluster == "\n" {
                continue;
            }

            let span = cluster_display_width(cluster);
            let cluster_end = cluster_column.saturating_add(span);
            if cluster_end > position.first_column
                && cluster_column < position.max_column
                && let Some(font) = font.as_ref()
            {
                draw_text_cluster(
                    canvas,
                    cluster_column,
                    top,
                    cluster,
                    font,
                    metrics,
                    &fg_paint,
                );
            }
            cluster_column = cluster_end;
        }
        if !is_plain_blank {
            draw_text_decorations(
                canvas,
                visible_start,
                top,
                visible_width,
                metrics,
                &resolved,
            );
        }
    }
}

fn render_grid_cursor(
    canvas: &Canvas,
    state: &Editor,
    rendered_lines: &[Vec<Atom>],
    cursor_shape_config: &CursorShapeConfig,
    layout: LayoutMetrics,
    metrics: &CellMetrics,
) {
    let grid = &state.grid;
    let cursor_mode = state.cursor_mode;
    let cols = layout.cols;
    let rows = layout.rows;
    if cols == 0 || rows == 0 {
        return;
    }

    let cursor = grid.cursor_pos;
    if cursor.line >= rows || cursor.column >= cols {
        return;
    }

    let cell = cursor_cell(
        rendered_lines.get(cursor.line).map(Vec::as_slice),
        cursor.column,
    )
    .unwrap_or_else(|| CursorCell {
        face: grid.default_face.clone(),
        text: Some(" ".to_string()),
    });

    let cell_resolved =
        resolve_derived_face(&grid.default_face, &cell.face, FALLBACK_FG, FALLBACK_BG);
    let cursor_resolved = resolve_derived_face(
        &grid.default_face,
        &grid.cursor_face,
        FALLBACK_FG,
        FALLBACK_BG,
    );
    let top = row_top(cursor.line, metrics, layout.grid_top);
    if is_drawing_mode(cursor_mode) {
        let drawing_cursor = resolve_derived_face(
            &grid.default_face,
            &state.theme.cursor_drawing,
            FALLBACK_FG,
            FALLBACK_BG,
        );
        render_hollow_drawing_cursor(
            canvas,
            cursor.column,
            top,
            &cell,
            metrics,
            &cell_resolved,
            &drawing_cursor,
        );
        return;
    }
    match cursor_shape_for_mode(cursor_shape_config, cursor_mode) {
        CursorShape::Block => {
            render_block_cursor(canvas, cursor.column, top, &cell, metrics, &cursor_resolved)
        }
        CursorShape::Beam => render_beam_cursor(
            canvas,
            cursor.column,
            top,
            &cell,
            metrics,
            &cell_resolved,
            &cursor_resolved,
        ),
        CursorShape::Underline => render_underline_cursor(
            canvas,
            cursor.column,
            top,
            &cell,
            metrics,
            &cell_resolved,
            &cursor_resolved,
        ),
    }
}

fn grid_cursor_is_visible(state: &Editor) -> bool {
    !state.view_active() && !state.jump_active()
}

fn is_drawing_mode(mode: CursorMode) -> bool {
    matches!(
        mode,
        CursorMode::MoveDraw | CursorMode::Stamp | CursorMode::Shapes | CursorMode::Utilities
    )
}

fn render_hollow_drawing_cursor(
    canvas: &Canvas,
    column: usize,
    top: f32,
    cell: &CursorCell,
    metrics: &CellMetrics,
    cell_resolved: &ResolvedFace,
    cursor_resolved: &ResolvedFace,
) {
    let stroke_width = (metrics.cell_height * DRAWING_CURSOR_WIDTH_RATIO)
        .round()
        .max(1.0);
    render_cursor_base_cell(canvas, column, top, cell, metrics, cell_resolved);

    let outline = drawing_cursor_outline(column, top, metrics);
    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_style(skia_safe::paint::Style::Stroke)
        .set_stroke_join(skia_safe::paint::Join::Miter)
        .set_color(cursor_resolved.fg.to_color())
        .set_stroke_width(stroke_width);

    canvas.draw_rect(
        Rect::new(outline.left, outline.top, outline.right, outline.bottom),
        &paint,
    );
}

fn drawing_cursor_outline(
    column: usize,
    top: f32,
    metrics: &CellMetrics,
) -> CanvasSelectionOutline {
    let inset =
        (metrics.cell_width.min(metrics.cell_height) * DRAWING_CURSOR_INSET_RATIO).clamp(1.0, 4.0);
    CanvasSelectionOutline {
        left: PADDING as f32 + column as f32 * metrics.cell_width + inset,
        top: top + inset,
        right: PADDING as f32 + (column + 1) as f32 * metrics.cell_width - 1.0 - inset,
        bottom: top + metrics.cell_height - 1.0 - inset,
    }
}

fn render_block_cursor(
    canvas: &Canvas,
    column: usize,
    top: f32,
    cell: &CursorCell,
    metrics: &CellMetrics,
    resolved: &ResolvedFace,
) {
    let mut bg_paint = Paint::default();
    bg_paint
        .set_anti_alias(false)
        .set_color(resolved.bg.to_color());
    fill_cells(canvas, column, top, 1, metrics, &bg_paint);

    let mut fg_paint = Paint::default();
    fg_paint
        .set_anti_alias(true)
        .set_color(resolved.fg.to_color());
    let font = font_for_face(metrics, resolved);
    if let Some(text) = &cell.text {
        draw_text_cluster(canvas, column, top, text, &font, metrics, &fg_paint);
    }
    draw_text_decorations(canvas, column, top, 1, metrics, resolved);
}

fn render_beam_cursor(
    canvas: &Canvas,
    column: usize,
    top: f32,
    cell: &CursorCell,
    metrics: &CellMetrics,
    base_resolved: &ResolvedFace,
    resolved: &ResolvedFace,
) {
    render_cursor_base_cell(canvas, column, top, cell, metrics, base_resolved);
    let width = (metrics.cell_width * CURSOR_BEAM_WIDTH_RATIO)
        .round()
        .clamp(1.0, metrics.cell_width);
    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_color(cursor_indicator_color(CursorShape::Beam, resolved).to_color());
    fill_rect_pixels(
        canvas,
        PADDING as f32 + column as f32 * metrics.cell_width,
        top,
        width,
        metrics.cell_height,
        &paint,
    );
}

fn render_underline_cursor(
    canvas: &Canvas,
    column: usize,
    top: f32,
    cell: &CursorCell,
    metrics: &CellMetrics,
    base_resolved: &ResolvedFace,
    resolved: &ResolvedFace,
) {
    render_cursor_base_cell(canvas, column, top, cell, metrics, base_resolved);
    let height = (metrics.cell_height * CURSOR_UNDERLINE_HEIGHT_RATIO)
        .round()
        .clamp(1.0, metrics.cell_height);
    let max_top = top + metrics.cell_height - height;
    let y = underline_start_y(top, metrics, height)
        .min(max_top)
        .max(top);
    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_color(cursor_indicator_color(CursorShape::Underline, resolved).to_color());
    fill_rect_pixels(
        canvas,
        PADDING as f32 + column as f32 * metrics.cell_width,
        y,
        metrics.cell_width,
        height,
        &paint,
    );
}

fn cursor_shape_for_mode(config: &CursorShapeConfig, mode: CursorMode) -> CursorShape {
    match mode {
        CursorMode::MoveDraw => config.move_draw.unwrap_or(CursorShape::Block),
        CursorMode::Insert => config.insert.unwrap_or(CursorShape::Block),
        CursorMode::Replace => config.replace.unwrap_or(CursorShape::Block),
        CursorMode::Stamp | CursorMode::Shapes | CursorMode::Utilities => {
            config.move_draw.unwrap_or(CursorShape::Block)
        }
        CursorMode::Text | CursorMode::Navigation => config.insert.unwrap_or(CursorShape::Block),
    }
}

fn cursor_indicator_color(shape: CursorShape, resolved: &ResolvedFace) -> Rgba {
    match shape {
        CursorShape::Block => resolved.bg,
        CursorShape::Beam | CursorShape::Underline => resolved.bg,
    }
}

fn render_cursor_base_cell(
    canvas: &Canvas,
    column: usize,
    top: f32,
    cell: &CursorCell,
    metrics: &CellMetrics,
    resolved: &ResolvedFace,
) {
    let mut bg_paint = Paint::default();
    bg_paint
        .set_anti_alias(false)
        .set_color(resolved.bg.to_color());
    fill_cells(canvas, column, top, 1, metrics, &bg_paint);

    let mut fg_paint = Paint::default();
    fg_paint
        .set_anti_alias(true)
        .set_color(resolved.fg.to_color());
    let font = font_for_face(metrics, resolved);
    if let Some(text) = &cell.text {
        draw_text_cluster(canvas, column, top, text, &font, metrics, &fg_paint);
    }
    draw_text_decorations(canvas, column, top, 1, metrics, resolved);
}

fn cursor_cell(line: Option<&[Atom]>, target_column: usize) -> Option<CursorCell> {
    let line = line?;
    let mut column = 0;

    for atom in line {
        for cluster in text_clusters(&atom.contents) {
            if cluster == "\n" {
                if column == target_column {
                    return Some(CursorCell {
                        face: atom.face.clone(),
                        text: Some(" ".to_string()),
                    });
                }
                continue;
            }

            let span = cluster_display_width(cluster);
            if target_column >= column && target_column < column + span {
                return Some(CursorCell {
                    face: atom.face.clone(),
                    text: Some(cluster.to_string()),
                });
            }
            column += span;
        }
    }

    None
}

pub fn atom_display_width(contents: &str) -> usize {
    text_clusters(contents)
        .filter(|cluster| *cluster != "\n")
        .map(cluster_display_width)
        .sum()
}

fn truncate_atoms(line: &[Atom], max_width: usize) -> Vec<Atom> {
    let mut remaining = max_width;
    let mut result = Vec::new();

    for atom in line {
        if remaining == 0 {
            break;
        }

        let mut contents = String::new();
        for cluster in text_clusters(&atom.contents) {
            if cluster == "\n" {
                continue;
            }
            let width = cluster_display_width(cluster);
            if width > remaining {
                break;
            }
            contents.push_str(cluster);
            remaining -= width;
        }

        if !contents.is_empty() {
            result.push(Atom {
                face: atom.face.clone(),
                contents,
            });
        }
    }

    result
}

fn truncate_title(title: &str, max_width: usize) -> String {
    let atoms = [Atom {
        face: Face::default(),
        contents: title.to_string(),
    }];
    truncate_atoms(&atoms, max_width)
        .into_iter()
        .map(|atom| atom.contents)
        .collect()
}

fn row_top(row: usize, metrics: &CellMetrics, top_padding: f32) -> f32 {
    top_padding + row as f32 * metrics.cell_height
}

fn line_top(origin: DrawOrigin, row: usize, metrics: &CellMetrics) -> f32 {
    match origin {
        DrawOrigin::Grid { top_padding } => row_top(row, metrics, top_padding),
    }
}

fn fill_cells(
    canvas: &Canvas,
    column: usize,
    top: f32,
    width_in_cells: usize,
    metrics: &CellMetrics,
    paint: &Paint,
) {
    let left = PADDING as f32 + column as f32 * metrics.cell_width;
    let rect = Rect::from_xywh(
        left,
        top,
        metrics.cell_width * width_in_cells as f32,
        metrics.cell_height,
    );
    canvas.draw_rect(rect, paint);
}

fn fill_rect_pixels(canvas: &Canvas, left: f32, top: f32, width: f32, height: f32, paint: &Paint) {
    let rect = Rect::from_xywh(left, top, width, height);
    canvas.draw_rect(rect, paint);
}

fn draw_text_cluster(
    canvas: &Canvas,
    column: usize,
    top: f32,
    text: &str,
    font: &Font,
    metrics: &CellMetrics,
    paint: &Paint,
) {
    if text.chars().all(char::is_control) {
        return;
    }
    if cell_graphics::draw(canvas, column, top, text, metrics, paint) {
        return;
    }
    let left = PADDING as f32 + column as f32 * metrics.cell_width;
    let baseline = top + metrics.baseline_offset;
    let font = font_for_text(metrics, font, text);
    canvas.draw_str(text, (left, baseline), &font, paint);
}

fn text_clusters(text: &str) -> impl Iterator<Item = &str> {
    UnicodeSegmentation::graphemes(text, true)
}

fn cluster_display_width(cluster: &str) -> usize {
    UnicodeWidthStr::width(cluster).max(usize::from(!cluster.is_empty()))
}

fn font_for_text(metrics: &CellMetrics, font: &Font, text: &str) -> Font {
    if text.is_ascii() {
        return font.clone();
    }
    if !prefers_fallback_font(text) && typeface_supports_text(&font.typeface(), text) {
        return font.clone();
    }

    let Some(character) = fallback_character(text) else {
        return font.clone();
    };
    let key = FallbackFontKey {
        character: character as u32,
        size_bits: font.size().to_bits(),
        bold: font.is_embolden(),
        italic: font.skew_x() != 0.0,
    };

    if !metrics.fallback_fonts.borrow().contains_key(&key)
        && let Some(typeface) = metrics.font_mgr.match_family_style_character(
            "",
            FontStyle::normal(),
            &[],
            character as i32,
        )
    {
        let mut fallback = Font::new(typeface, font.size());
        fallback
            .set_subpixel(font.is_subpixel())
            .set_edging(font.edging())
            .set_hinting(font.hinting())
            .set_baseline_snap(font.is_baseline_snap())
            .set_linear_metrics(font.is_linear_metrics())
            .set_embolden(font.is_embolden())
            .set_skew_x(font.skew_x());
        metrics.fallback_fonts.borrow_mut().insert(key, fallback);
    }

    metrics
        .fallback_fonts
        .borrow()
        .get(&key)
        .cloned()
        .unwrap_or_else(|| font.clone())
}

fn face_is_default(face: &Face) -> bool {
    face.fg == "default"
        && face.bg == "default"
        && face.underline == "default"
        && face.attributes.is_empty()
}

fn typeface_supports_text(typeface: &skia_safe::Typeface, text: &str) -> bool {
    let mut glyphs = vec![0; text.chars().count().max(1)];
    let count = typeface.str_to_glyphs(text, &mut glyphs);
    count > 0 && glyphs.into_iter().take(count).all(|glyph| glyph != 0)
}

fn fallback_character(text: &str) -> Option<char> {
    text.chars()
        .find(|&ch| ch != '\u{200d}' && !('\u{fe00}'..='\u{fe0f}').contains(&ch))
}

fn prefers_fallback_font(text: &str) -> bool {
    text.chars()
        .any(|ch| ch == '\u{200d}' || ('\u{fe00}'..='\u{fe0f}').contains(&ch) || is_emojiish(ch))
}

fn is_emojiish(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1f000..=0x1faff | 0x2600..=0x27bf
    )
}

fn font_for_face(metrics: &CellMetrics, face: &ResolvedFace) -> Font {
    let mut font = metrics.font.clone();
    font.set_embolden(face.bold);
    font.set_skew_x(if face.italic { -0.2 } else { 0.0 });
    font
}

fn draw_text_decorations(
    canvas: &Canvas,
    column: usize,
    top: f32,
    width_in_cells: usize,
    metrics: &CellMetrics,
    face: &ResolvedFace,
) {
    if width_in_cells == 0 {
        return;
    }

    let left = PADDING as f32 + column as f32 * metrics.cell_width;
    let width = metrics.cell_width * width_in_cells as f32;
    let stroke_width = (metrics.cell_height / 14.0).max(1.0);
    let decoration_color = face.underline.unwrap_or(face.fg).to_color();

    if let Some(style) = face.underline_style {
        let mut paint = Paint::default();
        paint
            .set_anti_alias(true)
            .set_color(decoration_color)
            .set_stroke_width(stroke_width);

        match style {
            UnderlineStyle::Straight => {
                let y = underline_start_y(top, metrics, stroke_width);
                canvas.draw_line((left, y), (left + width, y), &paint);
            }
            UnderlineStyle::Double => {
                let y = underline_start_y(top, metrics, stroke_width);
                let gap = stroke_width + 1.0;
                canvas.draw_line((left, y), (left + width, y), &paint);
                canvas.draw_line((left, y + gap), (left + width, y + gap), &paint);
            }
            UnderlineStyle::Curly => {
                let y = underline_start_y(top, metrics, stroke_width);
                let wave = (metrics.cell_height / 8.0).max(1.5);
                let mut x = left;
                let end = left + width;
                let step = (metrics.cell_width / 2.0).max(2.0);
                let mut up = true;
                while x < end {
                    let next = (x + step).min(end);
                    let next_y = if up { y - wave } else { y + wave };
                    canvas.draw_line((x, y), (next, next_y), &paint);
                    x = next;
                    up = !up;
                }
            }
        }
    }

    if face.strikethrough {
        let mut paint = Paint::default();
        paint
            .set_anti_alias(true)
            .set_color(face.fg.to_color())
            .set_stroke_width(stroke_width);
        let y = top + metrics.cell_height * 0.55;
        canvas.draw_line((left, y), (left + width, y), &paint);
    }
}

fn underline_start_y(top: f32, metrics: &CellMetrics, stroke_width: f32) -> f32 {
    top + metrics.baseline_offset + stroke_width + metrics.underline_offset
}

pub fn load_renderer(config: &AppConfig) -> Renderer {
    Renderer {
        font_mgr: FontMgr::new(),
        preferred_font_family: config.font_family.clone(),
        default_logical_font_size: config.font_size,
        underline_offset: config.cell.underline_offset,
        logical_font_size: Cell::new(config.font_size),
        content_metrics_cache: RefCell::new(None),
        fixed_metrics_cache: RefCell::new(None),
    }
}

impl Renderer {
    pub fn apply_config(&mut self, config: &AppConfig) {
        let font_size_delta = self.logical_font_size.get() - self.default_logical_font_size;
        self.preferred_font_family = config.font_family.clone();
        self.default_logical_font_size = config.font_size;
        self.underline_offset = config.cell.underline_offset;
        self.logical_font_size
            .set((config.font_size + font_size_delta).max(6.0));
        self.content_metrics_cache.borrow_mut().take();
        self.fixed_metrics_cache.borrow_mut().take();
    }

    pub fn adjust_font_size(&self, delta: f32) -> bool {
        const MIN_FONT_SIZE: f32 = 6.0;

        let next = (self.logical_font_size.get() + delta).max(MIN_FONT_SIZE);
        if (next - self.logical_font_size.get()).abs() < f32::EPSILON {
            return false;
        }

        self.logical_font_size.set(next);
        self.content_metrics_cache.borrow_mut().take();
        true
    }

    pub fn reset_font_size(&self) -> bool {
        if (self.logical_font_size.get() - self.default_logical_font_size).abs() < f32::EPSILON {
            return false;
        }

        self.logical_font_size.set(self.default_logical_font_size);
        self.content_metrics_cache.borrow_mut().take();
        true
    }

    pub fn metrics(&self, scale_factor: f64) -> CellMetrics {
        self.metrics_for_logical_font_size(
            scale_factor,
            self.logical_font_size.get(),
            &self.content_metrics_cache,
        )
    }

    pub fn title_metrics(&self, scale_factor: f64) -> CellMetrics {
        self.metrics_for_logical_font_size(
            scale_factor,
            self.default_logical_font_size,
            &self.fixed_metrics_cache,
        )
    }

    fn metrics_for_logical_font_size(
        &self,
        scale_factor: f64,
        logical_font_size: f32,
        cache: &RefCell<Option<(u64, CellMetrics)>>,
    ) -> CellMetrics {
        let cache_key = scale_factor.to_bits();
        if let Some((cached_key, metrics)) = cache.borrow().as_ref()
            && *cached_key == cache_key
        {
            return metrics.clone();
        }

        let physical_font_size = (logical_font_size as f64 * scale_factor) as f32;
        let typeface = preferred_typeface(&self.font_mgr, &self.preferred_font_family)
            .unwrap_or_else(|| {
                self.font_mgr
                    .legacy_make_typeface(None, FontStyle::normal())
                    .expect("expected a fallback system typeface")
            });

        let mut font = Font::new(typeface, physical_font_size);
        font.set_subpixel(true)
            .set_edging(Edging::SubpixelAntiAlias)
            .set_hinting(FontHinting::Full)
            .set_baseline_snap(false)
            .set_linear_metrics(false);

        let (_, metrics) = font.metrics();
        let cell_width = font.measure_str("M", None).0.max(1.0);
        let cell_height = (metrics.descent - metrics.ascent + metrics.leading).max(1.0);
        let baseline_offset = -metrics.ascent;

        let metrics = CellMetrics {
            font,
            cell_width,
            cell_height,
            baseline_offset,
            underline_offset: self.underline_offset,
            font_mgr: self.font_mgr.clone(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        cache.borrow_mut().replace((cache_key, metrics.clone()));
        metrics
    }
}

fn preferred_typeface(font_mgr: &FontMgr, configured_family: &str) -> Option<skia_safe::Typeface> {
    font_mgr
        .match_family_style(configured_family, FontStyle::normal())
        .or_else(|| {
            matching_font_family(configured_family, font_mgr.family_names())
                .and_then(|family| font_mgr.match_family_style(family, FontStyle::normal()))
        })
        .or_else(|| {
            [
                "SF Mono",
                "Menlo",
                "Monaco",
                "JetBrains Mono",
                "Courier New",
            ]
            .iter()
            .find_map(|family| font_mgr.match_family_style(family, FontStyle::normal()))
        })
}

fn matching_font_family(
    configured_family: &str,
    available_families: impl IntoIterator<Item = String>,
) -> Option<String> {
    let configured = normalize_font_family(configured_family);
    if configured.is_empty() {
        return None;
    }
    available_families
        .into_iter()
        .find(|family| normalize_font_family(family) == configured)
}

fn normalize_font_family(family: &str) -> String {
    family
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

unsafe fn buffer_as_u8_mut(buffer: &mut [u32]) -> &mut [u8] {
    unsafe {
        std::slice::from_raw_parts_mut(
            buffer.as_mut_ptr() as *mut u8,
            std::mem::size_of_val(buffer),
        )
    }
}

pub fn resize_surface(
    surface: &mut Surface<Rc<Window>, Rc<Window>>,
    size: winit::dpi::PhysicalSize<u32>,
) -> Result<()> {
    let width = NonZeroU32::new(size.width.max(1)).expect("width is non-zero");
    let height = NonZeroU32::new(size.height.max(1)).expect("height is non-zero");
    surface
        .resize(width, height)
        .map_err(|error| anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::app::{AppConfig, CursorMode, CursorShape, ThemeConfig};
    use crate::editor::Editor;
    use crate::layout::TOOLTIP_BOTTOM_PAD;
    use crate::model::{ColorId, Coord, Direction, LayerId};
    use crate::toolbar::{MainMode, ToggleKind, ToolbarAction};
    use winit::keyboard::{Key, ModifiersState};

    use super::*;

    #[test]
    fn title_truncation_limits_display_width() {
        let title = truncate_title("ascdraw - /tmp/long/path.txt", 12);

        assert_eq!(title, "ascdraw - /t");
        assert_eq!(atom_display_width(&title), 12);
    }

    #[test]
    fn configured_font_family_matches_canonical_spacing_and_case() {
        let available = ["SF Mono", "JetBrainsMono Nerd Font", "Courier New"]
            .into_iter()
            .map(str::to_owned);

        assert_eq!(
            matching_font_family("Jetbrains Mono Nerd Font", available),
            Some("JetBrainsMono Nerd Font".to_owned())
        );
        assert_eq!(
            matching_font_family("---", ["SF Mono"].into_iter().map(str::to_owned)),
            None
        );
    }

    #[test]
    fn view_hides_grid_cursor_while_move_lift_keeps_it_visible() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state
            .selection
            .select(Coord::default(), Coord { line: 0, column: 1 });

        assert!(grid_cursor_is_visible(&state));
        assert!(state.begin_selected_move_lift());
        assert!(grid_cursor_is_visible(&state));
        assert!(state.cancel_move_lift());
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities,)));
        assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        }));
        assert!(!grid_cursor_is_visible(&state));
        assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 0,
        }));
        assert!(grid_cursor_is_visible(&state));
    }

    #[test]
    fn title_metrics_stay_at_default_size_when_content_font_changes() {
        let config = AppConfig {
            font_size: 12.0,
            ..AppConfig::default()
        };
        let renderer = load_renderer(&config);

        let original_title_metrics = renderer.title_metrics(1.0);
        assert!(renderer.adjust_font_size(4.0));

        let zoomed_metrics = renderer.metrics(1.0);
        let title_metrics = renderer.title_metrics(1.0);

        assert!(zoomed_metrics.cell_height >= original_title_metrics.cell_height);
        assert_eq!(
            title_metrics.cell_height,
            original_title_metrics.cell_height
        );
        assert_eq!(title_metrics.cell_width, original_title_metrics.cell_width);
    }

    #[test]
    fn canvas_zoom_stops_at_the_existing_minimum_font_size() {
        let config = AppConfig {
            font_size: 12.0,
            ..AppConfig::default()
        };
        let renderer = load_renderer(&config);

        assert!(renderer.adjust_font_size(-100.0));
        assert!(!renderer.adjust_font_size(-1.0));
        assert!(renderer.adjust_font_size(1.0));
    }

    #[test]
    fn parses_rgba_colors_by_ignoring_alpha() {
        let resolved = resolve_root_face(
            &Face {
                fg: "#ffffff80".into(),
                bg: "default".into(),
                underline: "default".into(),
                attributes: Vec::new(),
            },
            FALLBACK_FG,
            FALLBACK_BG,
        );
        assert_eq!(
            resolved.fg,
            Rgba {
                r: 0xff,
                g: 0xff,
                b: 0xff,
                a: 0x80
            }
        );
    }

    #[test]
    fn underline_offset_shifts_decoration_y_without_changing_other_metrics() {
        let base_metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.0,
            cell_height: 16.0,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let shifted_metrics = CellMetrics {
            underline_offset: 1.5,
            ..base_metrics.clone()
        };
        let stroke_width = (base_metrics.cell_height / 14.0).max(1.0);

        let base_y = underline_start_y(3.0, &base_metrics, stroke_width);
        let shifted_y = underline_start_y(3.0, &shifted_metrics, stroke_width);

        assert_eq!(shifted_y - base_y, 1.5);
        assert_eq!(shifted_metrics.cell_height, base_metrics.cell_height);
        assert_eq!(
            shifted_metrics.baseline_offset,
            base_metrics.baseline_offset
        );
    }

    #[test]
    fn cursor_cell_uses_visible_placeholder_at_end_of_line() {
        let cursor_face = Face {
            fg: "#000000".into(),
            bg: "#ffffff".into(),
            underline: "default".into(),
            attributes: Vec::new(),
        };
        let line = vec![Atom {
            face: cursor_face.clone(),
            contents: "\n".into(),
        }];

        let cursor = cursor_cell(Some(&line), 0).expect("cursor cell should exist");
        assert_eq!(cursor.face.bg, "#ffffff");
        assert_eq!(cursor.text, Some(" ".to_string()));
    }

    #[test]
    fn cursor_cell_finds_character_under_cursor() {
        let line = vec![
            Atom {
                face: Face::default(),
                contents: "ab".into(),
            },
            Atom {
                face: Face {
                    fg: "#000000".into(),
                    bg: "#ffffff".into(),
                    underline: "default".into(),
                    attributes: Vec::new(),
                },
                contents: "c".into(),
            },
        ];

        let cursor = cursor_cell(Some(&line), 2).expect("cursor cell should exist");
        assert_eq!(cursor.text, Some("c".to_string()));
        assert_eq!(cursor.face.bg, "#ffffff");
    }

    #[test]
    fn shape_preview_provides_the_cell_beneath_the_cursor() {
        let config = AppConfig::default();
        let mut state = Editor::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
        state.toggle_shape_preview();
        state.move_cursor(Direction::Right);

        let preview = state.lines_with_shape_preview().expect("preview is active");
        let cell = cursor_cell(
            preview.get(state.grid.cursor_pos.line).map(Vec::as_slice),
            state.grid.cursor_pos.column,
        )
        .expect("preview cell exists beneath the cursor");

        assert_eq!(cell.text, Some("┐".to_string()));
    }

    #[test]
    fn atom_display_width_counts_single_emoji_as_double_width() {
        assert_eq!(atom_display_width("😀"), 2);
    }

    #[test]
    fn atom_display_width_treats_emoji_variation_sequence_as_one_cluster() {
        assert_eq!(atom_display_width("❤️"), 2);
    }

    #[test]
    fn cursor_shape_uses_block_fallback_for_missing_modes() {
        let config = AppConfig::default();

        assert_eq!(
            cursor_shape_for_mode(&config.display.cursor_shape, CursorMode::Insert),
            CursorShape::Block
        );
    }

    #[test]
    fn drawing_modes_use_the_hollow_drawing_cursor() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            assert!(is_drawing_mode(mode));
        }
        for mode in [CursorMode::Insert, CursorMode::Replace, CursorMode::Text] {
            assert!(!is_drawing_mode(mode));
        }
    }

    #[test]
    fn canvas_selection_geometry_uses_inclusive_cell_bounds() {
        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.0,
            cell_height: 16.0,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let outline = canvas_selection_outline(
            SelectionBounds {
                left: 2,
                right: 4,
                top: 3,
                bottom: 5,
            },
            &metrics,
            100.0,
        );
        assert_eq!(
            outline,
            CanvasSelectionOutline {
                left: 36.0,
                top: 148.0,
                right: 59.0,
                bottom: 195.0,
            }
        );
    }

    #[test]
    fn canvas_rows_preserve_fractional_pitch_with_toolbar_row_spacing() {
        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.0,
            cell_height: 16.375,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };

        assert_eq!(
            row_top(1, &metrics, 100.0) - row_top(0, &metrics, 100.0),
            16.375
        );
        assert_eq!(
            row_top(2, &metrics, 100.0) - row_top(1, &metrics, 100.0),
            16.375
        );
    }

    #[test]
    fn active_corner_cursor_uses_the_same_fractional_grid_as_selection() {
        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.25,
            cell_height: 16.375,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let selection = canvas_selection_outline(
            SelectionBounds {
                left: 1,
                right: 2,
                top: 1,
                bottom: 2,
            },
            &metrics,
            50.0,
        );
        let cursor_top = row_top(2, &metrics, 50.0);
        let cursor = drawing_cursor_outline(2, cursor_top, &metrics);

        let cell_left = PADDING as f32 + 2.0 * metrics.cell_width;
        let cell_top = row_top(2, &metrics, 50.0);
        let inset = (metrics.cell_width.min(metrics.cell_height) * DRAWING_CURSOR_INSET_RATIO)
            .clamp(1.0, 4.0);
        assert_eq!(cursor.left - cell_left, inset);
        assert_eq!(cursor.top - cell_top, inset);
        assert_eq!(selection.right - cursor.right, inset);
        assert_eq!(selection.bottom - cursor.bottom, inset);
    }

    #[test]
    fn collapsed_selection_is_hidden_behind_the_grid_cursor() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        assert!(!canvas_selection_is_visible(&state));

        state
            .selection
            .select(Coord { line: 1, column: 2 }, Coord { line: 3, column: 4 });
        assert!(canvas_selection_is_visible(&state));
    }

    #[test]
    fn cursor_shape_uses_mode_specific_override_when_present() {
        let mut config = AppConfig::default();
        config.display.cursor_shape.insert = Some(CursorShape::Beam);
        config.display.cursor_shape.move_draw = Some(CursorShape::Underline);

        assert_eq!(
            cursor_shape_for_mode(&config.display.cursor_shape, CursorMode::Insert),
            CursorShape::Beam
        );
        assert_eq!(
            cursor_shape_for_mode(&config.display.cursor_shape, CursorMode::MoveDraw),
            CursorShape::Underline
        );
    }

    #[test]
    fn non_block_cursor_shapes_use_cursor_face_background_color() {
        let resolved = ResolvedFace {
            fg: Rgba::rgb(0xaa, 0xbb, 0xcc),
            bg: Rgba::rgb(0x11, 0x22, 0x33),
            underline: Some(Rgba::rgb(0xff, 0x00, 0x00)),
            underline_style: Some(UnderlineStyle::Straight),
            reverse: false,
            blink: false,
            bold: false,
            dim: false,
            italic: false,
            strikethrough: false,
        };

        assert_eq!(
            cursor_indicator_color(CursorShape::Beam, &resolved),
            resolved.bg
        );
        assert_eq!(
            cursor_indicator_color(CursorShape::Underline, &resolved),
            resolved.bg
        );
    }

    #[test]
    fn toolbar_selection_and_pending_prefix_use_theme_colors() {
        let config = AppConfig::default();
        let mut state = Editor::new(&config.theme, "test");
        let selected = state
            .toolbar
            .toolbar_spans(1)
            .into_iter()
            .find(|span| span.selected)
            .unwrap();
        assert_eq!(
            toolbar_span_outline_color(&state, &selected),
            Some(Rgba::rgb(0xff, 0x00, 0x00))
        );

        assert!(
            state
                .toolbar
                .handle_shortcut(&Key::Character("1".into()), ModifiersState::empty())
        );
    }

    #[test]
    fn color_menu_blocks_keep_their_explicit_palette_foregrounds() {
        let config = AppConfig::default();
        let mut state = Editor::new(&config.theme, "test");
        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        let spans = (crate::toolbar::MENU_FIRST_ROW + 1..crate::toolbar::MENU_FIRST_ROW + 3)
            .flat_map(|row| state.toolbar.toolbar_spans(row))
            .collect::<Vec<_>>();
        let palette_spans = spans
            .iter()
            .filter(|span| matches!(span.action, Some(ToolbarAction::SelectColor(_))))
            .cloned()
            .collect::<Vec<_>>();
        let atoms = toolbar_atoms(&palette_spans, &state);
        let blocks = atoms
            .iter()
            .filter(|atom| atom.contents == "■")
            .collect::<Vec<_>>();

        assert_eq!(blocks.len(), ColorId::COUNT);
        for (index, atom) in blocks.into_iter().enumerate() {
            assert_eq!(atom.face.fg, ColorId(index as u8).hex().unwrap());
        }

        let selected = palette_spans.iter().find(|span| span.selected).unwrap();
        let expected = resolve_derived_face(
            &state.grid.default_face,
            &config.theme.color_selection,
            FALLBACK_FG,
            FALLBACK_BG,
        )
        .fg;
        assert_eq!(toolbar_span_outline_color(&state, selected), Some(expected));
    }

    #[test]
    fn layer_state_glyphs_have_no_outline_while_the_pending_row_prefix_does() {
        let config = AppConfig::default();
        let mut state = Editor::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode,)));

        let row = crate::toolbar::MENU_FIRST_ROW + 1;
        let spans = state.toolbar_spans(row);
        let operation_spans = spans
            .iter()
            .filter(|span| matches!(span.action, Some(ToolbarAction::Layer { .. })))
            .collect::<Vec<_>>();
        assert!(operation_spans.iter().any(|span| span.contents == "×"));
        assert!(operation_spans.iter().any(|span| span.contents == "▪"));
        assert!(
            operation_spans
                .iter()
                .all(|span| toolbar_span_outline_color(&state, span).is_none())
        );

        assert!(state.apply_toolbar_action(ToolbarAction::BeginLayerPath(LayerId(0))));
        let spans = state.toolbar_spans(row);
        let prefix = spans
            .iter()
            .find(|span| span.action == Some(ToolbarAction::BeginLayerPath(LayerId(0))))
            .unwrap();
        assert!(prefix.highlighted);
        assert_eq!(
            toolbar_span_outline_color(&state, prefix),
            Some(
                resolve_derived_face(
                    &state.grid.default_face,
                    &config.theme.selection_highlight,
                    FALLBACK_FG,
                    FALLBACK_BG,
                )
                .fg
            )
        );
    }

    #[test]
    fn toolbar_hover_fill_appears_only_in_the_top_right_corner_cell() {
        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.0,
            cell_height: 16.0,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let columns = 20;
        let top = 7.0;
        let width = (PADDING as f32 * 2.0 + columns as f32 * metrics.cell_width) as usize;
        let height = (top + metrics.cell_height + 2.0) as usize;
        let hotspot_x = PADDING as f32 + (columns - 1) as f32 * metrics.cell_width;

        let render_pixels = |hovered| {
            let mut pixels = vec![0xff; width * height * 4];
            let image_info = ImageInfo::new(
                (width as i32, height as i32),
                ColorType::BGRA8888,
                AlphaType::Premul,
                None,
            );
            let mut surface =
                surfaces::wrap_pixels(&image_info, pixels.as_mut_slice(), width * 4, None)
                    .expect("test surface");
            render_toolbar_hotspot(surface.canvas(), hovered, columns, &metrics, top);
            drop(surface);
            pixels
        };

        let hidden = render_pixels(false);
        let shown = render_pixels(true);
        let center = (((top + metrics.cell_height / 2.0) as usize * width)
            + (hotspot_x + metrics.cell_width / 2.0) as usize)
            * 4;
        let immediately_left =
            ((top + metrics.cell_height / 2.0) as usize * width + (hotspot_x - 1.0) as usize) * 4;
        assert_eq!(&hidden[center..center + 4], &[0xff; 4]);
        assert_eq!(&shown[center..center + 4], &[0, 0, 0, 0xff]);
        assert_eq!(&shown[immediately_left..immediately_left + 4], &[0xff; 4]);
    }

    #[test]
    fn structural_toolbar_atoms_resolve_bold_without_changing_theme_colors() {
        let config = AppConfig::default();
        let mut state = Editor::new(&config.theme, "test");
        let atoms = toolbar_atoms(&state.toolbar.toolbar_spans(1), &state);
        let mode_label = atoms
            .iter()
            .find(|atom| atom.contents == "1.")
            .expect("structural mode path atom");
        let mode_value = atoms
            .iter()
            .find(|atom| atom.contents == "Stamp")
            .expect("ordinary mode value atom");
        let base = resolve_root_face(&state.grid.default_face, FALLBACK_FG, FALLBACK_BG);
        let label = resolve_derived_face(
            &state.grid.default_face,
            &mode_label.face,
            FALLBACK_FG,
            FALLBACK_BG,
        );
        let value = resolve_derived_face(
            &state.grid.default_face,
            &mode_value.face,
            FALLBACK_FG,
            FALLBACK_BG,
        );

        assert!(label.bold);
        assert_eq!((label.fg, label.bg), (base.fg, base.bg));
        assert_eq!(value.bold, base.bold);
        assert_eq!((value.fg, value.bg), (base.fg, base.bg));

        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.0,
            cell_height: 16.0,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        assert!(font_for_face(&metrics, &label).is_embolden());
        assert_eq!(font_for_face(&metrics, &value).is_embolden(), base.bold);

        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
        let menu_atoms = toolbar_atoms(
            &state.toolbar.toolbar_spans(crate::toolbar::MENU_FIRST_ROW),
            &state,
        );
        assert!(menu_atoms.iter().any(|atom| {
            atom.contents == "Decorators:" && atom.face.attributes.iter().any(|attr| attr == "bold")
        }));
        assert!(
            menu_atoms
                .iter()
                .filter(|atom| atom.contents == "◆")
                .all(|atom| { !atom.face.attributes.iter().any(|attr| attr == "bold") })
        );
    }

    #[test]
    fn complete_page_prefix_renders_as_one_outline_without_an_internal_seam() {
        let config = AppConfig::default();
        let mut state = Editor::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
        for key in ["2", "1"] {
            assert!(
                state
                    .toolbar
                    .handle_shortcut(&Key::Character(key.into()), ModifiersState::empty())
            );
        }

        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.0,
            cell_height: 16.0,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let max_columns = 100;
        let logical_row = crate::toolbar::MENU_FIRST_ROW + 1;
        let physical_row = crate::toolbar::toolbar_content_row(logical_row);
        let spans = crate::toolbar::boxed_toolbar_spans(
            &state.toolbar.toolbar_spans(logical_row),
            max_columns,
        );
        let expected_color = spans
            .iter()
            .find(|span| span.highlighted)
            .and_then(|span| toolbar_span_outline_color(&state, span))
            .expect("page prefix highlight color");
        let highlighted: Vec<_> =
            toolbar_span_outlines(physical_row, &spans, &state, &metrics, 0.0)
                .into_iter()
                .filter(|outline| outline.color == expected_color)
                .collect();
        assert_eq!(highlighted.len(), 1);
        let outline = highlighted[0];
        assert_eq!(
            outline.right - outline.left,
            4.0 * metrics.cell_width - 1.0 + TOOLBAR_SELECTION_PADDING * 2.0
        );

        let width = (PADDING as f32 * 2.0 + max_columns as f32 * metrics.cell_width) as usize;
        let height = crate::toolbar::toolbar_height(&state.toolbar, metrics.cell_height) as usize;
        let mut pixels = vec![0xff; width * height * 4];
        let image_info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::BGRA8888,
            AlphaType::Premul,
            None,
        );
        let mut surface =
            surfaces::wrap_pixels(&image_info, pixels.as_mut_slice(), width * 4, None)
                .expect("test surface");
        render_toolbar_span_outlines(
            surface.canvas(),
            physical_row,
            &spans,
            &state,
            &metrics,
            0.0,
        );

        let seam_x = (outline.left + TOOLBAR_SELECTION_PADDING + 2.0 * metrics.cell_width) as usize;
        let middle_y = ((outline.top + outline.bottom) / 2.0) as usize;
        let seam_offset = (middle_y * width + seam_x) * 4;
        assert_eq!(&pixels[seam_offset..seam_offset + 4], &[0xff; 4]);

        let top_x = ((outline.left + outline.right) / 2.0) as usize;
        let top_y = outline.top as usize;
        let top_offset = (top_y * width + top_x) * 4;
        assert_eq!(
            &pixels[top_offset..top_offset + 4],
            &[
                expected_color.b,
                expected_color.g,
                expected_color.r,
                expected_color.a,
            ]
        );
    }

    #[test]
    fn bottom_tooltip_uses_screen_bottom_geometry_width_clipping_and_theme_face() {
        let config = AppConfig::default();
        let state = Editor::new(&config.theme, "test");
        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.0,
            cell_height: 16.0,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let width = (PADDING as f32 * 2.0 + 120.0 * metrics.cell_width) as usize;
        let height = 320;
        let layout = layout_metrics(
            width,
            height,
            &metrics,
            (metrics.cell_width, metrics.cell_height),
            &state.toolbar,
            false,
            1.0,
        );

        assert!(layout.tooltip_visible);
        assert_eq!(
            layout.tooltip_top,
            height as f32 - metrics.cell_height - TOOLTIP_BOTTOM_PAD as f32
        );
        let spans = crate::toolbar::tooltip_spans(state.tooltip(), 12);
        assert_eq!(UnicodeWidthStr::width(spans[0].contents.as_str()), 12);
        let atoms = toolbar_atoms(&spans, &state);
        assert_eq!(atoms[0].face, state.theme.tooltip);
    }

    #[test]
    fn last_menu_row_selection_bottom_is_rendered_after_later_row_backgrounds() {
        let config = AppConfig::default();
        let mut state = Editor::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
        assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 10,
        }));

        assert_toolbar_bottom_edge_visible(
            &state,
            crate::toolbar::MENU_FIRST_ROW + 2,
            Rgba::rgb(0xff, 0x00, 0x00),
        );
    }

    #[test]
    fn pending_prefix_highlight_bottom_is_rendered_after_later_row_backgrounds() {
        let config = AppConfig::default();
        let mut state = Editor::new(&config.theme, "test");
        assert!(
            state
                .toolbar
                .handle_shortcut(&Key::Character("2".into()), ModifiersState::empty())
        );

        assert_toolbar_bottom_edge_visible(
            &state,
            crate::toolbar::MENU_FIRST_ROW + 1,
            Rgba::rgb(0x80, 0x00, 0x80),
        );
    }

    fn assert_toolbar_bottom_edge_visible(state: &Editor, logical_row: usize, _: Rgba) {
        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.0,
            cell_height: 16.0,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let max_columns = 100;
        let height = crate::toolbar::toolbar_height(&state.toolbar, metrics.cell_height);
        let physical_row = crate::toolbar::toolbar_content_row(logical_row);
        let spans = crate::toolbar::boxed_toolbar_spans(
            &state.toolbar.toolbar_spans(logical_row),
            max_columns,
        );
        let outline = toolbar_span_outlines(physical_row, &spans, state, &metrics, 0.0)
            .into_iter()
            .next()
            .expect("expected toolbar outline");

        let next_row_top = row_top(physical_row + 1, &metrics, 0.0)
            + crate::toolbar::toolbar_row_offset(physical_row + 1, metrics.cell_height);
        assert!(outline.bottom < next_row_top);
        assert!(outline.bottom < height);
    }

    #[test]
    fn cursor_theme_faces_resolve_to_drawing_blue_and_reversed_block() {
        let theme = ThemeConfig::default();
        let drawing = resolve_derived_face(
            &theme.default,
            &theme.cursor_drawing,
            FALLBACK_FG,
            FALLBACK_BG,
        );
        let block = resolve_derived_face(
            &theme.default,
            &theme.cursor_block,
            FALLBACK_FG,
            FALLBACK_BG,
        );
        assert_eq!(drawing.fg, Rgba::rgb(0x00, 0x00, 0x8b));
        assert_eq!(block.fg, Rgba::rgb(0xff, 0xff, 0xff));
        assert_eq!(block.bg, Rgba::rgb(0x00, 0x00, 0x00));
        assert!(block.reverse);
    }

    #[test]
    fn truncate_atoms_does_not_split_emoji_variation_sequence() {
        let line = vec![Atom {
            face: Face::default(),
            contents: "a❤️b".into(),
        }];

        let truncated = truncate_atoms(&line, 2);

        assert_eq!(truncated[0].contents, "a");
    }
}
