use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::drawing::{ARROWS, CornerStyle, DECORATORS, LINE_ENDINGS, LineEnding, LineStyle};
use crate::export::ExportAction;
#[cfg(test)]
use crate::{
    drawing::{DIRECTIONAL_ENDINGS, line_ending_glyph},
    model::Direction,
};

mod menu_layout;
mod selections;
pub use selections::DurableMenuSelections;

pub const TOOLBAR_ROW_GAP: usize = 0;

const MAIN_LABEL_ROW: usize = 0;
const MAIN_SHORTCUT_ROW: usize = 1;
const MENU_FIRST_ROW: usize = 2;
const OPTIONS_PER_PAGE: usize = 10;
const GAP: &str = "    ";
pub const TOOLTIP_ROTATION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

pub fn toolbar_height(toolbar: &ToolbarState, cell_height: usize) -> usize {
    let rows = toolbar.rows();
    rows * cell_height + rows.saturating_sub(1) * TOOLBAR_ROW_GAP
}

pub fn toolbar_row_offset(row: usize, _cell_height: usize) -> usize {
    row * TOOLBAR_ROW_GAP
}

pub fn toolbar_content_row(row: usize) -> usize {
    row + 1
}

pub fn toolbar_border_spans(width: usize, top: bool) -> Vec<ToolbarSpan> {
    if width == 0 {
        return Vec::new();
    }
    if width == 1 {
        return vec![plain_span(if top { "┌" } else { "└" }.to_string())];
    }
    vec![plain_span(format!(
        "{}{}{}",
        if top { '┌' } else { '└' },
        "─".repeat(width - 2),
        if top { '┐' } else { '┘' }
    ))]
}

pub fn boxed_toolbar_spans(spans: &[ToolbarSpan], width: usize) -> Vec<ToolbarSpan> {
    if width == 0 {
        return Vec::new();
    }
    if width == 1 {
        return vec![plain_span("│".to_string())];
    }

    let interior_width = width - 2;
    let left_padding = usize::from(interior_width > 0);
    let right_padding = usize::from(interior_width > 1);
    let content_width = interior_width - left_padding - right_padding;
    let mut boxed = vec![plain_span("│".to_string())];
    if left_padding > 0 {
        boxed.push(plain_span(" ".to_string()));
    }
    let split = spans.iter().position(|span| span.right_aligned);
    let (left, right) = split.map_or((spans, &[][..]), |index| spans.split_at(index));
    let right_width: usize = right
        .iter()
        .map(|span| UnicodeWidthStr::width(span.contents.as_str()))
        .sum();
    let show_right = !right.is_empty() && right_width < content_width;
    let left_width = content_width.saturating_sub(if show_right { right_width + 1 } else { 0 });
    let used_left = push_clipped_spans(&mut boxed, left, left_width);
    let mut remaining = content_width.saturating_sub(used_left);
    if show_right {
        let padding = remaining.saturating_sub(right_width);
        if padding > 0 {
            boxed.push(plain_span(" ".repeat(padding)));
            remaining -= padding;
        }
        let used_right = push_clipped_spans(&mut boxed, right, remaining);
        remaining = remaining.saturating_sub(used_right);
    }
    if remaining > 0 {
        boxed.push(plain_span(" ".repeat(remaining)));
    }
    if right_padding > 0 {
        boxed.push(plain_span(" ".to_string()));
    }
    boxed.push(plain_span("│".to_string()));
    boxed
}

pub fn tooltip_spans(tooltip: Tooltip, width: usize) -> Vec<ToolbarSpan> {
    if tooltip == Tooltip::None || width == 0 {
        return Vec::new();
    }
    let (contents, _) = clipped_to_width(tooltip.text().as_str(), width);
    vec![ToolbarSpan {
        contents,
        bold_prefix: 0,
        selected: false,
        highlighted: false,
        tooltip: true,
        action: None,
        right_aligned: false,
    }]
}

fn push_clipped_spans(
    target: &mut Vec<ToolbarSpan>,
    spans: &[ToolbarSpan],
    max_width: usize,
) -> usize {
    let mut remaining = max_width;
    for span in spans {
        if remaining == 0 {
            break;
        }
        let (contents, used) = clipped_to_width(&span.contents, remaining);
        if used > 0 {
            target.push(ToolbarSpan {
                contents,
                bold_prefix: span.bold_prefix.min(used),
                ..span.clone()
            });
            remaining -= used;
        }
    }
    max_width - remaining
}

fn clipped_to_width(contents: &str, max_width: usize) -> (String, usize) {
    let mut clipped = String::new();
    let mut width = 0;
    for character in contents.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > max_width {
            break;
        }
        clipped.push(character);
        width += character_width;
    }
    (clipped, width)
}

const LINE_LABELS: [&str; 4] = ["Start", "End", "Width", "Corner"];
const LINE_START_OPTIONS: [&str; LINE_ENDINGS.len()] = [
    " ", "◁", "◀", "←", "◃", "◂", "↔", "□", "■", "▫", "▪", "◆", "◊", "·", "∙", "•", "●", "◦",
    "Ø", "ø", "╳", "╱", "╲", "÷", "×", "±", "¤",
];
const LINE_END_OPTIONS: [&str; LINE_ENDINGS.len()] = [
    " ", "▷", "▶", "→", "▹", "▸", "↔", "□", "■", "▫", "▪", "◆", "◊", "·", "∙", "•", "●", "◦",
    "Ø", "ø", "╳", "╱", "╲", "÷", "×", "±", "¤",
];
const LINE_OPTIONS: [&[&str]; 4] = [
    &LINE_START_OPTIONS,
    &LINE_END_OPTIONS,
    &["─", "━", "═"],
    &["Smooth", "Sharp"],
];
// Stamp contains Uniline's standalone drawing vocabularies. Connected box-drawing
// lines remain the responsibility of the connection-mask engine in drawing.rs;
// only the three standalone diagonal operators are repeated here.
#[cfg(test)]
const SQUARES_AND_DIAMONDS: [&str; 6] = ["□", "■", "▫", "▪", "◆", "◊"];
#[cfg(test)]
const DOTS_AND_CIRCLES: [&str; 7] = ["·", "∙", "•", "●", "◦", "Ø", "ø"];
#[cfg(test)]
const CROSSES_AND_OPERATORS: [&str; 7] = ["╳", "╱", "╲", "÷", "×", "±", "¤"];
#[cfg(test)]
const ARROW_ROTATIONS: [[&str; 4]; 6] = [
    ["△", "▽", "◁", "▷"],
    ["▲", "▼", "◀", "▶"],
    ["↑", "↓", "←", "→"],
    ["▵", "▿", "◃", "▹"],
    ["▴", "▾", "◂", "▸"],
    ["↕", "↕", "↔", "↔"],
];
const GREY_SHADING: [&str; 4] = ["░", "▒", "▓", "█"];
const QUADRANT_BLOCKS: [&str; 15] = [
    "▘", "▝", "▀", "▖", "▌", "▞", "▛", "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
];

