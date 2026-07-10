use crate::render::CellMetrics;

pub const PADDING: usize = 12;
const TRANSPARENT_MENUBAR_TOP_INSET_PT: f64 = 24.0;

#[derive(Clone, Copy, Debug)]
pub struct LayoutMetrics {
    pub top_padding: usize,
    pub cols: usize,
    pub rows: usize,
}

pub fn content_top_padding(scale_factor: f64, transparent_menubar: bool) -> usize {
    content_top_padding_for_scale_factor(scale_factor, transparent_menubar)
}

pub fn content_top_padding_for_scale_factor(scale_factor: f64, transparent_menubar: bool) -> usize {
    if transparent_menubar {
        PADDING + (TRANSPARENT_MENUBAR_TOP_INSET_PT * scale_factor).round() as usize
    } else {
        PADDING
    }
}

pub fn layout_metrics(
    width: usize,
    height: usize,
    metrics: &CellMetrics,
    transparent_menubar: bool,
    scale_factor: f64,
) -> LayoutMetrics {
    let top_padding = content_top_padding(scale_factor, transparent_menubar);
    let cols = width.saturating_sub(PADDING * 2) / metrics.cell_width.max(1);
    let rows = layout_rows(
        height,
        metrics.cell_height.max(1),
        transparent_menubar,
        scale_factor,
    );
    LayoutMetrics {
        top_padding,
        cols,
        rows: rows.max(1),
    }
}

pub fn layout_rows(
    height: usize,
    cell_height: usize,
    transparent_menubar: bool,
    scale_factor: f64,
) -> usize {
    let top_padding = content_top_padding_for_scale_factor(scale_factor, transparent_menubar);
    height.saturating_sub(top_padding + PADDING) / cell_height.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_menubar_uses_fixed_point_top_inset() {
        assert_eq!(content_top_padding_for_scale_factor(1.0, false), PADDING);
        assert_eq!(
            content_top_padding_for_scale_factor(1.0, true),
            PADDING + 24
        );
        assert_eq!(
            content_top_padding_for_scale_factor(2.0, true),
            PADDING + 48
        );
    }

    #[test]
    fn transparent_menubar_reduces_available_rows_by_fixed_inset() {
        let height = PADDING * 2 + 10 * 18;
        assert_eq!(layout_rows(height, 18, false, 1.0), 10);
        assert_eq!(layout_rows(height, 18, true, 1.0), 8);
    }
}
