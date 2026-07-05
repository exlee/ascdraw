use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow};
use skia_safe::{
    AlphaType, Canvas, Color, ColorType, Font, FontHinting, FontMgr, FontStyle, ImageInfo, Paint,
    PixelGeometry, Rect, SurfaceProps, SurfacePropsFlags, font::Edging, surfaces,
};
use softbuffer::Surface;
use unicode_width::UnicodeWidthChar;
use winit::window::Window;

use crate::app::{AppConfig, AppState, GridState, InfoState, MenuState, StatusState};
use crate::kakoune_messages::{Atom, Face, InfoStyle, MenuStyle};
use crate::layout::{LayoutMetrics, PADDING, layout_metrics};

const FALLBACK_BG: Rgb = Rgb::new(0x1e, 0x1e, 0x2e);
const FALLBACK_FG: Rgb = Rgb::new(0xdd, 0xdd, 0xdd);

#[derive(Clone)]
pub struct Renderer {
    font_mgr: FontMgr,
    preferred_font_family: String,
    logical_font_size: Cell<f32>,
    metrics_cache: RefCell<Option<(u64, CellMetrics)>>,
}

#[derive(Clone)]
pub struct CellMetrics {
    pub font: Font,
    pub cell_width: usize,
    pub cell_height: usize,
    pub baseline_offset: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

impl Rgb {
    const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    fn to_color(self) -> Color {
        Color::from_rgb(self.r, self.g, self.b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellRect {
    row: usize,
    column: usize,
    width: usize,
    height: usize,
}

#[derive(Clone)]
struct CursorCell {
    face: Face,
    ch: Option<char>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MenuLayout {
    rect: CellRect,
    visible_columns: usize,
    total_columns: usize,
    rows_per_column: usize,
    column_width: usize,
    first_visible_column: usize,
    single_row: bool,
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
    );

    buffer
        .present()
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(())
}

fn render_canvas(canvas: &Canvas, state: &AppState, metrics: &CellMetrics, layout: LayoutMetrics) {
    let bg = resolve_color(&state.grid.default_face.bg, FALLBACK_BG).to_color();
    canvas.clear(bg);

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
            row,
            0,
            &prompt_line,
            &status.default_face,
            prompt_limit,
            metrics,
            top_padding,
        );
    }

    if !status.mode_line.is_empty() {
        render_line_at(
            canvas,
            row,
            right_start,
            &status.mode_line,
            &status.default_face,
            cols,
            metrics,
            top_padding,
        );
    }
}

fn render_menu(
    canvas: &Canvas,
    menu: &MenuState,
    cols: usize,
    rows: usize,
    metrics: &CellMetrics,
    top_padding: usize,
) -> Option<CellRect> {
    if cols == 0 || rows == 0 || menu.items.is_empty() {
        return None;
    }

    let layout = menu_layout(menu, cols, rows)?;
    fill_rect(canvas, layout.rect, &menu.menu_face, metrics, top_padding);

    if layout.single_row {
        render_single_row_menu(canvas, menu, layout, metrics, top_padding);
    } else if layout.visible_columns == 1 {
        render_single_column_menu(canvas, menu, layout, metrics, top_padding);
    } else {
        render_multi_column_menu(canvas, menu, layout, metrics, top_padding);
    }

    Some(layout.rect)
}

fn menu_layout(menu: &MenuState, cols: usize, rows: usize) -> Option<MenuLayout> {
    let item_count = menu.items.len();
    let longest = menu
        .items
        .iter()
        .map(|line| line_display_width(line))
        .max()
        .unwrap_or(1)
        .max(1);
    let anchor_line = match menu.style {
        MenuStyle::Inline => menu.anchor.line,
        MenuStyle::Prompt | MenuStyle::Search => rows,
    };

    match menu.style {
        MenuStyle::Inline => {
            let width = longest.saturating_add(1).min(cols);
            let height = item_count.max(1).min(rows).min(10);
            if width == 0 || height == 0 {
                return None;
            }

            Some(MenuLayout {
                rect: CellRect {
                    row: inline_popup_row(anchor_line, height, rows),
                    column: menu.anchor.column.min(cols.saturating_sub(width)),
                    width,
                    height,
                },
                visible_columns: 1,
                total_columns: item_count.div_ceil(height.max(1)),
                rows_per_column: height,
                column_width: width,
                first_visible_column: 0,
                single_row: false,
            })
        }
        MenuStyle::Search => {
            let width = cols - cols / 2;
            if width < 4 {
                return None;
            }

            Some(MenuLayout {
                rect: CellRect {
                    row: rows.saturating_sub(1),
                    column: cols / 2,
                    width,
                    height: 1,
                },
                visible_columns: 0,
                total_columns: item_count,
                rows_per_column: 1,
                column_width: width.saturating_sub(3),
                first_visible_column: menu_first_search_item(menu, width.saturating_sub(3)),
                single_row: true,
            })
        }
        MenuStyle::Prompt => {
            if cols <= 1 {
                return None;
            }

            let max_width = cols.saturating_sub(1);
            let visible_columns = (max_width / longest.saturating_add(1)).max(1);
            let max_height = rows
                .min(10)
                .min(anchor_line.max(rows.saturating_sub(anchor_line).saturating_sub(1)));
            let height = item_count.div_ceil(visible_columns).min(max_height);
            if height == 0 {
                return None;
            }

            let total_columns = item_count.div_ceil(height);
            let first_visible_column =
                menu_first_visible_column(menu, height, visible_columns, total_columns);

            Some(MenuLayout {
                rect: CellRect {
                    row: rows.saturating_sub(height),
                    column: 0,
                    width: cols,
                    height,
                },
                visible_columns,
                total_columns,
                rows_per_column: height,
                column_width: (cols.saturating_sub(1) / visible_columns).max(1),
                first_visible_column,
                single_row: false,
            })
        }
    }
}

fn menu_first_visible_column(
    menu: &MenuState,
    rows_per_column: usize,
    visible_columns: usize,
    total_columns: usize,
) -> usize {
    let Some(selected) = menu.selected else {
        return 0;
    };
    if rows_per_column == 0 || visible_columns >= total_columns {
        return 0;
    }

    let selected_column = selected / rows_per_column;
    if selected_column < visible_columns {
        0
    } else {
        selected_column
            .saturating_add(1)
            .saturating_sub(visible_columns)
            .min(total_columns.saturating_sub(visible_columns))
    }
}

fn menu_first_search_item(menu: &MenuState, available_width: usize) -> usize {
    let Some(selected) = menu.selected else {
        return 0;
    };
    if available_width == 0 {
        return 0;
    }

    let mut first = 0;
    let mut used_width = 0;
    for index in 0..=selected.min(menu.items.len().saturating_sub(1)) {
        let item_width = line_display_width(&menu.items[index]).saturating_add(1);
        if used_width + item_width > available_width {
            first = index;
            used_width = item_width;
        } else {
            used_width += item_width;
        }
    }
    first
}

fn render_single_column_menu(
    canvas: &Canvas,
    menu: &MenuState,
    layout: MenuLayout,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    for (index, item) in menu.items.iter().take(layout.rect.height).enumerate() {
        let face = if menu.selected == Some(index) {
            &menu.selected_face
        } else {
            &menu.menu_face
        };
        fill_line_segment(
            canvas,
            layout.rect.row + index,
            layout.rect.column,
            layout.rect.width,
            face,
            metrics,
            top_padding,
        );
        render_line_at(
            canvas,
            layout.rect.row + index,
            layout.rect.column,
            item,
            face,
            layout.rect.column + layout.rect.width,
            metrics,
            top_padding,
        );
    }
}

fn render_single_row_menu(
    canvas: &Canvas,
    menu: &MenuState,
    layout: MenuLayout,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let row = layout.rect.row;
    let mut column = layout.rect.column;

    if layout.first_visible_column > 0 {
        render_string_line(
            canvas,
            row,
            column,
            "< ",
            &menu.menu_face,
            metrics,
            top_padding,
        );
    }
    column += 2;

    let end_column = layout.rect.column + layout.rect.width.saturating_sub(2);
    let mut index = layout.first_visible_column;
    while index < menu.items.len() && column < end_column {
        let item = &menu.items[index];
        let face = if menu.selected == Some(index) {
            &menu.selected_face
        } else {
            &menu.menu_face
        };
        let available_width = end_column.saturating_sub(column);
        let item_width = line_display_width(item);
        let truncated = if item_width > available_width {
            truncate_atoms(item, available_width.saturating_sub(1))
        } else {
            item.clone()
        };
        render_line_at(
            canvas,
            row,
            column,
            &truncated,
            face,
            end_column,
            metrics,
            top_padding,
        );

        if item_width > available_width {
            render_string_line(
                canvas,
                row,
                end_column,
                "…",
                &menu.menu_face,
                metrics,
                top_padding,
            );
            break;
        }

        column += item_width;
        if column < end_column {
            render_string_line(
                canvas,
                row,
                column,
                " ",
                &menu.menu_face,
                metrics,
                top_padding,
            );
            column += 1;
        }
        index += 1;
    }

    if index < menu.items.len() {
        render_string_line(
            canvas,
            row,
            layout.rect.column + layout.rect.width.saturating_sub(1),
            ">",
            &menu.menu_face,
            metrics,
            top_padding,
        );
    }
}

fn render_multi_column_menu(
    canvas: &Canvas,
    menu: &MenuState,
    layout: MenuLayout,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let mark_height = layout
        .rect
        .height
        .saturating_mul(layout.rect.height)
        .div_ceil(layout.total_columns.max(layout.visible_columns))
        .min(layout.rect.height)
        .max(1);
    let mark_row = if layout.total_columns > layout.visible_columns {
        layout
            .rect
            .height
            .saturating_sub(mark_height)
            .saturating_mul(layout.first_visible_column)
            / (layout.total_columns - layout.visible_columns)
    } else {
        0
    };

    for row_offset in 0..layout.rect.height {
        let row = layout.rect.row + row_offset;
        for col_offset in 0..layout.visible_columns {
            let column_index = layout.first_visible_column + col_offset;
            if column_index >= layout.total_columns {
                break;
            }

            let item_index = column_index * layout.rows_per_column + row_offset;
            let face = if menu.selected == Some(item_index) {
                &menu.selected_face
            } else {
                &menu.menu_face
            };
            let start_column = layout.rect.column + col_offset * layout.column_width;
            fill_line_segment(
                canvas,
                row,
                start_column,
                layout.column_width,
                face,
                metrics,
                top_padding,
            );

            if let Some(item) = menu.items.get(item_index) {
                let truncated = truncate_atoms(item, layout.column_width.saturating_sub(1));
                render_line_at(
                    canvas,
                    row,
                    start_column,
                    &truncated,
                    face,
                    start_column + layout.column_width,
                    metrics,
                    top_padding,
                );
            }
        }

        let scrollbar_face = &menu.menu_face;
        let marker = if row_offset >= mark_row && row_offset < mark_row + mark_height {
            "█"
        } else {
            "░"
        };
        render_string_line(
            canvas,
            row,
            layout.rect.column + layout.rect.width.saturating_sub(1),
            marker,
            scrollbar_face,
            metrics,
            top_padding,
        );
    }
}

fn render_info(
    canvas: &Canvas,
    info: &InfoState,
    menu_rect: Option<CellRect>,
    cols: usize,
    rows: usize,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    if cols == 0 || rows == 0 {
        return;
    }

    let framed = matches!(info.style, InfoStyle::Prompt | InfoStyle::Modal);
    let title_width = line_display_width(&info.title);
    let content_width = info
        .content
        .iter()
        .map(|line| line_display_width(line))
        .max()
        .unwrap_or(0);
    let inner_width = title_width.max(content_width).max(1);
    let width = (inner_width + if framed { 4 } else { 0 }).min(cols);
    let content_height = info.content.len().max(1);
    let height = (content_height + if framed { 2 } else { 0 }).min(rows);
    if width == 0 || height == 0 {
        return;
    }

    let rect = info_rect(info, menu_rect, cols, rows, width, height);
    fill_rect(canvas, rect, &info.face, metrics, top_padding);

    if framed {
        render_framed_info(canvas, info, rect, metrics, top_padding);
    } else {
        for (index, line) in info.content.iter().take(rect.height).enumerate() {
            render_line_at(
                canvas,
                rect.row + index,
                rect.column,
                line,
                &info.face,
                rect.column + rect.width,
                metrics,
                top_padding,
            );
        }
    }
}

fn render_framed_info(
    canvas: &Canvas,
    info: &InfoState,
    rect: CellRect,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    if rect.width < 2 || rect.height < 2 {
        return;
    }

    let inner_width = rect.width.saturating_sub(4);
    let mut top = String::from("╭─");
    if info.title.is_empty() || inner_width < 2 {
        top.push_str(&"─".repeat(inner_width));
    } else {
        let title = truncate_atoms(&info.title, inner_width.saturating_sub(2));
        let title_width = line_display_width(&title);
        let dash_width = inner_width.saturating_sub(title_width + 2);
        top.push_str(&"─".repeat(dash_width / 2));
        top.push('┤');
        render_string_line(
            canvas,
            rect.row,
            rect.column,
            &top,
            &info.face,
            metrics,
            top_padding,
        );
        render_line_at(
            canvas,
            rect.row,
            rect.column + top.chars().count(),
            &title,
            &info.face,
            rect.column + rect.width,
            metrics,
            top_padding,
        );
        let mut right = String::from("├");
        right.push_str(&"─".repeat(dash_width - dash_width / 2));
        right.push_str("─╮");
        render_string_line(
            canvas,
            rect.row,
            rect.column + rect.width.saturating_sub(right.chars().count()),
            &right,
            &info.face,
            metrics,
            top_padding,
        );
        return render_framed_info_body(canvas, info, rect, metrics, top_padding);
    }
    top.push_str("─╮");
    render_string_line(
        canvas,
        rect.row,
        rect.column,
        &top,
        &info.face,
        metrics,
        top_padding,
    );
    render_framed_info_body(canvas, info, rect, metrics, top_padding);
}

fn render_framed_info_body(
    canvas: &Canvas,
    info: &InfoState,
    rect: CellRect,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let inner_width = rect.width.saturating_sub(4);
    let body_rows = rect.height.saturating_sub(2);
    for row_offset in 0..body_rows {
        let row = rect.row + 1 + row_offset;
        if let Some(line) = info.content.get(row_offset) {
            render_string_line(
                canvas,
                row,
                rect.column,
                "│ ",
                &info.face,
                metrics,
                top_padding,
            );
            render_line_at(
                canvas,
                row,
                rect.column + 2,
                line,
                &info.face,
                rect.column + 2 + inner_width,
                metrics,
                top_padding,
            );
            render_string_line(
                canvas,
                row,
                rect.column + rect.width.saturating_sub(2),
                " │",
                &info.face,
                metrics,
                top_padding,
            );
        } else {
            render_string_line(
                canvas,
                row,
                rect.column,
                &format!("│ {} │", " ".repeat(inner_width)),
                &info.face,
                metrics,
                top_padding,
            );
        }
    }

    let bottom = format!("╰─{}─╯", "─".repeat(inner_width));
    render_string_line(
        canvas,
        rect.row + rect.height.saturating_sub(1),
        rect.column,
        &bottom,
        &info.face,
        metrics,
        top_padding,
    );
}

fn info_rect(
    info: &InfoState,
    menu_rect: Option<CellRect>,
    cols: usize,
    rows: usize,
    width: usize,
    height: usize,
) -> CellRect {
    match info.style {
        InfoStyle::InlineAbove => CellRect {
            row: info
                .anchor
                .line
                .saturating_sub(height)
                .min(rows.saturating_sub(height)),
            column: info.anchor.column.min(cols.saturating_sub(width)),
            width,
            height,
        },
        InfoStyle::InlineBelow | InfoStyle::Inline => CellRect {
            row: inline_popup_row(info.anchor.line, height, rows),
            column: info.anchor.column.min(cols.saturating_sub(width)),
            width,
            height,
        },
        InfoStyle::MenuDoc => {
            if let Some(menu) = menu_rect {
                let right_column = menu.column + menu.width;
                let left_column = menu.column.saturating_sub(width);
                let column = if right_column + width <= cols || right_column >= menu.column {
                    right_column.min(cols.saturating_sub(width))
                } else {
                    left_column
                };
                CellRect {
                    row: menu.row.min(rows.saturating_sub(height)),
                    column,
                    width,
                    height,
                }
            } else {
                centered_rect(cols, rows, width, height)
            }
        }
        InfoStyle::Modal => centered_rect(cols, rows, width, height),
        InfoStyle::Prompt => CellRect {
            row: rows.saturating_sub(height),
            column: cols.saturating_sub(width),
            width,
            height,
        },
    }
}

fn centered_rect(cols: usize, rows: usize, width: usize, height: usize) -> CellRect {
    CellRect {
        row: rows.saturating_sub(height) / 2,
        column: cols.saturating_sub(width) / 2,
        width,
        height,
    }
}

fn inline_popup_row(anchor_row: usize, height: usize, rows: usize) -> usize {
    let below = anchor_row.saturating_add(1);
    if below + height <= rows {
        below
    } else {
        anchor_row.saturating_sub(height)
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
        row,
        0,
        line,
        default_face,
        max_columns,
        metrics,
        top_padding,
    );
}

fn render_line_at(
    canvas: &Canvas,
    row: usize,
    start_column: usize,
    line: &[Atom],
    default_face: &Face,
    max_columns: usize,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let top = top_padding + row * metrics.cell_height;
    let mut column = start_column;
    let mut bg_paint = Paint::default();
    bg_paint.set_anti_alias(false);
    let mut fg_paint = Paint::default();
    fg_paint.set_anti_alias(true);

    for atom in line {
        let fg = resolve_face_color(&atom.face.fg, &default_face.fg, FALLBACK_FG);
        let bg = resolve_face_color(&atom.face.bg, &default_face.bg, FALLBACK_BG);
        let _ = (&atom.face.underline, &atom.face.attributes);
        bg_paint.set_color(bg.to_color());
        fg_paint.set_color(fg.to_color());

        let atom_width = atom_display_width(&atom.contents);
        if atom_width == 0 {
            continue;
        }

        let atom_start = column;
        let atom_width = atom_width.min(max_columns.saturating_sub(atom_start));
        if atom_width == 0 {
            return;
        }
        fill_cells(canvas, atom_start, top, atom_width, metrics, &bg_paint);

        for ch in atom.contents.chars() {
            if ch == '\n' {
                continue;
            }

            let span = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
            draw_glyph(canvas, column, top, ch, metrics, &fg_paint);
            column += span;
            if column >= max_columns {
                return;
            }
        }
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
        ch: Some(' '),
    });

