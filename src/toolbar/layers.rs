use crate::model::{LayerId, LayerSummary, MAX_LAYERS};

use super::{
    PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState, bold_span, pad_spans_to_width,
    panels::LAYER_PANEL_WIDTH, plain_span,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerOperation {
    Select,
    Show,
    MoveUp,
    MoveDown,
    MergeUp,
    MergeDown,
    New,
    Delete,
}

// Rows are listed bottom-to-top, so moving upward on screen lowers the stack index.
const GRID_OPERATIONS: [LayerOperation; 7] = [
    LayerOperation::Select,
    LayerOperation::Select,
    LayerOperation::Show,
    LayerOperation::MoveDown,
    LayerOperation::MoveUp,
    LayerOperation::New,
    LayerOperation::Delete,
];
const SHIFT_GRID_OPERATIONS: [LayerOperation; 7] = [
    LayerOperation::Select,
    LayerOperation::Select,
    LayerOperation::Show,
    LayerOperation::MergeUp,
    LayerOperation::MergeDown,
    LayerOperation::New,
    LayerOperation::Delete,
];

fn operation_enabled(operation: LayerOperation, index: usize, layer_count: usize) -> bool {
    match operation {
        LayerOperation::Select | LayerOperation::Show => true,
        LayerOperation::MoveUp => index > 0 && index + 1 < layer_count,
        LayerOperation::MoveDown => index > 1,
        LayerOperation::MergeUp => index > 0,
        LayerOperation::MergeDown => index > 0 && index + 1 < layer_count,
        LayerOperation::New => layer_count < MAX_LAYERS,
        LayerOperation::Delete => index > 0,
    }
}

impl ToolbarState {
    pub(super) fn layer_panel_spans(
        &self,
        panel_row: usize,
        layers: &[LayerSummary],
    ) -> Vec<ToolbarSpan> {
        if panel_row == 0 {
            let mut spans = vec![plain_span("     ".to_owned())];
            for digit in 1..=GRID_OPERATIONS.len() {
                if digit > 1 {
                    spans.push(plain_span(" ".to_owned()));
                }
                spans.push(bold_span(digit.to_string()));
            }
            return spans;
        }

        let Some(index) = panel_row
            .checked_sub(1)
            .filter(|index| *index < layers.len())
        else {
            return vec![plain_span(" ".repeat(LAYER_PANEL_WIDTH))];
        };
        let layer = layers[index];
        let mut row_prefix = bold_span(format!("8.{}.", index + 1));
        row_prefix.highlighted = self.pending_shortcut() == Some(PendingShortcut::Layer(layer.id));
        row_prefix.action = Some(ToolbarAction::BeginLayerPath(layer.id));
        let mut spans = vec![row_prefix, plain_span(" ".to_owned())];
        let glyphs = [
            layer.id.symbol(),
            if layer.active { "×" } else { " " },
            if layer.visible { "▪" } else { "▫" },
            layer_operation_for_column(self.shift_layer(), 3, index, layers.len())
                .map_or(" ", layer_operation_glyph),
            layer_operation_for_column(self.shift_layer(), 4, index, layers.len())
                .map_or(" ", layer_operation_glyph),
            "+",
            "ø",
        ];
        for (column, glyph) in glyphs.into_iter().enumerate() {
            if column > 0 {
                spans.push(plain_span(" ".to_owned()));
            }
            let action_operation =
                layer_operation_for_column(self.shift_layer(), column, index, layers.len());
            spans.push(ToolbarSpan {
                contents: glyph.to_owned(),
                bold_prefix: 0,
                selected: false,
                highlighted: false,
                tooltip: false,
                action: action_operation.map(|operation| ToolbarAction::Layer {
                    layer: layer.id,
                    operation,
                }),
                shift_action: operation_enabled(SHIFT_GRID_OPERATIONS[column], index, layers.len())
                    .then_some(ToolbarAction::Layer {
                        layer: layer.id,
                        operation: SHIFT_GRID_OPERATIONS[column],
                    }),
                right_aligned: false,
                foreground: None,
            });
        }
        pad_spans_to_width(&mut spans, LAYER_PANEL_WIDTH);
        spans
    }

    pub(super) fn layer_operation_for_digit(
        &self,
        layers: &[LayerSummary],
        layer: LayerId,
        digit: usize,
    ) -> Option<LayerOperation> {
        let index = layers.iter().position(|summary| summary.id == layer)?;
        let column = digit.checked_sub(1)?;
        layer_operation_for_column(self.shift_layer(), column, index, layers.len())
    }
}

fn layer_operation_for_column(
    shift: bool,
    column: usize,
    index: usize,
    layer_count: usize,
) -> Option<LayerOperation> {
    let operation = *(if shift {
        SHIFT_GRID_OPERATIONS.get(column)
    } else {
        GRID_OPERATIONS.get(column)
    })?;
    operation_enabled(operation, index, layer_count).then_some(operation)
}

fn layer_operation_glyph(operation: LayerOperation) -> &'static str {
    match operation {
        LayerOperation::MoveUp => "↓",
        LayerOperation::MoveDown => "↑",
        LayerOperation::MergeUp => "▲",
        LayerOperation::MergeDown => "▼",
        _ => " ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::{
        MAIN_LABEL_ROW, MENU_FIRST_ROW, MainMode, ToggleKind, boxed_toolbar_spans,
    };
    use unicode_width::UnicodeWidthStr;
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
        toolbar.sync_layer_count(layers.len());
        assert!(toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }

    fn action_columns(spans: &[ToolbarSpan]) -> Vec<(usize, ToolbarAction)> {
        let mut start = 0;
        spans
            .iter()
            .filter_map(|span| {
                let span_start = start;
                start += UnicodeWidthStr::width(span.contents.as_str());
                span.action
                    .filter(|action| matches!(action, ToolbarAction::Layer { .. }))
                    .map(|action| (span_start, action))
            })
            .collect()
    }

    #[test]
    fn exact_three_layer_grid_uses_greek_symbols_and_display_width_alignment() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);

        let rows = (0..4)
            .map(|row| text(toolbar.layer_panel_spans(row, &layers)))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                "     1 2 3 4 5 6 7",
                "8.1. α × ▪     + ø",
                "8.2. β   ▫   ↓ + ø",
                "8.3. γ   ▫ ↑   + ø",
            ]
        );
        assert!(
            rows.iter()
                .all(|row| UnicodeWidthStr::width(row.as_str()) == row.chars().count())
        );
        assert_eq!(toolbar.menu_row_count(), 5);
    }

    #[test]
    fn layer_state_glyphs_are_self_describing_without_operation_outline_state() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);

        let operation_spans = (1..=layers.len())
            .flat_map(|row| toolbar.layer_panel_spans(row, &layers))
            .filter(|span| matches!(span.action, Some(ToolbarAction::Layer { .. })))
            .collect::<Vec<_>>();

        assert!(operation_spans.iter().all(|span| !span.selected));
        assert!(operation_spans.iter().all(|span| !span.highlighted));
        assert!(operation_spans.iter().any(|span| span.contents == "×"));
        assert!(operation_spans.iter().any(|span| span.contents == " "));
        assert!(operation_spans.iter().any(|span| span.contents == "▪"));
        assert!(operation_spans.iter().any(|span| span.contents == "▫"));

        press(&mut toolbar, &layers, "8");
        press(&mut toolbar, &layers, "2");
        let row = toolbar.layer_panel_spans(2, &layers);
        let prefix = row
            .iter()
            .find(|span| span.action == Some(ToolbarAction::BeginLayerPath(LayerId(1))))
            .unwrap();
        assert!(prefix.highlighted);
    }

    #[test]
    fn keyboard_grid_paths_map_columns_and_escape_cancels_only_the_pending_row() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);

        press(&mut toolbar, &layers, "8");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Layers));
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
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        for (row, column, operation) in [
            (2, 1, LayerOperation::Select),
            (2, 2, LayerOperation::Select),
            (2, 3, LayerOperation::Show),
            (3, 4, LayerOperation::MoveDown),
            (2, 5, LayerOperation::MoveUp),
            (2, 6, LayerOperation::New),
            (2, 7, LayerOperation::Delete),
        ] {
            press(&mut toolbar, &layers, "8");
            press(&mut toolbar, &layers, &row.to_string());
            press(&mut toolbar, &layers, &column.to_string());
            assert_eq!(
                toolbar.take_layer_action(),
                Some((LayerId((row - 1) as u8), operation))
            );
            press(&mut toolbar, &layers, "8");
            assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Layers));
            toolbar.cancel_shortcut();
        }
    }

    #[test]
    fn mouse_actions_share_exact_columns_and_boundaries_are_not_clickable() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);
        let width = 80;

        let expected_columns = [5, 7, 9, 11, 13, 15, 17];
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
                let panel_start = width - 2 - LAYER_PANEL_WIDTH;
                assert!(expected_columns.contains(&(column - panel_start)));
                assert_eq!(
                    action,
                    ToolbarAction::Layer {
                        layer: layer.id,
                        operation: match column - panel_start {
                            5 | 7 => LayerOperation::Select,
                            9 => LayerOperation::Show,
                            11 => LayerOperation::MoveDown,
                            13 => LayerOperation::MoveUp,
                            15 => LayerOperation::New,
                            17 => LayerOperation::Delete,
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
    fn shift_replaces_move_arrows_with_merge_actions_and_boundaries() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        enter_layers(&mut toolbar, &layers);

        for key in ["8", "2"] {
            assert!(toolbar.handle_shortcut_with_layers(
                &Key::Character(key.into()),
                ModifiersState::empty(),
                &layers,
            ));
        }
        assert!(toolbar.handle_shortcut_with_layers(
            &Key::Character("4".into()),
            ModifiersState::SHIFT,
            &layers,
        ));
        assert_eq!(
            toolbar.take_layer_action(),
            Some((LayerId(1), LayerOperation::MergeUp))
        );
        assert!(toolbar.set_shift_layer(false));

        let operation_for = |toolbar: &ToolbarState, row: usize, glyph: &str| {
            toolbar
                .layer_panel_spans(row, &layers)
                .into_iter()
                .find(|span| span.contents == glyph)
                .and_then(|span| span.action)
        };
        assert_eq!(operation_for(&toolbar, 2, "↑"), None);
        let second_row_up = toolbar.layer_panel_spans(2, &layers).remove(8);
        assert_eq!(
            second_row_up.shift_action,
            Some(ToolbarAction::Layer {
                layer: LayerId(1),
                operation: LayerOperation::MergeUp,
            })
        );
        assert_eq!(
            operation_for(&toolbar, 2, "↓"),
            Some(ToolbarAction::Layer {
                layer: LayerId(1),
                operation: LayerOperation::MoveUp,
            })
        );
        assert_eq!(operation_for(&toolbar, 3, "↓"), None);

        assert!(toolbar.set_shift_layer(true));
        assert_eq!(operation_for(&toolbar, 1, "▲"), None);
        assert_eq!(operation_for(&toolbar, 1, "▼"), None);
        assert_eq!(
            operation_for(&toolbar, 2, "▲"),
            Some(ToolbarAction::Layer {
                layer: LayerId(1),
                operation: LayerOperation::MergeUp,
            })
        );
        assert_eq!(
            operation_for(&toolbar, 2, "▼"),
            Some(ToolbarAction::Layer {
                layer: LayerId(1),
                operation: LayerOperation::MergeDown,
            })
        );
        assert_eq!(
            operation_for(&toolbar, 3, "▲"),
            Some(ToolbarAction::Layer {
                layer: LayerId(2),
                operation: LayerOperation::MergeUp,
            })
        );
        assert_eq!(operation_for(&toolbar, 3, "▼"), None);

        for key in ["8", "2"] {
            assert!(toolbar.handle_shortcut_with_layers(
                &Key::Character(key.into()),
                ModifiersState::empty(),
                &layers,
            ));
        }
        assert!(toolbar.handle_shortcut_with_layers(
            &Key::Character("4".into()),
            ModifiersState::SHIFT,
            &layers,
        ));
        assert_eq!(
            toolbar.take_layer_action(),
            Some((LayerId(1), LayerOperation::MergeUp))
        );
    }

    #[test]
    fn top_level_eight_begins_a_persistent_panel_path_without_changing_mode() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, &layers, "8");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert!(!text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('8'));

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        assert!(text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('8'));
        press(&mut toolbar, &layers, "1");
        press(&mut toolbar, &layers, "5");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        press(&mut toolbar, &layers, "8");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Layers));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        press(&mut toolbar, &layers, "8");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Layers));

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        assert!(!text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('8'));
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
            text(toolbar.layer_panel_spans(4, &layers)),
            "8.4. δ   ▪ ↑ ↓ + ø"
        );
        assert_eq!(
            text(toolbar.layer_panel_spans(6, &layers)),
            "8.6. ζ   ▪ ↑   + ø"
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

        press(&mut toolbar, &layers, "8");
        press(&mut toolbar, &layers, "1");
        press(&mut toolbar, &layers, "6");
        assert_eq!(toolbar.take_layer_action(), None);
    }

    #[test]
    fn durable_layers_toggle_restores_panel_without_replacing_the_drawing_mode() {
        let layers = sample_layers();
        let mut source = ToolbarState::default();
        enter_layers(&mut source, &layers);
        let durable = source.durable_selections();

        let mut restored = ToolbarState::default();
        restored.restore_durable_selections(&durable);
        assert!(restored.multi_layer_mode());
        assert_eq!(restored.main_mode(), MainMode::Stamp);
        assert_eq!(restored.pending_shortcut(), None);
        assert!(!restored.export_menu_open());

        restored.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        assert_eq!(restored.main_mode(), MainMode::Stamp);
    }
}
