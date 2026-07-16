use unicode_width::UnicodeWidthStr;

use crate::model::{BASE_COLORS, BRIGHT_COLORS, ColorId};

use super::{
    GAP, MENU_FIRST_ROW, PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState,
    menu_prefix_width, pad_right_to_width, pad_spans_to_width, plain_span, push_shortcut_path,
    spans_width, submenu_cell_width, submenu_option_column_widths,
};

const GROUPS: [(&str, &[&str; 8]); 2] = [("Base", &BASE_COLORS), ("Bright", &BRIGHT_COLORS)];
const COLOR_BLOCKS: [&str; 8] = ["■"; 8];

impl ToolbarState {
    pub(super) fn color_menu_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let header = row == MENU_FIRST_ROW;
        let mut spans = Vec::new();
        for (group, (label, colors)) in GROUPS.iter().enumerate() {
            if group > 0 {
                spans.push(plain_span(GAP.to_owned()));
            }
            let path = format!("{}.", group + 2);
            let prefix_width = menu_prefix_width(label, std::iter::once(path.as_str()));
            let cell_width = submenu_cell_width(prefix_width, &COLOR_BLOCKS);
            let cell_start = spans_width(&spans);
            if header {
                let label = format!("{label}:");
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
                for (position, width) in submenu_option_column_widths(&COLOR_BLOCKS)
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
                    == Some(PendingShortcut::ColorGroup(group)))
                .then_some(path.as_str());
                push_shortcut_path(&mut spans, &path, prefix_width, highlighted);
                for index in 0..colors.len() {
                    let color = ColorId((group * colors.len() + index) as u8);
                    if index > 0 {
                        spans.push(plain_span(" ".to_owned()));
                    }
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
            }
            pad_spans_to_width(&mut spans, cell_start + cell_width);
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
    use crate::toolbar::{MainMode, ToggleKind};
    use winit::keyboard::{Key, ModifiersState};

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

    #[test]
    fn all_sixteen_palette_entries_have_exact_colors_and_actions() {
        let mut toolbar = ToolbarState::default();
        assert!(toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode)));
        assert!(toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Colors)));
        let spans = toolbar.color_menu_spans(MENU_FIRST_ROW + 1);
        let colors = spans
            .iter()
            .filter_map(|span| {
                let ToolbarAction::SelectColor(color) = span.action? else {
                    return None;
                };
                Some((color, span.foreground.as_deref()))
            })
            .collect::<Vec<_>>();

        assert_eq!(colors.len(), ColorId::COUNT);
        for (index, (color, foreground)) in colors.into_iter().enumerate() {
            assert_eq!(color, ColorId(index as u8));
            assert_eq!(foreground, color.hex());
        }
        assert_eq!(
            digit_starts(&toolbar.color_menu_spans(MENU_FIRST_ROW)),
            action_starts(&toolbar.color_menu_spans(MENU_FIRST_ROW + 1))
        );
    }

    #[test]
    fn keyboard_paths_select_the_eighth_color_in_each_group() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Colors));
        for (group, expected) in [("2", ColorId(7)), ("3", ColorId(15))] {
            assert!(
                toolbar.handle_shortcut(&Key::Character(group.into()), ModifiersState::empty())
            );
            assert!(toolbar.handle_shortcut(&Key::Character("8".into()), ModifiersState::empty()));
            assert_eq!(toolbar.active_color(), expected);
        }
    }

    #[test]
    fn mode_shortcut_leaves_colors_mode() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Colors));

        assert!(toolbar.handle_shortcut(&Key::Character("1".into()), ModifiersState::empty()));
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Mode));
        assert!(toolbar.handle_shortcut(&Key::Character("1".into()), ModifiersState::empty()));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }
}
