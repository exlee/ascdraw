use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Instant;

use anyhow::{Result, bail};
use ascdraw::automation_protocol::{AutomationCommand, AutomationResponse, KeyModifiers};
use serde_json::json;
use winit::event::MouseScrollDelta;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::WindowId;

use crate::app::{AppConfig, AutomationEnvelope};
use crate::editor_event::{KeyInput, classify_key};
use crate::history::HistoryGroup;
use crate::input::{EditCommand, HistoryCommand, edit_command, view_command};
use crate::perf::FrameTiming;
use crate::runtime::input_dispatch::{ChangePolicy, change_policy_for_key, navigation_target};
use crate::runtime::window::EditorWindow;
use crate::{
    apply_navigation_command, dispatch_editor_event, jump_mode_handles_key,
    selection_action as editor_selection_action,
};

pub struct PendingAutomation {
    id: u64,
    started: Instant,
    handled: Instant,
    response: Sender<AutomationResponse>,
}

pub type PendingAutomationMap = HashMap<WindowId, Vec<PendingAutomation>>;

pub fn handle_automation(
    envelope: AutomationEnvelope,
    windows: &mut HashMap<WindowId, EditorWindow>,
    config: &AppConfig,
    frame_sequences: &HashMap<WindowId, u64>,
    pending: &mut PendingAutomationMap,
) -> bool {
    let AutomationEnvelope { request, response } = envelope;
    let id = request.id;
    if matches!(request.command, AutomationCommand::Shutdown) {
        let _ = response.send(AutomationResponse::success(
            id,
            json!({"status": "shutting_down"}),
        ));
        return true;
    }
    if matches!(request.command, AutomationCommand::Ping) {
        let _ = response.send(AutomationResponse::success(
            id,
            json!({"status": "pong", "ready": !windows.is_empty()}),
        ));
        return false;
    }

    let Some(window_id) = windows.keys().next().copied() else {
        let _ = response.send(AutomationResponse::error(id, "no editor window is ready"));
        return false;
    };
    let editor = windows
        .get_mut(&window_id)
        .expect("selected automation window exists");
    let started = Instant::now();
    let outcome = match request.command {
        AutomationCommand::Key {
            key,
            modifiers,
            repeat,
            count,
        } => {
            if count > 10_000 {
                Err(anyhow::anyhow!("key count exceeds 10000"))
            } else {
                apply_keys(editor, &key, modifiers, repeat, count, config).map(|()| true)
            }
        }
        AutomationCommand::Text { text } => apply_text(editor, &text).map(|()| true),
        AutomationCommand::Scroll { x, y, steps } => {
            apply_scroll(editor, x, y, steps).map(|()| true)
        }
        AutomationCommand::Zoom { delta } => {
            if !delta.is_finite() {
                Err(anyhow::anyhow!("zoom delta must be finite"))
            } else {
                Ok(editor.zoom_canvas_by(delta))
            }
        }
        AutomationCommand::State => {
            let mut state = editor.automation_state();
            state["frame_sequence"] = json!(frame_sequences.get(&window_id).copied().unwrap_or(0));
            let _ = response.send(AutomationResponse::success(id, state));
            return false;
        }
        AutomationCommand::Metrics { reset } => {
            let snapshot = editor.perf_snapshot(reset);
            let result = serde_json::to_value(snapshot)
                .map(|mut value| {
                    value["frame_sequence"] =
                        json!(frame_sequences.get(&window_id).copied().unwrap_or(0));
                    value
                })
                .map_err(anyhow::Error::from);
            send_result(id, response, result);
            return false;
        }
        AutomationCommand::Screenshot { path } => {
            let result = editor.capture_canvas(&path, config).map(|(width, height)| {
                json!({"path": path, "width": width, "height": height, "kind": "canvas"})
            });
            send_result(id, response, result);
            return false;
        }
        AutomationCommand::Ping | AutomationCommand::Shutdown => unreachable!(),
    };

    match outcome {
        Ok(true) => {
            let handled = Instant::now();
            editor.request_redraw();
            pending
                .entry(window_id)
                .or_default()
                .push(PendingAutomation {
                    id,
                    started,
                    handled,
                    response,
                });
        }
        Ok(false) => {
            let _ = response.send(AutomationResponse::success(id, json!({"changed": false})));
        }
        Err(error) => {
            let _ = response.send(AutomationResponse::error(id, format!("{error:#}")));
        }
    }
    false
}

pub fn complete_present(
    pending: &mut PendingAutomationMap,
    window_id: WindowId,
    frame_sequence: u64,
    timing: FrameTiming,
    presented: Instant,
) {
    let Some(commands) = pending.remove(&window_id) else {
        return;
    };
    for command in commands {
        let result = json!({
            "frame_sequence": frame_sequence,
            "timing": {
                "handling_ms": command.handled.saturating_duration_since(command.started).as_secs_f64() * 1_000.0,
                "buffer_ms": timing.buffer_acquisition.as_secs_f64() * 1_000.0,
                "raster_ms": timing.rasterization.as_secs_f64() * 1_000.0,
                "present_submit_ms": timing.presentation.as_secs_f64() * 1_000.0,
                "frame_cpu_ms": timing.total().as_secs_f64() * 1_000.0,
                "event_to_submit_ms": presented.saturating_duration_since(command.started).as_secs_f64() * 1_000.0,
            }
        });
        let _ = command
            .response
            .send(AutomationResponse::success(command.id, result));
    }
}

