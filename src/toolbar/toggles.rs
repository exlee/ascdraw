use unicode_width::UnicodeWidthStr;

use super::{
    MAIN_LABEL_ROW, MAIN_SHORTCUT_ROW, MainMode, PendingShortcut, ToolbarAction, ToolbarSpan,
    ToolbarState, aligned_shortcut, bold_prefix_span, bold_span, plain_span,
};

const TOGGLE_LABELS: [&str; 3] = ["Dark Mode", "Multi Color Mode", "Multi Layer Mode"];
const TOGGLE_SHORTCUT_OFFSET: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum ToggleKind {
    DarkMode,
    MultiColorMode,
    MultiLayerMode,
}

impl ToggleKind {
    const ALL: [Self; 3] = [Self::DarkMode, Self::MultiColorMode, Self::MultiLayerMode];

    fn index(self) -> usize {
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
            vec![plain_span("   ".to_string()), prefix, plain_span(" ".to_string())]
        };
        for (index, mode) in MainMode::ALL.iter().enumerate() {
            if index > 0 {
                spans.push(plain_span(" ".to_string()));
            }
            let contents = if row == MAIN_LABEL_ROW {
                if index + 1 == MainMode::ALL.len() {
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
                selected: row == MAIN_SHORTCUT_ROW
                    && *mode == self.main_mode
                    && !self.export_open
                    && !self.toggles_open,
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::SelectMain(*mode)),
                right_aligned: false,
            });
        }
        if row == MAIN_LABEL_ROW {
            spans.push(ToolbarSpan {
                contents: "9. Toggles".to_string(),
                bold_prefix: UnicodeWidthStr::width("9."),
                selected: self.toggles_open,
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::ToggleTogglesMenu),
                right_aligned: true,
            });
            let mut gap = plain_span("  ".to_string());
            gap.right_aligned = true;
            spans.push(gap);
            spans.push(ToolbarSpan {
                contents: "0. Save/Load/Export".to_string(),
                bold_prefix: UnicodeWidthStr::width("0."),
                selected: self.export_open,
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::ToggleExportMenu),
                right_aligned: true,
            });
        }
        spans
    }

    pub(super) fn toggles_menu_spans(&self, _row: usize) -> Vec<ToolbarSpan> {
        let mut spans = Vec::new();
        for (index, (toggle, label)) in ToggleKind::ALL.iter().zip(TOGGLE_LABELS).enumerate() {
            if index > 0 {
                spans.push(plain_span("  ".to_string()));
            }
            spans.push(ToolbarSpan {
                contents: format!("{}: {}", label, index + TOGGLE_SHORTCUT_OFFSET),
                bold_prefix: UnicodeWidthStr::width(label) + 1,
                selected: self.toggles[toggle.index()],
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::Toggle(*toggle)),
                right_aligned: false,
            });
        }
        spans
    }

    pub fn toggles_menu_open(&self) -> bool {
        self.toggles_open
    }

    pub fn dark_mode(&self) -> bool {
        self.toggles[ToggleKind::DarkMode.index()]
    }

    pub(super) fn close_toggles_menu(&mut self) {
        self.toggles_open = false;
        if self.shortcut_prefix == Some(PendingShortcut::Toggles) {
            self.shortcut_prefix = None;
        }
    }

    pub(super) fn toggle_toggles_menu(&mut self) {
        if self.toggles_open {
            self.close_toggles_menu();
        } else {
            self.close_export_menu();
            self.toggles_open = true;
            self.shortcut_prefix = Some(PendingShortcut::Toggles);
        }
    }

    pub(super) fn toggle_setting(&mut self, toggle: ToggleKind) {
        let enabled = &mut self.toggles[toggle.index()];
        *enabled = !*enabled;
        self.toggles_open = true;
        self.shortcut_prefix = Some(PendingShortcut::Toggles);
    }

    pub(super) fn select_toggle_digit(&mut self, digit: usize) {
        if let Some(toggle) = digit
            .checked_sub(TOGGLE_SHORTCUT_OFFSET)
            .and_then(|index| ToggleKind::ALL.get(index))
        {
            self.toggle_setting(*toggle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::{MENU_FIRST_ROW, ToolbarAction};
    use winit::keyboard::{Key, ModifiersState};

    fn press(toolbar: &mut ToolbarState, key: &str) {
        assert!(toolbar.handle_shortcut(&Key::Character(key.into()), ModifiersState::empty()));
    }

    #[test]
    fn keyboard_opens_toggles_and_changes_each_visible_state() {
        let mut toolbar = ToolbarState::default();
        let durable = toolbar.durable_selections();

        press(&mut toolbar, "9");
        assert!(toolbar.toggles_menu_open());
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Toggles));
        let labels: Vec<_> = toolbar
            .toolbar_spans(MENU_FIRST_ROW)
            .into_iter()
            .filter_map(|span| span.action.map(|action| (span.contents, action)))
            .collect();
        assert_eq!(
            labels,
            [
                ("Dark Mode: 2".to_string(), ToolbarAction::Toggle(ToggleKind::DarkMode)),
                (
                    "Multi Color Mode: 3".to_string(),
                    ToolbarAction::Toggle(ToggleKind::MultiColorMode),
                ),
                (
                    "Multi Layer Mode: 4".to_string(),
                    ToolbarAction::Toggle(ToggleKind::MultiLayerMode),
                ),
            ]
        );

        for key in ["2", "3", "4"] {
            press(&mut toolbar, key);
        }
        assert!(toolbar.dark_mode());
        assert!(toolbar
            .toolbar_spans(MENU_FIRST_ROW)
            .iter()
            .filter(|span| span.action.is_some())
            .all(|span| span.selected));
        assert_eq!(toolbar.durable_selections(), durable);
    }

    #[test]
    fn mode_prefix_remains_available_while_toggles_are_open() {
        let mut toolbar = ToolbarState::default();

        press(&mut toolbar, "9");
        press(&mut toolbar, "1");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Mode));

        press(&mut toolbar, "2");
        assert_eq!(toolbar.main_mode(), MainMode::Line);
        assert!(!toolbar.toggles_menu_open());
    }

    #[test]
    fn toggles_selection_ends_at_the_label_before_the_unselected_gap() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::ToggleTogglesMenu);

        let spans = toolbar.toolbar_spans(MAIN_LABEL_ROW);
        let toggle = spans
            .iter()
            .position(|span| span.action == Some(ToolbarAction::ToggleTogglesMenu))
            .expect("Toggles action is visible");
        assert_eq!(spans[toggle].contents, "9. Toggles");
        assert!(spans[toggle].selected);
        assert_eq!(spans[toggle + 1].contents, "  ");
        assert!(!spans[toggle + 1].selected);
        assert_eq!(spans[toggle + 1].action, None);
    }

    #[test]
    fn mouse_toggles_are_peer_actions_of_export() {
        let mut toolbar = ToolbarState::default();
        assert!(toolbar.apply_action(ToolbarAction::ToggleTogglesMenu));
        assert!(toolbar.toggles_menu_open());

        assert!(toolbar.apply_action(ToolbarAction::Toggle(ToggleKind::DarkMode)));
        assert!(toolbar.dark_mode());
        assert!(toolbar.toggles_menu_open());

        assert!(toolbar.apply_action(ToolbarAction::ToggleExportMenu));
        assert!(toolbar.export_menu_open());
        assert!(!toolbar.toggles_menu_open());
    }
}
