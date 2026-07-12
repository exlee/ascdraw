use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::drawing::{CornerStyle, LineEnding, LineStyle};
use crate::export::ExportAction;

pub const TOOLBAR_ROW_GAP: usize = 0;
pub const TOOLTIP_GAP_ROWS: usize = 1;

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
const STAMP_LABELS: [&str; 3] = ["Decorators", "Fills", "Blocks"];
const STAMP_OPTIONS: [&[&str]; 3] = [
    &[
        "○", "●", "◇", "◆", "□", "■", "△", "▲", "☆", "★", "+", "×", "※", "•",
    ],
    &[
        "░", "▒", "▓", "█", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "▀", "▌", "▐", "▊", "▉",
    ],
    &[
        "▘", "▝", "▀", "▖", "▌", "▞", "▛", "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
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

    pub fn tooltip_row(&self) -> usize {
        MENU_FIRST_ROW + self.menu_row_count() + TOOLTIP_GAP_ROWS
    }

    pub fn content_rows(&self) -> usize {
        self.tooltip_row() + 1
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
        let mut prefix = plain_span(if row == MAIN_LABEL_ROW {
            "Mode: ".to_string()
        } else {
            "1.    ".to_string()
        });
        prefix.highlighted =
            row == MAIN_SHORTCUT_ROW && self.pending_shortcut() == Some(PendingShortcut::Mode);
        let mut spans = vec![prefix];
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
                highlighted: self.pending_shortcut() == Some(PendingShortcut::ExportCategory),
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
            let prefix_width = UnicodeWidthStr::width(*label) + 2;
            let mut prefix = plain_span(if label_row {
                format!("{label}: ")
            } else {
                format!(
                    "{:>width$} ",
                    format!("0.{}.", category + 1),
                    width = prefix_width - 1
                )
            });
            prefix.highlighted = !label_row
                && (self.pending_shortcut() == Some(PendingShortcut::ExportCategory)
                    || self.pending_shortcut() == Some(PendingShortcut::ExportOption(category)));
            spans.push(prefix);
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
            if page_start >= options.len() {
                continue;
            }
            if !spans.is_empty() {
                spans.push(plain_span(GAP.to_string()));
            }
            let prefix_width = UnicodeWidthStr::width(layout.labels[category]) + 2;
            let mut prefix = plain_span(if label_row {
                if page == 0 {
                    format!("{}: ", layout.labels[category])
                } else {
                    " ".repeat(prefix_width)
                }
            } else {
                let path = if options.len() <= OPTIONS_PER_PAGE {
                    format!("{}.", category + 2)
                } else {
                    format!("{}.{}.", category + 2, page + 1)
                };
                format!("{path:>width$} ", width = prefix_width - 1)
            });
            prefix.highlighted = (matches!(
                self.pending_shortcut(),
                Some(PendingShortcut::Category(pending)) if pending == category
            ) || matches!(
                self.pending_shortcut(),
                Some(PendingShortcut::Option {
                    category: pending_category,
                    page: pending_page,
                }) if pending_category == category && pending_page == page
            )) && !label_row;
            spans.push(prefix);

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
        }
        spans
    }

    pub fn tooltip(&self) -> &'static str {
        if self.export_open {
            return "PNG is deferred; clipboard/save PNG will use an Egui canvas-only screenshot";
        }
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
    use super::*;

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
    fn boxed_toolbar_height_tracks_actual_menu_rows_and_compact_tooltip_gap() {
        let mut toolbar = ToolbarState::default();
        assert_eq!(toolbar_content_row(0), 1);
        assert_eq!(toolbar.menu_row_count(), 2);
        assert_eq!(toolbar.tooltip_row(), 5);
        assert_eq!(toolbar.rows(), 8);
        assert_eq!(
            toolbar_content_row(toolbar.tooltip_row()),
            toolbar.rows() - 2
        );
        assert_eq!(toolbar_height(&toolbar, 18), toolbar.rows() * 18);

        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Stamp));
        assert_eq!(toolbar.menu_row_count(), 4);
        assert_eq!(toolbar.tooltip_row(), 7);
        assert_eq!(toolbar.rows(), 10);
        assert_eq!(
            toolbar.tooltip_row() - (MENU_FIRST_ROW + toolbar.menu_row_count() - 1) - 1,
            TOOLTIP_GAP_ROWS
        );

        for mode in [MainMode::Shapes, MainMode::Utilities] {
            toolbar.apply_action(ToolbarAction::SelectMain(mode));
            assert_eq!(toolbar.menu_row_count(), 2);
            assert_eq!(toolbar.rows(), 8);
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
        for key in ["1", "2", "2", "1", "0"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.main_mode(), MainMode::Stamp);
        assert_eq!(toolbar.stamp(), "★");

        for key in ["2", "2", "4"] {
            press(&mut toolbar, key);
        }
        assert_eq!(toolbar.stamp(), "•");
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
    fn stamp_pages_keep_every_existing_symbol_visible() {
        let mut toolbar = ToolbarState::default();
        for key in ["1", "2"] {
            press(&mut toolbar, key);
        }
        assert!(row(&toolbar, 2).starts_with("Decorators: ○ ● ◇ ◆ □ ■ △ ▲ ☆ ★"));
        assert!(row(&toolbar, 2).contains("Fills: ░ ▒ ▓ █ ▁ ▂ ▃ ▄ ▅ ▆"));
        assert!(row(&toolbar, 2).contains("Blocks: ▘ ▝ ▀ ▖ ▌ ▞ ▛ ▗ ▚ ▐"));
        assert!(row(&toolbar, 4).contains("            + × ※ •"));
        assert!(row(&toolbar, 4).contains("       ▇ ▀ ▌ ▐ ▊ ▉"));
        assert!(row(&toolbar, 4).contains("        ▜ ▄ ▙ ▟ █"));
        assert!(row(&toolbar, 5).contains("2.2. 1 2 3 4"));
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
        assert_eq!(toolbar.stamp(), "◇");

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
    fn every_stamp_symbol_is_one_utf8_character_and_one_display_cell() {
        for symbol in STAMP_OPTIONS.into_iter().flatten() {
            assert_eq!(symbol.chars().count(), 1, "{symbol:?}");
            assert_eq!(UnicodeWidthStr::width(*symbol), 1, "{symbol:?}");
        }
    }

    #[test]
    fn block_stamps_match_uniline_quadrant_combinations() {
        assert_eq!(
            STAMP_OPTIONS[2].iter().copied().collect::<String>(),
            "▘▝▀▖▌▞▛▗▚▐▜▄▙▟█"
        );
    }

    #[test]
    fn unrelated_modified_keys_are_not_toolbar_shortcuts() {
        let mut toolbar = ToolbarState::default();
        assert!(!toolbar.handle_shortcut(&Key::Character("2".into()), ModifiersState::ALT));
        assert_eq!(toolbar.main_mode(), MainMode::Line);
    }
}
