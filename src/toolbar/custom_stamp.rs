use super::{ToolbarSpan, ToolbarState, plain_span, toolbar_minimap_border_spans};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

pub(super) fn attach_cap(spans: Vec<ToolbarSpan>, width: usize) -> Vec<ToolbarSpan> {
    let prefix = match width {
        0 => return spans,
        1 => "├".to_owned(),
        2 => "├│".to_owned(),
        3 => "├─┐".to_owned(),
        _ => "├─┐".to_owned(),
    };
    attach_prefix(spans, prefix)
}

pub(super) fn attach_glyph(spans: Vec<ToolbarSpan>, width: usize, stamp: &str) -> Vec<ToolbarSpan> {
    let prefix = match width {
        0 => return spans,
        1 => "│".to_owned(),
        2 => "││".to_owned(),
        3 => format!("│{stamp}│"),
        _ => format!("│{stamp}│"),
    };
    attach_prefix(spans, prefix)
}

fn attach_prefix(spans: Vec<ToolbarSpan>, prefix: String) -> Vec<ToolbarSpan> {
    let mut remaining = UnicodeWidthStr::width(prefix.as_str());
    let mut attached = vec![plain_span(prefix)];
    for mut span in spans {
        if remaining == 0 {
            attached.push(span);
            continue;
        }
        let span_width = UnicodeWidthStr::width(span.contents.as_str());
        if span_width <= remaining {
            remaining -= span_width;
            continue;
        }
        let split = byte_index_after_width(&span.contents, remaining);
        span.contents = span.contents[split..].to_owned();
        span.bold_prefix = span.bold_prefix.saturating_sub(remaining);
        remaining = 0;
        attached.push(span);
    }
    attached
}

fn byte_index_after_width(contents: &str, target: usize) -> usize {
    let mut width = 0;
    for (index, character) in contents.char_indices() {
        if width >= target {
            return index;
        }
        width += UnicodeWidthChar::width(character).unwrap_or(0);
    }
    contents.len()
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

    fn text(spans: &[ToolbarSpan]) -> String {
        spans.iter().map(|span| span.contents.as_str()).collect()
    }

    #[test]
    fn indicator_attaches_to_the_lower_left_border() {
        let mut toolbar = ToolbarState::default();
        let standard_rows = toolbar.content_rows_for_width(12);
        toolbar.select_custom_stamp("▼".to_owned());

        assert_eq!(toolbar.content_rows_for_width(12), standard_rows);
        let cap_row = standard_rows - 2;
        let glyph_row = standard_rows - 1;
        assert!(
            text(&toolbar.boxed_spans_with_layers_for_width(cap_row, 12, &[])).starts_with("├─┐")
        );
        assert!(
            text(&toolbar.boxed_spans_with_layers_for_width(glyph_row, 12, &[])).starts_with("│▼│")
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
