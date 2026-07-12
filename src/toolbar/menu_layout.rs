use super::*;

pub(super) fn hierarchical_menu_spans(toolbar: &ToolbarState, row: usize) -> Vec<ToolbarSpan> {
    let layout = toolbar
        .layout()
        .expect("hierarchical menu rendering excludes Utilities");
    let relative_row = row - MENU_FIRST_ROW;
    let page = relative_row.checked_sub(1);
    let mut spans = Vec::new();
    for category in 0..layout.labels.len() {
        if category > 0 {
            spans.push(plain_span(GAP.to_string()));
        }
        let options = layout.options[category];
        let prefix_width = submenu_prefix_width(layout.labels[category], category, options.len());
        let cell_width = submenu_cell_width(prefix_width, options);
        let cell_start = spans_width(&spans);
        match page {
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
