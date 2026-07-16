use unicode_width::UnicodeWidthStr;

use crate::model::{LayerId, LayerSummary};

use super::{
    GAP, MENU_FIRST_ROW, PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState,
    menu_prefix_width, pad_right_to_width, pad_spans_to_width, plain_span, push_shortcut_path,
    spans_width, submenu_cell_width, submenu_option_column_widths,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerOperation {
    Select,
    Show,
    MoveUp,
    MoveDown,
    New,
    Delete,
}

const BASE_OPERATIONS: [(&str, LayerOperation); 3] = [
    ("Sel", LayerOperation::Select),
    ("Shw", LayerOperation::Show),
    ("New", LayerOperation::New),
];
const LAYER_OPERATIONS: [(&str, LayerOperation); 6] = [
    ("Sel", LayerOperation::Select),
    ("Shw", LayerOperation::Show),
    ("Mv↑", LayerOperation::MoveUp),
    ("Mv↓", LayerOperation::MoveDown),
    ("New", LayerOperation::New),
    ("Del", LayerOperation::Delete),
];

fn operations(index: usize) -> &'static [(&'static str, LayerOperation)] {
    if index == 0 {
        &BASE_OPERATIONS
    } else {
        &LAYER_OPERATIONS
    }
}

impl ToolbarState {
    pub(super) fn layer_menu_spans(&self, row: usize, layers: &[LayerSummary]) -> Vec<ToolbarSpan> {
        let header = row == MENU_FIRST_ROW;
        let mut spans = Vec::new();
        for (index, layer) in layers.iter().enumerate() {
            if index > 0 {
                spans.push(plain_span(GAP.to_owned()));
            }
            let items = operations(index);
            let labels = items.iter().map(|(label, _)| *label).collect::<Vec<_>>();
            let path = format!("{}.", index + 2);
            let prefix_width = menu_prefix_width(
                &format!("Layer {}", layer.id.symbol()),
                std::iter::once(path.as_str()),
            );
            let cell_width = submenu_cell_width(prefix_width, &labels);
            let cell_start = spans_width(&spans);
            if header {
                let label = format!("Layer {}:", layer.id.symbol());
                spans.push(ToolbarSpan {
                    contents: pad_right_to_width(label.clone(), prefix_width),
                    bold_prefix: UnicodeWidthStr::width(label.as_str()),
                    selected: false,
                    highlighted: false,
                    tooltip: false,
                    action: None,
                    right_aligned: false,
                    foreground: None,
                });
                for (position, width) in submenu_option_column_widths(&labels)
                    .into_iter()
                    .enumerate()
                {
                    if position > 0 {
                        spans.push(plain_span(" ".to_owned()));
                    }
                    spans.push(plain_span(pad_right_to_width(
                        (position + 1).to_string(),
                        width,
                    )));
                }
            } else {
                let highlighted = (self.pending_shortcut()
                    == Some(PendingShortcut::Layer(layer.id)))
                .then_some(path.as_str());
                push_shortcut_path(&mut spans, &path, prefix_width, highlighted);
                for (position, (label, operation)) in items.iter().enumerate() {
                    if position > 0 {
                        spans.push(plain_span(" ".to_owned()));
                    }
                    spans.push(ToolbarSpan {
                        contents: (*label).to_owned(),
                        bold_prefix: 0,
                        selected: match operation {
                            LayerOperation::Select => layer.active,
                            LayerOperation::Show => layer.visible,
                            _ => false,
                        },
                        highlighted: false,
                        tooltip: false,
                        action: Some(ToolbarAction::Layer {
                            layer: layer.id,
                            operation: *operation,
                        }),
                        right_aligned: false,
                        foreground: None,
                    });
                }
            }
            pad_spans_to_width(&mut spans, cell_start + cell_width);
        }
        spans
    }

    pub(super) fn layer_operation_for_digit(
        &self,
        layers: &[LayerSummary],
        layer: LayerId,
        digit: usize,
    ) -> Option<LayerOperation> {
        let index = layers.iter().position(|summary| summary.id == layer)?;
        digit
            .checked_sub(1)
            .and_then(|position| operations(index).get(position))
            .map(|(_, operation)| *operation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::{MainMode, ToggleKind};
    use winit::keyboard::{Key, ModifiersState};

    fn text(spans: Vec<ToolbarSpan>) -> String {
        spans.into_iter().map(|span| span.contents).collect()
    }

    fn action_starts(spans: &[ToolbarSpan]) -> Vec<usize> {
        let mut start = 0;
        spans
            .iter()
            .filter_map(|span| {
                let span_start = start;
                start += UnicodeWidthStr::width(span.contents.as_str());
                span.action.is_some().then_some(span_start)
            })
            .collect()
    }

    fn digit_starts(spans: &[ToolbarSpan]) -> Vec<usize> {
        let mut start = 0;
        spans
            .iter()
            .filter_map(|span| {
                let span_start = start;
                start += UnicodeWidthStr::width(span.contents.as_str());
                span.contents
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .map(|_| span_start)
            })
            .collect()
    }

    fn press(toolbar: &mut ToolbarState, layers: &[LayerSummary], key: &str) {
        assert!(toolbar.handle_shortcut_with_layers(
            &Key::Character(key.into()),
            ModifiersState::empty(),
            layers,
        ));
    }

    #[test]
    fn exact_layer_rows_and_keyboard_paths_match_the_compact_menu() {
        let layers = [
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
        ];
        let mut toolbar = ToolbarState::default();
        assert!(toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
        assert!(toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Layers)));

        let header = text(toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW, &layers));
        let values = text(toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + 1, &layers));
        assert!(header.contains("Layer ⍺: 1   2   3"));
        assert!(header.contains("Layer β: 1   2   3   4   5   6"));
        assert!(values.contains("2. Sel Shw New"));
        assert!(values.contains("3. Sel Shw Mv↑ Mv↓ New Del"));
        assert_eq!(
            digit_starts(&toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW, &layers)),
            action_starts(&toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + 1, &layers))
        );

        press(&mut toolbar, &layers, "3");
        press(&mut toolbar, &layers, "4");
        assert_eq!(
            toolbar.take_layer_action(),
            Some((LayerId(1), LayerOperation::MoveDown))
        );
    }

    #[test]
    fn mode_shortcut_leaves_layers_mode() {
        let layers = [LayerSummary {
            id: LayerId(0),
            visible: true,
            active: true,
        }];
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Layers));

        press(&mut toolbar, &layers, "1");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Mode));
        press(&mut toolbar, &layers, "1");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }
}
