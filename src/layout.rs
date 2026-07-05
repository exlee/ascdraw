use crate::render::CellMetrics;

pub const PADDING: usize = 12;

#[derive(Clone, Copy, Debug)]
pub struct LayoutMetrics {
    pub top_padding: usize,
    pub cols: usize,
    pub rows: usize,
}

pub fn content_top_padding(metrics: &CellMetrics, transparent_menubar: bool) -> usize {
    content_top_padding_for_cell_height(metrics.cell_height, transparent_menubar)
}

pub fn content_top_padding_for_cell_height(cell_height: usize, transparent_menubar: bool) -> usize {
    if transparent_menubar {
        PADDING + cell_height
    } else {
        PADDING
    }
}

pub fn layout_metrics(
    width: usize,
    height: usize,
    metrics: &CellMetrics,
    transparent_menubar: bool,
) -> LayoutMetrics {
    let top_padding = content_top_padding(metrics, transparent_menubar);
    let cols = width.saturating_sub(PADDING * 2) / metrics.cell_width.max(1);
    let rows = layout_rows(height, metrics.cell_height.max(1), transparent_menubar);
    LayoutMetrics {
        top_padding,
        cols,
        rows: rows.max(1),
    }
}

pub fn layout_rows(height: usize, cell_height: usize, transparent_menubar: bool) -> usize {
    let top_padding = content_top_padding_for_cell_height(cell_height, transparent_menubar);
    height.saturating_sub(top_padding + PADDING) / cell_height.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_menubar_adds_one_extra_top_row_of_padding() {
        assert_eq!(content_top_padding_for_cell_height(18, false), PADDING);
        assert_eq!(content_top_padding_for_cell_height(18, true), PADDING + 18);
    }

    #[test]
    fn transparent_menubar_reduces_available_rows_by_one() {
        let height = PADDING * 2 + 10 * 18;
        assert_eq!(layout_rows(height, 18, false), 10);
        assert_eq!(layout_rows(height, 18, true), 9);
    }
}
