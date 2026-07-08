use skia_safe::Canvas;

use crate::app::{InfoState, MenuState};
use crate::kakoune_messages::{InfoStyle, MenuStyle};
use crate::layout::bottom_overlay_top;
use crate::render::{
    CellMetrics, DrawOrigin, LineRenderPosition, fill_line_segment, fill_line_segment_at_top, fill_rect,
    fill_rect_at_top, line_display_width, render_line_at, render_line_at_top, render_string_line,
    render_string_line_at_top, truncate_atoms,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CellRect {
    pub row: usize,
    pub column: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MenuLayout {
    rect: CellRect,
    top: usize,
    visible_columns: usize,
    total_columns: usize,
    rows_per_column: usize,
    column_width: usize,
    first_visible_item: usize,
    is_single_row: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PopupRect {
    pub rect: CellRect,
    pub top: usize,
}

pub(super) fn render_menu(
    canvas: &Canvas,
    menu: &MenuState,
    cols: usize,
    rows: usize,
    metrics: &CellMetrics,
    top_padding: usize,
    window_height: usize,
) -> Option<PopupRect> {
    if cols == 0 || rows == 0 || menu.items.is_empty() {
        return None;
    }

    let layout = menu_layout(menu, cols, rows, window_height, metrics.cell_height)?;
    if matches!(menu.style, MenuStyle::Inline) {
        fill_rect(
            canvas,
            layout.rect,
            &menu.menu_face,
            metrics,
            DrawOrigin::Grid { top_padding },
        );
    } else {
        fill_rect_at_top(canvas, layout.rect, &menu.menu_face, metrics, layout.top);
    }

    if layout.is_single_row {
        render_single_row_menu(canvas, menu, layout, metrics, top_padding);
    } else if layout.visible_columns == 1 {
        render_single_column_menu(canvas, menu, layout, metrics, top_padding);
    } else {
        render_multi_column_menu(canvas, menu, layout, metrics, top_padding);
    }

    Some(PopupRect {
        rect: layout.rect,
        top: layout.top,
    })
}

fn menu_layout(
    menu: &MenuState,
    cols: usize,
    rows: usize,
    window_height: usize,
    cell_height: usize,
) -> Option<MenuLayout> {
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
            let max_height = rows
                .min(10)
                .min(anchor_line.max(rows.saturating_sub(anchor_line).saturating_sub(1)));
            let height = item_count.max(1).min(max_height);
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
                top: 0,
                visible_columns: 1,
                total_columns: item_count.div_ceil(height.max(1)),
                rows_per_column: height,
                column_width: width,
                first_visible_item: 0,
                is_single_row: false,
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
                top: bottom_overlay_top(window_height, cell_height, 1),
                visible_columns: 0,
                total_columns: item_count,
                rows_per_column: 1,
                column_width: width.saturating_sub(3),
                first_visible_item: menu_first_search_item(menu, width.saturating_sub(3)),
                is_single_row: true,
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
                top: bottom_overlay_top(window_height, cell_height, height + 1),
                visible_columns,
                total_columns,
                rows_per_column: height,
                column_width: (cols.saturating_sub(1) / visible_columns).max(1),
                first_visible_item: first_visible_column,
                is_single_row: false,
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
    let inline = layout.top == 0;
    for (index, item) in menu.items.iter().take(layout.rect.height).enumerate() {
        let face = if menu.selected == Some(index) {
            &menu.selected_face
        } else {
            &menu.menu_face
        };
        let row_top = layout.top + index * metrics.cell_height;
        if inline {
            fill_line_segment(
                canvas,
                layout.rect.row + index,
                layout.rect.column,
                layout.rect.width,
                face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
            render_line_at(
                canvas,
                LineRenderPosition {
                    row: layout.rect.row + index,
                    start_column: layout.rect.column,
                    max_columns: layout.rect.column + layout.rect.width,
                },
                item,
                face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
        } else {
            fill_line_segment_at_top(
                canvas,
                layout.rect.column,
                layout.rect.width,
                face,
                metrics,
                row_top,
            );
            render_line_at_top(
                canvas,
                LineRenderPosition {
                    row: 0,
                    start_column: layout.rect.column,
                    max_columns: layout.rect.column + layout.rect.width,
                },
                item,
                face,
                metrics,
                row_top,
            );
        }
    }
}

fn render_single_row_menu(
    canvas: &Canvas,
    menu: &MenuState,
    layout: MenuLayout,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let mut column = layout.rect.column;
    let inline = layout.top == 0;

    if layout.first_visible_item > 0 {
        if inline {
            render_string_line(
                canvas,
                layout.rect.row,
                column,
                "< ",
                &menu.menu_face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
        } else {
            render_string_line_at_top(canvas, column, "< ", &menu.menu_face, metrics, layout.top);
        }
    }
    column += 2;

    let end_column = layout.rect.column + layout.rect.width.saturating_sub(2);
    let mut index = layout.first_visible_item;
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
        if inline {
            render_line_at(
                canvas,
                LineRenderPosition {
                    row: layout.rect.row,
                    start_column: column,
                    max_columns: end_column,
                },
                &truncated,
                face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
        } else {
            render_line_at_top(
                canvas,
                LineRenderPosition {
                    row: 0,
                    start_column: column,
                    max_columns: end_column,
                },
                &truncated,
                face,
                metrics,
                layout.top,
            );
        }

        if item_width > available_width {
            if inline {
                render_string_line(
                    canvas,
                    layout.rect.row,
                    end_column,
                    "…",
                    &menu.menu_face,
                    metrics,
                    DrawOrigin::Grid { top_padding },
                );
            } else {
                render_string_line_at_top(
                    canvas,
                    end_column,
                    "…",
                    &menu.menu_face,
                    metrics,
                    layout.top,
                );
            }
            break;
        }

        column += item_width;
        if column < end_column {
            if inline {
                render_string_line(
                    canvas,
                    layout.rect.row,
                    column,
                    " ",
                    &menu.menu_face,
                    metrics,
                    DrawOrigin::Grid { top_padding },
                );
            } else {
                render_string_line_at_top(
                    canvas,
                    column,
                    " ",
                    &menu.menu_face,
                    metrics,
                    layout.top,
                );
            }
            column += 1;
        }
        index += 1;
    }

    if index < menu.items.len() {
        if inline {
            render_string_line(
                canvas,
                layout.rect.row,
                layout.rect.column + layout.rect.width.saturating_sub(1),
                ">",
                &menu.menu_face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
        } else {
            render_string_line_at_top(
                canvas,
                layout.rect.column + layout.rect.width.saturating_sub(1),
                ">",
                &menu.menu_face,
                metrics,
                layout.top,
            );
        }
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
            .saturating_mul(layout.first_visible_item)
            / (layout.total_columns - layout.visible_columns)
    } else {
        0
    };
    let inline = layout.top == 0;

    for row_offset in 0..layout.rect.height {
        let row = layout.rect.row + row_offset;
        let row_top = layout.top + row_offset * metrics.cell_height;
        for col_offset in 0..layout.visible_columns {
            let column_index = layout.first_visible_item + col_offset;
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
            if inline {
                fill_line_segment(
                    canvas,
                    row,
                    start_column,
                    layout.column_width,
                    face,
                    metrics,
                    DrawOrigin::Grid { top_padding },
                );
            } else {
                fill_line_segment_at_top(
                    canvas,
                    start_column,
                    layout.column_width,
                    face,
                    metrics,
                    row_top,
                );
            }

            if let Some(item) = menu.items.get(item_index) {
                let truncated = truncate_atoms(item, layout.column_width.saturating_sub(1));
                if inline {
                    render_line_at(
                        canvas,
                        LineRenderPosition {
                            row,
                            start_column,
                            max_columns: start_column + layout.column_width,
                        },
                        &truncated,
                        face,
                        metrics,
                        DrawOrigin::Grid { top_padding },
                    );
                } else {
                    render_line_at_top(
                        canvas,
                        LineRenderPosition {
                            row: 0,
                            start_column,
                            max_columns: start_column + layout.column_width,
                        },
                        &truncated,
                        face,
                        metrics,
                        row_top,
                    );
                }
            }
        }

        let scrollbar_face = &menu.menu_face;
        let marker = if row_offset >= mark_row && row_offset < mark_row + mark_height {
            "█"
        } else {
            "░"
        };
        if inline {
            render_string_line(
                canvas,
                row,
                layout.rect.column + layout.rect.width.saturating_sub(1),
                marker,
                scrollbar_face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
        } else {
            render_string_line_at_top(
                canvas,
                layout.rect.column + layout.rect.width.saturating_sub(1),
                marker,
                scrollbar_face,
                metrics,
                row_top,
            );
        }
    }
}

pub(super) fn render_info(
    canvas: &Canvas,
    info: &InfoState,
    menu_rect: Option<PopupRect>,
    cols: usize,
    rows: usize,
    metrics: &CellMetrics,
    top_padding: usize,
    window_height: usize,
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

    let layout = info_rect_with_window(
        info,
        menu_rect,
        cols,
        rows,
        width,
        height,
        window_height,
        metrics.cell_height,
    );
    if matches!(info.style, InfoStyle::Prompt) {
        fill_rect_at_top(canvas, layout.rect, &info.face, metrics, layout.top);
    } else {
        fill_rect(
            canvas,
            layout.rect,
            &info.face,
            metrics,
            DrawOrigin::Grid { top_padding },
        );
    }

    if framed {
        render_framed_info(canvas, info, layout, metrics, top_padding);
    } else {
        for (index, line) in info.content.iter().take(layout.rect.height).enumerate() {
            if matches!(info.style, InfoStyle::Prompt) {
                render_line_at_top(
                    canvas,
                    LineRenderPosition {
                        row: 0,
                        start_column: layout.rect.column,
                        max_columns: layout.rect.column + layout.rect.width,
                    },
                    line,
                    &info.face,
                    metrics,
                    layout.top + index * metrics.cell_height,
                );
            } else {
                render_line_at(
                    canvas,
                    LineRenderPosition {
                        row: layout.rect.row + index,
                        start_column: layout.rect.column,
                        max_columns: layout.rect.column + layout.rect.width,
                    },
                    line,
                    &info.face,
                    metrics,
                    DrawOrigin::Grid { top_padding },
                );
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InfoLayout {
    rect: CellRect,
    top: usize,
}

fn render_framed_info(
    canvas: &Canvas,
    info: &InfoState,
    layout: InfoLayout,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let rect = layout.rect;
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
        let dash_width = inner_width.saturating_sub(title_width.saturating_add(2));
        let top_width = top.chars().count();
        let max_columns = rect.column.saturating_add(rect.width);
        top.push_str(&"─".repeat(dash_width / 2));
        top.push('┤');
        if matches!(info.style, InfoStyle::Prompt) {
            render_string_line_at_top(canvas, rect.column, &top, &info.face, metrics, layout.top);
            render_line_at_top(
                canvas,
                LineRenderPosition {
                    row: 0,
                    start_column: rect.column.saturating_add(top_width),
                    max_columns,
                },
                &title,
                &info.face,
                metrics,
                layout.top,
            );
        } else {
            render_string_line(
                canvas,
                rect.row,
                rect.column,
                &top,
                &info.face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
            render_line_at(
                canvas,
                LineRenderPosition {
                    row: rect.row,
                    start_column: rect.column.saturating_add(top_width),
                    max_columns,
                },
                &title,
                &info.face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
        }
        let mut right = String::from("├");
        right.push_str(&"─".repeat(dash_width - dash_width / 2));
        right.push_str("─╮");
        let right_column = rect
            .column
            .saturating_add(rect.width.saturating_sub(right.chars().count()));
        if matches!(info.style, InfoStyle::Prompt) {
            render_string_line_at_top(
                canvas,
                right_column,
                &right,
                &info.face,
                metrics,
                layout.top,
            );
        } else {
            render_string_line(
                canvas,
                rect.row,
                right_column,
                &right,
                &info.face,
                metrics,
                DrawOrigin::Grid { top_padding },
            );
        }
        return render_framed_info_body(canvas, info, layout, metrics, top_padding);
    }
    top.push_str("─╮");
    if matches!(info.style, InfoStyle::Prompt) {
        render_string_line_at_top(canvas, rect.column, &top, &info.face, metrics, layout.top);
    } else {
        render_string_line(
            canvas,
            rect.row,
            rect.column,
            &top,
            &info.face,
            metrics,
            DrawOrigin::Grid { top_padding },
        );
    }
    render_framed_info_body(canvas, info, layout, metrics, top_padding);
}

fn render_framed_info_body(
    canvas: &Canvas,
    info: &InfoState,
    layout: InfoLayout,
    metrics: &CellMetrics,
    top_padding: usize,
) {
    let rect = layout.rect;
    let inner_width = rect.width.saturating_sub(4);
    let body_rows = rect.height.saturating_sub(2);
    let content_start = rect.column.saturating_add(2);
    let right_border_column = rect.column.saturating_add(rect.width.saturating_sub(2));
    for row_offset in 0..body_rows {
        let row = rect.row.saturating_add(1).saturating_add(row_offset);
        let row_top = layout
            .top
            .saturating_add(row_offset.saturating_add(1).saturating_mul(metrics.cell_height));
        if let Some(line) = info.content.get(row_offset) {
            if matches!(info.style, InfoStyle::Prompt) {
                render_string_line_at_top(canvas, rect.column, "│ ", &info.face, metrics, row_top);
                render_line_at_top(
                    canvas,
                    LineRenderPosition {
                        row: 0,
                        start_column: content_start,
                        max_columns: content_start.saturating_add(inner_width),
                    },
                    line,
                    &info.face,
                    metrics,
                    row_top,
                );
                render_string_line_at_top(
                    canvas,
                    right_border_column,
                    " │",
                    &info.face,
                    metrics,
                    row_top,
                );
            } else {
                render_string_line(
                    canvas,
                    row,
                    rect.column,
                    "│ ",
                    &info.face,
                    metrics,
                    DrawOrigin::Grid { top_padding },
                );
                render_line_at(
                    canvas,
                    LineRenderPosition {
                        row,
                        start_column: content_start,
                        max_columns: content_start.saturating_add(inner_width),
                    },
                    line,
                    &info.face,
                    metrics,
                    DrawOrigin::Grid { top_padding },
                );
                render_string_line(
                    canvas,
                    row,
                    right_border_column,
                    " │",
                    &info.face,
                    metrics,
                    DrawOrigin::Grid { top_padding },
                );
            }
        } else {
            let blank = format!("│ {} │", " ".repeat(inner_width));
            if matches!(info.style, InfoStyle::Prompt) {
                render_string_line_at_top(
                    canvas,
                    rect.column,
                    &blank,
                    &info.face,
                    metrics,
                    row_top,
                );
            } else {
                render_string_line(
                    canvas,
                    row,
                    rect.column,
                    &blank,
                    &info.face,
                    metrics,
                    DrawOrigin::Grid { top_padding },
                );
            }
        }
    }

    let bottom = format!("╰─{}─╯", "─".repeat(inner_width));
    if matches!(info.style, InfoStyle::Prompt) {
        render_string_line_at_top(
            canvas,
            rect.column,
            &bottom,
            &info.face,
            metrics,
            layout.top.saturating_add(
                rect.height
                    .saturating_sub(1)
                    .saturating_mul(metrics.cell_height),
            ),
        );
    } else {
        render_string_line(
            canvas,
            rect.row.saturating_add(rect.height.saturating_sub(1)),
            rect.column,
            &bottom,
            &info.face,
            metrics,
            DrawOrigin::Grid { top_padding },
        );
    }
}

fn info_rect_with_window(
    info: &InfoState,
    menu_rect: Option<PopupRect>,
    cols: usize,
    rows: usize,
    width: usize,
    height: usize,
    window_height: usize,
    cell_height: usize,
) -> InfoLayout {
    match info.style {
        InfoStyle::InlineAbove => {
            let rect = CellRect {
                row: info
                    .anchor
                    .line
                    .saturating_sub(height)
                    .min(rows.saturating_sub(height)),
                column: info.anchor.column.min(cols.saturating_sub(width)),
                width,
                height,
            };
            InfoLayout { rect, top: 0 }
        }
        InfoStyle::InlineBelow | InfoStyle::Inline => {
            let rect = CellRect {
                row: inline_popup_row(info.anchor.line, height, rows),
                column: info.anchor.column.min(cols.saturating_sub(width)),
                width,
                height,
            };
            InfoLayout { rect, top: 0 }
        }
        InfoStyle::MenuDoc => {
            if let Some(menu) = menu_rect {
                let right_column = menu.rect.column + menu.rect.width;
                let right_max_width = cols.saturating_sub(right_column);
                let left_max_width = menu.rect.column;
                let left_column = menu.rect.column.saturating_sub(width);
                let column = if width <= right_max_width || right_max_width >= left_max_width {
                    right_column.min(cols.saturating_sub(width))
                } else {
                    left_column
                };
                let rect = CellRect {
                    row: menu.rect.row.min(rows.saturating_sub(height)),
                    column,
                    width,
                    height,
                };
                InfoLayout { rect, top: 0 }
            } else {
                centered_rect(cols, rows, width, height)
            }
        }
        InfoStyle::Modal => centered_rect(cols, rows, width, height),
        InfoStyle::Prompt => {
            let row = menu_rect
                .map(|menu| menu.rect.row.saturating_sub(height))
                .unwrap_or_else(|| rows.saturating_sub(height));
            let top = menu_rect
                .map(|menu| menu.top.saturating_sub(height * cell_height))
                .unwrap_or_else(|| bottom_overlay_top(window_height, cell_height, height + 1));
            let rect = CellRect {
                row,
                column: cols.saturating_sub(width),
                width,
                height,
            };
            InfoLayout { rect, top }
        }
    }
}

fn centered_rect(cols: usize, rows: usize, width: usize, height: usize) -> InfoLayout {
    let rect = CellRect {
        row: rows.saturating_sub(height) / 2,
        column: cols.saturating_sub(width) / 2,
        width,
        height,
    };
    InfoLayout { rect, top: 0 }
}

fn inline_popup_row(anchor_row: usize, height: usize, rows: usize) -> usize {
    let below = anchor_row.saturating_add(1);
    if below + height <= rows {
        below
    } else {
        anchor_row.saturating_sub(height)
    }
}

#[cfg(test)]
mod tests {
    use crate::kakoune_messages::{Atom, Coord, Face};

    use super::*;

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

        let layout = menu_layout(&menu, 40, 12, 240, 18).expect("prompt layout");
        assert_eq!(layout.rect.width, 40);
        assert_eq!(layout.rect.height, 2);
        assert_eq!(layout.visible_columns, 6);
        assert_eq!(layout.total_columns, 6);
        assert_eq!(layout.column_width, 6);
        assert_eq!(layout.top, bottom_overlay_top(240, 18, 3));
    }

    #[test]
    fn inline_menu_height_is_limited_by_space_around_anchor() {
        let menu = MenuState {
            items: vec![menu_item("alpha"); 10],
            anchor: Coord { line: 1, column: 3 },
            selected: Some(0),
            selected_face: Face::default(),
            menu_face: Face::default(),
            style: MenuStyle::Inline,
        };

        let layout = menu_layout(&menu, 40, 8, 240, 18).expect("inline layout");
        assert_eq!(layout.rect.height, 6);
        assert_eq!(layout.rect.row, 2);
        assert_eq!(layout.rect.column, 3);
    }

    #[test]
    fn inline_menu_near_bottom_opens_above_anchor() {
        let menu = MenuState {
            items: vec![menu_item("alpha"); 10],
            anchor: Coord { line: 6, column: 4 },
            selected: Some(0),
            selected_face: Face::default(),
            menu_face: Face::default(),
            style: MenuStyle::Inline,
        };

        let layout = menu_layout(&menu, 40, 8, 240, 18).expect("inline layout");
        assert_eq!(layout.rect.height, 6);
        assert_eq!(layout.rect.row, 0);
        assert_eq!(layout.rect.column, 4);
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

        let layout = menu_layout(&menu, 30, 10, 240, 18).expect("prompt layout");
        assert_eq!(layout.visible_columns, 2);
        assert_eq!(layout.total_columns, 4);
        assert_eq!(layout.first_visible_item, 2);
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

        let layout = menu_layout(&menu, 20, 8, 240, 18).expect("search layout");
        assert!(layout.is_single_row);
        assert_eq!(layout.first_visible_item, 2);
        assert_eq!(layout.top, bottom_overlay_top(240, 18, 1));
    }

    #[test]
    fn prompt_info_is_placed_above_prompt_menu() {
        let info = InfoState {
            title: Vec::new(),
            content: vec![menu_item("help")],
            anchor: Coord { line: 0, column: 0 },
            face: Face::default(),
            style: InfoStyle::Prompt,
        };
        let menu = PopupRect {
            rect: CellRect {
                row: 8,
                column: 0,
                width: 30,
                height: 2,
            },
            top: bottom_overlay_top(240, 18, 3),
        };

        let rect = info_rect_with_window(&info, Some(menu), 30, 10, 8, 3, 240, 18);
        assert_eq!(rect.rect.row, 5);
        assert_eq!(rect.rect.column, 22);
        assert_eq!(rect.top, bottom_overlay_top(240, 18, 6));
    }

    #[test]
    fn menu_doc_info_uses_left_side_when_right_side_is_too_narrow() {
        let info = InfoState {
            title: Vec::new(),
            content: vec![menu_item("help")],
            anchor: Coord { line: 0, column: 0 },
            face: Face::default(),
            style: InfoStyle::MenuDoc,
        };
        let menu = PopupRect {
            rect: CellRect {
                row: 2,
                column: 24,
                width: 6,
                height: 3,
            },
            top: 0,
        };

        let rect = info_rect_with_window(&info, Some(menu), 30, 10, 10, 4, 240, 18);
        assert_eq!(rect.rect.row, 2);
        assert_eq!(rect.rect.column, 14);
    }

    #[test]
    fn prompt_menu_bottom_edge_stays_fixed_when_cell_height_changes() {
        let menu = MenuState {
            items: vec![menu_item("alpha"); 6],
            anchor: Coord { line: 0, column: 0 },
            selected: Some(0),
            selected_face: Face::default(),
            menu_face: Face::default(),
            style: MenuStyle::Prompt,
        };

        let layout_small = menu_layout(&menu, 40, 12, 240, 18).expect("prompt layout");
        let layout_large = menu_layout(&menu, 40, 9, 240, 24).expect("prompt layout");

        assert_eq!(layout_small.top + layout_small.rect.height * 18, 210);
        assert_eq!(layout_large.top + layout_large.rect.height * 24, 204);
    }

    #[test]
    fn framed_prompt_math_uses_saturating_column_arithmetic() {
        let rect = CellRect {
            row: usize::MAX - 2,
            column: usize::MAX - 3,
            width: 4,
            height: 3,
        };
        let max_columns = rect.column.saturating_add(rect.width);
        let content_start = rect.column.saturating_add(2);

        assert_eq!(max_columns, usize::MAX);
        assert_eq!(content_start, usize::MAX - 1);
        assert_eq!(content_start.saturating_add(rect.width.saturating_sub(4)), usize::MAX - 1);
    }

    #[test]
    fn framed_prompt_border_columns_saturate_at_usize_max() {
        let rect = CellRect {
            row: usize::MAX - 1,
            column: usize::MAX - 2,
            width: 4,
            height: 3,
        };
        let right = "├─╮";

        let title_border = rect
            .column
            .saturating_add(rect.width.saturating_sub(right.chars().count()));
        let body_border = rect.column.saturating_add(rect.width.saturating_sub(2));
        let bottom_row = rect.row.saturating_add(rect.height.saturating_sub(1));

        assert_eq!(title_border, usize::MAX - 1);
        assert_eq!(body_border, usize::MAX);
        assert_eq!(bottom_row, usize::MAX);
    }

    fn menu_item(text: &str) -> Vec<Atom> {
        vec![Atom {
            face: Face::default(),
            contents: text.into(),
        }]
    }
}
