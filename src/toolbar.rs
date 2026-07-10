use unicode_width::UnicodeWidthStr;
use winit::keyboard::{Key, ModifiersState};

use crate::drawing::LineStyle;

pub const TOOLBAR_ROWS: usize = 2;

const LABELS: [&str; 6] = [
    "1. Line",
    "2. Line Start",
    "3. Line End",
    "4. Decorators",
    "5. Shapes",
    "6. Tools",
];
const OPTIONS: [&[&str]; 6] = [
    &["─", "━", "═"],
    &["·", "◀", "◆", "●"],
    &["·", "▶", "◆", "●"],
    &["·", "○", "◇", "□"],
    &["□", "○", "◇", "△"],
    &["✎", "T", "⌫", "⌖"],
];
const GAP: &str = "    ";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolbarState {
    selected: [usize; LABELS.len()],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarSpan {
    pub contents: String,
    pub selected: bool,
}

impl ToolbarState {
    pub fn cycle_shortcut(&mut self, key: &Key, modifiers: ModifiersState) -> bool {
        if !modifiers.is_empty() {
            return false;
        }

        let Key::Character(text) = key else {
            return false;
        };
        let Some(index) = text
            .chars()
            .next()
            .filter(|_| text.chars().count() == 1)
            .and_then(|digit| digit.to_digit(10))
            .and_then(|digit| usize::try_from(digit).ok())
            .and_then(|digit| digit.checked_sub(1))
            .filter(|index| *index < LABELS.len())
        else {
            return false;
        };

        self.selected[index] = (self.selected[index] + 1) % OPTIONS[index].len();
        true
    }

    pub fn header_line(&self) -> String {
        LABELS.join(GAP)
    }

    pub fn value_spans(&self) -> Vec<ToolbarSpan> {
        let mut spans = Vec::new();
        for (index, label) in LABELS.iter().enumerate() {
            if index > 0 {
                spans.push(ToolbarSpan {
                    contents: GAP.to_string(),
                    selected: false,
                });
            }
            let mut options_width = 0;
            for (option_index, option) in OPTIONS[index].iter().enumerate() {
                if option_index > 0 {
                    spans.push(ToolbarSpan {
                        contents: " ".to_string(),
                        selected: false,
                    });
                    options_width += 1;
                }
                spans.push(ToolbarSpan {
                    contents: (*option).to_string(),
                    selected: option_index == self.selected[index],
                });
                options_width += UnicodeWidthStr::width(*option);
            }
            let padding = UnicodeWidthStr::width(*label).saturating_sub(options_width);
            if padding > 0 {
                spans.push(ToolbarSpan {
                    contents: " ".repeat(padding),
                    selected: false,
                });
            }
        }
        spans
    }

    pub fn line_style(&self) -> LineStyle {
        match self.selected[0] {
            0 => LineStyle::Thin,
            1 => LineStyle::Heavy,
            2 => LineStyle::Double,
            _ => unreachable!("line toolbar selection is always normalized"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_shortcuts_cycle_the_corresponding_symbol() {
        let mut toolbar = ToolbarState::default();
        assert!(toolbar.cycle_shortcut(&Key::Character("2".into()), ModifiersState::empty()));
        let spans = toolbar.value_spans();
        assert!(
            spans
                .iter()
                .any(|span| span.contents == "◀" && span.selected)
        );
        assert!(
            spans
                .iter()
                .any(|span| span.contents == "─" && span.selected)
        );
        assert!(!toolbar.cycle_shortcut(&Key::Character("2".into()), ModifiersState::SUPER));
    }

    #[test]
    fn value_row_contains_every_option_and_one_selection_per_category() {
        let toolbar = ToolbarState::default();
        let spans = toolbar.value_spans();
        let contents: String = spans.iter().map(|span| span.contents.as_str()).collect();

        for option in OPTIONS.into_iter().flatten() {
            assert!(contents.contains(option));
        }
        assert_eq!(
            spans.iter().filter(|span| span.selected).count(),
            LABELS.len()
        );
    }

    #[test]
    fn line_shortcut_cycles_drawing_style() {
        let mut toolbar = ToolbarState::default();
        assert_eq!(toolbar.line_style(), LineStyle::Thin);
        toolbar.cycle_shortcut(&Key::Character("1".into()), ModifiersState::empty());
        assert_eq!(toolbar.line_style(), LineStyle::Heavy);
        toolbar.cycle_shortcut(&Key::Character("1".into()), ModifiersState::empty());
        assert_eq!(toolbar.line_style(), LineStyle::Double);
        toolbar.cycle_shortcut(&Key::Character("1".into()), ModifiersState::empty());
        assert_eq!(toolbar.line_style(), LineStyle::Thin);
    }
}
