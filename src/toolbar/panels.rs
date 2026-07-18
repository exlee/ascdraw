use unicode_width::UnicodeWidthStr;

use crate::model::LayerSummary;

use super::{
    MAIN_LABEL_ROW, MAIN_SHORTCUT_ROW, PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState,
    plain_span,
};

pub(super) const LAYER_PANEL_WIDTH: usize = 18;
pub(super) const COLOR_PANEL_WIDTH: usize = 20;
pub(super) const FILES_HEADER_WIDTH: usize = 11;
const PANEL_GAP: usize = 1;

impl ToolbarState {
    pub(super) fn append_auxiliary_header_spans(&self, spans: &mut Vec<ToolbarSpan>, row: usize) {
        let mut entries = Vec::with_capacity(3);
        if self.multi_layer_mode() {
            entries.push((
                "Lyrs",
                8,
                LAYER_PANEL_WIDTH,
                ToolbarAction::BeginLayersPath,
                matches!(
                    self.pending_shortcut(),
                    Some(PendingShortcut::Layers | PendingShortcut::Layer(_))
                ),
            ));
        }
        if self.multi_color_mode() {
            entries.push((
                "Clrs",
                9,
                COLOR_PANEL_WIDTH,
                ToolbarAction::BeginColorsPath,
                matches!(
                    self.pending_shortcut(),
                    Some(PendingShortcut::Colors | PendingShortcut::ColorGroup(_))
                ),
            ));
        }
        entries.push((
            "Files/Togls",
            0,
            FILES_HEADER_WIDTH,
            ToolbarAction::ToggleExportMenu,
            self.export_menu_open(),
        ));

        for (index, (label, digit, width, action, active)) in entries.into_iter().enumerate() {
            if index > 0 {
                spans.push(plain_span(" ".repeat(PANEL_GAP)));
            }
            let contents = if row == MAIN_LABEL_ROW {
                digit.to_string()
            } else {
                label.to_owned()
            };
            let used = UnicodeWidthStr::width(contents.as_str());
            spans.push(ToolbarSpan {
                contents,
                bold_prefix: usize::from(row == MAIN_SHORTCUT_ROW) * used,
                selected: row == MAIN_SHORTCUT_ROW
                    && active
                    && action == ToolbarAction::ToggleExportMenu,
                highlighted: row == MAIN_LABEL_ROW
                    && active
                    && action != ToolbarAction::ToggleExportMenu,
                tooltip: false,
                action: Some(action),
                right_aligned: index == 0,
                foreground: None,
            });
            if width > used {
                spans.push(ToolbarSpan {
                    contents: " ".repeat(width - used),
                    bold_prefix: 0,
                    selected: false,
                    highlighted: false,
                    tooltip: false,
                    action: Some(action),
                    right_aligned: false,
                    foreground: None,
                });
            }
        }
    }

    pub(super) fn append_auxiliary_panel_spans(
        &self,
        spans: &mut Vec<ToolbarSpan>,
        panel_row: usize,
        layers: &[LayerSummary],
    ) {
        if !self.auxiliary_panels_visible() {
            return;
        }
        for span in spans.iter_mut() {
            span.right_aligned = false;
        }

        let mut panels = Vec::new();
        if self.multi_layer_mode() {
            panels.extend(self.layer_panel_spans(panel_row, layers));
        }
        if self.multi_color_mode() {
            if !panels.is_empty() {
                panels.push(plain_span(" ".repeat(PANEL_GAP)));
            }
            panels.extend(self.color_panel_spans(panel_row));
        }
        panels.push(plain_span(" ".repeat(PANEL_GAP)));
        panels.push(plain_span(" ".repeat(FILES_HEADER_WIDTH)));
        if let Some(first) = panels.first_mut() {
            first.right_aligned = true;
        }
        spans.extend(panels);
    }

    pub(crate) fn auxiliary_panels_visible(&self) -> bool {
        self.multi_layer_mode() || self.multi_color_mode()
    }

    pub(crate) const fn auxiliary_trailing_width(&self) -> usize {
        FILES_HEADER_WIDTH
    }

