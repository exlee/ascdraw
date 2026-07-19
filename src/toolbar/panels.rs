use unicode_width::UnicodeWidthStr;

use crate::model::LayerSummary;

use super::{
    MAIN_LABEL_ROW, MAIN_SHORTCUT_ROW, PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState,
    plain_span,
};

pub(super) const LAYER_PANEL_WIDTH: usize = 18;
pub(super) const COLOR_PANEL_WIDTH: usize = 20;
pub(super) const FILES_HEADER_WIDTH: usize = 11;
const PANEL_GAP: usize = 2;

#[derive(Clone, Copy)]
enum PanelKind {
    Layers,
    Colors,
    Files,
}

#[derive(Clone, Copy)]
struct PanelPlacement {
    kind: PanelKind,
    row: usize,
    height: usize,
    width: usize,
}

impl ToolbarState {
    pub(super) fn append_auxiliary_header_spans(&self, spans: &mut Vec<ToolbarSpan>, row: usize) {
        let mut entries = Vec::with_capacity(3);
        let layers_visible = self.multi_layer_mode() && !self.export_menu_open();
        let colors_visible = self.multi_color_mode() && !self.export_menu_open();
        let trailing_files_width = PANEL_GAP + FILES_HEADER_WIDTH;
        if layers_visible {
            entries.push((
                "Lyrs",
                8,
                LAYER_PANEL_WIDTH - usize::from(!colors_visible) * trailing_files_width,
                ToolbarAction::BeginLayersPath,
                matches!(
                    self.pending_shortcut(),
                    Some(PendingShortcut::Layers | PendingShortcut::Layer(_))
                ),
            ));
        }
        if colors_visible {
            entries.push((
                "Clrs",
                9,
                COLOR_PANEL_WIDTH - trailing_files_width,
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
                shift_action: None,
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
                    shift_action: None,
                    right_aligned: false,
                    foreground: None,
                });
            }
        }
    }

    pub(super) fn append_auxiliary_panel_spans_for_width(
        &self,
        spans: &mut Vec<ToolbarSpan>,
        panel_row: usize,
        box_width: usize,
        layers: &[LayerSummary],
    ) {
        let placements = self.auxiliary_panel_placements(box_width);
        if placements.is_empty() || panel_row >= self.auxiliary_panel_row_count_for_width(box_width)
        {
            return;
        }
        for span in spans.iter_mut() {
            span.right_aligned = false;
        }

        let mut panels = Vec::new();
        let active_band_row = placements
            .iter()
            .filter(|placement| placement.row <= panel_row)
            .map(|placement| placement.row)
            .max();
        let band_height = placements
            .iter()
            .filter(|placement| Some(placement.row) == active_band_row)
            .map(|placement| placement.height)
            .max()
            .unwrap_or(0);
        for placement in placements.into_iter().filter(|placement| {
            Some(placement.row) == active_band_row && panel_row < placement.row + band_height
        }) {
            if !panels.is_empty() {
                panels.push(plain_span(" ".repeat(PANEL_GAP)));
            }
            let local_row = panel_row - placement.row;
            if matches!(placement.kind, PanelKind::Files) && local_row >= placement.height {
                panels.push(plain_span(" ".repeat(placement.width)));
            } else if self.top_level_headers_wrap(box_width) && local_row < 2 {
                panels.extend(self.auxiliary_block_header_spans(placement.kind, local_row));
            } else {
                let data_row = local_row
                    .saturating_sub(usize::from(self.top_level_headers_wrap(box_width)) * 2);
                match placement.kind {
                    PanelKind::Layers => panels.extend(self.layer_panel_spans(data_row, layers)),
                    PanelKind::Colors => panels.extend(self.color_panel_spans(data_row)),
                    PanelKind::Files => {}
                }
            }
        }
        if let Some(first) = panels.first_mut() {
            first.right_aligned = true;
        }
        spans.extend(panels);
    }

    pub(crate) fn auxiliary_panels_visible(&self) -> bool {
        !self.export_menu_open() && (self.multi_layer_mode() || self.multi_color_mode())
    }

    pub(super) fn auxiliary_panel_row_count_for_width(&self, box_width: usize) -> usize {
        self.auxiliary_panel_placements(box_width)
            .into_iter()
            .map(|placement| placement.row + placement.height)
            .max()
            .unwrap_or(0)
    }

    pub(super) fn auxiliary_panel_width_for_width(&self, box_width: usize) -> usize {
        let placements = self.auxiliary_panel_placements(box_width);
        let mut bands = Vec::<(usize, usize)>::new();
        for placement in placements {
            if let Some((_, width)) = bands.iter_mut().find(|(row, _)| *row == placement.row) {
                *width += PANEL_GAP + placement.width;
            } else {
                bands.push((placement.row, placement.width));
            }
        }
        bands.into_iter().map(|(_, width)| width).max().unwrap_or(0)
    }

    fn auxiliary_panel_placements(&self, box_width: usize) -> Vec<PanelPlacement> {
        let available_width = super::toolbar_content_width(box_width);
        let wrap_headers = self.top_level_headers_wrap(box_width);
        let header_rows = usize::from(wrap_headers) * 2;
        let layer_rows = 1 + self.layer_count;
        let candidates = [
            (self.multi_layer_mode() && !self.export_menu_open()).then_some(PanelPlacement {
                kind: PanelKind::Layers,
                row: 0,
                height: header_rows + layer_rows,
                width: LAYER_PANEL_WIDTH,
            }),
            (self.multi_color_mode() && !self.export_menu_open()).then_some(PanelPlacement {
                kind: PanelKind::Colors,
                row: 0,
                height: header_rows + 3,
                width: COLOR_PANEL_WIDTH,
            }),
            wrap_headers.then_some(PanelPlacement {
                kind: PanelKind::Files,
                row: 0,
                height: 2,
                width: FILES_HEADER_WIDTH,
            }),
        ];
        let mut placements = Vec::new();
        let mut band_row = 0;
        let mut band_width = 0usize;
        let mut band_height = 0;
        for mut placement in candidates.into_iter().flatten() {
            let width_with_gap = placement.width + usize::from(band_width > 0) * PANEL_GAP;
            if band_width > 0 && band_width.saturating_add(width_with_gap) > available_width {
                band_row += band_height;
                band_width = 0;
                band_height = 0;
            }
            placement.row = band_row;
            band_width += placement.width + usize::from(band_width > 0) * PANEL_GAP;
            band_height = band_height.max(placement.height);
            placements.push(placement);
        }
        placements
    }

    fn auxiliary_block_header_spans(&self, kind: PanelKind, row: usize) -> Vec<ToolbarSpan> {
        let (label, digit, width, action, active) = match kind {
            PanelKind::Layers => (
                "Lyrs",
                8,
                LAYER_PANEL_WIDTH,
                ToolbarAction::BeginLayersPath,
                matches!(
                    self.pending_shortcut(),
                    Some(PendingShortcut::Layers | PendingShortcut::Layer(_))
                ),
            ),
            PanelKind::Colors => (
                "Clrs",
                9,
                COLOR_PANEL_WIDTH,
                ToolbarAction::BeginColorsPath,
                matches!(
                    self.pending_shortcut(),
                    Some(PendingShortcut::Colors | PendingShortcut::ColorGroup(_))
                ),
            ),
            PanelKind::Files => (
                "Files/Togls",
                0,
                FILES_HEADER_WIDTH,
                ToolbarAction::ToggleExportMenu,
                self.export_menu_open(),
            ),
        };
        let contents = if row == 0 {
            digit.to_string()
        } else {
            label.to_owned()
        };
        let used = UnicodeWidthStr::width(contents.as_str());
        vec![ToolbarSpan {
            contents: format!("{contents}{}", " ".repeat(width.saturating_sub(used))),
            bold_prefix: usize::from(row == 1) * used,
            selected: row == 1 && active && matches!(kind, PanelKind::Files),
            highlighted: row == 0 && active && !matches!(kind, PanelKind::Files),
            tooltip: false,
            action: Some(action),
            shift_action: None,
            right_aligned: false,
            foreground: None,
        }]
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
            "     1 2 3 4 5 6 7       1 2 3 4 5 6 7 8"
        );
        assert_eq!(
            right_text(&rows[1]),
            "8.1. α × ▪ ↑ ↓ + ø  9.1. ■ ■ ■ ■ ■ ■ ■ ■"
        );
        assert_eq!(
            right_text(&rows[2]),
            "8.2. β   ▫ ↑ ↓ + ø  9.2. ■ ■ ■ ■ ■ ■ ■ ■"
        );
        assert_eq!(
            right_text(&rows[3]),
            "8.3. γ   ▪ ↑ ↓ + ø                      "
        );
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }

    #[test]
    fn panel_action_cells_remain_clickable_when_panels_are_right_aligned() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        toolbar.sync_layer_count(layers.len());
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        let width = 160;

        let data = boxed_toolbar_spans(
            &toolbar.toolbar_spans_with_layers(MENU_FIRST_ROW + 1, &layers),
            width,
        );
        assert_eq!(
            action_start(&data, ToolbarAction::SelectColor(ColorId(7))),
            width - 3
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
    fn colors_only_header_starts_at_the_palette_prefix() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        let width = 120;

        let header = boxed_toolbar_spans(&toolbar.toolbar_spans(MAIN_LABEL_ROW), width);
        let palette = boxed_toolbar_spans(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1), width);

        assert_eq!(
            action_start(&header, ToolbarAction::BeginColorsPath),
            action_start(&palette, ToolbarAction::BeginColorPath(0))
        );
    }

    #[test]
    fn narrow_toolbar_stacks_complete_auxiliary_blocks_before_drawing_submenus() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        toolbar.sync_layer_count(layers.len());
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        let width = 34;

        let rows = (0..13)
            .map(|panel_row| {
                toolbar.toolbar_spans_with_layers_for_width(
                    MENU_FIRST_ROW + panel_row,
                    width,
                    &layers,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(right_text(&rows[0]), format!("8{}", " ".repeat(17)));
        assert_eq!(right_text(&rows[1]), format!("Lyrs{}", " ".repeat(14)));
        assert_eq!(right_text(&rows[3]), "8.1. α × ▪ ↑ ↓ + ø");
        assert_eq!(right_text(&rows[6]), format!("9{}", " ".repeat(19)));
        assert_eq!(right_text(&rows[7]), format!("Clrs{}", " ".repeat(16)));
        assert_eq!(right_text(&rows[9]), "9.1. ■ ■ ■ ■ ■ ■ ■ ■");
        assert_eq!(right_text(&rows[11]), format!("0{}", " ".repeat(10)));
        assert_eq!(right_text(&rows[12]), "Files/Togls");
        assert!(rows.iter().all(|row| !text(row).contains("Decorators:")));

        let separator =
            toolbar.toolbar_spans_with_layers_for_width(MENU_FIRST_ROW + 13, width, &layers);
        assert!(separator.is_empty());
        let drawing_row =
            toolbar.toolbar_spans_with_layers_for_width(MENU_FIRST_ROW + 14, width, &layers);
        assert!(text(&drawing_row).starts_with("Decorators:"));

        for (row, action) in [
            (MENU_FIRST_ROW, ToolbarAction::BeginLayersPath),
            (MENU_FIRST_ROW + 6, ToolbarAction::BeginColorsPath),
            (MENU_FIRST_ROW + 11, ToolbarAction::ToggleExportMenu),
        ] {
            let boxed = boxed_toolbar_spans(
                &toolbar.toolbar_spans_with_layers_for_width(row, width, &layers),
                width,
            );
            let column = action_start(&boxed, action);
            assert_eq!(
                toolbar.action_at_with_layers(row, column, width, &layers),
                Some(action)
            );
        }
    }

    #[test]
    fn open_files_menu_hides_enabled_layer_and_color_surfaces() {
        let layers = sample_layers();
        let mut toolbar = ToolbarState::default();
        toolbar.sync_layer_count(layers.len());
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        toolbar.apply_action(ToolbarAction::ToggleExportMenu);

        for row in 0..toolbar.content_rows() {
            assert!(
                toolbar
                    .toolbar_spans_with_layers(row, &layers)
                    .iter()
                    .all(|span| !matches!(
                        span.action,
                        Some(
                            ToolbarAction::BeginLayersPath
                                | ToolbarAction::BeginLayerPath(_)
                                | ToolbarAction::Layer { .. }
                                | ToolbarAction::BeginColorsPath
                                | ToolbarAction::BeginColorPath(_)
                                | ToolbarAction::SelectColor(_)
                        )
                    ))
            );
        }
        assert!(!toolbar.auxiliary_panels_visible());
        assert!(toolbar.multi_layer_mode());
        assert!(toolbar.multi_color_mode());
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
