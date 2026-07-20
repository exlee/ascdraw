use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::editor::Editor;
use crate::history::HistoryGroup;
use crate::input::{EditCommand, OrderedModifierTracker, edit_command, ordered_direction_command};
use crate::model::Coord;
use crate::toolbar::MainMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangePolicy {
    Navigation { command: EditCommand, steps: usize },
    Edit,
    GroupedEdit(HistoryGroup),
}

pub fn history_group_for_key(
    state: &Editor,
    key: &Key,
    modifiers: ModifiersState,
    ordered_modifiers: &OrderedModifierTracker,
) -> Option<HistoryGroup> {
    if state.has_line_preview()
        || (state.toolbar.main_mode() == MainMode::Line
            && modifiers == ModifiersState::empty()
            && matches!(key, Key::Named(NamedKey::Space)))
    {
        return Some(HistoryGroup::LineRoute);
    }
    if state.cursor_mode.accepts_text() {
        return Some(HistoryGroup::TextSession);
    }
    ordered_direction_command(key, modifiers, ordered_modifiers, state.cursor_mode)
        .is_some_and(|command| {
            matches!(
                command.command,
                EditCommand::Draw(_) | EditCommand::DrawStamp(_)
            )
        })
        .then_some(HistoryGroup::ControlStroke)
}

pub fn change_policy_for_key(
    state: &Editor,
    key: &Key,
    repeat: bool,
    modifiers: ModifiersState,
    ordered_modifiers: &OrderedModifierTracker,
) -> ChangePolicy {
    if state.jump_active() || state.move_lift_active() {
        return ChangePolicy::Edit;
    }
    if state.cursor_mode.accepts_text()
        && state.toolbar.pending_shortcut().is_none()
        && let Some(command @ (EditCommand::Move(_) | EditCommand::ExtendSelection(_))) =
            edit_command(key, repeat, modifiers, state.cursor_mode)
    {
        return if matches!(command, EditCommand::ExtendSelection(_)) {
            ChangePolicy::Edit
        } else {
            ChangePolicy::Navigation { command, steps: 1 }
        };
    }
    if let Some(group) = history_group_for_key(state, key, modifiers, ordered_modifiers) {
        return ChangePolicy::GroupedEdit(group);
    }
    if let Some(command) =
        ordered_direction_command(key, modifiers, ordered_modifiers, state.cursor_mode)
        && matches!(
            command.command,
            EditCommand::Move(_) | EditCommand::ExtendSelection(_)
        )
    {
        return ChangePolicy::Navigation {
            command: command.command,
            steps: command.steps,
        };
    }
    if state.toolbar.pending_shortcut().is_none()
        && let Some(command @ (EditCommand::Move(_) | EditCommand::ExtendSelection(_))) =
            edit_command(key, repeat, modifiers, state.cursor_mode)
    {
        return ChangePolicy::Navigation { command, steps: 1 };
    }
    ChangePolicy::Edit
}

pub fn navigation_target(state: &Editor, command: EditCommand, steps: usize) -> Option<Coord> {
    let direction = match command {
        EditCommand::Move(direction) | EditCommand::ExtendSelection(direction) => direction,
        _ => return None,
    };
    state.navigation_target(direction, steps)
}
