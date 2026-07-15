use unicode_width::UnicodeWidthStr;

use crate::model::{LayerId, LayerSummary};

use super::{
    MENU_FIRST_ROW, PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState, bold_prefix_span,
    plain_span,
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
                spans.push(plain_span("    ".to_owned()));
            }
            let cell_start = spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.contents.as_str()))
                .sum::<usize>();
            let items = operations(index);
            if header {
                let label = format!("Layer {}:", layer.id.symbol());
                spans.push(bold_prefix_span(label.clone(), &label));
                for number in 1..=items.len() {
                    spans.push(plain_span(format!(" {number}")));
                }
            } else {
                let prefix = format!("{}.", index + 2);
                spans.push(ToolbarSpan {
                    contents: prefix.clone(),
                    bold_prefix: UnicodeWidthStr::width(prefix.as_str()),
                    selected: false,
                    highlighted: self.pending_shortcut() == Some(PendingShortcut::Layer(layer.id)),
                    tooltip: false,
                    action: None,
                    right_aligned: false,
                });
                for (position, (label, operation)) in items.iter().enumerate() {
                    spans.push(plain_span(" ".to_owned()));
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
                    });
                    if position + 1 == items.len() {
                        break;
                    }
                }
            }
            let cell_width = if index == 0 { 24 } else { 36 };
            let used = spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.contents.as_str()))
                .sum::<usize>()
                .saturating_sub(cell_start);
            if used < cell_width {
                spans.push(plain_span(" ".repeat(cell_width - used)));
            }
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
        assert!(header.contains("Layer ⍺: 1 2 3"));
        assert!(header.contains("Layer β: 1 2 3 4 5 6"));
        assert!(values.contains("2. Sel Shw New"));
        assert!(values.contains("3. Sel Shw Mv↑ Mv↓ New Del"));

        press(&mut toolbar, &layers, "3");
        press(&mut toolbar, &layers, "4");
        assert_eq!(
            toolbar.take_layer_action(),
            Some((LayerId(1), LayerOperation::MoveDown))
        );
    }
}