const STAMP_LABELS: [&str; 4] = ["Decorators", "Arrows", "Fills", "Blocks"];
const STAMP_OPTIONS: [&[&str]; 4] = [&DECORATORS, &ARROWS, &GREY_SHADING, &QUADRANT_BLOCKS];
const SHAPE_LABELS: [&str; 3] = ["Shape", "Line", "Fill"];
const SHAPE_OPTIONS: [&[&str]; 3] = [
    &["Rect", "Round"],
    &["─", "━", "═"],
    &[" ", "░", "▒", "▓", "█"],
];
const UTILITY_OPTIONS: [&[&str]; 1] = [["Move", "Push", "Pull", "View"].as_slice()];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MainMode {
    #[default]
    Stamp,
    Line,
    Shapes,
    Utilities,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ShapeKind {
    #[default]
    Rect,
    RoundedRect,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UtilityKind {
    #[default]
    Move,
    Push,
    Pull,
    View,
}

impl MainMode {
    pub const ALL: [Self; 4] = [Self::Stamp, Self::Line, Self::Shapes, Self::Utilities];

    fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Stamp => "Stamp",
            Self::Shapes => "Shape",
            Self::Utilities => "Utils",
        }
    }

    pub fn tooltip(self) -> Tooltip {
        match self {
            Self::Line => Tooltip::Line,
            Self::Stamp => Tooltip::Stamp,
            Self::Shapes => Tooltip::Shapes,
            Self::Utilities => Tooltip::UtilitiesMove,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tooltip {
    #[default]
    None,
    Line,
    Stamp,
    Shapes,
    UtilitiesPush,
    UtilitiesPull,
    UtilitiesView,
    UtilitiesMove,
    MoveLift,
    ShapePreview,
    SingleReplace,
    LineStroke,
    Text,
    Replace,
    Export,
    Selection,
}

impl Tooltip {
    pub fn text(self) -> String {
        const MISC_TIP: [&str; 3] = [
            "Canvas: u undo; U redo; Ctrl/Cmd-Z undo; Ctrl/Cmd-R redo",
            "Direction keys are ←→↓↑ and hjkl",
            "When drawing/selecting/resizing add Ctrl/Alt/Shift for 5/10 steps",
        ];
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;

        let selector = (timestamp / TOOLTIP_ROTATION_INTERVAL.as_secs() as usize) % MISC_TIP.len();
        let misc = MISC_TIP[selector];

        let primary = match self {
            Self::None => "",
            Self::Line => {
                "Line: Shift-direction draws; Alt-direction erases; Ctrl-direction selects"
            }
            Self::Stamp => {
                "Stamp: Space places; Shift-direction draws continuously; Alt-direction erases; Ctrl-direction selects"
            }
            Self::Shapes => {
                "Shape: Space starts a preview; Alt-direction erases; Ctrl-direction selects"
            }
            Self::UtilitiesPush => {
                "Push: Shift-direction inserts a blank row or column; Alt-direction erases"
            }
            Self::UtilitiesPull => "Pull: Shift-direction pulls; Alt-direction erases",
            Self::UtilitiesView => "View: directions pan; Space centers; Alt-direction erases",
            Self::UtilitiesMove => {
                "Move: Space lifts the current cell; Alt-direction erases; Ctrl-direction selects"
            }
            Self::MoveLift => {
                "Move: directions or Alt-direction move; Space/Enter confirms; Esc cancels"
            }
            Self::ShapePreview => "Shape preview: directions resize; Space confirms; Esc cancels",
            Self::SingleReplace => "Replace selection: type one character; Esc cancels",
            Self::LineStroke => "Line stroke: Shift-direction continues; release Shift to finish",
            Self::Text => "<Ret> exits text mode; arrows move freely over the canvas",
            Self::Replace => "<Shift-Ret> exits replace mode; arrows move freely over the canvas",
            Self::Export => {
                "TXT/PNG export selection or visible viewport; JSON exports the whole project"
            }
            Self::Selection => {
                "Selection: Alt-direction lifts and moves; Ctrl-direction expands; Esc collapses; Space/Backspace clears; r then KEY replaces"
            }
        };
        if matches!(
            self,
            Self::MoveLift
                | Self::ShapePreview
                | Self::SingleReplace
                | Self::LineStroke
                | Self::Export
                | Self::Selection
        ) {
            return primary.to_string();
        }
        let secondary = if primary.is_empty() { "" } else { "; " };

        format!("{primary}{secondary}{misc}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingShortcut {
    Mode,
    Category(usize),
    Option { category: usize, page: usize },
    ExportCategory,
    ExportOption(usize),
    ExportFlat(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CategoryShortcut {
    Select(usize),
    Page(usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolbarState {
    main_mode: MainMode,
    line_selected: [usize; LINE_LABELS.len()],
    stamp_selected: [usize; STAMP_LABELS.len()],
    stamp_active_category: usize,
    shape_selected: [usize; SHAPE_LABELS.len()],
    utility_selected: usize,
    shortcut_prefix: Option<PendingShortcut>,
    export_open: bool,
    active_export_category: Option<usize>,
    pending_export_action: Option<ExportAction>,
    successful_export_action: Option<ExportAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarSpan {
    pub contents: String,
    pub bold_prefix: usize,
    pub selected: bool,
    pub highlighted: bool,
    pub tooltip: bool,
    pub action: Option<ToolbarAction>,
    pub right_aligned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarAction {
    SelectMain(MainMode),
    SelectSubmenu { submenu: usize, option: usize },
    ToggleExportMenu,
    SelectExportCategory(usize),
    RunExport(ExportAction),
}

const EXPORT_LABELS: [&str; 4] = ["Clipboard", "Save", "Load", "Clear"];
const EXPORT_MODE_OFFSET: usize = 2;
const EXPORT_CLEAR_DIGIT: usize = 9;
const EXPORT_OPTIONS: [&[(&str, ExportAction)]; 4] = [
    &[
        ("TXT", ExportAction::ClipboardTxt),
        ("PNG", ExportAction::ClipboardPng),
    ],
    &[
        ("TXT", ExportAction::SaveTxt),
        ("PNG", ExportAction::SavePng),
        ("JSON", ExportAction::SaveJson),
    ],
    &[
        ("TXT", ExportAction::LoadTxt),
        ("JSON", ExportAction::LoadJson),
    ],
    &[("Clear", ExportAction::Clear)],
];

struct MenuLayout<'a> {
    labels: &'a [&'a str],
    options: &'a [&'a [&'a str]],
    selected: &'a [usize],
    exclusive_submenu: Option<usize>,
}

impl ToolbarState {
    pub fn handle_shortcut(&mut self, key: &Key, modifiers: ModifiersState) -> bool {
        if matches!(key, Key::Named(NamedKey::Escape))
            && (self.shortcut_prefix.is_some() || self.export_open)
        {
            self.close_export_menu();
            return true;
        }

        if modifiers != ModifiersState::empty() {
            return self.cancel_pending_shortcut();
        }
        let Key::Character(text) = key else {
            return self.cancel_pending_shortcut();
        };
        let Some(digit) = shortcut_digit(text) else {
            return self.cancel_pending_shortcut();
        };

        if digit == 0 && self.export_open {
            self.close_export_menu();
            return true;
        }

        match self.shortcut_prefix.take() {
            None => {
                if digit == 0 {
                    self.toggle_export_menu();
                } else if self.export_open {
                    self.select_export_mode_digit(digit);
                } else if self.main_mode == MainMode::Utilities
                    && let Some(option) = digit
                        .checked_sub(2)
                        .filter(|option| *option < UTILITY_OPTIONS[0].len())
                {
                    self.apply_action(ToolbarAction::SelectSubmenu { submenu: 0, option });
                } else {
                    self.shortcut_prefix = if digit == 1 {
                        Some(PendingShortcut::Mode)
                    } else {
                        digit
                            .checked_sub(2)
                            .filter(|category| {
                                self.layout()
                                    .is_some_and(|layout| *category < layout.labels.len())
                            })
                            .map(PendingShortcut::Category)
                    };
                }
            }
            Some(PendingShortcut::Mode) => {
                if let Some(mode) = digit
                    .checked_sub(1)
                    .and_then(|index| MainMode::ALL.get(index))
                {
                    self.main_mode = *mode;
                }
            }
            Some(PendingShortcut::Category(category)) => {
                let option_count = self
                    .layout()
                    .and_then(|layout| layout.options.get(category))
                    .map_or(0, |options| options.len());
                match category_shortcut(option_count, digit) {
                    Some(CategoryShortcut::Select(option)) => {
                        self.apply_action(ToolbarAction::SelectSubmenu {
                            submenu: category,
                            option,
                        });
                    }
                    Some(CategoryShortcut::Page(page)) => {
                        self.shortcut_prefix = Some(PendingShortcut::Option { category, page });
                    }
                    None => {}
                }
            }
            Some(PendingShortcut::Option { category, page }) => {
                let position = shortcut_position(digit);
                let option = page * OPTIONS_PER_PAGE + position;
                self.apply_action(ToolbarAction::SelectSubmenu {
                    submenu: category,
                    option,
                });
            }
            Some(PendingShortcut::ExportCategory) => {
                if digit == 0 {
                    self.close_export_menu();
                } else {
                    self.select_export_mode_digit(digit);
                }
            }
            Some(PendingShortcut::ExportOption(category)) => {
                if let Some((_, action)) = EXPORT_OPTIONS
                    .get(category)
                    .and_then(|options| options.get(shortcut_position(digit)))
                {
                    self.queue_export(*action);
                }
            }
            Some(PendingShortcut::ExportFlat(_)) => {
                self.select_export_mode_digit(digit);
            }
        }
        true
    }

    pub fn cancel_shortcut(&mut self) {
        self.cancel_pending_shortcut();
    }

    pub fn close_export_menu(&mut self) {
        self.export_open = false;
        self.active_export_category = None;
        self.shortcut_prefix = None;
        self.successful_export_action = None;
    }

    pub fn export_menu_open(&self) -> bool {
        self.export_open
    }

    pub fn take_export_action(&mut self) -> Option<ExportAction> {
        self.pending_export_action.take()
    }

    fn queue_export(&mut self, action: ExportAction) {
        self.pending_export_action = Some(action);
        self.keep_export_active(action);
    }

    pub fn keep_export_active(&mut self, action: ExportAction) {
        self.successful_export_action = None;
        let category = EXPORT_OPTIONS
            .iter()
            .position(|options| options.iter().any(|(_, candidate)| *candidate == action))
            .expect("every export action belongs to an export category");
        self.export_open = true;
        self.active_export_category = Some(category);
        self.shortcut_prefix = Some(if EXPORT_OPTIONS[category].len() == 1 {
            PendingShortcut::ExportFlat(category)
        } else {
            PendingShortcut::ExportOption(category)
        });
    }

    pub fn mark_export_successful(&mut self, action: ExportAction) {
        self.keep_export_active(action);
        self.successful_export_action = Some(action);
    }

    pub fn clear_export_success(&mut self) -> bool {
        self.successful_export_action.take().is_some()
    }

    fn toggle_export_menu(&mut self) {
        if self.export_open {
            self.close_export_menu();
        } else {
            self.export_open = true;
            self.active_export_category = None;
            self.shortcut_prefix = Some(PendingShortcut::ExportCategory);
        }
    }

    fn select_export_mode_digit(&mut self, digit: usize) {
        if digit == 1 {
            self.close_export_menu();
            self.shortcut_prefix = Some(PendingShortcut::Mode);
            return;
        }
        let category = if digit == EXPORT_CLEAR_DIGIT {
            EXPORT_LABELS.len() - 1
        } else if let Some(category) = digit
            .checked_sub(EXPORT_MODE_OFFSET)
            .filter(|category| *category < EXPORT_LABELS.len() - 1)
        {
            category
        } else {
            return;
        };
        self.select_export_category(category);
    }

    fn select_export_category(&mut self, category: usize) -> bool {
        let Some(options) = EXPORT_OPTIONS.get(category) else {
            return false;
        };
        if let [(_, action)] = options {
            self.queue_export(*action);
        } else {
            self.export_open = true;
            self.active_export_category = Some(category);
            self.shortcut_prefix = Some(PendingShortcut::ExportOption(category));
        }
        true
    }

    fn cancel_pending_shortcut(&mut self) -> bool {
        let pending = self.shortcut_prefix.take();
        if matches!(pending, Some(PendingShortcut::ExportOption(_))) {
            self.active_export_category = None;
        }
        pending.is_some()
    }

    pub fn main_mode(&self) -> MainMode {
        self.main_mode
    }

    pub fn pending_shortcut(&self) -> Option<PendingShortcut> {
        self.shortcut_prefix
    }

    pub fn menu_row_count(&self) -> usize {
        if self.export_open {
            return 2;
        }
        self.layout().map_or(2, |layout| {
            1 + layout
                .options
                .iter()
                .map(|options| options.len().div_ceil(OPTIONS_PER_PAGE))
                .max()
                .unwrap_or(0)
        })
    }

    pub fn content_rows(&self) -> usize {
        MENU_FIRST_ROW + self.menu_row_count()
    }

    pub fn rows(&self) -> usize {
        self.content_rows() + 2
    }

    pub fn toolbar_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        match row {
            MAIN_LABEL_ROW | MAIN_SHORTCUT_ROW => self.main_spans(row),
            MENU_FIRST_ROW.. if row < MENU_FIRST_ROW + self.menu_row_count() => {
                if self.export_open {
                    self.export_menu_spans(row)
                } else if self.main_mode == MainMode::Utilities {
                    self.utilities_menu_spans(row)
                } else {
                    self.menu_spans(row)
                }
            }
            _ => Vec::new(),
        }
    }

    fn main_spans(&self, row: usize) -> Vec<ToolbarSpan> {
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
                selected: row == MAIN_SHORTCUT_ROW && *mode == self.main_mode && !self.export_open,
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::SelectMain(*mode)),
                right_aligned: false,
            });
        }
        if row == MAIN_LABEL_ROW {
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

    fn export_menu_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let header_row = row == MENU_FIRST_ROW;
        let mut spans = Vec::new();
        for (category, label) in EXPORT_LABELS.iter().take(3).enumerate() {
            if category > 0 {
                spans.push(plain_span(GAP.to_string()));
            }
            let cell_start = spans_width(&spans);
            let mode_number = category + EXPORT_MODE_OFFSET;
            let path = format!("{mode_number}.");
            let prefix_width = menu_prefix_width(label, std::iter::once(path.as_str()));
            let options_width = EXPORT_OPTIONS[category]
                .iter()
                .map(|(option, _)| UnicodeWidthStr::width(*option))
                .sum::<usize>()
                + EXPORT_OPTIONS[category].len().saturating_sub(1);
            let cell_width = prefix_width + options_width;
            if header_row {
                let label_contents = format!("{label}:");
                spans.push(ToolbarSpan {
                    bold_prefix: UnicodeWidthStr::width(label_contents.as_str()),
                    contents: pad_right_to_width(label_contents, prefix_width),
                    selected: self.active_export_category == Some(category),
                    highlighted: false,
                    tooltip: false,
                    action: Some(ToolbarAction::SelectExportCategory(category)),
                    right_aligned: false,
                });
            } else {
                let highlighted_prefix = match self.pending_shortcut() {
                    Some(PendingShortcut::ExportOption(pending)) if pending == category => {
                        Some(path.clone())
                    }
                    _ => None,
                };
                push_shortcut_path(
                    &mut spans,
                    &path,
                    prefix_width,
                    highlighted_prefix.as_deref(),
                );
            }
            for (position, (option, action)) in EXPORT_OPTIONS[category].iter().enumerate() {
                if position > 0 {
                    spans.push(plain_span(" ".to_string()));
                }
                spans.push(ToolbarSpan {
                    contents: if header_row {
                        aligned_shortcut(position + 1, option)
                    } else {
                        (*option).to_string()
                    },
                    bold_prefix: 0,
                    selected: !header_row && self.successful_export_action == Some(*action),
                    highlighted: false,
                    tooltip: false,
                    action: Some(ToolbarAction::RunExport(*action)),
                    right_aligned: false,
                });
            }
            pad_spans_to_width(&mut spans, cell_start + cell_width);
        }
        if !header_row {
            spans.push(ToolbarSpan {
                contents: format!("{EXPORT_CLEAR_DIGIT}. Clear"),
                bold_prefix: UnicodeWidthStr::width("9."),
                selected: self.successful_export_action == Some(ExportAction::Clear),
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::RunExport(ExportAction::Clear)),
                right_aligned: true,
            });
        }
        spans
    }

    fn menu_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        menu_layout::hierarchical_menu_spans(self, row)
    }

    fn utilities_menu_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let label_row = row == MENU_FIRST_ROW;
        let mut spans = Vec::new();
        for (option, label) in UTILITY_OPTIONS[0].iter().enumerate() {
            if option > 0 {
                spans.push(plain_span(GAP.to_string()));
            }
            let action = ToolbarAction::SelectSubmenu { submenu: 0, option };
            spans.push(ToolbarSpan {
                contents: if label_row {
                    (*label).to_string()
                } else {
                    aligned_shortcut(option + 2, label)
                },
                bold_prefix: 0,
                selected: label_row && option == self.utility_selected,
                highlighted: false,
                tooltip: false,
                action: Some(action),
                right_aligned: false,
            });
        }
        spans
    }

    pub fn tooltip(&self) -> Tooltip {
        if self.export_open {
            return Tooltip::Export;
        }
        if self.main_mode == MainMode::Utilities {
            return match self.utility_kind() {
                UtilityKind::Move => Tooltip::UtilitiesMove,
                UtilityKind::Push => Tooltip::UtilitiesPush,
                UtilityKind::Pull => Tooltip::UtilitiesPull,
                UtilityKind::View => Tooltip::UtilitiesView,
            };
        }
        self.main_mode.tooltip()
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

    pub fn line_corner(&self) -> CornerStyle {
        match self.line_selected[3] {
            0 => CornerStyle::Smooth,
            1 => CornerStyle::Sharp,
            _ => unreachable!("line corner selection is always normalized"),
        }
    }

    pub fn stamp(&self) -> &'static str {
        STAMP_OPTIONS[self.stamp_active_category][self.stamp_selected[self.stamp_active_category]]
    }

    pub fn shape_kind(&self) -> ShapeKind {
        match self.shape_selected[0] {
            0 => ShapeKind::Rect,
            1 => ShapeKind::RoundedRect,
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

    pub fn utility_kind(&self) -> UtilityKind {
        match self.utility_selected {
            0 => UtilityKind::Move,
            1 => UtilityKind::Push,
            2 => UtilityKind::Pull,
            3 => UtilityKind::View,
            _ => unreachable!("utility selection is always normalized"),
        }
    }

    pub fn action_at(&self, row: usize, column: usize, box_width: usize) -> Option<ToolbarAction> {
        let mut start = 0;
        for span in boxed_toolbar_spans(&self.toolbar_spans(row), box_width) {
            let end = start + UnicodeWidthStr::width(span.contents.as_str());
            if (start..end).contains(&column) {
                return span.action;
            }
            start = end;
        }
        None
    }

    pub fn apply_action(&mut self, action: ToolbarAction) -> bool {
        self.cancel_shortcut();
        match action {
            ToolbarAction::SelectMain(mode) => {
                self.close_export_menu();
                self.main_mode = mode;
                true
            }
            ToolbarAction::SelectSubmenu { submenu, option } => {
                self.close_export_menu();
                let option_count = match self.main_mode {
                    MainMode::Line => LINE_OPTIONS.get(submenu),
                    MainMode::Stamp => STAMP_OPTIONS.get(submenu),
                    MainMode::Shapes => SHAPE_OPTIONS.get(submenu),
                    MainMode::Utilities => UTILITY_OPTIONS.get(submenu),
                }
                .map(|options| options.len());
                if option_count.is_none_or(|count| option >= count) {
                    return false;
                }
                let selected = match self.main_mode {
                    MainMode::Line => self.line_selected.get_mut(submenu),
                    MainMode::Stamp => {
                        self.stamp_active_category = submenu;
                        self.stamp_selected.get_mut(submenu)
                    }
                    MainMode::Shapes => self.shape_selected.get_mut(submenu),
                    MainMode::Utilities => (submenu == 0).then_some(&mut self.utility_selected),
                };
                let Some(selected) = selected else {
                    return false;
                };
                *selected = option;
                true
            }
            ToolbarAction::ToggleExportMenu => {
                self.toggle_export_menu();
                true
            }
            ToolbarAction::SelectExportCategory(category) => self.select_export_category(category),
            ToolbarAction::RunExport(action) => {
                self.queue_export(action);
                true
            }
        }
    }

    fn layout(&self) -> Option<MenuLayout<'_>> {
        match self.main_mode {
            MainMode::Line => {
                let category_count = if self.line_style() == LineStyle::Thin {
                    4
                } else {
                    3
                };
                Some(MenuLayout {
                    labels: &LINE_LABELS[..category_count],
                    options: &LINE_OPTIONS[..category_count],
                    selected: &self.line_selected[..category_count],
                    exclusive_submenu: None,
                })
            }
            MainMode::Stamp => Some(MenuLayout {
                labels: &STAMP_LABELS,
                options: &STAMP_OPTIONS,
                selected: &self.stamp_selected,
                exclusive_submenu: Some(self.stamp_active_category),
            }),
            MainMode::Shapes => Some(MenuLayout {
                labels: &SHAPE_LABELS,
                options: &SHAPE_OPTIONS,
                selected: &self.shape_selected,
                exclusive_submenu: None,
            }),
            MainMode::Utilities => None,
        }
    }
}

fn plain_span(contents: String) -> ToolbarSpan {
    ToolbarSpan {
        contents,
        bold_prefix: 0,
        selected: false,
        highlighted: false,
        tooltip: false,
        action: None,
        right_aligned: false,
    }
}

fn bold_span(contents: String) -> ToolbarSpan {
    let bold_prefix = UnicodeWidthStr::width(contents.as_str());
    ToolbarSpan {
        bold_prefix,
        ..plain_span(contents)
    }
}

fn bold_prefix_span(contents: String, prefix: &str) -> ToolbarSpan {
    ToolbarSpan {
        bold_prefix: UnicodeWidthStr::width(prefix),
        ..plain_span(contents)
    }
}

fn spans_width(spans: &[ToolbarSpan]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.contents.as_str()))
        .sum()
}

fn pad_spans_to_width(spans: &mut Vec<ToolbarSpan>, width: usize) {
    let padding = width.saturating_sub(spans_width(spans));
    if padding > 0 {
        spans.push(plain_span(" ".repeat(padding)));
    }
}

fn pad_right_to_width(contents: String, width: usize) -> String {
    let padding = width.saturating_sub(UnicodeWidthStr::width(contents.as_str()));
    contents + &" ".repeat(padding)
}

fn push_shortcut_path(
    spans: &mut Vec<ToolbarSpan>,
    path: &str,
    prefix_width: usize,
    highlighted_prefix: Option<&str>,
) {
    let path_width = UnicodeWidthStr::width(path);
    let leading_padding = prefix_width.saturating_sub(path_width + 1);
    if leading_padding > 0 {
        spans.push(plain_span(" ".repeat(leading_padding)));
    }

    if let Some(highlighted) = highlighted_prefix.filter(|prefix| path.starts_with(prefix)) {
        let mut span = bold_span(highlighted.to_string());
        span.highlighted = true;
        spans.push(span);
        if let Some(remainder) = path.strip_prefix(highlighted)
            && !remainder.is_empty()
        {
            spans.push(bold_span(remainder.to_string()));
        }
    } else {
        spans.push(bold_span(path.to_string()));
    }
    spans.push(plain_span(" ".to_string()));
}

fn menu_prefix_width<'a>(label: &str, paths: impl IntoIterator<Item = &'a str>) -> usize {
    paths
        .into_iter()
        .map(|path| UnicodeWidthStr::width(path) + 1)
        .chain(std::iter::once(UnicodeWidthStr::width(label) + 2))
        .max()
        .unwrap_or(0)
}

fn submenu_path(category: usize, page: usize, option_count: usize) -> String {
    if option_count <= OPTIONS_PER_PAGE {
        format!("{}.", category + 2)
    } else {
        format!("{}.{}.", category + 2, page + 1)
    }
}

fn submenu_prefix_width(label: &str, category: usize, option_count: usize) -> usize {
    let page_count = option_count.div_ceil(OPTIONS_PER_PAGE);
    let paths: Vec<_> = (0..page_count)
        .map(|page| submenu_path(category, page, option_count))
        .collect();
    menu_prefix_width(label, paths.iter().map(String::as_str))
}

fn submenu_cell_width(prefix_width: usize, options: &[&str]) -> usize {
    let columns = submenu_option_column_widths(options);
    prefix_width + columns.iter().sum::<usize>() + columns.len().saturating_sub(1)
}

fn submenu_option_column_widths(options: &[&str]) -> Vec<usize> {
    let mut widths = vec![0; options.len().min(OPTIONS_PER_PAGE)];
    for (index, option) in options.iter().enumerate() {
        widths[index % OPTIONS_PER_PAGE] =
            widths[index % OPTIONS_PER_PAGE].max(UnicodeWidthStr::width(*option));
    }
    widths
}

fn aligned_shortcut(digit: usize, label: &str) -> String {
    let digit = digit % 10;
    format!("{digit:<width$}", width = UnicodeWidthStr::width(label))
}

fn shortcut_digit(text: &str) -> Option<usize> {
    match text {
        "0" => Some(0),
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        "5" => Some(5),
        "6" => Some(6),
        "7" => Some(7),
        "8" => Some(8),
        "9" => Some(9),
        _ => None,
    }
}

fn category_shortcut(option_count: usize, digit: usize) -> Option<CategoryShortcut> {
    if option_count <= OPTIONS_PER_PAGE {
        let option = shortcut_position(digit);
        (option < option_count).then_some(CategoryShortcut::Select(option))
    } else {
        digit
            .checked_sub(1)
            .filter(|page| page * OPTIONS_PER_PAGE < option_count)
            .map(CategoryShortcut::Page)
    }
}

fn shortcut_position(digit: usize) -> usize {
    if digit == 0 { 9 } else { digit - 1 }
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
    LINE_ENDINGS
        .get(selected)
        .copied()
        .expect("line ending selection is always normalized")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

    fn press(toolbar: &mut ToolbarState, key: &str) {
        assert!(toolbar.handle_shortcut(&Key::Character(key.into()), ModifiersState::empty()));
    }

    fn row(toolbar: &ToolbarState, row: usize) -> String {
        toolbar
            .toolbar_spans(row)
            .iter()
            .take_while(|span| !span.right_aligned)
            .map(|span| span.contents.as_str())
            .collect()
    }

    fn spans_text(spans: &[ToolbarSpan]) -> String {
        spans.iter().map(|span| span.contents.as_str()).collect()
    }

    fn span_starts(spans: &[ToolbarSpan]) -> Vec<(usize, &ToolbarSpan)> {
        spans
            .iter()
            .scan(0, |column, span| {
                let start = *column;
                *column += UnicodeWidthStr::width(span.contents.as_str());
                Some((start, span))
            })
            .collect()
    }

    fn prefix_start(spans: &[ToolbarSpan], prefix: &str) -> usize {
        span_starts(spans)
            .into_iter()
            .find_map(|(start, span)| (span.contents.trim() == prefix).then_some(start))
            .unwrap_or_else(|| panic!("prefix {prefix:?} missing from {:?}", spans_text(spans)))
    }

    fn path_cell_start(spans: &[ToolbarSpan], path: &str, prefix_width: usize) -> usize {
        prefix_start(spans, path)
            .saturating_sub(prefix_width.saturating_sub(UnicodeWidthStr::width(path) + 1))
    }

    fn category_cell_text(toolbar: &ToolbarState, row: usize, category: usize) -> String {
        let layout = toolbar.layout().expect("hierarchical editor mode");
        let start = (0..category)
            .map(|index| {
                submenu_cell_width(
                    submenu_prefix_width(layout.labels[index], index, layout.options[index].len()),
                    layout.options[index],
                ) + UnicodeWidthStr::width(GAP)
            })
            .sum::<usize>();
        let width = submenu_cell_width(
            submenu_prefix_width(
                layout.labels[category],
                category,
                layout.options[category].len(),
            ),
            layout.options[category],
        );
        let text = spans_text(&toolbar.toolbar_spans(row));
        text.chars().skip(start).take(width).collect()
    }

    fn highlighted_contents(spans: &[ToolbarSpan]) -> Vec<&str> {
        spans
            .iter()
            .filter(|span| span.highlighted)
            .map(|span| span.contents.as_str())
            .collect()
    }

    fn bold_contents(spans: &[ToolbarSpan]) -> Vec<String> {
        spans
            .iter()
            .filter(|span| span.bold_prefix > 0)
            .map(|span| clipped_to_width(&span.contents, span.bold_prefix).0)
            .collect()
    }

    #[test]
    fn only_structural_labels_and_numeric_paths_are_bold() {
        let mut toolbar = ToolbarState::default();
        let main_labels = toolbar.toolbar_spans(MAIN_LABEL_ROW);
        assert_eq!(bold_contents(&main_labels), ["Mode:", "0."]);
        assert!(main_labels.iter().any(|span| {
            span.contents.contains("Save/Load/Export")
                && span.bold_prefix == UnicodeWidthStr::width("0.")
        }));
        assert_eq!(
            bold_contents(&toolbar.toolbar_spans(MAIN_SHORTCUT_ROW)),
            ["1."]
        );

        for (mode, expected_labels) in [
            (MainMode::Line, LINE_LABELS.as_slice()),
            (MainMode::Stamp, STAMP_LABELS.as_slice()),
            (MainMode::Shapes, SHAPE_LABELS.as_slice()),
        ] {
            toolbar.apply_action(ToolbarAction::SelectMain(mode));
            assert_eq!(
                bold_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW)),
                expected_labels
                    .iter()
                    .map(|label| format!("{label}:"))
                    .collect::<Vec<_>>()
            );
            let shortcut_spans = toolbar.toolbar_spans(MENU_FIRST_ROW + 1);
            assert!(bold_contents(&shortcut_spans).iter().all(|prefix| {
                prefix.ends_with('.') && prefix.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
            }));
            assert!(
                shortcut_spans
                    .iter()
                    .filter(|span| span.action.is_some())
                    .all(|span| { span.bold_prefix == 0 })
            );
        }

        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Utilities));
        assert!(
            toolbar
                .toolbar_spans(MENU_FIRST_ROW)
                .iter()
                .chain(toolbar.toolbar_spans(MENU_FIRST_ROW + 1).iter())
                .all(|span| span.bold_prefix == 0)
        );
    }

    #[test]
    fn export_categories_and_paths_are_bold_but_values_remain_normal() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::ToggleExportMenu);

        assert_eq!(
            bold_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW)),
            ["Clipboard:", "Save:", "Load:"]
        );
        let shortcuts = toolbar.toolbar_spans(MENU_FIRST_ROW + 1);
        assert_eq!(bold_contents(&shortcuts), ["2.", "3.", "4.", "9."]);
        assert!(
            shortcuts
                .iter()
                .filter(|span| {
                    span.action.is_some()
                        && span.action != Some(ToolbarAction::RunExport(ExportAction::Clear))
                })
                .all(|span| span.bold_prefix == 0)
        );
    }

    #[test]
    fn highlighted_and_clipped_prefixes_keep_bold_width_without_geometry_changes() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        assert!(toolbar.handle_shortcut(&Key::Character("3".into()), ModifiersState::empty(),));
        let spans = toolbar.toolbar_spans(MENU_FIRST_ROW + 1);
        let path_spans: Vec<_> = spans.iter().filter(|span| span.bold_prefix > 0).collect();
        assert!(path_spans.iter().any(|span| span.highlighted));
        assert!(
            path_spans
                .iter()
                .all(|span| { span.bold_prefix == UnicodeWidthStr::width(span.contents.as_str()) })
        );

        for width in 1..=spans_width(&spans) {
            let clipped = boxed_toolbar_spans(&spans, width);
            assert_eq!(spans_width(&clipped), width);
            assert!(clipped.iter().all(|span| {
                span.bold_prefix <= UnicodeWidthStr::width(span.contents.as_str())
            }));
        }
    }

    fn selected_main_mode_count(toolbar: &ToolbarState) -> usize {
        toolbar
            .toolbar_spans(MAIN_SHORTCUT_ROW)
            .iter()
            .filter(|span| {
                span.selected && matches!(span.action, Some(ToolbarAction::SelectMain(_)))
            })
            .count()
    }

    fn option_start(spans: &[ToolbarSpan], category: usize, option: usize) -> usize {
        span_starts(spans)
            .into_iter()
            .find_map(|(start, span)| {
                (span.action
                    == Some(ToolbarAction::SelectSubmenu {
                        submenu: category,
                        option,
                    }))
                .then_some(start)
            })
            .unwrap_or_else(|| {
                panic!(
                    "category {category} option {option} missing from {:?}",
                    spans_text(spans)
                )
            })
    }

    #[test]
    fn toolbar_box_has_exact_light_borders_and_consistent_width() {
        let toolbar = ToolbarState::default();
        let width = 36;
        let top = spans_text(&toolbar_border_spans(width, true));
        let contents = spans_text(&boxed_toolbar_spans(&toolbar.toolbar_spans(0), width));
        let bottom = spans_text(&toolbar_border_spans(width, false));

        assert_eq!(top, format!("┌{}┐", "─".repeat(width - 2)));
        assert_eq!(bottom, format!("└{}┘", "─".repeat(width - 2)));
        assert!(contents.starts_with("│ Mode: "));
        assert!(contents.ends_with(" │"));
        for line in [&top, &contents, &bottom] {
            assert_eq!(UnicodeWidthStr::width(line.as_str()), width);
        }
    }

    #[test]
    fn toolbar_box_clips_cleanly_at_narrow_widths() {
        let toolbar = ToolbarState::default();
        for width in 0..12 {
            for spans in [
                toolbar_border_spans(width, true),
                boxed_toolbar_spans(&toolbar.toolbar_spans(0), width),
                toolbar_border_spans(width, false),
            ] {
                assert_eq!(UnicodeWidthStr::width(spans_text(&spans).as_str()), width);
            }
        }
        assert_eq!(
            spans_text(&boxed_toolbar_spans(&toolbar.toolbar_spans(0), 8)),
            "│ Mode │"
        );
    }

    #[test]
    fn boxed_toolbar_height_tracks_only_actual_menu_rows() {
        let mut toolbar = ToolbarState::default();
        assert_eq!(toolbar_content_row(0), 1);
        assert_eq!(toolbar.menu_row_count(), 4);
        assert_eq!(toolbar.content_rows(), 6);
        assert_eq!(toolbar.rows(), 8);
        assert_eq!(toolbar_height(&toolbar, 18), toolbar.rows() * 18);

        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        assert_eq!(toolbar.menu_row_count(), 4);
        assert_eq!(toolbar.content_rows(), 6);
        assert_eq!(toolbar.rows(), 8);

        for mode in [MainMode::Shapes, MainMode::Utilities] {
            toolbar.apply_action(ToolbarAction::SelectMain(mode));
            assert_eq!(toolbar.menu_row_count(), 2);
            assert_eq!(toolbar.rows(), 6);
        }
    }

    #[test]
    fn mode_path_selects_an_exact_mode() {
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, "1");
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        press(&mut toolbar, "3");
        assert_eq!(toolbar.main_mode(), MainMode::Shapes);

        assert_eq!(row(&toolbar, 0), "Mode: 1     2    3     4");
        assert_eq!(row(&toolbar, 1), "   1. Stamp Line Shape Utils");
        assert_eq!(
            toolbar
                .toolbar_spans(1)
                .iter()
                .filter(|span| span.selected)
                .count(),
            1
        );
    }

    #[test]
    fn corner_is_available_only_for_thin_lines() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Line));
        assert!(row(&toolbar, MENU_FIRST_ROW).contains("Corner:"));

        for key in ["4", "3"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.line_style(), LineStyle::Double);
        assert!(!row(&toolbar, MENU_FIRST_ROW).contains("Corner:"));

        assert_eq!(toolbar.line_corner(), CornerStyle::Smooth);

        assert!(toolbar.apply_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 0,
        }));
        assert!(row(&toolbar, MENU_FIRST_ROW).contains("Corner:"));

        assert!(row(&toolbar, 2).contains("Start: 1 2 3 4 5 6 7 8 9 0"));
        assert!(row(&toolbar, 3).contains("2.1.   ◁ ◀ ← ◃ ◂ ↔ □ ■ ▫"));
        assert!(row(&toolbar, 3).contains("4. ─ ━ ═"));
        assert!(row(&toolbar, 3).contains("5. Smooth Sharp"));
    }

    #[test]
    fn line_endings_exactly_map_the_documented_directional_and_decorator_sets() {
        assert_eq!(
            LINE_ENDINGS.len(),
            1 + DIRECTIONAL_ENDINGS.len() + DECORATORS.len()
        );
        assert_eq!(&LINE_START_OPTIONS[7..], DECORATORS);
        assert_eq!(&LINE_END_OPTIONS[7..], DECORATORS);
        assert_eq!(&LINE_START_OPTIONS[1..7], ["◁", "◀", "←", "◃", "◂", "↔"]);
        assert_eq!(&LINE_END_OPTIONS[1..7], ["▷", "▶", "→", "▹", "▸", "↔"]);

        for (index, ending) in LINE_ENDINGS.into_iter().enumerate() {
            assert_eq!(line_ending(index), ending);
            if index == 0 {
                assert_eq!(LINE_START_OPTIONS[index], " ");
                assert_eq!(LINE_END_OPTIONS[index], " ");
                continue;
            }
            assert_eq!(
                LINE_START_OPTIONS[index],
                line_ending_glyph(ending, Direction::Right, LineStyle::Thin).to_string()
            );
            assert_eq!(
                LINE_END_OPTIONS[index],
                line_ending_glyph(ending, Direction::Left, LineStyle::Thin).to_string()
            );
            for option in [LINE_START_OPTIONS[index], LINE_END_OPTIONS[index]] {
                assert_eq!(UnicodeSegmentation::graphemes(option, true).count(), 1);
                assert_eq!(UnicodeWidthStr::width(option), 1);
            }
        }
    }

    #[test]
    fn line_start_and_end_use_independent_three_page_keyboard_paths() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Line));
        for key in ["2", "1", "0"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.line_start(), LineEnding::Fixed('▫'));
        assert_eq!(toolbar.line_end(), LineEnding::None);

        for key in ["3", "3", "7"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.line_start(), LineEnding::Fixed('▫'));
        assert_eq!(toolbar.line_end(), LineEnding::Fixed('¤'));
    }

    #[test]
    fn line_endpoint_pages_highlight_and_mouse_map_with_stable_columns() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Line));
        press(&mut toolbar, "2");
        for row_index in [MENU_FIRST_ROW + 1, MENU_FIRST_ROW + 2, MENU_FIRST_ROW + 3] {
            let highlighted: String = toolbar
                .toolbar_spans(row_index)
                .iter()
                .filter(|span| span.highlighted)
                .map(|span| span.contents.as_str())
                .collect();
            assert_eq!(highlighted, "2.");
        }
        press(&mut toolbar, "1");
        let highlighted: String = toolbar
            .toolbar_spans(MENU_FIRST_ROW + 1)
            .iter()
            .filter(|span| span.highlighted)
            .map(|span| span.contents.as_str())
            .collect();
        assert_eq!(highlighted, "2.1.");

        let expected = ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 26,
        };
        let box_width = 320;
        let row_index = MENU_FIRST_ROW + 3;
        let visible = boxed_toolbar_spans(&toolbar.toolbar_spans(row_index), box_width);
        let column = span_starts(&visible)
            .into_iter()
            .find_map(|(start, span)| (span.action == Some(expected)).then_some(start))
            .expect("third endpoint page remains mouse selectable");
        assert_eq!(
            toolbar.action_at(row_index, column, box_width),
            Some(expected)
        );

        let start_columns: Vec<_> = [MENU_FIRST_ROW + 1, MENU_FIRST_ROW + 2, MENU_FIRST_ROW + 3]
            .into_iter()
            .map(|row_index| {
                let spans = toolbar.toolbar_spans(row_index);
                span_starts(&spans)
                    .into_iter()
                    .find_map(|(start, span)| {
                        matches!(
                            span.action,
                            Some(ToolbarAction::SelectSubmenu { submenu: 0, .. })
                        )
                        .then_some(start)
                    })
                    .unwrap()
            })
            .collect();
        assert!(start_columns.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn three_key_multi_page_path_and_digit_zero_select_exact_options() {
        let mut toolbar = ToolbarState::default();
        for key in ["1", "2", "3", "1", "0"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.main_mode(), MainMode::Line);
        assert_eq!(toolbar.stamp(), "□");

        for key in ["3", "3", "2"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.stamp(), "□");

        for key in ["2", "2", "0"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.stamp(), "□");
    }

    #[test]
    fn a_ten_option_category_flattens_and_maps_zero_to_the_tenth_option() {
        assert_eq!(
            category_shortcut(OPTIONS_PER_PAGE, 0),
            Some(CategoryShortcut::Select(9))
        );
        assert_eq!(
            category_shortcut(OPTIONS_PER_PAGE + 1, 1),
            Some(CategoryShortcut::Page(0))
        );
    }

    #[test]
    fn stamp_pages_show_the_complete_uniline_standalone_inventory() {
        let mut toolbar = ToolbarState::default();
        for key in ["1", "1"] {
            press(&mut toolbar, key);
        }
        assert!(row(&toolbar, 2).starts_with("Decorators: 1 2 3 4 5 6 7 8 9 0"));
        assert!(row(&toolbar, 2).contains("Arrows: 1 2 3 4 5 6 7 8 9 0"));
        assert!(row(&toolbar, 2).contains("Fills: 1 2 3 4"));
        assert!(row(&toolbar, 2).contains("Blocks: 1 2 3 4 5 6 7 8 9 0"));
        assert!(row(&toolbar, 3).contains("2.1. □ ■ ▫ ▪ ◆ ◊ · ∙ • ●"));
        assert!(row(&toolbar, 3).contains("3.1. △ ▽ ◁ ▷ ▲ ▼ ◀ ▶ ↑ ↓"));
        assert!(row(&toolbar, 3).contains("4. ░ ▒ ▓ █"));
        assert!(row(&toolbar, 3).contains("5.1. ▘ ▝ ▀ ▖ ▌ ▞ ▛ ▗ ▚ ▐"));
        assert!(row(&toolbar, 4).contains("2.2. ◦ Ø ø ╳ ╱ ╲ ÷ × ± ¤"));
        assert!(row(&toolbar, 4).contains("3.2. ← → ▵ ▿ ◃ ▹ ▴ ▾ ◂ ▸"));
        assert!(row(&toolbar, 4).contains("5.2. ▜ ▄ ▙ ▟ █"));
        assert!(row(&toolbar, 5).contains("3.3. ↕ ↔"));
    }

    #[test]
    fn stamp_rows_match_the_shared_header_and_dotted_page_examples() {
        let toolbar = ToolbarState::default();

        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW, 1).trim_end(),
            "Arrows: 1 2 3 4 5 6 7 8 9 0"
        );
        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW + 1, 1).trim_end(),
            "   3.1. △ ▽ ◁ ▷ ▲ ▼ ◀ ▶ ↑ ↓"
        );
        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW + 2, 1).trim_end(),
            "   3.2. ← → ▵ ▿ ◃ ▹ ▴ ▾ ◂ ▸"
        );
        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW + 3, 1).trim_end(),
            "   3.3. ↕ ↔"
        );
        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW, 2).trim_end(),
            "Fills: 1 2 3 4"
        );
        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW + 1, 2).trim_end(),
            "    4. ░ ▒ ▓ █"
        );
        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW, 3).trim_end(),
            "Blocks: 1 2 3 4 5 6 7 8 9 0"
        );
        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW + 1, 3).trim_end(),
            "   5.1. ▘ ▝ ▀ ▖ ▌ ▞ ▛ ▗ ▚ ▐"
        );
        assert_eq!(
            category_cell_text(&toolbar, MENU_FIRST_ROW + 2, 3).trim_end(),
            "   5.2. ▜ ▄ ▙ ▟ █"
        );
    }

    #[test]
    fn every_menu_category_keeps_fixed_prefix_and_option_columns_across_pages() {
        let mut toolbar = ToolbarState::default();

        for mode in [MainMode::Line, MainMode::Stamp, MainMode::Shapes] {
            toolbar.apply_action(ToolbarAction::SelectMain(mode));
            let layout = toolbar.layout().expect("hierarchical editor mode");
            let header = toolbar.toolbar_spans(MENU_FIRST_ROW);
            let expected: Vec<_> = layout
                .labels
                .iter()
                .enumerate()
                .map(|(category, label)| {
                    let first_page = toolbar.toolbar_spans(MENU_FIRST_ROW + 1);
                    (
                        prefix_start(&header, &format!("{label}:")),
                        option_start(&first_page, category, 0),
                    )
                })
                .collect();

            for (category, (options, expected)) in
                layout.options.iter().zip(expected.iter()).enumerate()
            {
                assert_eq!(spans_text(&header).chars().nth(expected.1), Some('1'));
                for page in 0..options.len().div_ceil(OPTIONS_PER_PAGE) {
                    let page_spans = toolbar.toolbar_spans(MENU_FIRST_ROW + page + 1);
                    let path = submenu_path(category, page, options.len());
                    let prefix_width =
                        submenu_prefix_width(layout.labels[category], category, options.len());
                    assert_eq!(
                        path_cell_start(&page_spans, &path, prefix_width),
                        expected.0
                    );
                    assert_eq!(
                        option_start(&page_spans, category, page * OPTIONS_PER_PAGE),
                        expected.1
                    );
                }
            }
        }
    }

    #[test]
    fn stamp_page_paths_stay_aligned_when_earlier_categories_are_exhausted() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));

        let page_one = toolbar.toolbar_spans(MENU_FIRST_ROW + 1);
        let page_two = toolbar.toolbar_spans(MENU_FIRST_ROW + 2);
        let page_three = toolbar.toolbar_spans(MENU_FIRST_ROW + 3);
        assert_eq!(
            path_cell_start(
                &page_one,
                "2.1.",
                submenu_prefix_width(STAMP_LABELS[0], 0, STAMP_OPTIONS[0].len()),
            ),
            path_cell_start(
                &page_two,
                "2.2.",
                submenu_prefix_width(STAMP_LABELS[0], 0, STAMP_OPTIONS[0].len()),
            )
        );
        assert_eq!(
            path_cell_start(
                &page_one,
                "3.1.",
                submenu_prefix_width(STAMP_LABELS[1], 1, STAMP_OPTIONS[1].len()),
            ),
            path_cell_start(
                &page_two,
                "3.2.",
                submenu_prefix_width(STAMP_LABELS[1], 1, STAMP_OPTIONS[1].len()),
            )
        );
        assert_eq!(
            path_cell_start(
                &page_one,
                "3.1.",
                submenu_prefix_width(STAMP_LABELS[1], 1, STAMP_OPTIONS[1].len()),
            ),
            path_cell_start(
                &page_three,
                "3.3.",
                submenu_prefix_width(STAMP_LABELS[1], 1, STAMP_OPTIONS[1].len()),
            )
        );
        assert_eq!(
            path_cell_start(
                &page_one,
                "5.1.",
                submenu_prefix_width(STAMP_LABELS[3], 3, STAMP_OPTIONS[3].len()),
            ),
            path_cell_start(
                &page_two,
                "5.2.",
                submenu_prefix_width(STAMP_LABELS[3], 3, STAMP_OPTIONS[3].len()),
            )
        );

        let fills_prefix_width = submenu_prefix_width(STAMP_LABELS[2], 2, STAMP_OPTIONS[2].len());
        let fills_start = path_cell_start(&page_one, "4.", fills_prefix_width);
        let fills_cell_width = submenu_cell_width(fills_prefix_width, STAMP_OPTIONS[2]);
        let blocks_prefix_width = submenu_prefix_width(STAMP_LABELS[3], 3, STAMP_OPTIONS[3].len());
        let blocks_start = path_cell_start(&page_two, "5.2.", blocks_prefix_width);
        assert_eq!(
            blocks_start,
            fills_start + fills_cell_width + UnicodeWidthStr::width(GAP)
        );
    }

    #[test]
    fn export_categories_use_the_same_columns_for_labels_shortcuts_and_options() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::ToggleExportMenu);
        let labels = toolbar.toolbar_spans(MENU_FIRST_ROW);
        let shortcuts = toolbar.toolbar_spans(MENU_FIRST_ROW + 1);

        for (category, label) in EXPORT_LABELS.iter().take(3).enumerate() {
            let path = format!("{}.", category + EXPORT_MODE_OFFSET);
            let rendered_label = format!("{label}:");
            assert_eq!(
                prefix_start(&labels, &rendered_label),
                path_cell_start(
                    &shortcuts,
                    &path,
                    menu_prefix_width(label, std::iter::once(path.as_str())),
                )
            );
            let action = ToolbarAction::RunExport(EXPORT_OPTIONS[category][0].1);
            let header_option = prefix_start(&labels, &rendered_label)
                + menu_prefix_width(label, std::iter::once(path.as_str()));
            let value_option = span_starts(&shortcuts)
                .into_iter()
                .find_map(|(start, span)| (span.action == Some(action)).then_some(start))
                .unwrap();
            assert_eq!(header_option, value_option);
        }
    }

    #[test]
    fn invalid_and_cancelled_prefixes_do_not_change_selection() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Line));
        press(&mut toolbar, "1");
        press(&mut toolbar, "9");
        assert_eq!(toolbar.main_mode(), MainMode::Line);

        press(&mut toolbar, "4");
        press(&mut toolbar, "9");
        assert_eq!(toolbar.line_style(), LineStyle::Thin);

        press(&mut toolbar, "1");
        assert!(toolbar.handle_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty()));
        assert!(!toolbar.handle_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty()));
        assert_eq!(toolbar.main_mode(), MainMode::Line);
    }

    #[test]
    fn pending_prefix_highlight_is_assigned_and_cleared() {
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, "1");
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Mode));
        assert!(
            toolbar
                .toolbar_spans(MAIN_SHORTCUT_ROW)
                .iter()
                .any(|span| span.highlighted)
        );
        press(&mut toolbar, "2");
        assert_eq!(toolbar.pending_shortcut(), None);
        assert!(
            toolbar
                .toolbar_spans(MAIN_SHORTCUT_ROW)
                .iter()
                .all(|span| !span.highlighted)
        );

        press(&mut toolbar, "2");
        assert_eq!(
            toolbar.pending_shortcut(),
            Some(PendingShortcut::Category(0))
        );
        assert!(toolbar.toolbar_spans(3).iter().any(|span| span.highlighted));
        toolbar.cancel_shortcut();
        assert_eq!(toolbar.pending_shortcut(), None);
        assert!(
            toolbar
                .toolbar_spans(3)
                .iter()
                .all(|span| !span.highlighted)
        );
    }

    #[test]
    fn multi_page_category_highlights_only_its_common_prefix_on_every_page() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));

        for (key, prefix, page_count) in [("2", "2.", 2), ("3", "3.", 3), ("5", "5.", 2)] {
            press(&mut toolbar, key);
            for page in 0..page_count {
                assert_eq!(
                    highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + page + 1)),
                    vec![prefix]
                );
            }
            toolbar.cancel_shortcut();
        }
    }

    #[test]
    fn selected_page_highlights_one_contiguous_complete_path() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));

        press(&mut toolbar, "2");
        press(&mut toolbar, "1");
        assert_eq!(
            highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)),
            vec!["2.1."]
        );
        assert!(highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 2)).is_empty());
    }

    #[test]
    fn flattened_category_highlights_only_its_complete_category_path() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));

        press(&mut toolbar, "4");
        assert_eq!(
            highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)),
            vec!["4."]
        );
    }

    #[test]
    fn export_prefix_highlighting_follows_category_then_page_hierarchy() {
        let mut toolbar = ToolbarState::default();

        press(&mut toolbar, "0");
        assert!(highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)).is_empty());
        assert!(
            toolbar
                .toolbar_spans(MAIN_LABEL_ROW)
                .iter()
                .all(|span| !span.highlighted)
        );

        press(&mut toolbar, "2");
        assert_eq!(
            highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)),
            vec!["2."]
        );
    }

    #[test]
    fn cancellation_and_completion_clear_exact_prefix_highlights() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));

        press(&mut toolbar, "3");
        assert_eq!(
            highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)),
            vec!["3."]
        );
        assert!(toolbar.handle_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty()));
        for page in 0..3 {
            assert!(
                highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + page + 1)).is_empty()
            );
        }

        for key in ["3", "3", "2"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.pending_shortcut(), None);
        for page in 0..3 {
            assert!(
                highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + page + 1)).is_empty()
            );
        }
    }

    #[test]
    fn split_prefix_spans_preserve_category_and_option_column_alignment() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        let baseline: Vec<_> = (0..3)
            .map(|page| {
                let spans = toolbar.toolbar_spans(MENU_FIRST_ROW + page + 1);
                (
                    prefix_start(&spans, &format!("3.{}.", page + 1)),
                    option_start(&spans, 1, page * OPTIONS_PER_PAGE),
                )
            })
            .collect();

        press(&mut toolbar, "3");
        for (page, expected) in baseline.into_iter().enumerate() {
            let spans = toolbar.toolbar_spans(MENU_FIRST_ROW + page + 1);
            let common_prefix_start = span_starts(&spans)
                .into_iter()
                .find_map(|(start, span)| {
                    (span.highlighted && span.contents == "3.").then_some(start)
                })
                .unwrap();
            assert_eq!(common_prefix_start, expected.0);
            assert_eq!(option_start(&spans, 1, page * OPTIONS_PER_PAGE), expected.1);
        }
    }

    #[test]
    fn exact_highlight_spans_survive_narrow_box_clipping_without_width_drift() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        press(&mut toolbar, "2");

        for width in 8..48 {
            let spans = boxed_toolbar_spans(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1), width);
            assert_eq!(UnicodeWidthStr::width(spans_text(&spans).as_str()), width);
            let highlighted = highlighted_contents(&spans);
            assert!(
                highlighted.is_empty()
                    || (highlighted.len() == 1 && "2.".starts_with(highlighted[0]))
            );
        }
    }

    #[test]
    fn pending_prefix_consumes_an_invalid_editor_key_then_resets() {
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, "1");
        assert!(toolbar.handle_shortcut(&Key::Character("r".into()), ModifiersState::empty()));
        assert!(!toolbar.handle_shortcut(&Key::Character("r".into()), ModifiersState::empty()));
    }

    #[test]
    fn mouse_hit_testing_directly_selects_modes_and_options() {
        let mut toolbar = ToolbarState::default();
        let action = toolbar.action_at(0, 14, 80).expect("Line is clickable");
        assert_eq!(action, ToolbarAction::SelectMain(MainMode::Line));
        assert!(toolbar.apply_action(action));

        let expected_decorator = ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 5,
        };
        let decorator_column = span_starts(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1))
            .into_iter()
            .find_map(|(start, span)| {
                (span.action == Some(expected_decorator)).then_some(start + 2)
            })
            .expect("decorator is visible");
        let decorator = toolbar
            .action_at(MENU_FIRST_ROW + 1, decorator_column, 80)
            .expect("decorator is clickable");
        assert_eq!(decorator, expected_decorator);
        assert!(toolbar.apply_action(decorator));
        assert_eq!(toolbar.stamp(), "□");

        assert!(toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Line)));
        let shortcut_spans = toolbar.toolbar_spans(3);
        let (column, _) = shortcut_spans
            .iter()
            .scan(0, |column, span| {
                let start = *column;
                *column += UnicodeWidthStr::width(span.contents.as_str());
                Some((start, span))
            })
            .find(|(_, span)| {
                span.action
                    == Some(ToolbarAction::SelectSubmenu {
                        submenu: 2,
                        option: 2,
                    })
            })
            .expect("flattened Width shortcut is clickable");
        let width = toolbar
            .action_at(3, column + 2, 80)
            .expect("flattened Width shortcut hit tests");
        assert!(toolbar.apply_action(width));
        assert_eq!(toolbar.line_style(), LineStyle::Double);
    }

    #[test]
    fn utils_tools_are_keyboard_and_mouse_selectable() {
        let mut toolbar = ToolbarState::default();
        for key in ["1", "4", "3"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.main_mode(), MainMode::Utilities);
        assert_eq!(toolbar.utility_kind(), UtilityKind::Push);
        assert_eq!(toolbar.tooltip(), Tooltip::UtilitiesPush);
        assert_eq!(row(&toolbar, 2), "Move    Push    Pull    View");
        assert_eq!(row(&toolbar, 3), "2       3       4       5   ");
        assert!(!row(&toolbar, 2).contains("Tool"));
        for obsolete in ["2.1", "2.2", "2.3"] {
            assert!(!row(&toolbar, 3).contains(obsolete));
        }

        let expected = ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        };
        for row in [2, 3] {
            let pull_column = (0..80)
                .find(|column| toolbar.action_at(row, *column, 80) == Some(expected))
                .expect("Pull label and shortcut are visible and clickable");
            let action = toolbar.action_at(row, pull_column, 80).unwrap();
            assert!(toolbar.apply_action(action));
        }
        assert_eq!(toolbar.utility_kind(), UtilityKind::Pull);
        assert_eq!(toolbar.tooltip(), Tooltip::UtilitiesPull);
        assert_eq!(
            toolbar
                .toolbar_spans(2)
                .iter()
                .filter(|span| span.selected)
                .count(),
            1
        );
        assert!(
            toolbar
                .toolbar_spans(3)
                .iter()
                .all(|span| !span.highlighted)
        );

        let view_action = ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 3,
        };
        for row in [2, 3] {
            let column = (0..80)
                .find(|column| toolbar.action_at(row, *column, 80) == Some(view_action))
                .expect("View label and shortcut are visible and clickable");
            assert!(
                toolbar.apply_action(toolbar.action_at(row, column, 80).expect("View hit tests"))
            );
        }
        assert_eq!(toolbar.utility_kind(), UtilityKind::View);
        assert_eq!(toolbar.tooltip(), Tooltip::UtilitiesView);

        let move_action = ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 0,
        };
        for row in [2, 3] {
            let column = (0..80)
                .find(|column| toolbar.action_at(row, *column, 80) == Some(move_action))
                .expect("Move label and shortcut are visible and clickable");
            assert!(
                toolbar.apply_action(toolbar.action_at(row, column, 80).expect("Move hit tests"))
            );
        }
        assert_eq!(toolbar.utility_kind(), UtilityKind::Move);
        assert_eq!(toolbar.tooltip(), Tooltip::UtilitiesMove);
    }

    #[test]
    fn utils_are_direct_peer_shortcuts_with_no_pending_prefix() {
        let mut toolbar = ToolbarState::default();
        for key in ["1", "4"] {
            press(&mut toolbar, key);
        }

        for (key, utility, tooltip) in [
            ("2", UtilityKind::Move, Tooltip::UtilitiesMove),
            ("3", UtilityKind::Push, Tooltip::UtilitiesPush),
            ("4", UtilityKind::Pull, Tooltip::UtilitiesPull),
            ("5", UtilityKind::View, Tooltip::UtilitiesView),
        ] {
            press(&mut toolbar, key);
            assert_eq!(toolbar.utility_kind(), utility);
            assert_eq!(toolbar.tooltip(), tooltip);
            assert_eq!(toolbar.pending_shortcut(), None);
            assert!(
                toolbar
                    .toolbar_spans(MENU_FIRST_ROW + 1)
                    .iter()
                    .all(|span| !span.highlighted)
            );
        }

        press(&mut toolbar, "9");
        assert_eq!(toolbar.utility_kind(), UtilityKind::View);
        assert_eq!(toolbar.pending_shortcut(), None);
        assert!(!toolbar.handle_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty()));
    }

    #[test]
    fn utils_peer_menu_clips_cleanly_at_narrow_widths() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Utilities));

        for width in 0..24 {
            for row in MENU_FIRST_ROW..=MENU_FIRST_ROW + 1 {
                let spans = boxed_toolbar_spans(&toolbar.toolbar_spans(row), width);
                assert_eq!(UnicodeWidthStr::width(spans_text(&spans).as_str()), width);
                assert!(spans.iter().all(|span| !span.highlighted));
            }
        }
    }

    #[test]
    fn mouse_hit_testing_tracks_later_page_options_after_column_padding() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        let box_width = 160;

        for (row, expected) in [
            (
                MENU_FIRST_ROW + 2,
                ToolbarAction::SelectSubmenu {
                    submenu: 0,
                    option: 10,
                },
            ),
            (
                MENU_FIRST_ROW + 3,
                ToolbarAction::SelectSubmenu {
                    submenu: 1,
                    option: 20,
                },
            ),
            (
                MENU_FIRST_ROW + 2,
                ToolbarAction::SelectSubmenu {
                    submenu: 3,
                    option: 10,
                },
            ),
        ] {
            let visible = boxed_toolbar_spans(&toolbar.toolbar_spans(row), box_width);
            let column = span_starts(&visible)
                .into_iter()
                .find_map(|(start, span)| (span.action == Some(expected)).then_some(start))
                .expect("later-page option remains visible");
            assert_eq!(toolbar.action_at(row, column, box_width), Some(expected));
        }
    }

    #[test]
    fn every_padded_unicode_menu_row_clips_to_exact_narrow_box_width() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));

        for width in 0..48 {
            for row in 0..toolbar.content_rows() {
                let boxed = boxed_toolbar_spans(&toolbar.toolbar_spans(row), width);
                assert_eq!(UnicodeWidthStr::width(spans_text(&boxed).as_str()), width);
                assert!(boxed.iter().all(|span| {
                    span.contents
                        .chars()
                        .all(|character| UnicodeWidthChar::width(character).is_some())
                }));
            }
        }
    }

    #[test]
    fn export_entry_is_right_aligned_and_narrow_box_stays_valid_unicode_width() {
        let toolbar = ToolbarState::default();
        let wide = spans_text(&boxed_toolbar_spans(&toolbar.toolbar_spans(0), 60));
        assert!(wide.ends_with("0. Save/Load/Export │"));
        assert!(wide.starts_with("│ Mode: 1"));
        for width in 0..32 {
            let text = spans_text(&boxed_toolbar_spans(&toolbar.toolbar_spans(0), width));
            assert_eq!(UnicodeWidthStr::width(text.as_str()), width);
        }
    }

    #[test]
    fn keyboard_export_paths_queue_actions_stay_active_and_escape_restores_mode() {
        let mut toolbar = ToolbarState::default();
        let mode = toolbar.main_mode();
        for key in ["0", "2", "1"] {
            press(&mut toolbar, key);
        }
        assert_eq!(
            toolbar.take_export_action(),
            Some(ExportAction::ClipboardTxt)
        );
        assert!(toolbar.export_menu_open());
        assert_eq!(toolbar.active_export_category, Some(0));
        assert_eq!(
            toolbar.pending_shortcut(),
            Some(PendingShortcut::ExportOption(0))
        );
        assert_eq!(toolbar.main_mode(), mode);

        assert!(toolbar.handle_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty()));
        assert!(!toolbar.export_menu_open());
        assert_eq!(toolbar.main_mode(), mode);
    }

    #[test]
    fn export_modes_are_peers_of_editor_mode_and_escape_restores_it() {
        let mut toolbar = ToolbarState::default();
        assert_eq!(selected_main_mode_count(&toolbar), 1);
        let durable = toolbar.durable_selections();

        press(&mut toolbar, "0");
        assert_eq!(toolbar.active_export_category, None);
        assert_eq!(selected_main_mode_count(&toolbar), 0);
        assert_eq!(toolbar.tooltip(), Tooltip::Export);
        assert_eq!(toolbar.durable_selections(), durable);
        assert!(
            toolbar
                .toolbar_spans(MENU_FIRST_ROW)
                .iter()
                .all(|span| !span.selected)
        );

        press(&mut toolbar, "3");
        assert_eq!(toolbar.active_export_category, Some(1));
        assert!(
            toolbar
                .toolbar_spans(MAIN_SHORTCUT_ROW)
                .iter()
                .filter(|span| matches!(span.action, Some(ToolbarAction::SelectMain(_))))
                .all(|span| !span.selected)
        );
        let selected: Vec<_> = toolbar
            .toolbar_spans(MENU_FIRST_ROW)
            .into_iter()
            .filter(|span| span.selected)
            .map(|span| span.contents.trim().to_string())
            .collect();
        assert_eq!(selected, vec!["Save:"]);
        assert_eq!(
            highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)),
            vec!["3."]
        );

        assert!(toolbar.handle_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty()));
        assert!(!toolbar.export_menu_open());
        assert_eq!(toolbar.active_export_category, None);
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert_eq!(selected_main_mode_count(&toolbar), 1);
        assert_eq!(toolbar.durable_selections(), durable);
    }

    #[test]
    fn export_open_and_close_preserve_every_editor_mode_and_tool_selection() {
        for mode in MainMode::ALL {
            let mut toolbar = ToolbarState::default();
            assert!(toolbar.apply_action(ToolbarAction::SelectMain(mode)));
            let action = match mode {
                MainMode::Line => ToolbarAction::SelectSubmenu {
                    submenu: 2,
                    option: 1,
                },
                MainMode::Stamp => ToolbarAction::SelectSubmenu {
                    submenu: 1,
                    option: 3,
                },
                MainMode::Shapes => ToolbarAction::SelectSubmenu {
                    submenu: 0,
                    option: 1,
                },
                MainMode::Utilities => ToolbarAction::SelectSubmenu {
                    submenu: 0,
                    option: 2,
                },
            };
            assert!(toolbar.apply_action(action));
            let durable = toolbar.durable_selections();

            assert!(toolbar.apply_action(ToolbarAction::ToggleExportMenu));
            assert!(toolbar.export_menu_open());
            assert_eq!(toolbar.active_export_category, None);
            assert_eq!(selected_main_mode_count(&toolbar), 0);
            assert_eq!(toolbar.durable_selections(), durable);

            assert!(toolbar.apply_action(ToolbarAction::ToggleExportMenu));
            assert!(!toolbar.export_menu_open());
            assert_eq!(selected_main_mode_count(&toolbar), 1);
            assert_eq!(toolbar.main_mode(), mode);
            assert_eq!(toolbar.durable_selections(), durable);
        }
    }

    #[test]
    fn export_keyboard_paths_use_peer_mode_numbers() {
        for (keys, expected) in [
            (&["0", "2", "2"][..], ExportAction::ClipboardPng),
            (&["0", "3", "2"][..], ExportAction::SavePng),
            (&["0", "4", "1"][..], ExportAction::LoadTxt),
            (&["0", "9"][..], ExportAction::Clear),
        ] {
            let mut toolbar = ToolbarState::default();
            for key in keys {
                press(&mut toolbar, key);
            }
            assert_eq!(toolbar.take_export_action(), Some(expected));
            assert!(toolbar.export_menu_open());
            let category = EXPORT_OPTIONS
                .iter()
                .position(|options| options.iter().any(|(_, action)| *action == expected))
                .unwrap();
            assert_eq!(toolbar.active_export_category, Some(category));
            assert_eq!(
                toolbar.pending_shortcut(),
                Some(if EXPORT_OPTIONS[category].len() == 1 {
                    PendingShortcut::ExportFlat(category)
                } else {
                    PendingShortcut::ExportOption(category)
                })
            );
        }
    }

    #[test]
    fn every_export_action_stays_in_its_category_after_take_and_can_repeat() {
        for (category, options) in EXPORT_OPTIONS.iter().enumerate() {
            for (_, action) in *options {
                let mut toolbar = ToolbarState::default();
                assert!(toolbar.apply_action(ToolbarAction::ToggleExportMenu));
                assert!(toolbar.apply_action(ToolbarAction::RunExport(*action)));
                let durable = toolbar.durable_selections();

                for _ in 0..2 {
                    assert_eq!(toolbar.take_export_action(), Some(*action));
                    assert_eq!(toolbar.take_export_action(), None);
                    assert!(toolbar.export_menu_open());
                    assert_eq!(toolbar.active_export_category, Some(category));
                    assert_eq!(toolbar.durable_selections(), durable);
                    assert_eq!(toolbar.tooltip(), Tooltip::Export);
                    assert_eq!(selected_main_mode_count(&toolbar), 0);
                    assert!(toolbar.apply_action(ToolbarAction::RunExport(*action)));
                }
            }
        }
    }

    #[test]
    fn successful_export_highlights_only_its_value_until_cleared() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::ToggleExportMenu);
        toolbar.mark_export_successful(ExportAction::SavePng);

        let selected: Vec<_> = toolbar
            .toolbar_spans(MENU_FIRST_ROW + 1)
            .into_iter()
            .filter(|span| span.selected)
            .map(|span| span.contents)
            .collect();
        assert_eq!(selected, ["PNG"]);

        assert!(toolbar.clear_export_success());
        assert!(
            toolbar
                .toolbar_spans(MENU_FIRST_ROW + 1)
                .iter()
                .all(|span| !span.selected)
        );
        assert!(!toolbar.clear_export_success());
    }

    #[test]
    fn completed_export_keeps_exact_prefix_highlight_and_zero_toggles_closed() {
        let mut toolbar = ToolbarState::default();
        for key in ["0", "3", "2"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.take_export_action(), Some(ExportAction::SavePng));
        assert_eq!(
            highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)),
            vec!["3."]
        );

        // The active option prefix accepts another action directly.
        press(&mut toolbar, "1");
        assert_eq!(toolbar.take_export_action(), Some(ExportAction::SaveTxt));
        assert_eq!(toolbar.active_export_category, Some(1));

        // Zero is always the peer-mode toggle, even while an option prefix is active.
        press(&mut toolbar, "0");
        assert!(!toolbar.export_menu_open());
        assert_eq!(toolbar.active_export_category, None);
        assert_eq!(selected_main_mode_count(&toolbar), 1);

        for key in ["0", "9"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.take_export_action(), Some(ExportAction::Clear));
        press(&mut toolbar, "9");
        assert_eq!(toolbar.take_export_action(), Some(ExportAction::Clear));
    }

    #[test]
    fn editor_mode_selection_closes_and_deselects_an_export_mode() {
        let mut toolbar = ToolbarState::default();
        for key in ["0", "3"] {
            press(&mut toolbar, key);
        }
        assert!(toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp)));
        assert!(!toolbar.export_menu_open());
        assert_eq!(toolbar.active_export_category, None);
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);

        press(&mut toolbar, "0");
        press(&mut toolbar, "1");
        assert!(!toolbar.export_menu_open());
        assert_eq!(toolbar.pending_shortcut(), Some(PendingShortcut::Mode));
        press(&mut toolbar, "3");
        assert_eq!(toolbar.main_mode(), MainMode::Shapes);
    }

    #[test]
    fn mouse_category_selection_matches_keyboard_state_and_action() {
        let mut toolbar = ToolbarState::default();
        assert!(toolbar.apply_action(ToolbarAction::ToggleExportMenu));
        assert_eq!(selected_main_mode_count(&toolbar), 0);
        assert_eq!(toolbar.active_export_category, None);
        let width = 100;
        let category_column = span_starts(&boxed_toolbar_spans(
            &toolbar.toolbar_spans(MENU_FIRST_ROW),
            width,
        ))
        .into_iter()
        .find_map(|(column, span)| {
            (span.action == Some(ToolbarAction::SelectExportCategory(1))).then_some(column)
        })
        .expect("Save category is clickable");
        let select = toolbar
            .action_at(MENU_FIRST_ROW, category_column, width)
            .expect("Save category hit tests");
        assert!(toolbar.apply_action(select));
        assert_eq!(toolbar.active_export_category, Some(1));
        assert_eq!(
            toolbar.pending_shortcut(),
            Some(PendingShortcut::ExportOption(1))
        );

        assert!(toolbar.apply_action(ToolbarAction::RunExport(ExportAction::SaveJson)));
        assert_eq!(toolbar.take_export_action(), Some(ExportAction::SaveJson));
        assert!(toolbar.export_menu_open());
        assert_eq!(toolbar.active_export_category, Some(1));
        assert_eq!(
            toolbar.pending_shortcut(),
            Some(PendingShortcut::ExportOption(1))
        );
    }

    #[test]
    fn clear_is_keyboard_selectable_as_its_own_export_category() {
        let mut toolbar = ToolbarState::default();
        for key in ["0", "9"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.take_export_action(), Some(ExportAction::Clear));
        assert!(toolbar.export_menu_open());
        assert_eq!(toolbar.active_export_category, Some(3));
        assert_eq!(
            toolbar.pending_shortcut(),
            Some(PendingShortcut::ExportFlat(3))
        );
    }

    #[test]
    fn clear_path_highlighting_and_layout_match_the_other_export_actions() {
        let mut toolbar = ToolbarState::default();
        press(&mut toolbar, "0");
        assert!(highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)).is_empty());
        assert!(row(&toolbar, MENU_FIRST_ROW).contains("Load:"));
        assert!(!row(&toolbar, MENU_FIRST_ROW).contains("Clear"));
        assert!(spans_text(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)).contains("9. Clear"));
        assert!(!row(&toolbar, MENU_FIRST_ROW + 1).contains("0."));
        assert_eq!(
            EXPORT_OPTIONS[1]
                .iter()
                .map(|(label, _)| *label)
                .collect::<Vec<_>>(),
            ["TXT", "PNG", "JSON"]
        );
        assert_eq!(EXPORT_OPTIONS[2].len(), 2);
        assert_eq!(EXPORT_OPTIONS[3], &[("Clear", ExportAction::Clear)]);
        for width in 0..48 {
            let boxed = boxed_toolbar_spans(&toolbar.toolbar_spans(MENU_FIRST_ROW), width);
            assert_eq!(UnicodeWidthStr::width(spans_text(&boxed).as_str()), width);
        }
    }

    #[test]
    fn mouse_export_entry_and_action_match_keyboard_paths() {
        let mut toolbar = ToolbarState::default();
        let width = 60;
        let entry_column = width - 2 - UnicodeWidthStr::width("0. Save/Load/Export");
        let toggle = toolbar
            .action_at(0, entry_column, width)
            .expect("export entry clickable");
        assert_eq!(toggle, ToolbarAction::ToggleExportMenu);
        assert!(toolbar.apply_action(toggle));

        let clipboard_txt = boxed_toolbar_spans(&toolbar.toolbar_spans(3), width)
            .iter()
            .scan(0, |column, span| {
                let start = *column;
                *column += UnicodeWidthStr::width(span.contents.as_str());
                Some((start, span.action))
            })
            .find_map(|(column, action)| {
                (action == Some(ToolbarAction::RunExport(ExportAction::ClipboardTxt)))
                    .then_some(column)
            })
            .expect("Clipboard TXT is visible");
        let action = toolbar.action_at(3, clipboard_txt, width).unwrap();
        assert!(toolbar.apply_action(action));
        assert_eq!(
            toolbar.take_export_action(),
            Some(ExportAction::ClipboardTxt)
        );
    }

    #[test]
    fn clear_export_action_is_visible_and_mouse_selectable() {
        let mut toolbar = ToolbarState::default();
        let width = 80;
        assert!(toolbar.apply_action(ToolbarAction::ToggleExportMenu));
        assert!(
            spans_text(&boxed_toolbar_spans(
                &toolbar.toolbar_spans(MENU_FIRST_ROW + 1),
                width,
            ))
            .ends_with("9. Clear │")
        );

        let clear_column = boxed_toolbar_spans(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1), width)
            .iter()
            .scan(0, |column, span| {
                let start = *column;
                *column += UnicodeWidthStr::width(span.contents.as_str());
                Some((start, span.action))
            })
            .find_map(|(column, action)| {
                (action == Some(ToolbarAction::RunExport(ExportAction::Clear))).then_some(column)
            })
            .expect("Clear is visible");
        let action = toolbar
            .action_at(MENU_FIRST_ROW + 1, clear_column, width)
            .unwrap();
        assert!(toolbar.apply_action(action));
        assert_eq!(toolbar.take_export_action(), Some(ExportAction::Clear));
    }

    #[test]
    fn main_modes_map_to_distinct_typed_tooltips() {
        let mut toolbar = ToolbarState::default();
        let expected = [
            Tooltip::Stamp,
            Tooltip::Line,
            Tooltip::Shapes,
            Tooltip::UtilitiesMove,
        ];
        for (mode, tooltip) in MainMode::ALL.into_iter().zip(expected) {
            assert_eq!(mode.tooltip(), tooltip);
            toolbar.apply_action(ToolbarAction::SelectMain(mode));
            assert_eq!(toolbar.tooltip(), tooltip);
        }

        assert_ne!(Tooltip::Line.text(), Tooltip::Stamp.text());
        assert_ne!(Tooltip::Stamp.text(), Tooltip::Shapes.text());
        assert_ne!(Tooltip::Shapes.text(), Tooltip::UtilitiesMove.text());
        for tooltip in [
            Tooltip::Line,
            Tooltip::Stamp,
            Tooltip::Shapes,
            Tooltip::UtilitiesPush,
            Tooltip::UtilitiesPull,
            Tooltip::UtilitiesView,
            Tooltip::UtilitiesMove,
        ] {
            assert!(tooltip.text().contains("Alt-direction erases"));
        }

        toolbar.apply_action(ToolbarAction::ToggleExportMenu);
        assert_eq!(toolbar.tooltip(), Tooltip::Export);
        assert!(toolbar.tooltip().text().contains("visible viewport"));
    }

    #[test]
    fn tooltip_is_clipped_by_unicode_display_width_and_never_boxed() {
        let spans = tooltip_spans(Tooltip::Stamp, 13);
        assert_eq!(spans.len(), 1);
        assert!(spans[0].tooltip);
        assert_eq!(UnicodeWidthStr::width(spans[0].contents.as_str()), 13);
        assert!(!spans[0].contents.contains('│'));

        let toolbar = ToolbarState::default();
        for row in 0..toolbar.content_rows() {
            assert!(toolbar.toolbar_spans(row).iter().all(|span| !span.tooltip));
        }
        assert!(tooltip_spans(Tooltip::Line, 0).is_empty());
        assert!(tooltip_spans(Tooltip::None, 20).is_empty());
    }

    #[test]
    fn every_stamp_symbol_is_one_grapheme_and_one_display_cell() {
        for symbol in STAMP_OPTIONS.into_iter().flatten() {
            assert_eq!(
                UnicodeSegmentation::graphemes(*symbol, true).count(),
                1,
                "{symbol:?}"
            );
            assert_eq!(UnicodeWidthStr::width(*symbol), 1, "{symbol:?}");
        }
    }

    #[test]
    fn stamp_families_exactly_match_the_documented_uniline_sets() {
        assert_eq!(&DECORATORS[..6], SQUARES_AND_DIAMONDS);
        assert_eq!(&DECORATORS[6..13], DOTS_AND_CIRCLES);
        assert_eq!(&DECORATORS[13..], CROSSES_AND_OPERATORS);
        assert_eq!(ARROWS.len(), 22);
        assert_eq!(
            ARROWS.iter().copied().collect::<String>(),
            "△▽◁▷▲▼◀▶↑↓←→▵▿◃▹▴▾◂▸↕↔"
        );
        assert_eq!(GREY_SHADING.iter().copied().collect::<String>(), "░▒▓█");
        assert_eq!(
            QUADRANT_BLOCKS.iter().copied().collect::<String>(),
            "▘▝▀▖▌▞▛▗▚▐▜▄▙▟█"
        );

        let counts =
            STAMP_OPTIONS
                .into_iter()
                .flatten()
                .fold(HashMap::new(), |mut counts, symbol| {
                    *counts.entry(*symbol).or_insert(0) += 1;
                    counts
                });
        assert_eq!(counts.len(), 60);
        assert_eq!(counts.get("█"), Some(&2));
        assert!(
            counts
                .iter()
                .all(|(symbol, count)| *symbol == "█" || *count == 1)
        );
    }

    #[test]
    fn arrow_styles_use_opposing_direction_pairs() {
        assert_eq!(
            ARROW_ROTATIONS,
            [
                ["△", "▽", "◁", "▷"],
                ["▲", "▼", "◀", "▶"],
                ["↑", "↓", "←", "→"],
                ["▵", "▿", "◃", "▹"],
                ["▴", "▾", "◂", "▸"],
                ["↕", "↕", "↔", "↔"],
            ]
        );
    }

    #[test]
    fn stars_excluded_decorations_ascii_and_connected_lines_are_not_stamps() {
        let stamps: Vec<_> = STAMP_OPTIONS.into_iter().flatten().copied().collect();
        for excluded in [
            "☆", "★", "○", "◇", "※", "▁", "▂", "▃", "▅", "▆", "▇", "▊", "▉",
        ] {
            assert!(
                !stamps.contains(&excluded),
                "excluded decoration {excluded:?}"
            );
        }
        for ascii in [
            "^", "v", "V", "|", "\"", "-", "_", ">", "<", "=", "+", "/", "\\", "'", "`", "#", "o",
            "O", "*", ".",
        ] {
            assert!(!stamps.contains(&ascii), "ASCII conversion glyph {ascii:?}");
        }
        for connected_line in ["─", "━", "═", "┌", "╬", "╎", "╏"] {
            assert!(
                !stamps.contains(&connected_line),
                "connected line {connected_line:?}"
            );
        }
        for diagonal in ["╳", "╱", "╲"] {
            assert!(
                stamps.contains(&diagonal),
                "standalone diagonal {diagonal:?}"
            );
        }
    }

    #[test]
    fn stamp_selection_is_exclusive_across_families() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        assert_eq!(toolbar.stamp(), "□");

        assert!(toolbar.apply_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 21
        }));
        assert_eq!(toolbar.stamp(), "↔");
        assert_eq!(
            toolbar
                .toolbar_spans(MENU_FIRST_ROW + 3)
                .iter()
                .filter(|span| span.selected)
                .count(),
            1
        );

        assert!(toolbar.apply_action(ToolbarAction::SelectSubmenu {
            submenu: 3,
            option: 14
        }));
        assert_eq!(toolbar.stamp(), "█");
        let selected_count: usize = (0..toolbar.menu_row_count())
            .map(|row| {
                toolbar
                    .toolbar_spans(MENU_FIRST_ROW + row)
                    .iter()
                    .filter(|span| span.selected)
                    .count()
            })
            .sum();
        assert_eq!(selected_count, 1);
    }

    #[test]
    fn unrelated_modified_keys_are_not_toolbar_shortcuts() {
        let mut toolbar = ToolbarState::default();
        assert!(!toolbar.handle_shortcut(&Key::Character("2".into()), ModifiersState::ALT));
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
    }
}