pub fn fail_pending_for_window(
    pending: &mut PendingAutomationMap,
    window_id: WindowId,
    error: &str,
) {
    if let Some(commands) = pending.remove(&window_id) {
        for command in commands {
            let _ = command
                .response
                .send(AutomationResponse::error(command.id, error));
        }
    }
}

pub fn fail_all_pending(pending: &mut PendingAutomationMap, error: &str) {
    for (_, commands) in pending.drain() {
        for command in commands {
            let _ = command
                .response
                .send(AutomationResponse::error(command.id, error));
        }
    }
}

fn send_result(id: u64, sender: Sender<AutomationResponse>, result: Result<serde_json::Value>) {
    let response = match result {
        Ok(result) => AutomationResponse::success(id, result),
        Err(error) => AutomationResponse::error(id, format!("{error:#}")),
    };
    let _ = sender.send(response);
}

fn apply_keys(
    editor: &mut EditorWindow,
    key: &str,
    modifiers: KeyModifiers,
    repeat: bool,
    count: u32,
    config: &AppConfig,
) -> Result<()> {
    let key = parse_key(key)?;
    let modifiers = modifiers_state(modifiers);
    let previous_modifiers = editor.modifiers;
    let previous_ordered_modifiers = editor.ordered_modifiers.clone();
    editor.modifiers = modifiers;
    editor.ordered_modifiers.update(modifiers);
    let result = (0..count)
        .try_for_each(|index| apply_key(editor, &key, modifiers, repeat || index > 0, config));
    editor.modifiers = previous_modifiers;
    editor.ordered_modifiers = previous_ordered_modifiers;
    result
}

fn apply_key(
    editor: &mut EditorWindow,
    key: &Key,
    modifiers: ModifiersState,
    repeat: bool,
    config: &AppConfig,
) -> Result<()> {
    let started = Instant::now();
    editor.cancel_scroll_pan();
    editor.note_keypress(started);
    let text = match key {
        Key::Character(text) => Some(text.as_str()),
        _ => None,
    };
    let key_type = classify_key(
        editor.state.state(),
        editor.state.cursor_mode.accepts_text(),
        KeyInput {
            key,
            text,
            repeat,
            modifiers,
        },
    );
    if jump_mode_handles_key(&editor.state, key_type) || key_type.is_cancel() {
        let previous_state = editor.state.clone();
        let previous_viewport = editor.viewport;
        let visible = editor.visible_canvas_cells();
        dispatch_editor_event(
            &mut editor.state,
            key_type,
            &editor.ordered_modifiers,
            visible,
            config.jump.inactivity(),
            started,
        );
        editor.apply_jump_viewport_pan();
        editor.finish_state_change(previous_state, previous_viewport, false);
    } else if let Some(command) = key_type.history_command() {
        match command {
            HistoryCommand::Undo => {
                editor.undo();
            }
            HistoryCommand::Redo => {
                editor.redo();
            }
        }
    } else if key_type.clipboard_command().is_some() {
        bail!("clipboard key combinations are unavailable through automation");
    } else if editor.state.toolbar.pending_shortcut().is_none()
        && let Some(command) = view_command(
            key,
            modifiers,
            editor.state.cursor_mode,
            editor.state.toolbar.utility_kind(),
        )
    {
        editor.state.end_stroke();
        editor.finish_history_transaction();
        editor.state.toolbar.cancel_shortcut();
        editor.apply_view_command(command);
    } else {
        apply_editor_key(editor, key, key_type, modifiers, repeat, config, started);
    }
    editor.finish_keypress(Instant::now());
    Ok(())
}

