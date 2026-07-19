use super::*;

const BAND_GAP_ROWS: usize = 1;

struct MenuBlock {
    width: usize,
    rows: Vec<Vec<ToolbarSpan>>,
}

#[derive(Clone, Copy)]
struct BlockPlacement {
    block: usize,
    row: usize,
    column: usize,
}

pub(super) fn hierarchical_menu_row_count(toolbar: &ToolbarState, box_width: usize) -> usize {
    hierarchical_menu_rows(toolbar, box_width).len()
}

pub(super) fn hierarchical_menu_spans(
    toolbar: &ToolbarState,
    row: usize,
    box_width: usize,
) -> Vec<ToolbarSpan> {
    let relative_row = row - MENU_FIRST_ROW;
    hierarchical_menu_rows(toolbar, box_width)
        .get(relative_row)
        .cloned()
        .unwrap_or_default()
}

fn hierarchical_menu_rows(toolbar: &ToolbarState, box_width: usize) -> Vec<Vec<ToolbarSpan>> {
    let blocks = category_blocks(toolbar);
    let placements = block_placements(toolbar, box_width, &blocks);
    let row_count = placements
        .iter()
        .map(|placement| placement.row + blocks[placement.block].rows.len())
        .max()
        .unwrap_or(0);
    let mut rows = vec![Vec::new(); row_count];
    for placement in placements {
        let block = &blocks[placement.block];
        for (local_row, block_row) in block.rows.iter().enumerate() {
            let row = &mut rows[placement.row + local_row];
            pad_spans_to_width(row, placement.column);
            row.extend(block_row.iter().cloned());
        }
    }
    rows
}

fn category_blocks(toolbar: &ToolbarState) -> Vec<MenuBlock> {
    let layout = toolbar
        .layout()
        .expect("hierarchical menu layout excludes Utilities");
    (0..layout.labels.len())
        .map(|category| category_block(toolbar, category))
        .collect()
}

fn category_block(toolbar: &ToolbarState, category: usize) -> MenuBlock {
    let layout = toolbar
        .layout()
        .expect("hierarchical menu layout excludes Utilities");
    let options = layout.options[category];
    let page_ranges = layout.page_ranges(category);
    let prefix_width =
        submenu_prefix_width_for_pages(layout.labels[category], category, page_ranges.len());
    let column_widths = submenu_option_column_widths_for_pages(options, &page_ranges);
    let width = submenu_cell_width_for_columns(prefix_width, &column_widths);
    let mut rows = Vec::with_capacity(1 + page_ranges.len());
    let mut header = Vec::new();
    push_header(
        &mut header,
        layout.labels[category],
        prefix_width,
        &column_widths,
    );
    pad_spans_to_width(&mut header, width);
    rows.push(header);
    for (page, range) in page_ranges.iter().cloned().enumerate() {
        let mut row = Vec::new();
        push_page(
            &mut row,
            toolbar,
            category,
            page,
            page_ranges.len(),
            prefix_width,
            options,
            range,
            &column_widths,
            layout.selected[category],
            layout.exclusive_submenu,
        );
        pad_spans_to_width(&mut row, width);
        rows.push(row);
    }
    MenuBlock { width, rows }
}

fn block_placements(
    toolbar: &ToolbarState,
    box_width: usize,
    blocks: &[MenuBlock],
) -> Vec<BlockPlacement> {
    let available_width = toolbar_content_width(box_width);
    let auxiliary_width = toolbar.auxiliary_panel_width_for_width(box_width);
    let auxiliary_height = toolbar.auxiliary_panel_row_count_for_width(box_width);
    let first_band_width = available_width.saturating_sub(
        auxiliary_width + usize::from(auxiliary_width > 0) * UnicodeWidthStr::width(GAP),
    );
    let gap_width = UnicodeWidthStr::width(GAP);
    let mut placements = Vec::with_capacity(blocks.len());
    let mut band_row = 0;
    let mut band_width = 0usize;
    let mut band_height = 0;
    for (block_index, block) in blocks.iter().enumerate() {
        let band_limit = if band_row == 0 {
            first_band_width
        } else {
            available_width
        };
        let width_with_gap = block.width + usize::from(band_width > 0) * gap_width;
        if (band_width > 0 && band_width.saturating_add(width_with_gap) > band_limit)
            || (band_width == 0 && band_row == 0 && block.width > band_limit)
        {
            let completed_height = if band_row == 0 {
                auxiliary_height.max(band_height)
            } else {
                band_height
            };
            if completed_height > 0 {
                band_row += completed_height + BAND_GAP_ROWS;
            }
            band_width = 0;
            band_height = 0;
        }
        let column = band_width + usize::from(band_width > 0) * gap_width;
        placements.push(BlockPlacement {
            block: block_index,
            row: band_row,
            column,
        });
        band_width = column.saturating_add(block.width);
        band_height = band_height.max(block.rows.len());
    }
    placements
}

fn push_header(spans: &mut Vec<ToolbarSpan>, label: &str, prefix_width: usize, widths: &[usize]) {
    let label = format!("{label}:");
    spans.push(bold_prefix_span(
        pad_right_to_width(label.clone(), prefix_width),
        &label,
    ));
    for (position, width) in widths.iter().copied().enumerate() {
        push_separator(spans, position);
        spans.push(plain_span(pad_right_to_width(
            ((position + 1) % OPTIONS_PER_PAGE).to_string(),
            width,
        )));
    }
}

#[allow(clippy::too_many_arguments)]
fn push_page(
    spans: &mut Vec<ToolbarSpan>,
    toolbar: &ToolbarState,
    category: usize,
    page: usize,
    page_count: usize,
    prefix_width: usize,
    options: &[&str],
    range: std::ops::Range<usize>,
    widths: &[usize],
    selected: usize,
    exclusive_submenu: Option<usize>,
) {
    let path = submenu_path_for_pages(category, page, page_count);
    let highlighted_prefix = match toolbar.pending_shortcut() {
        Some(PendingShortcut::Category(pending)) if pending == category => {
            Some(format!("{}.", category + 2))
        }
        Some(PendingShortcut::Option {
            category: pending_category,
            page: pending_page,
        }) if pending_category == category && pending_page == page => Some(path.clone()),
        _ => None,
    };
    push_shortcut_path(spans, &path, prefix_width, highlighted_prefix.as_deref());

    for (position, option_index) in range.enumerate() {
        let option = options[option_index];
        push_separator(spans, position);
        spans.push(ToolbarSpan {
            contents: option.to_string(),
            bold_prefix: 0,
            selected: option_index == selected
                && exclusive_submenu.is_none_or(|active| active == category),
            highlighted: false,
            tooltip: false,
            action: Some(ToolbarAction::SelectSubmenu {
                submenu: category,
                option: option_index,
            }),
            shift_action: None,
            right_aligned: false,
            foreground: None,
        });
        let padding = widths[position].saturating_sub(UnicodeWidthStr::width(option));
        if padding > 0 {
            spans.push(plain_span(" ".repeat(padding)));
        }
    }
}

fn push_separator(spans: &mut Vec<ToolbarSpan>, position: usize) {
    if position > 0 {
        spans.push(plain_span(" ".to_string()));
    }
}
