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

mod popup;

use popup::{render_info, render_menu};

use crate::app::{AppConfig, AppState, GridState, StatusState};
use crate::face_resolution::{
    ResolvedFace, Rgba, UnderlineStyle, resolve_derived_face, resolve_root_face,
};
use crate::kakoune_messages::{Atom, Face};
use crate::layout::{LayoutMetrics, PADDING, layout_metrics};

const FALLBACK_BG: Rgba = Rgba::rgb(0x1e, 0x1e, 0x2e);
const FALLBACK_FG: Rgba = Rgba::rgb(0xdd, 0xdd, 0xdd);

#[derive(Clone)]
pub struct Renderer {
    font_mgr: FontMgr,
    preferred_font_family: String,
    default_logical_font_size: f32,
    underline_offset: f32,
    logical_font_size: Cell<f32>,
    metrics_cache: RefCell<Option<(u64, CellMetrics)>>,
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

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct FallbackFontKey {
    character: u32,
    size_bits: u32,
    bold: bool,
    italic: bool,
}

#[derive(Clone, Copy)]
pub(in crate::render) struct LineRenderPosition {
    pub row: usize,
    pub start_column: usize,
    pub max_columns: usize,
}

pub fn render(
    window: &Window,
    surface: &mut Surface<Rc<Window>, Rc<Window>>,
    state: &AppState,
    renderer: &Renderer,
    config: &AppConfig,
) -> Result<()> {
    let size = window.inner_size();
    let width = size.width.max(1) as usize;
    let height = size.height.max(1) as usize;
    let metrics = renderer.metrics(window.scale_factor());

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
        &metrics,
        layout_metrics(
            width,
            height,
            &metrics,
            config.transparent_menubar,
            window.scale_factor(),
        ),
        width,
        config.transparent_menubar,
    );

    buffer
        .present()
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(())
}

fn render_canvas(
    canvas: &Canvas,
    state: &AppState,
    metrics: &CellMetrics,
    layout: LayoutMetrics,
    width: usize,
    transparent_menubar: bool,
) {
    let default_face = resolve_root_face(&state.grid.default_face, FALLBACK_FG, FALLBACK_BG);
    canvas.clear(default_face.bg.to_color());
    render_window_title(
        canvas,
        &state.window_title,
        &state.grid.default_face,
        metrics,
        layout,
        width,
        transparent_menubar,
    );

    let cols = layout.cols;
    let rows = layout.rows;
    let status_rows = usize::from(state.status.is_some());
    let content_rows = rows.saturating_sub(status_rows);

    for (row_index, line) in state.grid.lines.iter().take(content_rows).enumerate() {
        render_line(
            canvas,
            row_index,
            line,
            &state.grid.default_face,
            cols,
            metrics,
            layout.top_padding,
        );
    }

    render_grid_cursor(
        canvas,
        &state.grid,
        cols,
        content_rows,
        metrics,
        layout.top_padding,
    );

    if let Some(status) = &state.status {
        render_status(canvas, rows, cols, status, metrics, layout.top_padding);
    }

    let menu_rect = state.menu.as_ref().and_then(|menu| {
        render_menu(
            canvas,
            menu,
            cols,
            content_rows,
            metrics,
            layout.top_padding,
        )
    });

    if let Some(info) = &state.info {
        render_info(
            canvas,
            info,
            menu_rect,
            cols,
            content_rows,
            metrics,
            layout.top_padding,
        );
    }
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

fn render_status(
    canvas: &Canvas,
    total_rows: usize,
    cols: usize,
    status: &StatusState,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let row = total_rows.saturating_sub(1);
    fill_line_background(
        canvas,
        row,
        cols,
        &status.default_face,
        metrics,
        top_padding,
    );

    let mut prompt_line = status.prompt.clone();
    prompt_line.extend(status.content.clone());

    let mode_width = line_display_width(&status.mode_line);
    let right_start = cols.saturating_sub(mode_width);
    let prompt_limit = if prompt_line.is_empty() {
        cols
    } else {
        right_start
    };

    if !prompt_line.is_empty() {
        render_line_at(
            canvas,
            LineRenderPosition {
                row,
                start_column: 0,
                max_columns: prompt_limit,
            },
            &prompt_line,
            &status.default_face,
            metrics,
            top_padding,
        );
    }

    if !status.mode_line.is_empty() {
        render_line_at(
            canvas,
            LineRenderPosition {
                row,
                start_column: right_start,
                max_columns: cols,
            },
            &status.mode_line,
            &status.default_face,
            metrics,
            top_padding,
        );
    }
}

fn render_line(
    canvas: &Canvas,
    row: usize,
    line: &[Atom],
    default_face: &Face,
    max_columns: usize,
    metrics: &CellMetrics,
    top_padding: usize,
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
        top_padding,
    );
}

