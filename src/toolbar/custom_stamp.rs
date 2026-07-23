use super::{ToolbarSpan, ToolbarState, plain_span, toolbar_minimap_border_spans};

pub fn toolbar_bottom_border_spans(
    width: usize,
    minimap_width: usize,
    coordinates: (i128, i128),
    custom_stamp: bool,
) -> Vec<ToolbarSpan> {
    let mut spans = toolbar_minimap_border_spans(width, minimap_width, coordinates);
    if custom_stamp && width >= 4 {
        let mut border = spans[0].contents.chars().collect::<Vec<_>>();
        border[2] = '┴';
        spans[0].contents = border.into_iter().collect();
    }
    spans
}

pub(super) fn cap_spans(width: usize) -> Vec<ToolbarSpan> {
    let contents = match width {
        0 => return Vec::new(),
        1 => "├".to_owned(),
        2 => "├│".to_owned(),
        3 => "├─┐".to_owned(),
        _ => format!("├─┐{}│", " ".repeat(width - 4)),
    };
    vec![plain_span(contents)]
}

pub(super) fn glyph_spans(width: usize, stamp: &str) -> Vec<ToolbarSpan> {
    let contents = match width {
        0 => return Vec::new(),
        1 => "│".to_owned(),
        2 => "││".to_owned(),
        3 => format!("│{stamp}│"),
        _ => format!("│{stamp}│{}│", " ".repeat(width - 4)),
    };
    vec![plain_span(contents)]
}

impl ToolbarState {
    pub fn custom_stamp(&self) -> Option<&str> {
        self.custom_stamp.as_deref()
    }

    pub(crate) fn select_custom_stamp(&mut self, stamp: String) {
        self.close_export_menu();
        self.cancel_shortcut();
        self.custom_stamp = Some(stamp);
        self.main_mode = super::MainMode::Stamp;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn text(spans: &[ToolbarSpan]) -> String {
        spans.iter().map(|span| span.contents.as_str()).collect()
    }

    #[test]
    fn indicator_attaches_to_the_lower_left_border() {
        let mut toolbar = ToolbarState::default();
        let standard_rows = toolbar.content_rows_for_width(12);
        toolbar.select_custom_stamp("▼".to_owned());

        assert_eq!(toolbar.content_rows_for_width(12), standard_rows + 2);
        let cap_row = toolbar.content_rows_for_width(12) - 2;
        let glyph_row = toolbar.content_rows_for_width(12) - 1;
        assert_eq!(
            text(&toolbar.boxed_spans_with_layers_for_width(cap_row, 12, &[])),
            "├─┐        │"
        );
        assert_eq!(
            text(&toolbar.boxed_spans_with_layers_for_width(glyph_row, 12, &[])),
            "│▼│        │"
        );
        assert_eq!(
            text(&toolbar_bottom_border_spans(12, 0, (0, 0), true)),
            "└─┴────────┘"
        );

        for width in 0..12 {
            let content_rows = toolbar.content_rows_for_width(width);
            for row in [content_rows - 2, content_rows - 1] {
                assert_eq!(
                    UnicodeWidthStr::width(
                        text(&toolbar.boxed_spans_with_layers_for_width(row, width, &[])).as_str()
                    ),
                    width
                );
            }
            assert_eq!(
                UnicodeWidthStr::width(
                    text(&toolbar_bottom_border_spans(width, 0, (0, 0), true)).as_str()
                ),
                width
            );
        }
    }
}
