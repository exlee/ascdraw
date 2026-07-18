use super::{
    FILES_TOGGLE_CATEGORY, MAIN_LABEL_ROW, MAIN_SHORTCUT_ROW, PendingShortcut, ToolbarAction,
    ToolbarSpan, ToolbarState, aligned_shortcut, bold_prefix_span, bold_span, plain_span,
};

pub(super) const TOGGLE_LABELS: [&str; 3] = ["Dark Mode", "Multi Color Mode", "Multi Layer Mode"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum ToggleKind {
    DarkMode,
    MultiColorMode,
    MultiLayerMode,
}

impl ToggleKind {
    pub(super) const ALL: [Self; 3] = [Self::DarkMode, Self::MultiColorMode, Self::MultiLayerMode];

    pub(super) fn index(self) -> usize {
        match self {
            Self::DarkMode => 0,
            Self::MultiColorMode => 1,
            Self::MultiLayerMode => 2,
        }
    }
}

impl ToolbarState {
    pub(super) fn main_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let mut spans = if row == MAIN_LABEL_ROW {
            vec![bold_prefix_span("Mode: ".to_string(), "Mode:")]
        } else {
            let mut prefix = bold_span("1.".to_string());
            prefix.highlighted = self.pending_shortcut() == Some(PendingShortcut::Mode);
            vec![
                plain_span("   ".to_string()),
                prefix,
                plain_span(" ".to_string()),
            ]
        };
        let modes = self.available_modes();
        for (index, mode) in modes.iter().enumerate() {
            if index > 0 {
                spans.push(plain_span(" ".to_string()));
            }
            let contents = if row == MAIN_LABEL_ROW {
                if index + 1 == modes.len() {
                    (index + 1).to_string()
                } else {
                    aligned_shortcut(index + 1, mode.label())
                }
            } else {
                mode.label().to_string()
            };
            spans.push(ToolbarSpan {
                contents,
                bold_prefix: 0,
                selected: row == MAIN_SHORTCUT_ROW && *mode == self.main_mode && !self.export_open,
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::SelectMain(*mode)),
                right_aligned: false,
                foreground: None,
            });
        }
        self.append_auxiliary_header_spans(&mut spans, row);
        spans
    }

    pub fn dark_mode(&self) -> bool {
        self.toggles[ToggleKind::DarkMode.index()]
    }

    pub fn multi_layer_mode(&self) -> bool {
        self.toggles[ToggleKind::MultiLayerMode.index()]
    }

    pub fn multi_color_mode(&self) -> bool {
        self.toggles[ToggleKind::MultiColorMode.index()]
    }

    pub(super) fn toggle_setting(&mut self, toggle: ToggleKind) {
        let enabled = &mut self.toggles[toggle.index()];
        *enabled = !*enabled;
        if !*enabled {
            let cancels_pending = match toggle {
                ToggleKind::MultiLayerMode => matches!(
                    self.shortcut_prefix,
                    Some(PendingShortcut::Layers | PendingShortcut::Layer(_))
                ),
                ToggleKind::MultiColorMode => matches!(
                    self.shortcut_prefix,
                    Some(PendingShortcut::Colors | PendingShortcut::ColorGroup(_))
                ),
                ToggleKind::DarkMode => false,
            };
            if cancels_pending {
                self.shortcut_prefix = None;
            }
        }
    }

    pub(super) fn select_toggle_digit(&mut self, digit: usize) {
        if let Some(toggle) = digit
            .checked_sub(1)
            .and_then(|index| ToggleKind::ALL.get(index))
        {
            self.toggle_setting(*toggle);
            self.export_open = true;
            self.active_export_category = Some(FILES_TOGGLE_CATEGORY);
            self.shortcut_prefix = Some(PendingShortcut::ToggleOptions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::{MENU_FIRST_ROW, MainMode, ToolbarAction, boxed_toolbar_spans};
    use std::ops::Range;
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    use winit::keyboard::{Key, ModifiersState};

    fn press(toolbar: &mut ToolbarState, key: &str) {
        assert!(toolbar.handle_shortcut(&Key::Character(key.into()), ModifiersState::empty()));
    }

    fn toggle_category_open(toolbar: &ToolbarState) -> bool {
        toolbar.export_open && toolbar.active_export_category == Some(FILES_TOGGLE_CATEGORY)
    }

    fn action_column(
        toolbar: &ToolbarState,
        row: usize,
        width: usize,
        expected: ToolbarAction,
    ) -> usize {
        let mut column = 0;
        for span in boxed_toolbar_spans(&toolbar.toolbar_spans(row), width) {
            let span_width = UnicodeWidthStr::width(span.contents.as_str());
            if span.action == Some(expected) {
                return column;
            }
            column += span_width;
        }
        panic!("action {expected:?} is not visible on toolbar row {row}");
    }

    fn action_range(
        toolbar: &ToolbarState,
        row: usize,
        width: usize,
        expected: ToolbarAction,
    ) -> Range<usize> {
        let mut column = 0;
        let spans = boxed_toolbar_spans(&toolbar.toolbar_spans(row), width);
        let mut range = None;
        for span in spans {
            let start = column;
            column += UnicodeWidthStr::width(span.contents.as_str());
            if span.action == Some(expected) {
                range.get_or_insert(start..start).end = column;
            } else if range.is_some() {
                break;
            }
        }
        range.unwrap_or_else(|| panic!("action {expected:?} is not visible on toolbar row {row}"))
    }

    fn right_group_text(toolbar: &ToolbarState, row: usize) -> String {
        toolbar
            .toolbar_spans(row)
            .into_iter()
            .skip_while(|span| !span.right_aligned)
            .map(|span| span.contents)
            .collect()
    }

    #[test]
    fn files_keyboard_paths_toggle_every_setting_and_keep_the_menu_open() {
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, "0");
        press(&mut toolbar, "5");
        assert!(toggle_category_open(&toolbar));
        assert_eq!(
            toolbar.pending_shortcut(),
            Some(PendingShortcut::ToggleOptions)
        );
        let actions: Vec<_> = toolbar
            .toolbar_spans(MENU_FIRST_ROW)
            .into_iter()
            .filter_map(|span| match span.action {
                Some(action @ ToolbarAction::Toggle(_)) => Some(action),
                _ => None,
            })
            .collect();
        assert_eq!(
            actions,
            [
                ToolbarAction::Toggle(ToggleKind::DarkMode),
                ToolbarAction::Toggle(ToggleKind::MultiColorMode),
                ToolbarAction::Toggle(ToggleKind::MultiLayerMode),
            ]
        );
        let option_labels: Vec<_> = toolbar
            .toolbar_spans(MENU_FIRST_ROW + 1)
            .into_iter()
            .filter_map(|span| match span.action {
                Some(ToolbarAction::Toggle(_)) => Some(span.contents),
                _ => None,
            })
            .collect();
        assert_eq!(
            option_labels,
            ["Dark Mode", "Multi Color Mode", "Multi Layer Mode"]
        );

        for key in ["1", "2", "3"] {
            press(&mut toolbar, key);
            assert!(toolbar.export_menu_open());
            assert!(toggle_category_open(&toolbar));
            assert_eq!(
                toolbar.pending_shortcut(),
                Some(PendingShortcut::ToggleOptions)
            );
        }
        assert!(toolbar.dark_mode());
        assert!(
            toolbar
                .toolbar_spans(MENU_FIRST_ROW + 1)
                .iter()
                .filter(|span| matches!(span.action, Some(ToolbarAction::Toggle(_))))
                .all(|span| span.selected)
        );
        let durable = toolbar.durable_selections();
        let mut restored = ToolbarState::default();
        restored.restore_durable_selections(&durable);
        assert!(restored.dark_mode());
        assert!(restored.multi_color_mode());
        assert!(restored.multi_layer_mode());
    }

    #[test]
    fn escape_and_zero_close_the_combined_menu_from_toggle_options() {
        let mut toolbar = ToolbarState::default();
        for close_with in ["escape", "0"] {
            press(&mut toolbar, "0");
            press(&mut toolbar, "5");
            if close_with == "escape" {
                assert!(toolbar.handle_shortcut(
                    &Key::Named(winit::keyboard::NamedKey::Escape),
                    ModifiersState::empty()
                ));
            } else {
                press(&mut toolbar, "0");
            }
            assert!(!toolbar.export_menu_open());
            assert_eq!(toolbar.pending_shortcut(), None);
        }
    }

    #[test]
    fn top_level_feature_cells_stack_and_right_align_in_every_enabled_combination() {
        let width = 80;
        for (layers, colors, top, bottom, actions) in [
            (
                false,
                false,
                "0          ",
                "Files/Togls",
                vec![ToolbarAction::ToggleExportMenu],
            ),
            (
                true,
                false,
                "8                  0          ",
                "Lyrs               Files/Togls",
                vec![
                    ToolbarAction::BeginLayersPath,
                    ToolbarAction::ToggleExportMenu,
                ],
            ),
            (
                false,
                true,
                "9                    0          ",
                "Clrs                 Files/Togls",
                vec![
                    ToolbarAction::BeginColorsPath,
                    ToolbarAction::ToggleExportMenu,
                ],
            ),
            (
                true,
                true,
                "8                  9                    0          ",
                "Lyrs               Clrs                 Files/Togls",
                vec![
                    ToolbarAction::BeginLayersPath,
                    ToolbarAction::BeginColorsPath,
                    ToolbarAction::ToggleExportMenu,
                ],
            ),
        ] {
            let mut toolbar = ToolbarState::default();
            toolbar.toggles[ToggleKind::MultiLayerMode.index()] = layers;
            toolbar.toggles[ToggleKind::MultiColorMode.index()] = colors;

            assert_eq!(right_group_text(&toolbar, MAIN_LABEL_ROW), top);
            assert_eq!(right_group_text(&toolbar, MAIN_SHORTCUT_ROW), bottom);
            let expected_group_start = width - 2 - UnicodeWidthStr::width(bottom);
            for action in &actions {
                let top_range = action_range(&toolbar, MAIN_LABEL_ROW, width, *action);
                let bottom_range = action_range(&toolbar, MAIN_SHORTCUT_ROW, width, *action);
                assert_eq!(top_range, bottom_range);
                assert!(top_range.clone().all(|column| {
                    toolbar.action_at(MAIN_LABEL_ROW, column, width) == Some(*action)
                }));
                assert!(bottom_range.clone().all(|column| {
                    toolbar.action_at(MAIN_SHORTCUT_ROW, column, width) == Some(*action)
                }));
            }
            assert_eq!(
                action_range(&toolbar, MAIN_LABEL_ROW, width, actions[0]).start,
                expected_group_start
            );
            for narrow_width in 0..32 {
                for row in [MAIN_LABEL_ROW, MAIN_SHORTCUT_ROW] {
                    let boxed = boxed_toolbar_spans(&toolbar.toolbar_spans(row), narrow_width);
                    let text: String = boxed.iter().map(|span| span.contents.as_str()).collect();
                    assert_eq!(UnicodeWidthStr::width(text.as_str()), narrow_width);
                    assert!(
                        text.chars()
                            .all(|character| UnicodeWidthChar::width(character).is_some())
                    );
                }
            }
            for absent in [
                ToolbarAction::BeginLayersPath,
                ToolbarAction::BeginColorsPath,
            ]
            .into_iter()
            .filter(|action| !actions.contains(action))
            {
                assert!(
                    toolbar
                        .toolbar_spans(MAIN_LABEL_ROW)
                        .iter()
                        .all(|span| span.action != Some(absent))
                );
                assert!(
                    toolbar
                        .toolbar_spans(MAIN_SHORTCUT_ROW)
                        .iter()
                        .all(|span| span.action != Some(absent))
                );
            }
        }
    }

    #[test]
    fn persistent_panel_headers_begin_paths_without_becoming_selected_modes() {
        let mut toolbar = ToolbarState::default();
        toolbar.toggles[ToggleKind::MultiLayerMode.index()] = true;
        toolbar.toggles[ToggleKind::MultiColorMode.index()] = true;

        for (action, expected_pending) in [
            (ToolbarAction::BeginLayersPath, PendingShortcut::Layers),
            (ToolbarAction::BeginColorsPath, PendingShortcut::Colors),
        ] {
            assert!(toolbar.apply_action(action));
            assert_eq!(toolbar.pending_shortcut(), Some(expected_pending));
            assert_eq!(toolbar.main_mode(), MainMode::Stamp);
            assert!(
                toolbar
                    .toolbar_spans(MAIN_SHORTCUT_ROW)
                    .iter()
                    .filter(|span| {
                        matches!(
                            span.action,
                            Some(ToolbarAction::BeginLayersPath | ToolbarAction::BeginColorsPath)
                        )
                    })
                    .all(|span| !span.selected)
            );
        }
        assert!(toolbar.apply_action(ToolbarAction::ToggleExportMenu));
        assert!(
            toolbar
                .toolbar_spans(MAIN_SHORTCUT_ROW)
                .iter()
                .any(|span| span.selected && span.contents == "Files/Togls")
        );
    }

    #[test]
    fn mouse_category_and_toggle_actions_keep_the_combined_menu_open() {
        let mut toolbar = ToolbarState::default();
        assert!(toolbar.apply_action(ToolbarAction::ToggleExportMenu));
        let width = 180;
        let category = ToolbarAction::SelectExportCategory(FILES_TOGGLE_CATEGORY);
        let category_column = action_column(&toolbar, MENU_FIRST_ROW, width, category);
        assert_eq!(
            toolbar.action_at(MENU_FIRST_ROW, category_column, width),
            Some(category)
        );
        assert!(toolbar.apply_action(category));
        assert!(toggle_category_open(&toolbar));

        let toggle = ToolbarAction::Toggle(ToggleKind::DarkMode);
        let toggle_column = action_column(&toolbar, MENU_FIRST_ROW + 1, width, toggle);
        assert_eq!(
            toolbar.action_at(MENU_FIRST_ROW + 1, toggle_column, width),
            Some(toggle)
        );
        assert!(toolbar.apply_action(toggle));
        assert!(toolbar.dark_mode());
        assert!(toggle_category_open(&toolbar));

        let save = ToolbarAction::SelectExportCategory(1);
        let save_column = action_column(&toolbar, MENU_FIRST_ROW, width, save);
        assert_eq!(
            toolbar.action_at(MENU_FIRST_ROW, save_column, width),
            Some(save)
        );
        assert!(toolbar.apply_action(save));
        assert!(toolbar.export_menu_open());
        assert!(!toggle_category_open(&toolbar));
        assert_eq!(
            toolbar.pending_shortcut(),
            Some(PendingShortcut::ExportOption(1))
        );
    }

    #[test]
    fn feature_surfaces_stay_outside_mode_one_at_top_level_eight_and_nine() {
        let mut toolbar = ToolbarState::default();
        assert_eq!(toolbar.available_modes(), MainMode::ALL);

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        assert_eq!(toolbar.available_modes(), MainMode::ALL);
        press(&mut toolbar, "1");
        press(&mut toolbar, "5");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        press(&mut toolbar, "9");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Colors));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
        assert_eq!(toolbar.available_modes(), MainMode::ALL);
        press(&mut toolbar, "8");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Layers));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        assert_eq!(toolbar.available_modes(), MainMode::ALL);
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }

    #[test]
    fn disabling_a_panel_cancels_only_its_own_prefix_and_preserves_drawing_mode() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Line));
        toolbar.toggle_setting(ToggleKind::MultiLayerMode);
        toolbar.toggle_setting(ToggleKind::MultiColorMode);

        assert!(toolbar.apply_action(ToolbarAction::BeginColorsPath));
        toolbar.toggle_setting(ToggleKind::MultiLayerMode);
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Colors));
        assert_eq!(toolbar.main_mode(), MainMode::Line);

        toolbar.toggle_setting(ToggleKind::MultiLayerMode);
        assert!(toolbar.apply_action(ToolbarAction::BeginLayersPath));
        toolbar.toggle_setting(ToggleKind::MultiLayerMode);
        assert_eq!(toolbar.pending_shortcut(), None);
        assert_eq!(toolbar.main_mode(), MainMode::Line);

        assert!(toolbar.apply_action(ToolbarAction::BeginColorsPath));
        toolbar.toggle_setting(ToggleKind::MultiColorMode);
        assert_eq!(toolbar.pending_shortcut(), None);
        assert_eq!(toolbar.main_mode(), MainMode::Line);
    }
}