    let fg = resolve_face_color(&cell.face.fg, &grid.default_face.fg, FALLBACK_FG);
    let bg = resolve_face_color(&cell.face.bg, &grid.default_face.bg, FALLBACK_BG);
    let top = top_padding + cursor.line * metrics.cell_height;

    let mut bg_paint = Paint::default();
    bg_paint.set_anti_alias(false).set_color(bg.to_color());
    fill_cells(canvas, cursor.column, top, 1, metrics, &bg_paint);

    let mut fg_paint = Paint::default();
    fg_paint.set_anti_alias(true).set_color(fg.to_color());
    if let Some(ch) = cell.ch {
        draw_glyph(canvas, cursor.column, top, ch, metrics, &fg_paint);
    }

    draw_cursor_outline(canvas, cursor.column, top, metrics, fg);
}

fn cursor_cell(line: Option<&[Atom]>, target_column: usize) -> Option<CursorCell> {
    let line = line?;
    let mut column = 0;

    for atom in line {
        for ch in atom.contents.chars() {
            if ch == '\n' {
                if column == target_column {
                    return Some(CursorCell {
                        face: atom.face.clone(),
                        ch: Some(' '),
                    });
                }
                continue;
            }

            let span = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
            if target_column >= column && target_column < column + span {
                return Some(CursorCell {
                    face: atom.face.clone(),
                    ch: Some(ch),
                });
            }
            column += span;
        }
    }

    None
}

pub fn atom_display_width(contents: &str) -> usize {
    contents
        .chars()
        .filter(|&ch| ch != '\n')
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(1).max(1))
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
    let bg = resolve_face_color(&default_face.bg, &default_face.bg, FALLBACK_BG).to_color();
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

fn fill_line_segment(
    canvas: &Canvas,
    row: usize,
    column: usize,
    width: usize,
    face: &Face,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let bg = resolve_face_color(&face.bg, &face.bg, FALLBACK_BG).to_color();
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

fn fill_rect(
    canvas: &Canvas,
    rect: CellRect,
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

fn render_string_line(
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
        row,
        column,
        &atoms,
        default_face,
        column + atom_display_width(text),
        metrics,
        top_padding,
    );
}

fn truncate_atoms(line: &[Atom], max_width: usize) -> Vec<Atom> {
    let mut remaining = max_width;
    let mut result = Vec::new();

    for atom in line {
        if remaining == 0 {
            break;
        }

        let mut contents = String::new();
        for ch in atom.contents.chars() {
            if ch == '\n' {
                continue;
            }
            let width = UnicodeWidthChar::width(ch).unwrap_or(1).max(1);
            if width > remaining {
                break;
            }
            contents.push(ch);
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

fn draw_cursor_outline(
    canvas: &Canvas,
    column: usize,
    top: usize,
    metrics: &CellMetrics,
    color: Rgb,
) {
    let left = PADDING + column * metrics.cell_width;
    let stroke = (metrics.cell_width.min(metrics.cell_height) / 10).max(1) as f32;
    let rect = Rect::from_xywh(
        left as f32 + stroke / 2.0,
        top as f32 + stroke / 2.0,
        metrics.cell_width as f32 - stroke,
        metrics.cell_height as f32 - stroke,
    );

    let mut paint = Paint::default();
    paint
        .set_anti_alias(false)
        .set_color(color.to_color())
        .set_style(skia_safe::paint::Style::Stroke)
        .set_stroke_width(stroke);
    canvas.draw_rect(rect, &paint);
}

fn draw_glyph(
    canvas: &Canvas,
    column: usize,
    top: usize,
    ch: char,
    metrics: &CellMetrics,
    paint: &Paint,
) {
    if ch.is_control() {
        return;
    }

    let left = PADDING + column * metrics.cell_width;
    let baseline = top as f32 + metrics.baseline_offset;
    let mut utf8 = [0; 4];
    let text = ch.encode_utf8(&mut utf8);
    canvas.draw_str(text, (left as f32, baseline), &metrics.font, paint);
}

fn resolve_face_color(color: &str, inherited: &str, fallback: Rgb) -> Rgb {
    if color == "default" {
        resolve_color(inherited, fallback)
    } else {
        resolve_color(color, fallback)
    }
}

fn resolve_color(color: &str, fallback: Rgb) -> Rgb {
    if color == "default" {
        return fallback;
    }

    if let Some(rgb) = parse_prefixed_color(color) {
        return rgb;
    }

    match color {
        "black" => Rgb::new(0x00, 0x00, 0x00),
        "white" => Rgb::new(0xff, 0xff, 0xff),
        "red" => Rgb::new(0xff, 0x55, 0x55),
        "green" => Rgb::new(0x50, 0xfa, 0x7b),
        "yellow" => Rgb::new(0xf1, 0xfa, 0x8c),
        "blue" => Rgb::new(0x62, 0xd6, 0xe8),
        "magenta" => Rgb::new(0xff, 0x79, 0xc6),
        "cyan" => Rgb::new(0x8b, 0xe9, 0xfd),
        _ => fallback,
    }
}

fn parse_prefixed_color(value: &str) -> Option<Rgb> {
    if let Some(rgb) = value.strip_prefix("rgb:").and_then(parse_hex_color) {
        return Some(rgb);
    }
    if let Some(rgb) = value.strip_prefix("rgba:").and_then(parse_rgba_color) {
        return Some(rgb);
    }
    parse_hex_color(value)
}

fn parse_hex_color(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

fn parse_rgba_color(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 8 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

pub fn load_renderer(config: &AppConfig) -> Renderer {
    Renderer {
        font_mgr: FontMgr::new(),
        preferred_font_family: config.font_family.clone(),
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

    pub fn metrics(&self, scale_factor: f64) -> CellMetrics {
        let cache_key = scale_factor.to_bits();
        if let Some((cached_key, metrics)) = self.metrics_cache.borrow().as_ref() {
            if *cached_key == cache_key {
                return metrics.clone();
            }
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
    use crate::kakoune_messages::{Coord, StatusStyle};

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
    fn parses_rgba_colors_by_ignoring_alpha() {
        assert_eq!(
            parse_prefixed_color("rgba:ffffff80"),
            Some(Rgb::new(0xff, 0xff, 0xff))
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
        assert_eq!(cursor.ch, Some(' '));
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
        assert_eq!(cursor.ch, Some('c'));
        assert_eq!(cursor.face.bg, "white");
    }

    #[test]
    fn prompt_menu_layout_uses_multiple_columns() {
        let menu = MenuState {
            items: vec![menu_item("alpha"); 12],
            anchor: Coord { line: 0, column: 0 },
            selected: Some(0),
            selected_face: Face::default(),
            menu_face: Face::default(),
            style: MenuStyle::Prompt,
        };

        let layout = menu_layout(&menu, 40, 12).expect("prompt layout");
        assert_eq!(layout.rect.width, 40);
        assert_eq!(layout.rect.height, 2);
        assert_eq!(layout.visible_columns, 6);
        assert_eq!(layout.total_columns, 6);
        assert_eq!(layout.column_width, 6);
    }

    #[test]
    fn prompt_menu_scrolls_columns_to_selected_item() {
        let menu = MenuState {
            items: vec![menu_item("abcdefghij"); 40],
            anchor: Coord { line: 0, column: 0 },
            selected: Some(39),
            selected_face: Face::default(),
            menu_face: Face::default(),
            style: MenuStyle::Prompt,
        };

        let layout = menu_layout(&menu, 30, 10).expect("prompt layout");
        assert_eq!(layout.visible_columns, 2);
        assert_eq!(layout.total_columns, 4);
        assert_eq!(layout.first_visible_column, 2);
    }

    #[test]
    fn search_menu_tracks_first_visible_item_from_selection() {
        let menu = MenuState {
            items: vec![
                menu_item("aaaa"),
                menu_item("bbbb"),
                menu_item("cccc"),
                menu_item("dddd"),
            ],
            anchor: Coord { line: 0, column: 0 },
            selected: Some(2),
            selected_face: Face::default(),
            menu_face: Face::default(),
            style: MenuStyle::Search,
        };

        let layout = menu_layout(&menu, 20, 8).expect("search layout");
        assert!(layout.single_row);
        assert_eq!(layout.first_visible_column, 2);
    }

    fn menu_item(text: &str) -> Vec<Atom> {
        vec![Atom {
            face: Face::default(),
            contents: text.into(),
        }]
    }
}
