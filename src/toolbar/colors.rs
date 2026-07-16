use unicode_width::UnicodeWidthStr;

use crate::model::{BASE_COLORS, BRIGHT_COLORS, ColorId};

use super::{
    MENU_FIRST_ROW, PendingShortcut, ToolbarAction, ToolbarSpan, ToolbarState, bold_prefix_span,
    plain_span,
};

const GROUPS: [(&str, &[&str; 8]); 2] = [("Base", &BASE_COLORS), ("Bright", &BRIGHT_COLORS)];

impl ToolbarState {
    pub(super) fn color_menu_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let header = row == MENU_FIRST_ROW;
        let mut spans = Vec::new();
        for (group, (label, colors)) in GROUPS.iter().enumerate() {
            if group > 0 {
                spans.push(plain_span("    ".to_owned()));
            }
            if header {
                let label = format!("{label}:");
                spans.push(bold_prefix_span(label.clone(), &label));
                for digit in 1..=colors.len() {
                    spans.push(plain_span(format!(" {digit}")));
                }
                continue;
            }
            let prefix = format!("{}.", group + 2);
            spans.push(ToolbarSpan {
                contents: prefix.clone(),
                bold_prefix: UnicodeWidthStr::width(prefix.as_str()),
                selected: false,
                highlighted: self.pending_shortcut() == Some(PendingShortcut::ColorGroup(group)),
                tooltip: false,
                action: None,
                right_aligned: false,
                foreground: None,
            });
            for index in 0..colors.len() {
                let color = ColorId((group * colors.len() + index) as u8);
                spans.push(plain_span(" ".to_owned()));
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
}