fn apply_editor_key(
    editor: &mut EditorWindow,
    key: &Key,
    key_type: crate::editor_event::KeyType<'_>,
    modifiers: ModifiersState,
    repeat: bool,
    config: &AppConfig,
    started: Instant,
) {
    let state_history_started = Instant::now();
    let policy = change_policy_for_key(
        &editor.state,
        key,
        repeat,
        modifiers,
        &editor.ordered_modifiers,
    );
    if let ChangePolicy::Navigation { command, steps } = policy
        && let Some(target) = navigation_target(&editor.state, command, steps)
        && let Some(origin) = editor.navigation_origin_for(target)
    {
        editor.finish_history_transaction();
        apply_navigation_command(&mut editor.state, command, steps);
        editor.finish_navigation(origin);
        editor.record_state_history_time(state_history_started);
        return;
    }

    let history_group = match policy {
        ChangePolicy::GroupedEdit(group) => Some(group),
        ChangePolicy::Navigation { .. } | ChangePolicy::Edit => None,
    };
    let previous_state = editor.state.clone();
    let previous_viewport = editor.viewport;
    let clears_selection = editor_selection_action(editor.state.state(), key_type)
        == Some(crate::editor_event::SelectionAction::Clear)
        || matches!(
            edit_command(key, repeat, modifiers, editor.state.cursor_mode),
            Some(EditCommand::Clear | EditCommand::ClearAndBack)
        );
    let visible = editor.visible_canvas_cells();
    if let Some(document_changed) = dispatch_editor_event(
        &mut editor.state,
        key_type,
        &editor.ordered_modifiers,
        visible,
        config.jump.inactivity(),
        started,
    ) {
        let viewport_stable = editor.state.take_toolbar_viewport_stable();
        if history_group.is_none() {
            editor.state.end_stroke();
        }
        let changed = match history_group {
            Some(group) => editor.finish_grouped_state_change(
                previous_state,
                previous_viewport,
                document_changed,
                group,
            ),
            None if document_changed && clears_selection => {
                editor.finish_selection_clear(previous_state, previous_viewport)
            }
            None if viewport_stable => editor.finish_state_change_with_stable_viewport(
                previous_state,
                previous_viewport,
                document_changed,
            ),
            None => editor.finish_state_change(previous_state, previous_viewport, document_changed),
        };
        if changed {
            editor.mark_document_dirty();
        }
        if history_group == Some(HistoryGroup::TextSession)
            && !editor.state.cursor_mode.accepts_text()
        {
            editor.finish_history_transaction();
        }
        if history_group == Some(HistoryGroup::LineRoute) && !editor.state.has_line_preview() {
            editor.finish_history_transaction();
        }
    }
    editor.record_state_history_time(state_history_started);
}

fn apply_text(editor: &mut EditorWindow, text: &str) -> Result<()> {
    if !editor.state.cursor_mode.accepts_text() {
        bail!("text input requires a text or replace mode");
    }
    let started = Instant::now();
    editor.note_keypress(started);
    let previous_state = editor.state.clone();
    let previous_viewport = editor.viewport;
    editor.state.write_text(text);
    if editor.finish_grouped_state_change(
        previous_state,
        previous_viewport,
        true,
        HistoryGroup::TextSession,
    ) {
        editor.mark_document_dirty();
    }
    editor.finish_keypress(Instant::now());
    Ok(())
}

fn apply_scroll(editor: &mut EditorWindow, x: f32, y: f32, steps: u32) -> Result<()> {
    if !x.is_finite() || !y.is_finite() {
        bail!("scroll deltas must be finite");
    }
    if steps > 10_000 {
        bail!("scroll steps exceeds 10000");
    }
    editor.note_keypress(Instant::now());
    for _ in 0..steps {
        editor.note_scroll_event();
        editor.queue_scroll_pan(MouseScrollDelta::LineDelta(x, y));
    }
    editor.finish_keypress(Instant::now());
    Ok(())
}

fn modifiers_state(modifiers: KeyModifiers) -> ModifiersState {
    let mut state = ModifiersState::empty();
    state.set(ModifiersState::SHIFT, modifiers.shift);
    state.set(ModifiersState::CONTROL, modifiers.control);
    state.set(ModifiersState::ALT, modifiers.alt);
    state.set(ModifiersState::SUPER, modifiers.super_key);
    state
}

fn parse_key(value: &str) -> Result<Key> {
    let named = match value.to_ascii_lowercase().as_str() {
        "arrowup" | "up" => Some(NamedKey::ArrowUp),
        "arrowright" | "right" => Some(NamedKey::ArrowRight),
        "arrowdown" | "down" => Some(NamedKey::ArrowDown),
        "arrowleft" | "left" => Some(NamedKey::ArrowLeft),
        "escape" | "esc" => Some(NamedKey::Escape),
        "enter" | "return" => Some(NamedKey::Enter),
        "space" => Some(NamedKey::Space),
        "backspace" => Some(NamedKey::Backspace),
        "delete" => Some(NamedKey::Delete),
        "tab" => Some(NamedKey::Tab),
        "home" => Some(NamedKey::Home),
        "end" => Some(NamedKey::End),
        "pageup" => Some(NamedKey::PageUp),
        "pagedown" => Some(NamedKey::PageDown),
        _ => None,
    };
    if let Some(named) = named {
        return Ok(Key::Named(named));
    }
    if value.is_empty() {
        bail!("key must not be empty");
    }
    Ok(Key::Character(value.into()))
}

#[cfg(test)]
mod tests {
    use winit::keyboard::{Key, NamedKey};

    use super::parse_key;

    #[test]
    fn automation_keys_accept_names_aliases_and_characters() {
        assert_eq!(parse_key("left").unwrap(), Key::Named(NamedKey::ArrowLeft));
        assert_eq!(
            parse_key("ArrowLeft").unwrap(),
            Key::Named(NamedKey::ArrowLeft)
        );
        assert_eq!(parse_key("x").unwrap(), Key::Character("x".into()));
        assert!(parse_key("").is_err());
    }
}
