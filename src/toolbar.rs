use unicode_width::UnicodeWidthStr;
use winit::keyboard::{Key, ModifiersState};

use crate::drawing::{LineEnding, LineStyle};

pub const TOOLBAR_ROWS: usize = 3;
pub const TOOLBAR_ROW_GAP: usize = 6;

pub fn toolbar_height(cell_height: usize) -> usize {
    TOOLBAR_ROWS * cell_height + (TOOLBAR_ROWS - 1) * TOOLBAR_ROW_GAP
}

pub fn toolbar_row_offset(row: usize, _cell_height: usize) -> usize {
    row * TOOLBAR_ROW_GAP
}

const GAP: &str = "    ";
const LINE_LABELS: [&str; 3] = ["Line Start", "Line End", "Line Width"];
const LINE_OPTIONS: [&[&str]; 3] = [
    &["·", "◀", "◆", "●"],
    &["·", "▶", "◆", "●"],
    &["─", "━", "═"],
];
const STAMP_LABELS: [&str; 2] = ["Decorators", "Fills"];
const STAMP_OPTIONS: [&[&str]; 2] = [
    &[
        "○", "●", "◇", "◆", "□", "■", "△", "▲", "☆", "★", "+", "×", "※", "•",
    ],
    &[
        "░", "▒", "▓", "█", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "▀", "▌", "▐", "▊", "▉",
    ],
];
const SHAPE_LABELS: [&str; 3] = ["Shape", "Line", "Fill"];
const SHAPE_OPTIONS: [&[&str]; 3] = [
    &["Rect", "Rnd Rect", "Ellipsis"],
    &["─", "━", "═"],
    &["·", "░", "▒", "▓", "█"],
];
const UTILITY_LABELS: [&str; 1] = ["Select"];
const UTILITY_OPTIONS: [&[&str]; 1] = [["⌖"].as_slice()];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MainMode {
    #[default]
    Line,
    Stamp,
    Shapes,
    Utilities,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShapeKind {
    #[default]
    Rect,
    RoundedRect,
    Ellipse,
}

impl MainMode {
    const ALL: [Self; 4] = [Self::Line, Self::Stamp, Self::Shapes, Self::Utilities];

    fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Stamp => "Stamp",
            Self::Shapes => "Shape",
            Self::Utilities => "Utils",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolbarState {
    main_mode: MainMode,
    line_selected: [usize; LINE_LABELS.len()],
    stamp_selected: [usize; STAMP_LABELS.len()],
    stamp_active_submenu: usize,
    shape_selected: [usize; SHAPE_LABELS.len()],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarSpan {
    pub contents: String,
    pub selected: bool,
    pub action: Option<ToolbarAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarAction {
    CycleMain,
    CycleSubmenu(usize),
    SelectMain(MainMode),
    SelectSubmenu { submenu: usize, option: usize },
}

struct MenuLayout<'a> {
    labels: &'a [&'a str],
    options: &'a [&'a [&'a str]],
    selected: &'a [usize],
    exclusive_submenu: Option<usize>,
}

impl ToolbarState {
    pub fn cycle_shortcut(&mut self, key: &Key, modifiers: ModifiersState) -> bool {
        if modifiers != ModifiersState::empty() && modifiers != ModifiersState::SHIFT {
            return false;
        }

        let Key::Character(text) = key else {
            return false;
        };
        let Some(index) = toolbar_index(text, modifiers.shift_key()) else {
            return false;
        };
        let backwards = modifiers.shift_key();

        if index == 0 {
            let current = MainMode::ALL
                .iter()
                .position(|mode| *mode == self.main_mode)
                .expect("main mode is in the mode list");
            self.main_mode = MainMode::ALL[cycle_index(current, MainMode::ALL.len(), backwards)];
            return true;
        }

        let submenu = index - 1;
        match self.main_mode {
            MainMode::Line => {
                cycle_selected(&mut self.line_selected, &LINE_OPTIONS, submenu, backwards)
            }
            MainMode::Stamp => {
                if !cycle_selected(&mut self.stamp_selected, &STAMP_OPTIONS, submenu, backwards) {
                    return false;
                }
                self.stamp_active_submenu = submenu;
                true
            }
            MainMode::Shapes => {
                cycle_selected(&mut self.shape_selected, &SHAPE_OPTIONS, submenu, backwards)
            }
            MainMode::Utilities => submenu == 0,
        }
    }

    pub fn main_mode(&self) -> MainMode {
        self.main_mode
    }

    pub fn main_spans(&self) -> Vec<ToolbarSpan> {
        let mut spans = vec![ToolbarSpan {
            contents: "1. ".to_string(),
            selected: false,
            action: Some(ToolbarAction::CycleMain),
        }];
        for (index, mode) in MainMode::ALL.iter().enumerate() {
            if index > 0 {
                spans.push(ToolbarSpan {
                    contents: " ".to_string(),
                    selected: false,
                    action: None,
                });
            }
            spans.push(ToolbarSpan {
                contents: mode.label().to_string(),
                selected: *mode == self.main_mode,
                action: Some(ToolbarAction::SelectMain(*mode)),
            });
        }
        spans
    }

    pub fn submenu_spans(&self) -> Vec<ToolbarSpan> {
        let layout = self.layout();
        let mut spans = Vec::new();
        for (index, label) in layout.labels.iter().enumerate() {
            if index > 0 {
                spans.push(ToolbarSpan {
                    contents: GAP.to_string(),
                    selected: false,
                    action: None,
                });
            }
            spans.push(ToolbarSpan {
                contents: format!("{}. {label} ", index + 2),
                selected: false,
                action: Some(ToolbarAction::CycleSubmenu(index)),
            });
            for (option_index, option) in layout.options[index].iter().enumerate() {
                if option_index > 0 {
                    spans.push(ToolbarSpan {
                        contents: " ".to_string(),
                        selected: false,
                        action: None,
                    });
                }
                spans.push(ToolbarSpan {
                    contents: (*option).to_string(),
                    selected: option_index == layout.selected[index]
                        && layout
                            .exclusive_submenu
                            .is_none_or(|active| active == index),
                    action: Some(ToolbarAction::SelectSubmenu {
                        submenu: index,
                        option: option_index,
                    }),
                });
            }
        }
        spans
    }

    pub fn tooltip(&self) -> &'static str {
        match self.main_mode {
            MainMode::Line => {
                "Shift-<hjkl> or Shift-arrow to draw, Alt-<hjkl>, Alt-<arrow> to erase"
            }
            MainMode::Stamp => {
                "<Space> to put stamp, <Shift> + direction to draw in line, <Ctrl>+direction - fill rectangle"
            }
            MainMode::Shapes => {
                "<Escape> to start shape preview, <Space> to confirm, <Escape> to cancel"
            }
            MainMode::Utilities => {
                "Space start, then Space confirm, Shift arrows - move selection, Backspace/Del clear selection"
            }
        }
    }

    pub fn line_style(&self) -> LineStyle {
        line_style(self.line_selected[2])
    }

    pub fn line_start(&self) -> LineEnding {
        line_ending(self.line_selected[0])
    }

    pub fn line_end(&self) -> LineEnding {
        line_ending(self.line_selected[1])
    }

    pub fn stamp(&self) -> &'static str {
        STAMP_OPTIONS[self.stamp_active_submenu][self.stamp_selected[self.stamp_active_submenu]]
    }

    pub fn shape_kind(&self) -> ShapeKind {
        match self.shape_selected[0] {
            0 => ShapeKind::Rect,
            1 => ShapeKind::RoundedRect,
            2 => ShapeKind::Ellipse,
            _ => unreachable!("shape selection is always normalized"),
        }
    }

    pub fn shape_line_style(&self) -> LineStyle {
        line_style(self.shape_selected[1])
    }

    pub fn shape_fill(&self) -> Option<&'static str> {
        self.shape_selected[2]
            .checked_sub(1)
            .map(|index| SHAPE_OPTIONS[2][index + 1])
    }

    pub fn action_at(&self, row: usize, column: usize) -> Option<ToolbarAction> {
        let spans = match row {
            0 => self.main_spans(),
            1 => self.submenu_spans(),
            _ => return None,
        };
        let mut start = 0;
        for span in spans {
            let end = start + UnicodeWidthStr::width(span.contents.as_str());
            if (start..end).contains(&column) {
                return span.action;
            }
            start = end;
        }
        None
    }

    pub fn apply_action(&mut self, action: ToolbarAction) -> bool {
        match action {
            ToolbarAction::CycleMain => {
                let current = MainMode::ALL
                    .iter()
                    .position(|mode| *mode == self.main_mode)
                    .expect("main mode is in the mode list");
                self.main_mode = MainMode::ALL[cycle_index(current, MainMode::ALL.len(), false)];
                true
            }
            ToolbarAction::CycleSubmenu(submenu) => match self.main_mode {
                MainMode::Line => {
                    cycle_selected(&mut self.line_selected, &LINE_OPTIONS, submenu, false)
                }
                MainMode::Stamp => {
                    if !cycle_selected(&mut self.stamp_selected, &STAMP_OPTIONS, submenu, false) {
                        return false;
                    }
                    self.stamp_active_submenu = submenu;
                    true
                }
                MainMode::Shapes => {
                    cycle_selected(&mut self.shape_selected, &SHAPE_OPTIONS, submenu, false)
                }
                MainMode::Utilities => submenu == 0,
            },
            ToolbarAction::SelectMain(mode) => {
                self.main_mode = mode;
                true
            }
            ToolbarAction::SelectSubmenu { submenu, option } => {
                let option_count = match self.main_mode {
                    MainMode::Line => LINE_OPTIONS.get(submenu),
                    MainMode::Stamp => STAMP_OPTIONS.get(submenu),
                    MainMode::Shapes => SHAPE_OPTIONS.get(submenu),
                    MainMode::Utilities => return submenu == 0 && option == 0,
                }
                .map(|options| options.len());
                if option_count.is_none_or(|count| option >= count) {
                    return false;
                }
                let selected = match self.main_mode {
                    MainMode::Line => self.line_selected.get_mut(submenu),
                    MainMode::Stamp => {
                        self.stamp_active_submenu = submenu;
                        self.stamp_selected.get_mut(submenu)
                    }
                    MainMode::Shapes => self.shape_selected.get_mut(submenu),
                    MainMode::Utilities => unreachable!("utilities returned above"),
                };
                let Some(selected) = selected else {
                    return false;
                };
                *selected = option;
                true
            }
        }
    }

    fn layout(&self) -> MenuLayout<'_> {
        match self.main_mode {
            MainMode::Line => MenuLayout {
                labels: &LINE_LABELS,
                options: &LINE_OPTIONS,
                selected: &self.line_selected,
                exclusive_submenu: None,
            },
            MainMode::Stamp => MenuLayout {
                labels: &STAMP_LABELS,
                options: &STAMP_OPTIONS,
                selected: &self.stamp_selected,
                exclusive_submenu: Some(self.stamp_active_submenu),
            },
            MainMode::Shapes => MenuLayout {
                labels: &SHAPE_LABELS,
                options: &SHAPE_OPTIONS,
                selected: &self.shape_selected,
                exclusive_submenu: None,
            },
            MainMode::Utilities => MenuLayout {
                labels: &UTILITY_LABELS,
                options: &UTILITY_OPTIONS,
                selected: &[0],
                exclusive_submenu: None,
            },
        }
    }
}