pub(in crate::render) fn render_line_at(
    canvas: &Canvas,
    position: LineRenderPosition,
    line: &[Atom],
    default_face: &Face,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let top = top_padding + position.row * metrics.cell_height;
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
    cols: usize,
    rows: usize,
    metrics: &CellMetrics,
    top_padding: usize,
) {
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

    let resolved = resolve_derived_face(&grid.default_face, &cell.face, FALLBACK_FG, FALLBACK_BG);
    let top = top_padding + cursor.line * metrics.cell_height;

    let mut bg_paint = Paint::default();
    bg_paint
        .set_anti_alias(false)
        .set_color(resolved.bg.to_color());
    fill_cells(canvas, cursor.column, top, 1, metrics, &bg_paint);

    let mut fg_paint = Paint::default();
    fg_paint
        .set_anti_alias(true)
        .set_color(resolved.fg.to_color());
    let font = font_for_face(metrics, &resolved);
    if let Some(text) = cell.text {
        draw_text_cluster(canvas, cursor.column, top, &text, &font, metrics, &fg_paint);
    }
    draw_text_decorations(canvas, cursor.column, top, 1, metrics, &resolved);
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

pub fn line_display_width(line: &[Atom]) -> usize {
    line.iter()
        .map(|atom| atom_display_width(&atom.contents))
        .sum()
}

fn fill_line_background(
    canvas: &Canvas,
    row: usize,
    cols: usize,
    default_face: &Face,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let bg = resolve_root_face(default_face, FALLBACK_FG, FALLBACK_BG)
        .bg
        .to_color();
    let mut paint = Paint::default();
    paint.set_anti_alias(false).set_color(bg);
    fill_cells(
        canvas,
        0,
        top_padding + row * metrics.cell_height,
        cols,
        metrics,
        &paint,
    );
}

pub(in crate::render) fn fill_line_segment(
    canvas: &Canvas,
    row: usize,
    column: usize,
    width: usize,
    face: &Face,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let bg = resolve_root_face(face, FALLBACK_FG, FALLBACK_BG)
        .bg
        .to_color();
    let mut paint = Paint::default();
    paint.set_anti_alias(false).set_color(bg);
    fill_cells(
        canvas,
        column,
        top_padding + row * metrics.cell_height,
        width,
        metrics,
        &paint,
    );
}

pub(in crate::render) fn fill_rect(
    canvas: &Canvas,
    rect: popup::CellRect,
    face: &Face,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    for row in rect.row..rect.row + rect.height {
        fill_line_segment(
            canvas,
            row,
            rect.column,
            rect.width,
            face,
            metrics,
            top_padding,
        );
    }
}

pub(in crate::render) fn truncate_atoms(line: &[Atom], max_width: usize) -> Vec<Atom> {
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

pub(in crate::render) fn render_string_line(
    canvas: &Canvas,
    row: usize,
    column: usize,
    text: &str,
    default_face: &Face,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    if text.is_empty() {
        return;
    }

    let atoms = [Atom {
        face: Face::default(),
        contents: text.to_string(),
    }];
    render_line_at(
        canvas,
        LineRenderPosition {
            row,
            start_column: column,
            max_columns: column + atom_display_width(text),
        },
        &atoms,
        default_face,
        metrics,
        top_padding,
    );
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
        metrics_cache: RefCell::new(None),
    }
}

impl Renderer {
    pub fn adjust_font_size(&self, delta: f32) -> bool {
        const MIN_FONT_SIZE: f32 = 6.0;

        let next = (self.logical_font_size.get() + delta).max(MIN_FONT_SIZE);
        if (next - self.logical_font_size.get()).abs() < f32::EPSILON {
            return false;
        }

        self.logical_font_size.set(next);
        self.metrics_cache.borrow_mut().take();
        true
    }

    pub fn reset_font_size(&self) -> bool {
        if (self.logical_font_size.get() - self.default_logical_font_size).abs() < f32::EPSILON {
            return false;
        }

        self.logical_font_size.set(self.default_logical_font_size);
        self.metrics_cache.borrow_mut().take();
        true
    }

    pub fn metrics(&self, scale_factor: f64) -> CellMetrics {
        let cache_key = scale_factor.to_bits();
        if let Some((cached_key, metrics)) = self.metrics_cache.borrow().as_ref()
            && *cached_key == cache_key
        {
            return metrics.clone();
        }

        let physical_font_size = (self.logical_font_size.get() as f64 * scale_factor) as f32;
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
        self.metrics_cache
            .borrow_mut()
            .replace((cache_key, metrics.clone()));
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
    use crate::app::StatusState;
    use crate::kakoune_messages::StatusStyle;

    use super::*;

    #[test]
    fn status_uses_mode_line_and_prompt_rows() {
        let status = StatusState {
            prompt: vec![Atom {
                face: Face::default(),
                contents: ":".into(),
            }],
            content: vec![Atom {
                face: Face::default(),
                contents: "w".into(),
            }],
            cursor_pos: 1,
            mode_line: vec![Atom {
                face: Face::default(),
                contents: "status".into(),
            }],
            default_face: Face::default(),
            style: StatusStyle::Status,
        };
        let mut prompt_line = status.prompt.clone();
        prompt_line.extend(status.content.clone());
        assert_eq!(line_display_width(&prompt_line), 2);
        assert_eq!(line_display_width(&status.mode_line), 6);
    }

    #[test]
    fn line_display_width_ignores_empty_spacer_atoms() {
        let line = vec![
            Atom {
                face: Face::default(),
                contents: "left".into(),
            },
            Atom {
                face: Face::default(),
                contents: "".into(),
            },
            Atom {
                face: Face::default(),
                contents: "right".into(),
            },
        ];

        assert_eq!(atom_display_width(&line[1].contents), 0);
        assert_eq!(line_display_width(&line), 9);
    }

    #[test]
    fn title_truncation_limits_display_width() {
        let title = truncate_title("kakvide - /tmp/long/path.txt", 12);

        assert_eq!(title, "kakvide - /t");
        assert_eq!(atom_display_width(&title), 12);
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
    fn truncate_atoms_does_not_split_emoji_variation_sequence() {
        let line = vec![Atom {
            face: Face::default(),
            contents: "a❤️b".into(),
        }];

        let truncated = truncate_atoms(&line, 2);

        assert_eq!(truncated[0].contents, "a");
    }
}
