use super::*;

#[derive(Clone, Copy)]
struct CategoryPlacement {
    category: usize,
    row: usize,
}

pub(super) fn hierarchical_menu_row_count(toolbar: &ToolbarState, box_width: usize) -> usize {
    category_placements(toolbar, box_width)
        .into_iter()
        .map(|placement| placement.row + category_height(toolbar, placement.category))
        .max()
        .unwrap_or(0)
}

pub(super) fn hierarchical_menu_spans(
    toolbar: &ToolbarState,
    row: usize,
    box_width: usize,
) -> Vec<ToolbarSpan> {
    let layout = toolbar
        .layout()
        .expect("hierarchical menu rendering excludes Utilities");
    let relative_row = row - MENU_FIRST_ROW;
    let mut spans = Vec::new();
    let placements = category_placements(toolbar, box_width);
    let active_band_row = placements
        .iter()
        .filter(|placement| placement.row <= relative_row)
        .map(|placement| placement.row)
        .max();
    for placement in placements
        .into_iter()
        .filter(|placement| Some(placement.row) == active_band_row)
    {
        let category = placement.category;
        let category_row = relative_row - placement.row;
        if !spans.is_empty() {
            spans.push(plain_span(GAP.to_string()));
        }
        let options = layout.options[category];
        let prefix_width = submenu_prefix_width(layout.labels[category], category, options.len());
        let cell_width = submenu_cell_width(prefix_width, options);
        let cell_start = spans_width(&spans);
        match category_row.checked_sub(1) {
            None => push_header(&mut spans, layout.labels[category], prefix_width, options),
            Some(page) => push_page(
                &mut spans,
                toolbar,
                category,
                page,
                prefix_width,
                options,
                layout.selected[category],
                layout.exclusive_submenu,
                cell_width,
            ),
        }
        pad_spans_to_width(&mut spans, cell_start + cell_width);
    }
    spans
}

fn category_placements(toolbar: &ToolbarState, box_width: usize) -> Vec<CategoryPlacement> {
    let layout = toolbar
        .layout()
        .expect("hierarchical menu layout excludes Utilities");
    let available_width = toolbar_content_width(box_width);
    let auxiliary_width = toolbar.auxiliary_panel_width_for_width(box_width);
    let auxiliary_height = toolbar.auxiliary_panel_row_count_for_width(box_width);
    let first_band_width = available_width.saturating_sub(
        auxiliary_width + usize::from(auxiliary_width > 0) * UnicodeWidthStr::width(GAP),
    );
    let mut placements = Vec::with_capacity(layout.labels.len());
    let mut band_row = 0;
    let mut band_width = 0usize;
    let mut band_height = 0;
    for category in 0..layout.labels.len() {
        let options = layout.options[category];
        let prefix_width = submenu_prefix_width(layout.labels[category], category, options.len());
        let category_width = submenu_cell_width(prefix_width, options);
        let band_limit = if band_row == 0 {
            first_band_width
        } else {
            available_width
        };
        let width_with_gap =
            category_width + usize::from(band_width > 0) * UnicodeWidthStr::width(GAP);
        if (band_width > 0 && band_width.saturating_add(width_with_gap) > band_limit)
            || (band_width == 0 && band_row == 0 && category_width > band_limit)
        {
            band_row = if band_row == 0 {
                auxiliary_height.max(band_height)
            } else {
                band_row + band_height
            };
            band_width = 0;
            band_height = 0;
        }
        placements.push(CategoryPlacement {
            category,
            row: band_row,
        });
        band_width = band_width.saturating_add(
            category_width + usize::from(band_width > 0) * UnicodeWidthStr::width(GAP),
        );
        band_height = band_height.max(category_height(toolbar, category));
    }
    placements
}

fn category_height(toolbar: &ToolbarState, category: usize) -> usize {
    let options = toolbar
        .layout()
        .expect("hierarchical menu layout excludes Utilities")
        .options[category];
    1 + options.len().div_ceil(OPTIONS_PER_PAGE)
}

fn push_header(spans: &mut Vec<ToolbarSpan>, label: &str, prefix_width: usize, options: &[&str]) {
    let label = format!("{label}:");
    spans.push(bold_prefix_span(
        pad_right_to_width(label.clone(), prefix_width),
        &label,
    ));
    let widths = submenu_option_column_widths(options);
    for (position, width) in widths.into_iter().enumerate() {
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
    prefix_width: usize,
    options: &[&str],
    selected: usize,
    exclusive_submenu: Option<usize>,
    cell_width: usize,
) {
    let page_start = page * OPTIONS_PER_PAGE;
    if page_start >= options.len() {
        spans.push(plain_span(" ".repeat(cell_width)));
        return;
    }
    let path = submenu_path(category, page, options.len());
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

    let widths = submenu_option_column_widths(options);
    for (position, option) in options[page_start..]
        .iter()
        .take(OPTIONS_PER_PAGE)
        .enumerate()
    {
        push_separator(spans, position);
        let option_index = page_start + position;
        spans.push(ToolbarSpan {
            contents: (*option).to_string(),
            bold_prefix: 0,
            selected: option_index == selected
                && exclusive_submenu.is_none_or(|active| active == category),
            highlighted: false,
            tooltip: false,
            action: Some(ToolbarAction::SelectSubmenu {
                submenu: category,
                option: option_index,
            }),
            right_aligned: false,
            foreground: None,
        });
        let padding = widths[position].saturating_sub(UnicodeWidthStr::width(*option));
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
