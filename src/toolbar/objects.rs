//! The Objects menu: `2` Dfn runs directly; `3` Edt and `4` Anchr open a
//! path whose digits pick the option.

use super::*;
use crate::objects::AnchorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectCommand {
    Define,
    DefineEdit,
    Edit,
    Reset,
    Anchor(AnchorKind),
}

const DEFINE_DIGIT: usize = 2;
const EDIT_DIGIT: usize = 3;
const ANCHOR_DIGIT: usize = 4;
const EDIT_OPTIONS: [(&str, ObjectCommand); 3] = [
    ("Dfn", ObjectCommand::DefineEdit),
    ("Lcl", ObjectCommand::Edit),
    ("Res", ObjectCommand::Reset),
];
pub(super) const OBJECT_MENU_ROWS: usize = 2;
/// Every anchor hotkey takes the width of the widest label, so they are
/// evenly spaced.
const ANCHOR_CELL_WIDTH: usize = 2;

/// Menu state the editor owns and the toolbar only displays.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ObjectMenuState {
    pub define_enabled: bool,
    pub define_edit_enabled: bool,
    pub edit_enabled: bool,
    pub reset_enabled: bool,
    pub anchor_enabled: bool,
    pub define_edit_active: bool,
    pub edit_active: bool,
}

impl ObjectMenuState {
    pub fn enabled(self, command: ObjectCommand) -> bool {
        match command {
            ObjectCommand::Define => self.define_enabled,
            ObjectCommand::DefineEdit => self.define_edit_enabled,
            ObjectCommand::Edit => self.edit_enabled,
            ObjectCommand::Reset => self.reset_enabled,
            ObjectCommand::Anchor(_) => self.anchor_enabled,
        }
    }

    fn active(self, command: ObjectCommand) -> bool {
        match command {
            ObjectCommand::DefineEdit => self.define_edit_active,
            ObjectCommand::Edit => self.edit_active,
            ObjectCommand::Define | ObjectCommand::Reset | ObjectCommand::Anchor(_) => false,
        }
    }
}

struct PathGroup {
    label: &'static str,
    digit: usize,
    pending: PendingShortcut,
    options: Vec<(String, ObjectCommand)>,
    cell_width: usize,
}

impl ToolbarState {
    /// Resolves an Objects digit pressed without a pending path.
    pub(super) fn handle_object_digit(&mut self, digit: usize) -> bool {
        match digit {
            DEFINE_DIGIT => self.pending_object_command = Some(ObjectCommand::Define),
            EDIT_DIGIT => self.shortcut_prefix = Some(PendingShortcut::ObjectEdit),
            ANCHOR_DIGIT => self.shortcut_prefix = Some(PendingShortcut::ObjectAnchor),
            _ => return false,
        }
        true
    }

    pub(super) fn handle_object_edit_digit(&mut self, digit: usize) {
        if let Some((_, command)) = digit
            .checked_sub(1)
            .and_then(|index| EDIT_OPTIONS.get(index))
        {
            self.pending_object_command = Some(*command);
        }
    }

    pub(super) fn handle_object_anchor_digit(&mut self, digit: usize) {
        if let Some(kind) = digit
            .checked_sub(1)
            .and_then(|index| AnchorKind::ALL.get(index))
        {
            self.pending_object_command = Some(ObjectCommand::Anchor(*kind));
        }
    }

    pub fn take_object_command(&mut self) -> Option<ObjectCommand> {
        self.pending_object_command.take()
    }

    pub(super) fn objects_menu_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let header = row == MENU_FIRST_ROW;
        let define = ObjectCommand::Define;
        let label = "Dfn";
        let mut spans = vec![ToolbarSpan {
            contents: if header {
                label.to_owned()
            } else {
                pad_right_to_width(DEFINE_DIGIT.to_string(), UnicodeWidthStr::width(label))
            },
            bold_prefix: if header {
                UnicodeWidthStr::width(label)
            } else {
                0
            },
            selected: false,
            highlighted: false,
            tooltip: !self.object_menu.enabled(define),
            action: Some(ToolbarAction::Object(define)),
            shift_action: None,
            right_aligned: false,
            foreground: None,
        }];
        let groups = [
            PathGroup {
                label: "Edt",
                digit: EDIT_DIGIT,
                pending: PendingShortcut::ObjectEdit,
                options: EDIT_OPTIONS
                    .iter()
                    .map(|(label, command)| ((*label).to_owned(), *command))
                    .collect(),
                cell_width: 0,
            },
            PathGroup {
                label: "Anchr",
                digit: ANCHOR_DIGIT,
                pending: PendingShortcut::ObjectAnchor,
                options: AnchorKind::ALL
                    .iter()
                    .map(|kind| (kind.label().to_owned(), ObjectCommand::Anchor(*kind)))
                    .collect(),
                cell_width: ANCHOR_CELL_WIDTH,
            },
        ];
        for group in groups {
            spans.push(plain_span(GAP.to_owned()));
            self.push_path_group(&mut spans, &group, header);
        }
        spans
    }

    fn push_path_group(&self, spans: &mut Vec<ToolbarSpan>, group: &PathGroup, header: bool) {
        let first = spans.len();
        let path = format!("{}.", group.digit);
        let prefix_width = menu_prefix_width(group.label, [path.as_str()]);
        if header {
            let label = format!("{}:", group.label);
            spans.push(bold_prefix_span(
                pad_right_to_width(label.clone(), prefix_width),
                &label,
            ));
        } else {
            let highlighted =
                (self.shortcut_prefix == Some(group.pending)).then_some(path.as_str());
            push_shortcut_path(spans, &path, prefix_width, highlighted);
        }
        for (index, (label, command)) in group.options.iter().enumerate() {
            if index > 0 {
                spans.push(plain_span(" ".to_owned()));
            }
            let width = group.cell_width.max(UnicodeWidthStr::width(label.as_str()));
            spans.push(ToolbarSpan {
                contents: if header {
                    pad_right_to_width(label.clone(), width)
                } else {
                    pad_right_to_width((index + 1).to_string(), width)
                },
                bold_prefix: 0,
                selected: header && self.object_menu.active(*command),
                highlighted: false,
                tooltip: !self.object_menu.enabled(*command),
                action: Some(ToolbarAction::Object(*command)),
                shift_action: None,
                right_aligned: false,
                foreground: None,
            });
        }
        let dimmed = group
            .options
            .iter()
            .all(|(_, command)| !self.object_menu.enabled(*command));
        if dimmed {
            for span in spans[first..]
                .iter_mut()
                .filter(|span| span.action.is_none() && span.bold_prefix > 0)
            {
                span.tooltip = true;
            }
        }
    }

    pub fn set_object_menu(&mut self, state: ObjectMenuState) {
        self.object_menu = state;
    }
}
