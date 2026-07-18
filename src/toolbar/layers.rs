#[cfg(test)]
use unicode_width::UnicodeWidthStr;

use crate::model::{LayerId, LayerSummary, MAX_LAYERS};

use super::{
    MENU_FIRST_ROW, PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState, bold_prefix_span,
    bold_span, plain_span,
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

const GRID_OPERATIONS: [LayerOperation; 7] = [
    LayerOperation::Select,
    LayerOperation::Select,
    LayerOperation::Show,
    LayerOperation::MoveUp,
    LayerOperation::MoveDown,
    LayerOperation::New,
    LayerOperation::Delete,
];

fn operation_enabled(operation: LayerOperation, index: usize, layer_count: usize) -> bool {
    match operation {
        LayerOperation::Select | LayerOperation::Show => true,
        LayerOperation::MoveUp => index > 0 && index + 1 < layer_count,
        LayerOperation::MoveDown => index > 1,
        LayerOperation::New => layer_count < MAX_LAYERS,
        LayerOperation::Delete => index > 0,
    }
}

impl ToolbarState {
    pub(super) fn layer_menu_spans(&self, row: usize, layers: &[LayerSummary]) -> Vec<ToolbarSpan> {
        if row == MENU_FIRST_ROW {
            let mut title = bold_prefix_span("Layers 8:".to_owned(), "Layers");
            title.selected = true;
            title.action = Some(ToolbarAction::ToggleLayers);
            let mut spans = vec![title, plain_span(" ".to_owned())];
            for digit in 1..=GRID_OPERATIONS.len() {
                if digit > 1 {
                    spans.push(plain_span(" ".to_owned()));
                }
                spans.push(bold_span(digit.to_string()));
            }
            return spans;
        }

        let Some(index) = row
            .checked_sub(MENU_FIRST_ROW + 1)
            .filter(|index| *index < layers.len())
        else {
            return Vec::new();
        };
        let layer = layers[index];
        let mut row_prefix = bold_span(format!("{}.", index + 1));
        row_prefix.highlighted = self.pending_shortcut() == Some(PendingShortcut::Layer(layer.id));
        let mut spans = vec![row_prefix, plain_span(" ".to_owned())];
        let glyphs = [
            layer.id.symbol(),
            if layer.active { "×" } else { " " },
            if layer.visible { "▪" } else { "▫" },
            "↑",
            "↓",
            "+",
            "ø",
        ];
        for (column, (glyph, operation)) in glyphs.into_iter().zip(GRID_OPERATIONS).enumerate() {
            if column > 0 {
                spans.push(plain_span(" ".to_owned()));
            }
            let enabled = operation_enabled(operation, index, layers.len());
            spans.push(ToolbarSpan {
                contents: glyph.to_owned(),
                bold_prefix: 0,
                selected: column == 1 && layer.active || column == 2 && layer.visible,
                highlighted: false,
                tooltip: false,
                action: enabled.then_some(ToolbarAction::Layer {
                    layer: layer.id,
                    operation,
                }),
                right_aligned: false,
                foreground: None,
            });
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
        let operation = digit
            .checked_sub(1)
            .and_then(|column| GRID_OPERATIONS.get(column))
            .copied()?;
        operation_enabled(operation, index, layers.len()).then_some(operation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::{MAIN_LABEL_ROW, MainMode, ToggleKind, boxed_toolbar_spans};
    use winit::keyboard::{Key, ModifiersState, NamedKey};

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
                visible: false,
                active: false,
            },
        ]
    }

    fn enter_layers(toolbar: &mut ToolbarState, layers: &[LayerSummary]) {
        assert!(toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
        press(toolbar, layers, "0");
        press(toolbar, layers, "8");
        assert_eq!(toolbar.main_mode(), MainMode::Layers);
    }

    fn action_columns(spans: &[ToolbarSpan]) -> Vec<(usize, ToolbarAction)> {
        let mut start = 0;
        spans
            .iter()
            .filter_map(|span| {
                let span_start = start;
                start += UnicodeWidthStr::width(span.contents.as_str());
                span.action.map(|action| (span_start, action))
            })
            .collect()
    }

    #[test]
    fn exact_three_layer_grid_uses_greek_symbols_and_display_width_alignment() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);

        let rows = (MENU_FIRST_ROW..MENU_FIRST_ROW + 4)
            .map(|row| text(toolbar.toolbar_spans_with_layers(row, &layers)))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                "Layers 8: 1 2 3 4 5 6 7",
                "1. α × ▪ ↑ ↓ + ø",
                "2. β   ▫ ↑ ↓ + ø",
                "3. γ   ▫ ↑ ↓ + ø",
            ]
        );
        assert!(
            rows.iter()
                .all(|row| UnicodeWidthStr::width(row.as_str()) == row.chars().count())
        );
        let selected = toolbar
            .toolbar_spans_with_layers(MENU_FIRST_ROW + 1, &layers)
            .into_iter()
            .filter(|span| span.selected)
            .map(|span| span.contents)
            .collect::<Vec<_>>();
        assert_eq!(selected, ["×", "▪"]);
        assert_eq!(toolbar.menu_row_count(), 4);
    }

    #[test]
    fn keyboard_grid_paths_map_columns_and_escape_cancels_only_the_pending_row() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);

        press(&mut toolbar, &layers, "2");
        assert_eq!(
            toolbar.pending_shortcut(),
            Some(PendingShortcut::Layer(LayerId(1)))
        );
        assert!(toolbar.handle_shortcut_with_layers(
            &Key::Named(NamedKey::Escape),
            ModifiersState::empty(),
            &layers,
        ));
        assert_eq!(toolbar.pending_shortcut(), None);
        assert_eq!(toolbar.main_mode(), MainMode::Layers);

        for (row, column, operation) in [
            (2, 1, LayerOperation::Select),
            (2, 2, LayerOperation::Select),
            (2, 3, LayerOperation::Show),
            (2, 4, LayerOperation::MoveUp),
            (3, 5, LayerOperation::MoveDown),
            (2, 6, LayerOperation::New),
            (2, 7, LayerOperation::Delete),
        ] {
            press(&mut toolbar, &layers, &row.to_string());
            press(&mut toolbar, &layers, &column.to_string());
            assert_eq!(
                toolbar.take_layer_action(),
                Some((LayerId((row - 1) as u8), operation))
            );
        }
    }

    #[test]
    fn mouse_actions_share_exact_columns_and_boundaries_are_not_clickable() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);
        let width = 80;

        let expected_columns = [3, 5, 7, 9, 11, 13, 15];
        for (row_offset, layer) in layers.iter().enumerate() {
            let row = MENU_FIRST_ROW + 1 + row_offset;
            let boxed =
                boxed_toolbar_spans(&toolbar.toolbar_spans_with_layers(row, &layers), width);
            let columns = action_columns(&boxed);
            for (column, action) in columns {
                assert_eq!(
                    toolbar.action_at_with_layers(row, column, width, &layers),
                    Some(action)
                );
                assert!(expected_columns.contains(&(column - 2)));
                assert_eq!(
                    action,
                    ToolbarAction::Layer {
                        layer: layer.id,
                        operation: match column - 2 {
                            3 | 5 => LayerOperation::Select,
                            7 => LayerOperation::Show,
                            9 => LayerOperation::MoveUp,
                            11 => LayerOperation::MoveDown,
                            13 => LayerOperation::New,
                            15 => LayerOperation::Delete,
                            _ => unreachable!(),
                        },
                    }
                );
            }
        }

        let base_actions =
            action_columns(&toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + 1, &layers));
        assert!(!base_actions.iter().any(|(_, action)| matches!(
            action,
            ToolbarAction::Layer {
                operation: LayerOperation::MoveUp
                    | LayerOperation::MoveDown
                    | LayerOperation::Delete,
                ..
            }
        )));
        let top_actions =
            action_columns(&toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + 3, &layers));
        assert!(!top_actions.iter().any(|(_, action)| matches!(
            action,
            ToolbarAction::Layer {
                operation: LayerOperation::MoveUp,
                ..
            }
        )));
    }

    #[test]
    fn top_level_eight_is_available_only_with_the_toggle_and_never_joins_mode_one() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, &layers, "8");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert!(!text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('8'));

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        press(&mut toolbar, &layers, "0");
        assert!(!toolbar.available_modes().contains(&MainMode::Layers));
        assert!(text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('8'));
        press(&mut toolbar, &layers, "1");
        press(&mut toolbar, &layers, "5");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        press(&mut toolbar, &layers, "8");
        assert_eq!(toolbar.main_mode(), MainMode::Layers);
        press(&mut toolbar, &layers, "8");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        assert!(!text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('8'));
        press(&mut toolbar, &layers, "0");
        press(&mut toolbar, &layers, "8");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        press(&mut toolbar, &layers, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert_eq!(toolbar.pending_shortcut(), None);
    }

    #[test]
    fn maximum_layer_grid_keeps_sequential_symbols_and_disables_new() {
        let layers = (0..MAX_LAYERS)
            .map(|index| LayerSummary {
                id: LayerId(index as u8),
                visible: true,
                active: index == 0,
            })
            .collect::<Vec<_>>();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);

        assert_eq!(toolbar.menu_row_count(), 1 + MAX_LAYERS);
        assert_eq!(
            text(toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + 4, &layers)),
            "4. δ   ▪ ↑ ↓ + ø"
        );
        assert_eq!(
            text(toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + 6, &layers)),
            "6. ζ   ▪ ↑ ↓ + ø"
        );
        for row in MENU_FIRST_ROW + 1..=MENU_FIRST_ROW + MAX_LAYERS {
            assert!(
                !toolbar
                    .toolbar_spans_with_layers(row, &layers)
                    .iter()
                    .any(|span| matches!(
                        span.action,
                        Some(ToolbarAction::Layer {
                            operation: LayerOperation::New,
                            ..
                        })
                    ))
            );
        }

        press(&mut toolbar, &layers, "1");
        press(&mut toolbar, &layers, "6");
        assert_eq!(toolbar.take_layer_action(), None);
    }

    #[test]
    fn durable_layers_mode_restores_outside_the_mode_one_list() {
        let layers = sample_layers();
        let mut source = ToolbarState::default();
        enter_layers(&mut source, &layers);
        let durable = source.durable_selections();

        let mut restored = ToolbarState::default();
        restored.restore_durable_selections(&durable);
        assert!(restored.multi_layer_mode());
        assert_eq!(restored.main_mode(), MainMode::Layers);
        assert!(!restored.available_modes().contains(&MainMode::Layers));
        assert_eq!(restored.pending_shortcut(), None);
        assert!(!restored.export_menu_open());

        restored.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        assert_eq!(restored.main_mode(), MainMode::Stamp);
    }
}
