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
use crate::editor::{EditorState, GridState};
use crate::face_resolution::{
    ResolvedFace, Rgba, UnderlineStyle, resolve_derived_face, resolve_root_face,
};
use crate::layout::{LayoutMetrics, PADDING, ViewportOffset, layout_metrics};
use crate::model::{Atom, Face};

const FALLBACK_BG: Rgba = Rgba::rgb(0xff, 0xff, 0xff);
const FALLBACK_FG: Rgba = Rgba::rgb(0x00, 0x00, 0x00);
const TOOLBAR_SELECTION: Rgba = Rgba::rgb(0xff, 0x45, 0x00);
const TOOLBAR_SELECTION_PADDING: f32 = 2.0;
const TOOLBAR_SELECTION_STROKE_WIDTH: f32 = 2.0;
const DRAWING_CURSOR: Rgba = Rgba::rgb(0x00, 0x00, 0x8b);
const DRAWING_CURSOR_STROKE_WIDTH: f32 = 2.0;

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
    pub cell_width: usize,
    pub cell_height: usize,
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
    pub start_column: usize,
    pub max_columns: usize,
}

#[derive(Clone, Copy)]
enum DrawOrigin {
    Grid { top_padding: usize },
}

struct RenderFrame<'a> {
    metrics: &'a CellMetrics,
    toolbar_metrics: &'a CellMetrics,
    layout: LayoutMetrics,
    width: usize,
    height: usize,
    viewport: ViewportOffset,
}

pub fn render(
    window: &Window,
    surface: &mut Surface<Rc<Window>, Rc<Window>>,
    state: &EditorState,
    renderer: &Renderer,
    config: &AppConfig,
    viewport: ViewportOffset,
) -> Result<()> {
    let size = window.inner_size();
    let width = size.width.max(1) as usize;
    let height = size.height.max(1) as usize;
    let metrics = renderer.metrics(window.scale_factor());
    let title_metrics = renderer.title_metrics(window.scale_factor());

    let mut buffer = surface
        .buffer_mut()
        .map_err(|error| anyhow!(error.to_string()))?;
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
                title_metrics.cell_height,
                config.transparent_menubar,
                window.scale_factor(),
            ),
            width,
            height,
            viewport,
        },
    );

    buffer
        .present()
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(())
}