fn toolbar_index(text: &str, shifted: bool) -> Option<usize> {
    let digit: usize = match (text, shifted) {
        ("1" | "!", true) | ("1", false) => 1,
        ("2" | "@", true) | ("2", false) => 2,
        ("3" | "#", true) | ("3", false) => 3,
        ("4" | "$", true) | ("4", false) => 4,
        _ => return None,
    };
    digit.checked_sub(1)
}

fn cycle_selected(
    selected: &mut [usize],
    options: &[&[&str]],
    submenu: usize,
    backwards: bool,
) -> bool {
    let (Some(selected), Some(options)) = (selected.get_mut(submenu), options.get(submenu)) else {
        return false;
    };
    *selected = cycle_index(*selected, options.len(), backwards);
    true
}

fn cycle_index(current: usize, count: usize, backwards: bool) -> usize {
    if backwards {
        (current + count - 1) % count
    } else {
        (current + 1) % count
    }
}

fn line_style(selected: usize) -> LineStyle {
    match selected {
        0 => LineStyle::Thin,
        1 => LineStyle::Heavy,
        2 => LineStyle::Double,
        _ => unreachable!("line width selection is always normalized"),
    }
}

fn line_ending(selected: usize) -> LineEnding {
    match selected {
        0 => LineEnding::None,
        1 => LineEnding::Arrow,
        2 => LineEnding::Diamond,
        3 => LineEnding::Circle,
        _ => unreachable!("line ending selection is always normalized"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cycle(toolbar: &mut ToolbarState, key: &str) {
        assert!(toolbar.cycle_shortcut(&Key::Character(key.into()), ModifiersState::empty()));
    }

    #[test]
    fn one_cycles_the_main_mode_and_shift_one_reverses_it() {
        let mut toolbar = ToolbarState::default();
        cycle(&mut toolbar, "1");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert!(toolbar.cycle_shortcut(&Key::Character("!".into()), ModifiersState::SHIFT,));
        assert_eq!(toolbar.main_mode(), MainMode::Line);
        assert_eq!(
            toolbar
                .main_spans()
                .iter()
                .map(|span| span.contents.as_str())
                .collect::<String>(),
            "1. Line Stamp Shape Utils"
        );
        assert_eq!(
            toolbar
                .main_spans()
                .iter()
                .filter(|span| span.selected)
                .count(),
            1
        );
    }

    #[test]
    fn line_submenus_use_two_through_four() {
        let mut toolbar = ToolbarState::default();
        cycle(&mut toolbar, "2");
        cycle(&mut toolbar, "3");
        cycle(&mut toolbar, "4");

        assert_eq!(toolbar.line_start(), LineEnding::Arrow);
        assert_eq!(toolbar.line_end(), LineEnding::Arrow);
        assert_eq!(toolbar.line_style(), LineStyle::Heavy);
        assert_eq!(
            toolbar
                .submenu_spans()
                .iter()
                .map(|span| span.contents.as_str())
                .collect::<String>(),
            "2. Line Start · ◀ ◆ ●    3. Line End · ▶ ◆ ●    4. Line Width ─ ━ ═"
        );
    }

    #[test]
    fn stamp_decorators_and_fills_are_exclusive() {
        let mut toolbar = ToolbarState::default();
        cycle(&mut toolbar, "1");
        cycle(&mut toolbar, "2");
        assert_eq!(toolbar.stamp_selected, [1, 0]);

        cycle(&mut toolbar, "3");
        assert_eq!(toolbar.stamp_selected, [1, 1]);
        assert_eq!(toolbar.stamp_active_submenu, 1);
        assert_eq!(
            toolbar
                .submenu_spans()
                .iter()
                .filter(|span| span.selected)
                .count(),
            1
        );
    }

    #[test]
    fn mode_controls_visible_submenus_and_tooltip() {
        let mut toolbar = ToolbarState::default();
        cycle(&mut toolbar, "1");
        assert_eq!(
            toolbar
                .submenu_spans()
                .iter()
                .map(|span| span.contents.as_str())
                .collect::<String>(),
            "2. Decorators ○ ● ◇ ◆ □ ■ △ ▲ ☆ ★ + × ※ •    3. Fills ░ ▒ ▓ █ ▁ ▂ ▃ ▄ ▅ ▆ ▇ ▀ ▌ ▐ ▊ ▉"
        );
        assert_eq!(
            toolbar
                .submenu_spans()
                .iter()
                .filter(|span| span.selected)
                .count(),
            1
        );
        assert!(toolbar.tooltip().starts_with("<Space>"));

        cycle(&mut toolbar, "1");
        assert_eq!(
            toolbar.tooltip(),
            "<Space> to start creating shape, <Space> to confirm"
        );

        cycle(&mut toolbar, "1");
        assert!(
            toolbar
                .tooltip()
                .starts_with("Space start, then Space confirm")
        );
    }

    #[test]
    fn mouse_hit_testing_selects_main_modes_and_submenu_options() {
        let mut toolbar = ToolbarState::default();
        let action = toolbar.action_at(0, 8).expect("Stamp is clickable");
        assert_eq!(action, ToolbarAction::SelectMain(MainMode::Stamp));
        assert!(toolbar.apply_action(action));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        assert!(toolbar.apply_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 13,
        }));
        assert_eq!(toolbar.stamp(), "▐");
        assert_eq!(toolbar.stamp_active_submenu, 1);

        assert_eq!(
            toolbar.action_at(1, 0),
            Some(ToolbarAction::CycleSubmenu(0))
        );
    }

    #[test]
    fn every_stamp_symbol_is_one_utf8_character_and_one_display_cell() {
        for symbol in STAMP_OPTIONS.into_iter().flatten() {
            assert_eq!(symbol.chars().count(), 1, "{symbol:?}");
            assert_eq!(UnicodeWidthStr::width(*symbol), 1, "{symbol:?}");
        }
    }

    #[test]
    fn rejects_unavailable_submenu_and_unrelated_modifiers() {
        let mut toolbar = ToolbarState::default();
        assert!(!toolbar.cycle_shortcut(&Key::Character("5".into()), ModifiersState::empty()));
        assert!(!toolbar.cycle_shortcut(&Key::Character("2".into()), ModifiersState::SUPER));
    }
}
