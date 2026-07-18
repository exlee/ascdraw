#[cfg(test)]
use unicode_width::UnicodeWidthStr;

use crate::model::{BASE_COLORS, BRIGHT_COLORS, ColorId};

use super::{
    PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState, bold_span, pad_spans_to_width,
    panels::COLOR_PANEL_WIDTH, plain_span,
};

const GROUPS: [&[&str; 8]; 2] = [&BASE_COLORS, &BRIGHT_COLORS];

impl ToolbarState {
    pub(super) fn color_panel_spans(&self, panel_row: usize) -> Vec<ToolbarSpan> {
        if panel_row == 0 {
            let mut spans = vec![plain_span("     ".to_owned())];
            for digit in 1..=8 {
                if digit > 1 {
                    spans.push(plain_span(" ".to_owned()));
                }
                spans.push(bold_span(digit.to_string()));
            }
            return spans;
        }

        let Some(group) = panel_row
            .checked_sub(1)
            .filter(|group| *group < GROUPS.len())
        else {
            return vec![plain_span(" ".repeat(COLOR_PANEL_WIDTH))];
        };
        let mut row_prefix = bold_span(format!("9.{}.", group + 1));
        row_prefix.highlighted =
            self.pending_shortcut() == Some(PendingShortcut::ColorGroup(group));
        row_prefix.action = Some(ToolbarAction::BeginColorPath(group));
        let mut spans = vec![row_prefix, plain_span(" ".to_owned())];
        for index in 0..GROUPS[group].len() {
            if index > 0 {
                spans.push(plain_span(" ".to_owned()));
            }
            let color = ColorId((group * GROUPS[group].len() + index) as u8);
            spans.push(ToolbarSpan {
                contents: "■".to_owned(),
                bold_prefix: 0,
                selected: self.active_color == color,
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::SelectColor(color)),
                right_aligned: false,
                foreground: color.hex().map(str::to_owned),
            });
        }
        pad_spans_to_width(&mut spans, COLOR_PANEL_WIDTH);
        spans
    }

    pub fn active_color(&self) -> ColorId {
        self.active_color
    }

    pub(super) fn select_color(&mut self, color: ColorId) -> bool {
        if !color.is_valid() {
            return false;
        }
        self.active_color = color;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::{
        MAIN_LABEL_ROW, MENU_FIRST_ROW, MainMode, ToggleKind, boxed_toolbar_spans,
    };
    use winit::keyboard::{Key, ModifiersState};

    fn text(spans: Vec<ToolbarSpan>) -> String {
        spans.into_iter().map(|span| span.contents).collect()
    }

    fn press(toolbar: &mut ToolbarState, key: &str) {
        assert!(toolbar.handle_shortcut_with_layers(
            &Key::Character(key.into()),
            ModifiersState::empty(),
            &[],
        ));
    }

    fn enter_colors(toolbar: &mut ToolbarState) {
        assert!(toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode)));
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
                    .filter(|action| matches!(action, ToolbarAction::SelectColor(_)))
                    .map(|action| (span_start, action))
            })
            .collect()
    }

    #[test]
    fn exact_palette_grid_has_two_rows_and_sixteen_typed_actions() {
        let mut toolbar = ToolbarState::default();
        enter_colors(&mut toolbar);

        assert_eq!(
            (0..3)
                .map(|row| text(toolbar.color_panel_spans(row)))
                .collect::<Vec<_>>(),
            [
                "     1 2 3 4 5 6 7 8",
                "9.1. ■ ■ ■ ■ ■ ■ ■ ■",
                "9.2. ■ ■ ■ ■ ■ ■ ■ ■",
            ]
        );
        assert_eq!(toolbar.menu_row_count(), 4);

        let colors = (1..3)
            .flat_map(|row| toolbar.color_panel_spans(row))
            .filter_map(|span| {
                let ToolbarAction::SelectColor(color) = span.action? else {
                    return None;
                };
                Some((color, span.foreground))
            })
            .collect::<Vec<_>>();
        assert_eq!(colors.len(), ColorId::COUNT);
        for (index, (color, foreground)) in colors.into_iter().enumerate() {
            assert_eq!(color, ColorId(index as u8));
            assert_eq!(foreground.as_deref(), color.hex());
        }
    }

    #[test]
    fn exact_keyboard_paths_select_the_eighth_color_in_each_row() {
        let mut toolbar = ToolbarState::default();
        enter_colors(&mut toolbar);
        for (row, expected) in [("1", ColorId(7)), ("2", ColorId(15))] {
            press(&mut toolbar, "9");
            assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Colors));
            press(&mut toolbar, row);
            assert_eq!(
                toolbar.pending_shortcut(),
                Some(PendingShortcut::ColorGroup(
                    row.parse::<usize>().unwrap() - 1
                ))
            );
            press(&mut toolbar, "8");
            assert_eq!(toolbar.active_color(), expected);
            assert_eq!(toolbar.pending_shortcut(), None);
            assert_eq!(toolbar.main_mode(), MainMode::Stamp);
            press(&mut toolbar, "9");
            assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Colors));
            toolbar.cancel_shortcut();
        }
    }

    #[test]
    fn mouse_hit_testing_matches_every_visible_swatch_column() {
        let mut toolbar = ToolbarState::default();
        enter_colors(&mut toolbar);
        let width = 80;
        for group in 0..2 {
            let row = MENU_FIRST_ROW + 1 + group;
            let boxed = boxed_toolbar_spans(&toolbar.toolbar_spans(row), width);
            let actions = action_columns(&boxed);
            assert_eq!(actions.len(), 8);
            for (index, (column, action)) in actions.into_iter().enumerate() {
                let expected = ToolbarAction::SelectColor(ColorId((group * 8 + index) as u8));
                assert_eq!(action, expected);
                assert_eq!(toolbar.action_at(row, column, width), Some(expected));
            }
        }
    }

    #[test]
    fn top_level_nine_begins_a_persistent_panel_path_without_changing_mode() {
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert!(!text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('9'));

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        assert_eq!(toolbar.available_modes(), MainMode::ALL);
        assert!(text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('9'));
        press(&mut toolbar, "1");
        press(&mut toolbar, "5");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        press(&mut toolbar, "9");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Colors));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        press(&mut toolbar, "9");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Colors));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        assert!(!text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains('9'));
        press(&mut toolbar, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }

    #[test]
    fn durable_color_restores_with_persistent_panel_and_original_drawing_mode() {
        let mut source = ToolbarState::default();
        enter_colors(&mut source);
        press(&mut source, "9");
        press(&mut source, "2");
        press(&mut source, "4");
        let durable = source.durable_selections();

        let mut restored = ToolbarState::default();
        restored.restore_durable_selections(&durable);
        assert!(restored.multi_color_mode());
        assert_eq!(restored.main_mode(), MainMode::Stamp);
        assert_eq!(restored.available_modes(), MainMode::ALL);
        assert_eq!(restored.active_color(), ColorId(11));
        let selected = (1..3)
            .flat_map(|row| restored.color_panel_spans(row))
            .filter(|span| span.selected)
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].action,
            Some(ToolbarAction::SelectColor(ColorId(11)))
        );
    }
}
