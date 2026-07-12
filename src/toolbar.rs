use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::drawing::{CornerStyle, LineEnding, LineStyle};
use crate::export::ExportAction;

pub const TOOLBAR_ROW_GAP: usize = 0;

const MAIN_LABEL_ROW: usize = 0;
const MAIN_SHORTCUT_ROW: usize = 1;
const MENU_FIRST_ROW: usize = 2;
const OPTIONS_PER_PAGE: usize = 10;
const GAP: &str = "    ";

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
    let (contents, _) = clipped_to_width(tooltip.text(), width);
    vec![ToolbarSpan {
        contents,
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
const LINE_OPTIONS: [&[&str]; 4] = [
    &["·", "◀", "◆", "●"],
    &["·", "▶", "◆", "●"],
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
const DECORATORS: [&str; 20] = [
    "□", "■", "▫", "▪", "◆", "◊", "·", "∙", "•", "●", "◦", "Ø", "ø", "╳", "╱", "╲", "÷", "×", "±",
    "¤",
];

#[cfg(test)]
const ARROW_ROTATIONS: [[&str; 4]; 6] = [
    ["△", "▷", "▽", "◁"],
    ["▲", "▶", "▼", "◀"],
    ["↑", "→", "↓", "←"],
    ["▵", "▹", "▿", "◃"],
    ["▴", "▸", "▾", "◂"],
    ["↕", "↔", "↕", "↔"],
];
const ARROWS: [&str; 22] = [
    "△", "▷", "▽", "◁", "▲", "▶", "▼", "◀", "↑", "→", "↓", "←", "▵", "▹", "▿", "◃", "▴", "▸", "▾",
    "◂", "↕", "↔",
];
const GREY_SHADING: [&str; 4] = ["░", "▒", "▓", "█"];
const QUADRANT_BLOCKS: [&str; 15] = [
    "▘", "▝", "▀", "▖", "▌", "▞", "▛", "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
];

const STAMP_LABELS: [&str; 4] = ["Decorators", "Arrows", "Fills", "Blocks"];
const STAMP_OPTIONS: [&[&str]; 4] = [&DECORATORS, &ARROWS, &GREY_SHADING, &QUADRANT_BLOCKS];
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
    pub const ALL: [Self; 4] = [Self::Line, Self::Stamp, Self::Shapes, Self::Utilities];

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
            Self::Utilities => Tooltip::Utilities,
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
    Utilities,
    Text,
    Replace,
    Export,
}

impl Tooltip {
    pub fn text(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Line => {
                "Ctrl/Cmd-Z undo; Ctrl/Cmd-R redo; Shift-direction draws; Alt-hjkl/arrows resize selection; Escape collapses; Space/Backspace clears; r then character replaces"
            }
            Self::Stamp => {
                "Ctrl/Cmd-Z undo; Ctrl/Cmd-R redo; Alt-hjkl/arrows resize selection; Escape collapses; Backspace clears; Space fills with stamp; r then character replaces"
            }
            Self::Shapes => {
                "Ctrl/Cmd-Z undo; Ctrl/Cmd-R redo; Alt-hjkl/arrows resize selection; Escape collapses/cancels preview; Space confirms; Backspace clears; r then character replaces"
            }
            Self::Utilities => {
                "Ctrl/Cmd-Z undo; Ctrl/Cmd-R redo; Alt-hjkl/arrows resize selection; Escape collapses; Backspace clears; r then character replaces"
            }
            Self::Text => {
                "Ctrl/Cmd-Z undo; Ctrl/Cmd-R redo; <Ret> exits text mode; arrows move freely over the canvas"
            }
            Self::Replace => {
                "Ctrl/Cmd-Z undo; Ctrl/Cmd-R redo; <Shift-Ret> exits replace mode; arrows move freely over the canvas"
            }
            Self::Export => {
                "TXT/JSON export selection only; PNG canvas-only screenshot is deferred"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingShortcut {
    Mode,
    Category(usize),
    Option { category: usize, page: usize },
    ExportCategory,
    ExportOption(usize),
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
    shortcut_prefix: Option<PendingShortcut>,
    export_open: bool,
    pending_export_action: Option<ExportAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolbarSpan {
    pub contents: String,
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
    RunExport(ExportAction),
}

const EXPORT_LABELS: [&str; 3] = ["Clipboard", "Save", "Load"];
const EXPORT_OPTIONS: [&[(&str, ExportAction)]; 3] = [
    &[
        ("TXT", ExportAction::ClipboardTxt),
        ("PNG", ExportAction::ClipboardPng),
    ],
    &[
        ("TXT", ExportAction::SaveTxt),
        ("JSON", ExportAction::SaveJson),
        ("PNG", ExportAction::SavePng),
    ],
    &[
        ("TXT", ExportAction::LoadTxt),
        ("JSON", ExportAction::LoadJson),
    ],
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
            self.shortcut_prefix = None;
            self.export_open = false;
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

        match self.shortcut_prefix.take() {
            None => {
                if digit == 0 {
                    self.export_open = !self.export_open;
                    self.shortcut_prefix =
                        self.export_open.then_some(PendingShortcut::ExportCategory);
                } else if self.export_open {
                    self.shortcut_prefix = digit
                        .checked_sub(1)
                        .filter(|category| *category < EXPORT_LABELS.len())
                        .map(PendingShortcut::ExportOption);
                } else {
                    self.shortcut_prefix = if digit == 1 {
                        Some(PendingShortcut::Mode)
                    } else {
                        digit
                            .checked_sub(2)
                            .filter(|category| *category < self.layout().labels.len())
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
                    .options
                    .get(category)
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
                    self.export_open = false;
                } else {
                    self.shortcut_prefix = digit
                        .checked_sub(1)
                        .filter(|category| *category < EXPORT_LABELS.len())
                        .map(PendingShortcut::ExportOption);
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
        }
        true
    }

    pub fn cancel_shortcut(&mut self) {
        self.shortcut_prefix = None;
    }

    pub fn close_export_menu(&mut self) {
        self.export_open = false;
        self.shortcut_prefix = None;
    }

    pub fn export_menu_open(&self) -> bool {
        self.export_open
    }

    pub fn take_export_action(&mut self) -> Option<ExportAction> {
        self.pending_export_action.take()
    }

    fn queue_export(&mut self, action: ExportAction) {
        self.pending_export_action = Some(action);
        self.close_export_menu();
    }

    fn cancel_pending_shortcut(&mut self) -> bool {
        self.shortcut_prefix.take().is_some()
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
        self.layout()
            .options
            .iter()
            .map(|options| options.len().div_ceil(OPTIONS_PER_PAGE) * 2)
            .max()
            .unwrap_or(0)
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
                } else {
                    self.menu_spans(row)
                }
            }
            _ => Vec::new(),
        }
    }

    fn main_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let mut spans = if row == MAIN_LABEL_ROW {
            vec![plain_span("Mode: ".to_string())]
        } else {
            let mut prefix = plain_span("1.".to_string());
            prefix.highlighted = self.pending_shortcut() == Some(PendingShortcut::Mode);
            vec![prefix, plain_span("    ".to_string())]
        };
        for (index, mode) in MainMode::ALL.iter().enumerate() {
            if index > 0 {
                spans.push(plain_span(" ".to_string()));
            }
            let contents = if row == MAIN_LABEL_ROW {
                mode.label().to_string()
            } else if index + 1 == MainMode::ALL.len() {
                (index + 1).to_string()
            } else {
                aligned_shortcut(index + 1, mode.label())
            };
            spans.push(ToolbarSpan {
                contents,
                selected: row == MAIN_LABEL_ROW && *mode == self.main_mode,
                highlighted: false,
                tooltip: false,
                action: Some(ToolbarAction::SelectMain(*mode)),
                right_aligned: false,
            });
        }
        if row == MAIN_LABEL_ROW {
            spans.push(ToolbarSpan {
                contents: "0. Save/Load/Export".to_string(),
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
        let label_row = row == MENU_FIRST_ROW;
        let mut spans = Vec::new();
        for (category, label) in EXPORT_LABELS.iter().enumerate() {
            if category > 0 {
                spans.push(plain_span(GAP.to_string()));
            }
            let cell_start = spans_width(&spans);
            let path = format!("0.{}.", category + 1);
            let prefix_width = menu_prefix_width(label, std::iter::once(path.as_str()));
            let cell_width = prefix_width
                + EXPORT_OPTIONS[category]
                    .iter()
                    .map(|(option, _)| UnicodeWidthStr::width(*option))
                    .sum::<usize>()
                + EXPORT_OPTIONS[category].len().saturating_sub(1);
            if label_row {
                spans.push(plain_span(pad_right_to_width(
                    format!("{label}:"),
                    prefix_width,
                )));
            } else {
                let highlighted_prefix = match self.pending_shortcut() {
                    Some(PendingShortcut::ExportCategory) => Some("0.".to_string()),
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
                    contents: if label_row {
                        (*option).to_string()
                    } else {
                        aligned_shortcut(position + 1, option)
                    },
                    selected: false,
                    highlighted: false,
                    tooltip: false,
                    action: Some(ToolbarAction::RunExport(*action)),
                    right_aligned: false,
                });
            }
            pad_spans_to_width(&mut spans, cell_start + cell_width);
        }
        spans
    }

    fn menu_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let layout = self.layout();
        let relative_row = row - MENU_FIRST_ROW;
        let page = relative_row / 2;
        let label_row = relative_row.is_multiple_of(2);
        let mut spans = Vec::new();
        for category in 0..layout.labels.len() {
            let options = layout.options[category];
            let page_start = page * OPTIONS_PER_PAGE;
            if category > 0 {
                spans.push(plain_span(GAP.to_string()));
            }
            let prefix_width =
                submenu_prefix_width(layout.labels[category], category, options.len());
            let cell_width = submenu_cell_width(prefix_width, options);
            let cell_start = spans_width(&spans);
            if page_start >= options.len() {
                spans.push(plain_span(" ".repeat(cell_width)));
                continue;
            }
            if label_row {
                if page == 0 {
                    spans.push(plain_span(pad_right_to_width(
                        format!("{}:", layout.labels[category]),
                        prefix_width,
                    )));
                } else {
                    spans.push(plain_span(" ".repeat(prefix_width)));
                }
            } else {
                let path = submenu_path(category, page, options.len());
                let highlighted_prefix = match self.pending_shortcut() {
                    Some(PendingShortcut::Category(pending)) if pending == category => {
                        Some(format!("{}.", category + 2))
                    }
                    Some(PendingShortcut::Option {
                        category: pending_category,
                        page: pending_page,
                    }) if pending_category == category && pending_page == page => {
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

            for (position, option) in options[page_start..]
                .iter()
                .take(OPTIONS_PER_PAGE)
                .enumerate()
            {
                if position > 0 {
                    spans.push(plain_span(" ".to_string()));
                }
                let option_index = page_start + position;
                let action = ToolbarAction::SelectSubmenu {
                    submenu: category,
                    option: option_index,
                };
                spans.push(ToolbarSpan {
                    contents: if label_row {
                        (*option).to_string()
                    } else {
                        aligned_shortcut((position + 1) % 10, option)
                    },
                    selected: label_row
                        && option_index == layout.selected[category]
                        && layout
                            .exclusive_submenu
                            .is_none_or(|active| active == category),
                    highlighted: false,
                    tooltip: false,
                    action: Some(action),
                    right_aligned: false,
                });
            }
            pad_spans_to_width(&mut spans, cell_start + cell_width);
        }
        spans
    }

    pub fn tooltip(&self) -> Tooltip {
        if self.export_open {
            return Tooltip::Export;
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
                    MainMode::Utilities => return submenu == 0 && option == 0,
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
                    MainMode::Utilities => unreachable!("utilities returned above"),
                };
                let Some(selected) = selected else {
                    return false;
                };
                *selected = option;
                true
            }
            ToolbarAction::ToggleExportMenu => {
                self.export_open = !self.export_open;
                self.shortcut_prefix = None;
                true
            }
            ToolbarAction::RunExport(action) => {
                self.queue_export(action);
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
                exclusive_submenu: Some(self.stamp_active_category),
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

fn plain_span(contents: String) -> ToolbarSpan {
    ToolbarSpan {
        contents,
        selected: false,
        highlighted: false,
        tooltip: false,
        action: None,
        right_aligned: false,
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
        let mut span = plain_span(highlighted.to_string());
        span.highlighted = true;
        spans.push(span);
        if let Some(remainder) = path.strip_prefix(highlighted)
            && !remainder.is_empty()
        {
            spans.push(plain_span(remainder.to_string()));
        }
    } else {
        spans.push(plain_span(path.to_string()));
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
    prefix_width
        + options
            .chunks(OPTIONS_PER_PAGE)
            .map(|page| {
                page.iter()
                    .map(|option| UnicodeWidthStr::width(*option))
                    .sum::<usize>()
                    + page.len().saturating_sub(1)
            })
            .max()
            .unwrap_or(0)
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

    fn highlighted_contents(spans: &[ToolbarSpan]) -> Vec<&str> {
        spans
            .iter()
            .filter(|span| span.highlighted)
            .map(|span| span.contents.as_str())
            .collect()
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
        assert_eq!(toolbar.menu_row_count(), 2);
        assert_eq!(toolbar.content_rows(), 4);
        assert_eq!(toolbar.rows(), 6);
        assert_eq!(toolbar_height(&toolbar, 18), toolbar.rows() * 18);

        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        assert_eq!(toolbar.menu_row_count(), 6);
        assert_eq!(toolbar.content_rows(), 8);
        assert_eq!(toolbar.rows(), 10);

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
        assert_eq!(toolbar.main_mode(), MainMode::Line);
        press(&mut toolbar, "3");
        assert_eq!(toolbar.main_mode(), MainMode::Shapes);

        assert_eq!(row(&toolbar, 0), "Mode: Line Stamp Shape Utils");
        assert_eq!(row(&toolbar, 1), "1.    1    2     3     4");
        assert_eq!(
            toolbar
                .toolbar_spans(0)
                .iter()
                .filter(|span| span.selected)
                .count(),
            1
        );
    }

    #[test]
    fn two_key_line_width_and_corner_paths_select_without_cycling() {
        let mut toolbar = ToolbarState::default();
        for key in ["4", "3"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.line_style(), LineStyle::Double);

        for key in ["5", "2"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.line_corner(), CornerStyle::Sharp);

        assert!(row(&toolbar, 2).contains("Start: · ◀ ◆ ●"));
        assert!(row(&toolbar, 3).contains("2. 1 2 3 4"));
        assert!(row(&toolbar, 3).contains("4. 1 2 3"));
        assert!(row(&toolbar, 3).contains("5. 1      2"));
    }

    #[test]
    fn three_key_multi_page_path_and_digit_zero_select_exact_options() {
        let mut toolbar = ToolbarState::default();
        for key in ["1", "2", "3", "1", "0"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert_eq!(toolbar.stamp(), "→");

        for key in ["3", "3", "2"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.stamp(), "↔");

        for key in ["2", "2", "0"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.stamp(), "¤");
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
        for key in ["1", "2"] {
            press(&mut toolbar, key);
        }
        assert!(row(&toolbar, 2).starts_with("Decorators: □ ■ ▫ ▪ ◆ ◊ · ∙ • ●"));
        assert!(row(&toolbar, 2).contains("Arrows: △ ▷ ▽ ◁ ▲ ▶ ▼ ◀ ↑ →"));
        assert!(row(&toolbar, 2).contains("Fills: ░ ▒ ▓ █"));
        assert!(row(&toolbar, 2).contains("Blocks: ▘ ▝ ▀ ▖ ▌ ▞ ▛ ▗ ▚ ▐"));
        assert!(row(&toolbar, 4).contains("            ◦ Ø ø ╳ ╱ ╲ ÷ × ± ¤"));
        assert!(row(&toolbar, 4).contains("        ↓ ← ▵ ▹ ▿ ◃ ▴ ▸ ▾ ◂"));
        assert!(row(&toolbar, 4).contains("        ▜ ▄ ▙ ▟ █"));
        assert!(row(&toolbar, 5).contains("2.2. 1 2 3 4 5 6 7 8 9 0"));
        assert!(row(&toolbar, 6).contains("        ↕ ↔"));
        assert!(row(&toolbar, 7).contains("3.3. 1 2"));
    }

    #[test]
    fn every_menu_category_keeps_fixed_prefix_and_option_columns_across_pages() {
        let mut toolbar = ToolbarState::default();

        for mode in MainMode::ALL {
            toolbar.apply_action(ToolbarAction::SelectMain(mode));
            let layout = toolbar.layout();
            let expected: Vec<_> = layout
                .labels
                .iter()
                .enumerate()
                .map(|(category, label)| {
                    let spans = toolbar.toolbar_spans(MENU_FIRST_ROW);
                    (
                        prefix_start(&spans, &format!("{label}:")),
                        option_start(&spans, category, 0),
                    )
                })
                .collect();

            for (category, (options, expected)) in
                layout.options.iter().zip(expected.iter()).enumerate()
            {
                for page in 0..options.len().div_ceil(OPTIONS_PER_PAGE) {
                    let label_spans = toolbar.toolbar_spans(MENU_FIRST_ROW + page * 2);
                    let shortcut_spans = toolbar.toolbar_spans(MENU_FIRST_ROW + page * 2 + 1);
                    let path = submenu_path(category, page, options.len());
                    let prefix_width =
                        submenu_prefix_width(layout.labels[category], category, options.len());
                    assert_eq!(
                        path_cell_start(&shortcut_spans, &path, prefix_width),
                        expected.0
                    );
                    assert_eq!(
                        option_start(&label_spans, category, page * OPTIONS_PER_PAGE),
                        expected.1
                    );
                    assert_eq!(
                        option_start(&shortcut_spans, category, page * OPTIONS_PER_PAGE),
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
        let page_two = toolbar.toolbar_spans(MENU_FIRST_ROW + 3);
        let page_three = toolbar.toolbar_spans(MENU_FIRST_ROW + 5);
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

        for (category, label) in EXPORT_LABELS.iter().enumerate() {
            assert_eq!(
                prefix_start(&labels, &format!("{label}:")),
                path_cell_start(
                    &shortcuts,
                    &format!("0.{}.", category + 1),
                    menu_prefix_width(
                        label,
                        std::iter::once(format!("0.{}.", category + 1).as_str()),
                    ),
                )
            );
            let action = ToolbarAction::RunExport(EXPORT_OPTIONS[category][0].1);
            let label_option = span_starts(&labels)
                .into_iter()
                .find_map(|(start, span)| (span.action == Some(action)).then_some(start))
                .unwrap();
            let shortcut_option = span_starts(&shortcuts)
                .into_iter()
                .find_map(|(start, span)| (span.action == Some(action)).then_some(start))
                .unwrap();
            assert_eq!(label_option, shortcut_option);
        }
    }

    #[test]
    fn invalid_and_cancelled_prefixes_do_not_change_selection() {
        let mut toolbar = ToolbarState::default();
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
        assert!(toolbar.toolbar_spans(MAIN_SHORTCUT_ROW)[0].highlighted);
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
                    highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + page * 2 + 1)),
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
        assert!(highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 3)).is_empty());
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
        assert_eq!(
            highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)),
            vec!["0.", "0.", "0."]
        );
        assert!(
            toolbar
                .toolbar_spans(MAIN_LABEL_ROW)
                .iter()
                .all(|span| !span.highlighted)
        );

        press(&mut toolbar, "2");
        assert_eq!(
            highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + 1)),
            vec!["0.2."]
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
                highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + page * 2 + 1))
                    .is_empty()
            );
        }

        for key in ["3", "3", "2"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.pending_shortcut(), None);
        for page in 0..3 {
            assert!(
                highlighted_contents(&toolbar.toolbar_spans(MENU_FIRST_ROW + page * 2 + 1))
                    .is_empty()
            );
        }
    }

    #[test]
    fn split_prefix_spans_preserve_category_and_option_column_alignment() {
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        let baseline: Vec<_> = (0..3)
            .map(|page| {
                let spans = toolbar.toolbar_spans(MENU_FIRST_ROW + page * 2 + 1);
                (
                    prefix_start(&spans, &format!("3.{}.", page + 1)),
                    option_start(&spans, 1, page * OPTIONS_PER_PAGE),
                )
            })
            .collect();

        press(&mut toolbar, "3");
        for (page, expected) in baseline.into_iter().enumerate() {
            let spans = toolbar.toolbar_spans(MENU_FIRST_ROW + page * 2 + 1);
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
        let action = toolbar.action_at(0, 14, 80).expect("Stamp is clickable");
        assert_eq!(action, ToolbarAction::SelectMain(MainMode::Stamp));
        assert!(toolbar.apply_action(action));

        let decorator = toolbar
            .action_at(2, 18, 80)
            .expect("decorator is clickable");
        assert_eq!(
            decorator,
            ToolbarAction::SelectSubmenu {
                submenu: 0,
                option: 2
            }
        );
        assert!(toolbar.apply_action(decorator));
        assert_eq!(toolbar.stamp(), "▫");

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
                MENU_FIRST_ROW + 4,
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
        assert!(wide.starts_with("│ Mode: Line"));
        for width in 0..32 {
            let text = spans_text(&boxed_toolbar_spans(&toolbar.toolbar_spans(0), width));
            assert_eq!(UnicodeWidthStr::width(text.as_str()), width);
        }
    }

    #[test]
    fn keyboard_export_paths_queue_actions_and_escape_closes_without_changing_mode() {
        let mut toolbar = ToolbarState::default();
        let mode = toolbar.main_mode();
        for key in ["0", "1", "1"] {
            press(&mut toolbar, key);
        }
        assert_eq!(
            toolbar.take_export_action(),
            Some(ExportAction::ClipboardTxt)
        );
        assert!(!toolbar.export_menu_open());
        assert_eq!(toolbar.main_mode(), mode);

        press(&mut toolbar, "0");
        assert!(toolbar.export_menu_open());
        assert!(toolbar.handle_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty()));
        assert!(!toolbar.export_menu_open());
        assert_eq!(toolbar.main_mode(), mode);
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

        let clipboard_txt = boxed_toolbar_spans(&toolbar.toolbar_spans(2), width)
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
        let action = toolbar.action_at(2, clipboard_txt, width).unwrap();
        assert!(toolbar.apply_action(action));
        assert_eq!(
            toolbar.take_export_action(),
            Some(ExportAction::ClipboardTxt)
        );
    }

    #[test]
    fn main_modes_map_to_distinct_typed_tooltips() {
        let mut toolbar = ToolbarState::default();
        let expected = [
            Tooltip::Line,
            Tooltip::Stamp,
            Tooltip::Shapes,
            Tooltip::Utilities,
        ];
        for (mode, tooltip) in MainMode::ALL.into_iter().zip(expected) {
            assert_eq!(mode.tooltip(), tooltip);
            toolbar.apply_action(ToolbarAction::SelectMain(mode));
            assert_eq!(toolbar.tooltip(), tooltip);
            assert!(tooltip.text().contains("Alt-hjkl/arrows resize selection"));
        }

        assert_ne!(Tooltip::Line.text(), Tooltip::Stamp.text());
        assert_ne!(Tooltip::Stamp.text(), Tooltip::Shapes.text());
        assert_ne!(Tooltip::Shapes.text(), Tooltip::Utilities.text());

        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        assert!(toolbar.tooltip().text().contains("Space fills with stamp"));

        toolbar.apply_action(ToolbarAction::ToggleExportMenu);
        assert_eq!(toolbar.tooltip(), Tooltip::Export);
        assert!(toolbar.tooltip().text().contains("export selection only"));
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
            "△▷▽◁▲▶▼◀↑→↓←▵▹▿◃▴▸▾◂↕↔"
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
    fn arrow_styles_preserve_uniline_up_right_down_left_rotation_order() {
        assert_eq!(
            ARROW_ROTATIONS,
            [
                ["△", "▷", "▽", "◁"],
                ["▲", "▶", "▼", "◀"],
                ["↑", "→", "↓", "←"],
                ["▵", "▹", "▿", "◃"],
                ["▴", "▸", "▾", "◂"],
                ["↕", "↔", "↕", "↔"],
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
                .toolbar_spans(6)
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
        assert_eq!(toolbar.main_mode(), MainMode::Line);
    }
}