    pub(super) fn auxiliary_panel_row_count(&self) -> usize {
        let layer_rows = if self.multi_layer_mode() {
            1 + self.layer_count
        } else {
            0
        };
        let color_rows = if self.multi_color_mode() { 3 } else { 0 };
        layer_rows.max(color_rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColorId, LayerId};
    use crate::toolbar::{MENU_FIRST_ROW, MainMode, ToggleKind, boxed_toolbar_spans};
    use unicode_width::UnicodeWidthChar;

    fn sample_layers() -> [LayerSummary; 3] {
        [
            LayerSummary {
                id: LayerId(0),
                visible: true,
                active: true,
            },
            LayerSummary {
                id: LayerId(1),
                visible: false,
                active: false,
            },
            LayerSummary {
                id: LayerId(2),
                visible: true,
                active: false,
            },
        ]
    }

    fn text(spans: &[ToolbarSpan]) -> String {
        spans.iter().map(|span| span.contents.as_str()).collect()
    }

    fn right_text(spans: &[ToolbarSpan]) -> String {
        spans
            .iter()
            .skip_while(|span| !span.right_aligned)
            .map(|span| span.contents.as_str())
            .collect()
    }

    fn action_start(spans: &[ToolbarSpan], expected: ToolbarAction) -> usize {
        spans
            .iter()
            .scan(0, |column, span| {
                let start = *column;
                *column += UnicodeWidthStr::width(span.contents.as_str());
                Some((start, span))
            })
            .find_map(|(column, span)| (span.action == Some(expected)).then_some(column))
            .unwrap_or_else(|| panic!("missing action {expected:?} in {:?}", text(spans)))
    }

    #[test]
    fn both_panels_compose_side_by_side_while_the_drawing_menu_stays_on_the_left() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        toolbar.sync_layer_count(layers.len());
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));

        let rows = (0..4)
            .map(|panel_row| toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + panel_row, &layers))
            .collect::<Vec<_>>();
        assert!(text(&rows[0]).starts_with("Decorators:"));
        assert_eq!(
            right_text(&rows[0]),
            "     1 2 3 4 5 6 7      1 2 3 4 5 6 7 8            "
        );
        assert_eq!(
            right_text(&rows[1]),
            "8.1. α × ▪ ↑ ↓ + ø 9.1. ■ ■ ■ ■ ■ ■ ■ ■            "
        );
        assert_eq!(
            right_text(&rows[2]),
            "8.2. β   ▫ ↑ ↓ + ø 9.2. ■ ■ ■ ■ ■ ■ ■ ■            "
        );
        assert_eq!(
            right_text(&rows[3]),
            "8.3. γ   ▪ ↑ ↓ + ø                                 "
        );
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }

    #[test]
    fn header_prefixes_and_both_panel_action_cells_hit_exact_composed_columns() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        toolbar.sync_layer_count(layers.len());
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        let width = 160;

        let header = boxed_toolbar_spans(&toolbar.toolbar_spans(MAIN_LABEL_ROW), width);
        let data = boxed_toolbar_spans(
            &toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + 1, &layers),
            width,
        );
        assert_eq!(
            action_start(&header, ToolbarAction::BeginLayersPath),
            action_start(&data, ToolbarAction::BeginLayerPath(LayerId(0)))
        );
        assert_eq!(
            action_start(&header, ToolbarAction::BeginColorsPath),
            action_start(&data, ToolbarAction::BeginColorPath(0))
        );

        for action in [
            ToolbarAction::BeginLayerPath(LayerId(0)),
            ToolbarAction::Layer {
                layer: LayerId(0),
                operation: crate::toolbar::LayerOperation::Show,
            },
            ToolbarAction::BeginColorPath(0),
            ToolbarAction::SelectColor(ColorId(7)),
        ] {
            let column = action_start(&data, action);
            assert_eq!(
                toolbar.action_at_with_layers(MENU_FIRST_ROW + 1, column, width, &layers),
                Some(action)
            );
        }
    }

    #[test]
    fn auxiliary_panels_set_dynamic_max_rows_and_clip_unicode_safely() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Shapes));
        assert_eq!(toolbar.menu_row_count(), 2);

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        assert_eq!(toolbar.menu_row_count(), 3);
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        toolbar.sync_layer_count(layers.len());
        assert_eq!(toolbar.menu_row_count(), 4);
        toolbar.sync_layer_count(6);
        assert_eq!(toolbar.menu_row_count(), 7);

        for width in 0..72 {
            for row in 0..toolbar.content_rows() {
                let boxed =
                    boxed_toolbar_spans(&toolbar.toolbar_spans_with_layers(row, &layers), width);
                let contents = text(&boxed);
                assert_eq!(UnicodeWidthStr::width(contents.as_str()), width);
                assert!(
                    contents
                        .chars()
                        .all(|character| UnicodeWidthChar::width(character).is_some())
                );
            }
        }
    }
}