fn render_canvas(canvas: &Canvas, state: &EditorState, config: &AppConfig, frame: RenderFrame<'_>) {
    let RenderFrame {
        metrics,
        toolbar_metrics,
        layout,
        width,
        height,
        viewport,
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
    render_toolbar(canvas, state, toolbar_metrics, layout.top_padding, width);

    let grid_layout = visible_grid_layout(layout, metrics, viewport);
    canvas.save();
    canvas.clip_rect(
        Rect::from_xywh(
            0.0,
            layout.grid_top as f32,
            width as f32,
            height.saturating_sub(layout.grid_top) as f32,
        ),
        None,
        false,
    );
    canvas.translate((viewport.x as f32, viewport.y as f32));

    let preview_lines = state.lines_with_shape_preview();
    let lines = preview_lines.as_ref().unwrap_or(&state.grid.lines);
    for (row_index, line) in lines.iter().take(grid_layout.rows).enumerate() {
        render_line(
            canvas,
            row_index,
            line,
            &state.grid.default_face,
            grid_layout.cols,
            metrics,
            DrawOrigin::Grid {
                top_padding: layout.grid_top,
            },
        );
    }

    render_grid_cursor(
        canvas,
        &state.grid,
        state.cursor_mode,
        &config.display.cursor_shape,
        grid_layout,
        metrics,
    );
    canvas.restore();
}

fn render_toolbar(
    canvas: &Canvas,
    state: &EditorState,
    metrics: &CellMetrics,
    top_padding: usize,
    width: usize,
) {
    let max_columns = width.saturating_sub(PADDING * 2) / metrics.cell_width.max(1);
    render_toolbar_spans(
        canvas,
        0,
        &state.toolbar.main_spans(),
        state,
        max_columns,
        metrics,
        top_padding,
    );
    render_toolbar_spans(
        canvas,
        1,
        &state.toolbar.submenu_spans(1),
        state,
        max_columns,
        metrics,
        top_padding,
    );
    render_toolbar_spans(
        canvas,
        2,
        &state.toolbar.submenu_spans(2),
        state,
        max_columns,
        metrics,
        top_padding,
    );

    let tooltip = [Atom {
        face: Face::default(),
        contents: match state.cursor_mode {
            CursorMode::Text => {
                "<Ret> to exit text mode, arrows move freely over the canvas".to_string()
            }
            CursorMode::Replace => {
                "<Shift-Ret> to exit replace mode, arrows move freely over the canvas".to_string()
            }
            _ => state.toolbar.tooltip().to_string(),
        },
    }];
    render_line(
        canvas,
        3,
        &tooltip,
        &state.grid.default_face,
        max_columns,
        metrics,
        DrawOrigin::Grid {
            top_padding: top_padding + crate::toolbar::toolbar_row_offset(3, metrics.cell_height),
        },
    );
}

fn render_toolbar_spans(
    canvas: &Canvas,
    row: usize,
    spans: &[crate::toolbar::ToolbarSpan],
    state: &EditorState,
    max_columns: usize,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let atoms: Vec<_> = spans
        .iter()
        .map(|span| Atom {
            face: Face::default(),
            contents: span.contents.clone(),
        })
        .collect();
    render_line(
        canvas,
        row,
        &atoms,
        &state.grid.default_face,
        max_columns,
        metrics,
        DrawOrigin::Grid {
            top_padding: top_padding + crate::toolbar::toolbar_row_offset(row, metrics.cell_height),
        },
    );

    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_color(TOOLBAR_SELECTION.to_color())
        .set_stroke_width(TOOLBAR_SELECTION_STROKE_WIDTH);
    let top = (row_top(row, metrics, top_padding)
        + crate::toolbar::toolbar_row_offset(row, metrics.cell_height)) as f32
        - TOOLBAR_SELECTION_PADDING;
    let bottom =
        top + metrics.cell_height.saturating_sub(1) as f32 + TOOLBAR_SELECTION_PADDING * 2.0;
    let mut column = 0;
    for span in spans {
        let span_width = UnicodeWidthStr::width(span.contents.as_str());
        if span.selected && span_width > 0 {
            let left = (PADDING + column * metrics.cell_width) as f32 - TOOLBAR_SELECTION_PADDING;
            let right = left
                + (span_width * metrics.cell_width).saturating_sub(1) as f32
                + TOOLBAR_SELECTION_PADDING * 2.0;
            canvas.draw_line((left, top), (right, top), &paint);
            canvas.draw_line((left, bottom), (right, bottom), &paint);
            canvas.draw_line((left, top), (left, bottom), &paint);
            canvas.draw_line((right, top), (right, bottom), &paint);
        }
        column += span_width;
    }
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

fn hidden_leading_cells(offset: i64, cell_size: usize) -> usize {
    if offset >= 0 {
        return 0;
    }
    usize::try_from(offset.saturating_abs())
        .unwrap_or(usize::MAX)
        .checked_div(cell_size.max(1))
        .unwrap_or(0)
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
    if !transparent_menubar || layout.top_padding <= PADDING || title.is_empty() {
        return;
    }

    let max_columns = window_width
        .saturating_sub(PADDING * 2)
        .checked_div(metrics.cell_width.max(1))
        .unwrap_or(0);
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
    let top = layout.top_padding.saturating_sub(metrics.cell_height) / 2;
    let baseline = top as f32 + metrics.baseline_offset;
    canvas.draw_str(title, (left, baseline), &metrics.font, &paint);
}

fn render_line(
    canvas: &Canvas,
    row: usize,
    line: &[Atom],
    default_face: &Face,
    max_columns: usize,
    metrics: &CellMetrics,
    origin: DrawOrigin,
) {
    render_line_at(
        canvas,
        LineRenderPosition {
            row,
            start_column: 0,
            max_columns,
        },
        line,
        default_face,
        metrics,
        origin,
    );
}

fn render_line_at(
    canvas: &Canvas,
    position: LineRenderPosition,
    line: &[Atom],
    default_face: &Face,
    metrics: &CellMetrics,
    origin: DrawOrigin,
) {
    let top = line_top(origin, position.row, metrics);
    let mut column = position.start_column;
    let mut bg_paint = Paint::default();
    bg_paint.set_anti_alias(false);
    let mut fg_paint = Paint::default();
    fg_paint.set_anti_alias(true);

    for atom in line {
        let atom_width = atom_display_width(&atom.contents);
        if atom_width == 0 {
            continue;
        }

        let atom_start = column;
        let atom_width = atom_width.min(position.max_columns.saturating_sub(atom_start));
        if atom_width == 0 {
            return;
        }
        let resolved = resolve_derived_face(default_face, &atom.face, FALLBACK_FG, FALLBACK_BG);
        bg_paint.set_color(resolved.bg.to_color());
        fg_paint.set_color(resolved.fg.to_color());
        fill_cells(canvas, atom_start, top, atom_width, metrics, &bg_paint);
        let font = font_for_face(metrics, &resolved);

        for cluster in text_clusters(&atom.contents) {
            if cluster == "\n" {
                continue;
            }

            let span = cluster_display_width(cluster);
            draw_text_cluster(canvas, column, top, cluster, &font, metrics, &fg_paint);
            column += span;
            if column >= position.max_columns {
                draw_text_decorations(canvas, atom_start, top, atom_width, metrics, &resolved);
                return;
            }
        }
        draw_text_decorations(canvas, atom_start, top, atom_width, metrics, &resolved);
    }
}

fn render_grid_cursor(
    canvas: &Canvas,
    grid: &GridState,
    cursor_mode: CursorMode,
    cursor_shape_config: &CursorShapeConfig,
    layout: LayoutMetrics,
    metrics: &CellMetrics,
) {
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
        grid.lines.get(cursor.line).map(Vec::as_slice),
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
        render_hollow_drawing_cursor(canvas, cursor.column, top, &cell, metrics, &cell_resolved);
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

fn is_drawing_mode(mode: CursorMode) -> bool {
    matches!(
        mode,
        CursorMode::MoveDraw | CursorMode::Stamp | CursorMode::Shapes | CursorMode::Utilities
    )
}

fn render_hollow_drawing_cursor(
    canvas: &Canvas,
    column: usize,
    top: usize,
    cell: &CursorCell,
    metrics: &CellMetrics,
    resolved: &ResolvedFace,
) {
    render_cursor_base_cell(canvas, column, top, cell, metrics, resolved);

    let left = (PADDING + column * metrics.cell_width) as f32 + 1.0;
    let right = (PADDING + (column + 1) * metrics.cell_width) as f32 - 1.0;
    let top = top as f32 + 1.0;
    let bottom = top + metrics.cell_height.saturating_sub(2) as f32;
    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_color(DRAWING_CURSOR.to_color())
        .set_stroke_width(DRAWING_CURSOR_STROKE_WIDTH);
    canvas.draw_line((left, top), (right, top), &paint);
    canvas.draw_line((left, bottom), (right, bottom), &paint);
    canvas.draw_line((left, top), (left, bottom), &paint);
    canvas.draw_line((right, top), (right, bottom), &paint);
}

fn render_block_cursor(
    canvas: &Canvas,
    column: usize,
    top: usize,
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
    top: usize,
    cell: &CursorCell,
    metrics: &CellMetrics,
    base_resolved: &ResolvedFace,
    resolved: &ResolvedFace,
) {
    render_cursor_base_cell(canvas, column, top, cell, metrics, base_resolved);
    let width = (metrics.cell_width as f32 * CURSOR_BEAM_WIDTH_RATIO)
        .round()
        .clamp(1.0, metrics.cell_width as f32);
    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_color(cursor_indicator_color(CursorShape::Beam, resolved).to_color());
    fill_rect_pixels(
        canvas,
        (PADDING + column * metrics.cell_width) as f32,
        top as f32,
        width,
        metrics.cell_height as f32,
        &paint,
    );
}

fn render_underline_cursor(
    canvas: &Canvas,
    column: usize,
    top: usize,
    cell: &CursorCell,
    metrics: &CellMetrics,
    base_resolved: &ResolvedFace,
    resolved: &ResolvedFace,
) {
    render_cursor_base_cell(canvas, column, top, cell, metrics, base_resolved);
    let height = (metrics.cell_height as f32 * CURSOR_UNDERLINE_HEIGHT_RATIO)
        .round()
        .clamp(1.0, metrics.cell_height as f32);
    let max_top = top as f32 + metrics.cell_height as f32 - height;
    let y = underline_start_y(top, metrics, height)
        .min(max_top)
        .max(top as f32);
    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_color(cursor_indicator_color(CursorShape::Underline, resolved).to_color());
    fill_rect_pixels(
        canvas,
        (PADDING + column * metrics.cell_width) as f32,
        y,
        metrics.cell_width as f32,
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
        CursorMode::Text => config.insert.unwrap_or(CursorShape::Block),
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
    top: usize,
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

fn row_top(row: usize, metrics: &CellMetrics, top_padding: usize) -> usize {
    top_padding + row * metrics.cell_height
}

fn line_top(origin: DrawOrigin, row: usize, metrics: &CellMetrics) -> usize {
    match origin {
        DrawOrigin::Grid { top_padding } => row_top(row, metrics, top_padding),
    }
}

fn fill_cells(
    canvas: &Canvas,
    column: usize,
    top: usize,
    width_in_cells: usize,
    metrics: &CellMetrics,
    paint: &Paint,
) {
    let left = PADDING + column * metrics.cell_width;
    let rect = Rect::from_xywh(
        left as f32,
        top as f32,
        (metrics.cell_width * width_in_cells) as f32,
        metrics.cell_height as f32,
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
    top: usize,
    text: &str,
    font: &Font,
    metrics: &CellMetrics,
    paint: &Paint,
) {
    if text.chars().all(char::is_control) {
        return;
    }

    let left = PADDING + column * metrics.cell_width;
    let baseline = top as f32 + metrics.baseline_offset;
    let font = font_for_text(metrics, font, text);
    canvas.draw_str(text, (left as f32, baseline), &font, paint);
}

fn text_clusters(text: &str) -> impl Iterator<Item = &str> {
    UnicodeSegmentation::graphemes(text, true)
}

fn cluster_display_width(cluster: &str) -> usize {
    UnicodeWidthStr::width(cluster).max(usize::from(!cluster.is_empty()))
}

fn font_for_text(metrics: &CellMetrics, font: &Font, text: &str) -> Font {
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
    top: usize,
    width_in_cells: usize,
    metrics: &CellMetrics,
    face: &ResolvedFace,
) {
    if width_in_cells == 0 {
        return;
    }

    let left = PADDING + column * metrics.cell_width;
    let width = metrics.cell_width * width_in_cells;
    let stroke_width = (metrics.cell_height as f32 / 14.0).max(1.0);
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
                canvas.draw_line((left as f32, y), ((left + width) as f32, y), &paint);
            }
            UnderlineStyle::Double => {
                let y = underline_start_y(top, metrics, stroke_width);
                let gap = stroke_width + 1.0;
                canvas.draw_line((left as f32, y), ((left + width) as f32, y), &paint);
                canvas.draw_line(
                    (left as f32, y + gap),
                    ((left + width) as f32, y + gap),
                    &paint,
                );
            }
            UnderlineStyle::Curly => {
                let y = underline_start_y(top, metrics, stroke_width);
                let wave = (metrics.cell_height as f32 / 8.0).max(1.5);
                let mut x = left as f32;
                let end = (left + width) as f32;
                let step = (metrics.cell_width as f32 / 2.0).max(2.0);
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
        let y = top as f32 + metrics.cell_height as f32 * 0.55;
        canvas.draw_line((left as f32, y), ((left + width) as f32, y), &paint);
    }
}

fn underline_start_y(top: usize, metrics: &CellMetrics, stroke_width: f32) -> f32 {
    top as f32 + metrics.baseline_offset + stroke_width + metrics.underline_offset
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
                    .match_family_style("", FontStyle::normal())
                    .expect("expected a fallback system typeface")
            });

        let mut font = Font::new(typeface, physical_font_size);
        font.set_subpixel(true)
            .set_edging(Edging::SubpixelAntiAlias)
            .set_hinting(FontHinting::Full)
            .set_baseline_snap(false)
            .set_linear_metrics(false);

        let (_, metrics) = font.metrics();
        let cell_width = font.measure_str("M", None).0.ceil().max(1.0) as usize;
        let cell_height = (metrics.descent - metrics.ascent + metrics.leading)
            .ceil()
            .max(1.0) as usize;
        let baseline_offset = (-metrics.ascent).ceil();

        let metrics = CellMetrics {
            font,
            cell_width,
            cell_height: cell_height.max(16),
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
    [
        configured_family,
        "SF Mono",
        "Menlo",
        "Monaco",
        "JetBrains Mono",
        "Courier New",
    ]
    .iter()
    .find_map(|family| font_mgr.match_family_style(family, FontStyle::normal()))
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
    use crate::app::{AppConfig, CursorMode, CursorShape};

    use super::*;

    #[test]
    fn title_truncation_limits_display_width() {
        let title = truncate_title("ascdraw - /tmp/long/path.txt", 12);

        assert_eq!(title, "ascdraw - /t");
        assert_eq!(atom_display_width(&title), 12);
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
    fn parses_rgba_colors_by_ignoring_alpha() {
        let resolved = resolve_root_face(
            &Face {
                fg: "rgba:ffffff80".into(),
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
            cell_width: 8,
            cell_height: 16,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let shifted_metrics = CellMetrics {
            underline_offset: 1.5,
            ..base_metrics.clone()
        };
        let stroke_width = (base_metrics.cell_height as f32 / 14.0).max(1.0);

        let base_y = underline_start_y(3, &base_metrics, stroke_width);
        let shifted_y = underline_start_y(3, &shifted_metrics, stroke_width);

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
            fg: "black".into(),
            bg: "white".into(),
            underline: "default".into(),
            attributes: Vec::new(),
        };
        let line = vec![Atom {
            face: cursor_face.clone(),
            contents: "\n".into(),
        }];

        let cursor = cursor_cell(Some(&line), 0).expect("cursor cell should exist");
        assert_eq!(cursor.face.bg, "white");
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
                    fg: "black".into(),
                    bg: "white".into(),
                    underline: "default".into(),
                    attributes: Vec::new(),
                },
                contents: "c".into(),
            },
        ];

        let cursor = cursor_cell(Some(&line), 2).expect("cursor cell should exist");
        assert_eq!(cursor.text, Some("c".to_string()));
        assert_eq!(cursor.face.bg, "white");
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
    fn truncate_atoms_does_not_split_emoji_variation_sequence() {
        let line = vec![Atom {
            face: Face::default(),
            contents: "a❤️b".into(),
        }];

        let truncated = truncate_atoms(&line, 2);

        assert_eq!(truncated[0].contents, "a");
    }
}
