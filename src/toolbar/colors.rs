#[cfg(test)]
use unicode_width::UnicodeWidthStr;

use crate::model::{BASE_COLORS, BRIGHT_COLORS, ColorId};

use super::{
    MENU_FIRST_ROW, PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState, bold_prefix_span,
    bold_span, plain_span,
};

const GROUPS: [&[&str; 8]; 2] = [&BASE_COLORS, &BRIGHT_COLORS];

impl ToolbarState {
    pub(super) fn color_menu_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        if row == MENU_FIRST_ROW {
            let mut title = bold_prefix_span("Colors 9:".to_owned(), "Colors");
            title.selected = true;
            title.action = Some(ToolbarAction::ToggleColors);
            let mut spans = vec![title, plain_span(" ".to_owned())];
            for digit in 1..=8 {
                if digit > 1 {
                    spans.push(plain_span(" ".to_owned()));
                }
                spans.push(bold_span(digit.to_string()));
            }
            return spans;
        }

        let Some(group) = row
            .checked_sub(MENU_FIRST_ROW + 1)
            .filter(|group| *group < GROUPS.len())
        else {
            return Vec::new();
        };
        let mut row_prefix = bold_span(format!("{}.", group + 1));
        row_prefix.highlighted =
            self.pending_shortcut() == Some(PendingShortcut::ColorGroup(group));
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
    use crate::toolbar::{MAIN_LABEL_ROW, MainMode, ToggleKind, boxed_toolbar_spans};
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
        press(toolbar, "0");
        press(toolbar, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Colors);
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
    fn exact_palette_grid_has_two_rows_and_sixteen_typed_actions() {
        let mut toolbar = ToolbarState::default();
        enter_colors(&mut toolbar);

        assert_eq!(
            (MENU_FIRST_ROW..MENU_FIRST_ROW + 3)
                .map(|row| text(toolbar.toolbar_spans(row)))
                .collect::<Vec<_>>(),
            [
                "Colors 9: 1 2 3 4 5 6 7 8",
                "1. ■ ■ ■ ■ ■ ■ ■ ■",
                "2. ■ ■ ■ ■ ■ ■ ■ ■",
            ]
        );
        assert_eq!(toolbar.menu_row_count(), 3);

        let colors = (MENU_FIRST_ROW + 1..MENU_FIRST_ROW + 3)
            .flat_map(|row| toolbar.toolbar_spans(row))
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
            assert_eq!(toolbar.main_mode(), MainMode::Colors);
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
    fn top_level_nine_exposure_tracks_only_multi_color() {
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert!(!text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains("Colors 9"));

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        press(&mut toolbar, "0");
        assert_eq!(toolbar.available_modes(), MainMode::ALL);
        assert!(text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains("Colors 9"));
        press(&mut toolbar, "1");
        press(&mut toolbar, "5");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        press(&mut toolbar, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Colors);
        press(&mut toolbar, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        assert!(!text(toolbar.toolbar_spans(MAIN_LABEL_ROW)).contains("Colors 9"));
        press(&mut toolbar, "0");
        press(&mut toolbar, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }

    #[test]
    fn durable_colors_mode_restores_outside_mode_one_with_one_selected_swatch() {
        let mut source = ToolbarState::default();
        enter_colors(&mut source);
        press(&mut source, "2");
        press(&mut source, "4");
        let durable = source.durable_selections();

        let mut restored = ToolbarState::default();
        restored.restore_durable_selections(&durable);
        assert!(restored.multi_color_mode());
        assert_eq!(restored.main_mode(), MainMode::Colors);
        assert_eq!(restored.available_modes(), MainMode::ALL);
        assert_eq!(restored.active_color(), ColorId(11));
        let selected = (MENU_FIRST_ROW + 1..MENU_FIRST_ROW + 3)
            .flat_map(|row| restored.toolbar_spans(row))
            .filter(|span| span.selected)
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].action,
            Some(ToolbarAction::SelectColor(ColorId(11)))
        );
    }
}
