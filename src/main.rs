use std::collections::HashMap;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use winit::event::{ElementState, Event, Ime, MouseButton, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState};
use winit::window::WindowId;

mod app;
mod diagnostics;
mod document;
mod drawing;
mod editor;
mod export;
mod face_resolution;
mod history;
mod icon;
mod input;
mod layout;
#[cfg(target_os = "macos")]
mod macos;
mod model;
mod perf;
mod render;
mod runtime;
pub mod selection;
mod title_policy;
mod toolbar;
mod user_keys;

use app::{
    AppCommand, AppEvent, Args, checked_config_paths, load_config, show_config_toml,
    user_config_path,
};
use diagnostics::log_error;
use editor::EditorState;
use export::{ExportOutcome, NativeExportPlatform};
use history::HistoryGroup;
use input::{
    ClipboardCommand, EditCommand, HistoryCommand, clipboard_command, edit_command,
    history_command, line_preview_command, move_selection_command, ordered_direction_command,
    pointer_position_to_coord, pointer_position_to_toolbar_position, view_command,
};
use runtime::config_watch::{UserConfigWatch, poll_user_config_updates};
#[cfg(test)]
use runtime::input_dispatch::history_group_for_key;
use runtime::input_dispatch::{ChangePolicy, change_policy_for_key, navigation_target};
use runtime::window::{
    EditorWindow, close_window, create_editor_window, handle_command, save_windows_on_exit,
};
use user_keys::{FontSizeAction, UserAction, UserKeys};

fn main() -> ExitCode {
    if let Err(error) = diagnostics::init() {
        eprintln!("diagnostics setup failed: {error:#}");
    }
    diagnostics::install_panic_hook();

    match try_main() {
        Ok(code) => code,
        Err(error) => {
            log_error(format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

#[allow(deprecated)]
fn try_main() -> Result<ExitCode> {
    let args = Args::parse();
    let document_path = args.document.unwrap_or_else(document::default_path);
    let mut config = load_config()?;
    if args.show_config {
        println!("Checked configuration paths:");
        for path in checked_config_paths() {
            println!("  {}", path.display());
        }
        println!("\nCurrent configuration:\n{}", show_config_toml(&config)?);
        return Ok(ExitCode::SUCCESS);
    }

    let mut user_keys = UserKeys::from_config(&config.keys)?;
    let mut user_config_watch = user_config_path().map(UserConfigWatch::new);
    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();

    #[cfg(target_os = "macos")]
    if let Err(error) = macos::install(proxy) {
        log_error(format!("macOS integration setup failed: {error:#}"));
    }

    let mut windows: HashMap<WindowId, EditorWindow> = HashMap::new();
    let mut last_autosave_check = Instant::now();
    let mut last_tooltip_redraw = Instant::now();
    #[cfg(target_os = "macos")]
    let mut installed_macos_menus = false;
    #[cfg(target_os = "macos")]
    let mut should_apply_app_icon = true;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(500),
        ));

        match event {
            Event::Resumed => {
                #[cfg(target_os = "macos")]
                if !installed_macos_menus {
                    if let Err(error) = macos::install_menus() {
                        log_error(format!("macOS menu setup failed: {error:#}"));
                    }
                    installed_macos_menus = true;
                }

                if windows.is_empty() {
                    match create_editor_window(elwt, &config, &document_path) {
                        Ok(editor) => {
                            windows.insert(editor.window_id(), editor);
                        }
                        Err(error) => {
                            log_error(format!("window creation failed: {error:#}"));
                            elwt.exit();
                        }
                    }
                } else {
                    for editor in windows.values() {
                        editor.request_redraw();
                    }
                }
            }
            Event::UserEvent(AppEvent::Command(command)) => {
                handle_command(command, None, &mut windows, elwt, &config, &document_path);
            }
            Event::AboutToWait => {
                if let Some(watch) = user_config_watch.as_mut() {
                    poll_user_config_updates(watch, &mut config, &mut user_keys, &mut windows);
                }
                let now = Instant::now();
                for editor in windows.values_mut() {
                    if editor.state.move_lift_active() {
                        editor.request_redraw();
                    }
                    #[cfg(target_os = "macos")]
                    editor.hide_cursor_if_idle(now);
                    editor.clear_export_success_if_elapsed(now);
                }
                if now.saturating_duration_since(last_tooltip_redraw)
                    >= toolbar::TOOLTIP_ROTATION_INTERVAL
                {
                    for editor in windows.values() {
                        editor.request_redraw();
                    }
                    last_tooltip_redraw = now;
                }
                if now.saturating_duration_since(last_autosave_check) >= Duration::from_secs(1) {
                    for editor in windows.values_mut() {
                        if let Err(error) = editor.autosave_if_idle(now) {
                            log_error(format!("document autosave failed: {error:#}"));
                        }
                    }
                    last_autosave_check = now;
                }
            }
            Event::WindowEvent { window_id, event } => {
                let mut should_close = false;
                let mut pending_command = None;
                if let Some(editor) = windows.get_mut(&window_id) {
                    match event {
                        WindowEvent::CloseRequested => should_close = true,
                        WindowEvent::Resized(size) => {
                            if let Err(error) = editor.surface.resize(&editor.window, size) {
                                log_error(format!("surface resize failed: {error:#}"));
                                should_close = true;
                            }
                            editor.ensure_cursor_in_viewport();
                            editor.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            match editor.surface.render(
                                &editor.window,
                                &editor.state,
                                &editor.renderer,
                                &config,
                                editor.viewport,
                            ) {
                                Err(error) => {
                                    log_error(format!("render failed: {error:#}"));
                                    should_close = true;
                                }
                                Ok(timing) => {
                                    editor.record_present(timing, Instant::now());
                                    #[cfg(target_os = "macos")]
                                    if should_apply_app_icon {
                                        should_apply_app_icon = false;
                                        if let Err(error) = icon::apply_app_icon() {
                                            log_error(format!("app icon setup failed: {error:#}"));
                                        }
                                    }
                                }
                            }
                        }
                        WindowEvent::ModifiersChanged(modifiers) => {
                            let released_shift =
                                editor.modifiers.shift_key() && !modifiers.state().shift_key();
                            editor.modifiers = modifiers.state();
                            editor.ordered_modifiers.update(editor.modifiers);
                            if released_shift {
                                editor.state.end_stroke();
                                editor.finish_history_transaction();
                                editor.request_redraw();
                            }
                        }
                        WindowEvent::Focused(false) => {
                            editor.state.end_stroke();
                            editor.finish_history_transaction();
                            editor.modifiers = ModifiersState::empty();
                            editor.ordered_modifiers.update(editor.modifiers);
                            editor.request_redraw();
                        }
                        WindowEvent::Ime(Ime::Commit(text)) => {
                            if !text.is_empty()
                                && editor.state.cursor_mode.accepts_text()
                                && !editor.modifiers.control_key()
                                && !editor.modifiers.alt_key()
                                && !editor.modifiers.super_key()
                            {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                editor.state.write_text(&text);
                                if editor.finish_grouped_state_change(
                                    previous_state,
                                    previous_viewport,
                                    true,
                                    HistoryGroup::TextSession,
                                ) {
                                    editor.mark_document_dirty();
                                }
                                if !editor.state.cursor_mode.accepts_text() {
                                    editor.finish_history_transaction();
                                }
                                editor.request_redraw();
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. }
                            if event.state == ElementState::Pressed =>
                        {
                            editor.note_keypress(Instant::now());
                            if let Some(command) = history_command(
                                &event.logical_key,
                                editor.modifiers,
                                editor.state.cursor_mode,
                            ) {
                                match command {
                                    HistoryCommand::Undo => {
                                        editor.undo();
                                    }
                                    HistoryCommand::Redo => {
                                        editor.redo();
                                    }
                                }
                            } else if clipboard_command(&event.logical_key, editor.modifiers)
                                .is_some()
                            {
                                editor.state.end_stroke();
                                editor.finish_history_transaction();
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                let mut platform = NativeExportPlatform::text_only();
                                let result = handle_clipboard_shortcut(
                                    &mut editor.state,
                                    &event.logical_key,
                                    editor.modifiers,
                                    &mut platform,
                                )
                                .expect("clipboard shortcut was already recognized");
                                match result {
                                    Ok(true) => {
                                        if editor.finish_state_change(
                                            previous_state,
                                            previous_viewport,
                                            true,
                                        ) {
                                            editor.mark_document_dirty();
                                        }
                                    }
                                    Ok(false) => {}
                                    Err(error) => {
                                        editor.state = previous_state;
                                        editor.viewport = previous_viewport;
                                        log_error(format!("Clipboard operation failed: {error:#}"));
                                    }
                                }
                                editor.request_redraw();
                            } else if edit_command(
                                &event.logical_key,
                                event.repeat,
                                editor.modifiers,
                                editor.state.cursor_mode,
                            ) == Some(EditCommand::CancelTextEntry)
                            {
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                editor.state.cancel_text_entry();
                                editor.finish_state_change(
                                    previous_state,
                                    previous_viewport,
                                    false,
                                );
                                editor.request_redraw();
                            } else if let Some(action) =
                                user_keys.action_for_event(&event, editor.modifiers)
                            {
                                editor.state.end_stroke();
                                editor.finish_history_transaction();
                                pending_command = Some(app_command_from_user_action(action));
                            } else if editor.state.toolbar.pending_shortcut().is_none()
                                && let Some(command) = view_command(
                                    &event.logical_key,
                                    editor.modifiers,
                                    editor.state.cursor_mode,
                                    editor.state.toolbar.utility_kind(),
                                )
                            {
                                editor.state.end_stroke();
                                editor.finish_history_transaction();
                                editor.state.toolbar.cancel_shortcut();
                                editor.apply_view_command(command);
                            } else {
                                let state_history_started = Instant::now();
                                let policy = change_policy_for_key(
                                    &editor.state,
                                    &event.logical_key,
                                    event.repeat,
                                    editor.modifiers,
                                    &editor.ordered_modifiers,
                                );
                                let handled_navigation =
                                    if let ChangePolicy::Navigation { command, steps } = policy
                                        && let Some(target) =
                                            navigation_target(&editor.state, command, steps)
                                        && let Some(origin) = editor.navigation_origin_for(target)
                                    {
                                        editor.finish_history_transaction();
                                        apply_navigation_command(&mut editor.state, command, steps);
                                        editor.finish_navigation(origin);
                                        editor.request_redraw();
                                        perform_pending_export(editor, &config);
                                        true
                                    } else {
                                        false
                                    };
                                if !handled_navigation {
                                    let history_group = match policy {
                                        ChangePolicy::GroupedEdit(group) => Some(group),
                                        ChangePolicy::Navigation { .. } | ChangePolicy::Edit => {
                                            None
                                        }
                                    };
                                    let previous_state = editor.state.clone();
                                    let previous_viewport = editor.viewport;
                                    let handled = handle_editor_key_with_order(
                                        &mut editor.state,
                                        &event.logical_key,
                                        event.text.as_deref(),
                                        event.repeat,
                                        editor.modifiers,
                                        &editor.ordered_modifiers,
                                    );
                                    if let Some(document_changed) = handled {
                                        if history_group.is_none() {
                                            editor.state.end_stroke();
                                        }
                                        let document_changed = match history_group {
                                            Some(group) => editor.finish_grouped_state_change(
                                                previous_state,
                                                previous_viewport,
                                                document_changed,
                                                group,
                                            ),
                                            None => editor.finish_state_change(
                                                previous_state,
                                                previous_viewport,
                                                document_changed,
                                            ),
                                        };
                                        if document_changed {
                                            editor.mark_document_dirty();
                                        }
                                        if history_group == Some(HistoryGroup::TextSession)
                                            && !editor.state.cursor_mode.accepts_text()
                                        {
                                            editor.finish_history_transaction();
                                        }
                                        editor.request_redraw();
                                    }
                                    perform_pending_export(editor, &config);
                                }
                                editor.record_state_history_time(state_history_started);
                            }
                            editor.finish_keypress(Instant::now());
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            #[cfg(target_os = "macos")]
                            editor.note_cursor_activity(Instant::now());
                            editor.mouse_toolbar_position = pointer_position_to_toolbar_position(
                                position.x,
                                position.y,
                                editor.window.inner_size().width as usize,
                                &editor.renderer,
                                editor.window.scale_factor(),
                                &config,
                                &editor.state.toolbar,
                            );
                            editor.mouse_cell = pointer_position_to_coord(
                                position.x,
                                position.y,
                                &editor.renderer,
                                editor.window.scale_factor(),
                                &config,
                                &editor.state.toolbar,
                                editor.viewport,
                            );
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: MouseButton::Left,
                            ..
                        } => {
                            let toolbar_action =
                                editor
                                    .mouse_toolbar_position
                                    .and_then(|(row, column, width)| {
                                        editor.state.toolbar.action_at(row, column, width)
                                    });
                            if let Some(action) = toolbar_action {
                                editor.note_keypress(Instant::now());
                                let previous_state = editor.state.clone();
                                let previous_viewport = editor.viewport;
                                editor.state.apply_toolbar_action(action);
                                editor.finish_state_change(
                                    previous_state,
                                    previous_viewport,
                                    false,
                                );
                                perform_pending_export(editor, &config);
                                editor.request_redraw();
                            } else if let Some(coord) = editor.mouse_cell {
                                let target = editor.state.cursor_target_for_coord(coord);
                                if let Some(origin) = editor.navigation_origin_for(target) {
                                    editor.finish_history_transaction();
                                    editor.state.move_to(coord);
                                    editor.finish_navigation(origin);
                                    editor.request_redraw();
                                }
                            }
                        }
                        WindowEvent::ScaleFactorChanged { .. } => {
                            if let Err(error) = editor
                                .surface
                                .resize(&editor.window, editor.window.inner_size())
                            {
                                log_error(format!("surface scale update failed: {error:#}"));
                                should_close = true;
                            }
                            editor.ensure_cursor_in_viewport();
                            editor.request_redraw();
                        }
                        _ => {}
                    }
                }

                if let Some(command) = pending_command {
                    handle_command(
                        command,
                        Some(window_id),
                        &mut windows,
                        elwt,
                        &config,
                        &document_path,
                    );
                } else if should_close {
                    close_window(&mut windows, window_id, elwt);
                }
            }
            Event::LoopExiting => save_windows_on_exit(&mut windows),
            _ => {}
        }
    })?;

    Ok(ExitCode::SUCCESS)
}

fn handle_clipboard_shortcut(
    state: &mut EditorState,
    key: &Key,
    modifiers: ModifiersState,
    platform: &mut impl export::ExportPlatform,
) -> Option<Result<bool>> {
    let command = clipboard_command(key, modifiers)?;
    state.cancel_line_preview();
    state.cancel_move_lift();
    Some(match command {
        ClipboardCommand::Copy => export::copy_selection(state, platform).map(|()| false),
        ClipboardCommand::Cut => export::cut_selection(state, platform),
        ClipboardCommand::Paste => export::paste_selection(state, platform),
    })
}

fn perform_pending_export(editor: &mut EditorWindow, config: &app::AppConfig) {
    let Some(action) = editor.state.toolbar.take_export_action() else {
        return;
    };
    let previous_state = editor.state.clone();
    let previous_viewport = editor.viewport;
    let visible_canvas = editor.visible_canvas_cells();
    let mut platform = NativeExportPlatform::with_png(
        &editor.renderer,
        editor.window.scale_factor(),
        config.macos.color_space,
    );
    let outcome = perform_export_action(
        action,
        &mut editor.state,
        &mut editor.viewport,
        visible_canvas,
        &mut platform,
    );
    match outcome {
        Ok(ExportOutcome::DocumentLoaded) => {
            editor.viewport = layout::ViewportOffset::default();
            if editor.finish_state_change(previous_state, previous_viewport, true) {
                editor.mark_document_dirty();
            }
        }
        Ok(ExportOutcome::ProjectLoaded) => {
            editor.finish_project_load(previous_state, previous_viewport);
        }
        Ok(ExportOutcome::CanvasCleared) => {
            if editor.finish_state_change(previous_state, previous_viewport, true) {
                editor.mark_document_dirty();
            }
        }
        Ok(ExportOutcome::Unchanged) => {}
        Ok(ExportOutcome::Cancelled) => return,
        Err(error) => log_error(format!("Save/Load/Export failed: {error:#}")),
    }
    editor.show_export_success(action, Instant::now());
}

fn perform_export_action(
    action: export::ExportAction,
    state: &mut EditorState,
    viewport: &mut layout::ViewportOffset,
    visible_canvas: layout::VisibleCanvasCells,
    platform: &mut impl export::ExportPlatform,
) -> anyhow::Result<ExportOutcome> {
    let outcome = export::perform(action, state, viewport, visible_canvas, platform);
    // Loading a project restores its durable toolbar selections and therefore
    // resets transient interactions. Export is a peer mode, so re-establish it
    // before any outcome-specific viewport validation or history recording.
    state.toolbar.keep_export_active(action);
    outcome
}

fn app_command_from_user_action(action: UserAction) -> AppCommand {
    match action {
        UserAction::FontSize(FontSizeAction::Increase) => AppCommand::FontScaleUp,
        UserAction::FontSize(FontSizeAction::Decrease) => AppCommand::FontScaleDown,
        UserAction::FontSize(FontSizeAction::Reset) => AppCommand::FontScaleReset,
        UserAction::WindowNew => AppCommand::WindowNew,
        UserAction::WindowClose => AppCommand::WindowClose,
    }
}

fn apply_navigation_command(state: &mut EditorState, command: EditCommand, steps: usize) {
    state.toolbar.cancel_shortcut();
    for _ in 0..steps {
        apply_edit_command(state, command);
    }
}

fn handle_editor_key_with_order(
    state: &mut EditorState,
    key: &Key,
    text: Option<&str>,
    repeat: bool,
    modifiers: ModifiersState,
    ordered_modifiers: &input::OrderedModifierTracker,
) -> Option<bool> {
    if let Some(command) = move_selection_command(
        key,
        modifiers,
        state.cursor_mode,
        state.toolbar.utility_kind(),
        state.move_lift_active(),
        state.move_lift_plain_direction_confirms(),
        !state.selection.is_collapsed(),
    ) {
        state.cancel_line_preview();
        state.toolbar.cancel_shortcut();
        return Some(match command {
            input::MoveSelectionCommand::Begin => {
                state.begin_move_lift();
                false
            }
            input::MoveSelectionCommand::BeginAndStep(direction) => {
                state.begin_selected_move_lift();
                state.move_lift(direction);
                false
            }
            input::MoveSelectionCommand::Step(direction) => {
                state.move_lift(direction);
                false
            }
            input::MoveSelectionCommand::ConfirmAndMove(direction) => {
                let changed = state.confirm_move_lift();
                changed | state.move_cursor(direction)
            }
            input::MoveSelectionCommand::Confirm => state.confirm_move_lift(),
            input::MoveSelectionCommand::Cancel => {
                state.cancel_move_lift();
                false
            }
        });
    }
    if let Some(command) =
        line_preview_command(key, modifiers, state.cursor_mode, state.has_line_preview())
    {
        state.toolbar.cancel_shortcut();
        return Some(match command {
            input::LinePreviewCommand::StartOrAdvance => state.start_or_advance_line_preview(),
            input::LinePreviewCommand::Move(direction) => state.move_line_preview(direction),
            input::LinePreviewCommand::RemoveAnchor => state.remove_line_preview_anchor(),
            input::LinePreviewCommand::Cancel => {
                state.cancel_line_preview();
                false
            }
        });
    }
    if let Some(command) =
        ordered_direction_command(key, modifiers, ordered_modifiers, state.cursor_mode)
    {
        state.cancel_line_preview();
        state.cancel_move_lift();
        state.toolbar.cancel_shortcut();
        let mut document_changed = false;
        for _ in 0..command.steps {
            document_changed |= apply_edit_command(state, command.command);
        }
        if matches!(command.command, EditCommand::ExtendSelection(_)) {
            document_changed = false;
        }
        return Some(document_changed);
    }
    if let Some(command @ (EditCommand::ExtendSelection(_) | EditCommand::Erase(_))) =
        edit_command(key, repeat, modifiers, state.cursor_mode)
    {
        state.cancel_line_preview();
        state.cancel_move_lift();
        state.toolbar.cancel_shortcut();
        return Some(apply_edit_command(state, command));
    }
    if state.handle_toolbar_shortcut(key, modifiers) {
        return Some(false);
    }
    if let Some(command) = edit_command(key, repeat, modifiers, state.cursor_mode) {
        state.cancel_line_preview();
        state.cancel_move_lift();
        return Some(apply_edit_command(state, command));
    }
    if !modifiers.control_key()
        && state.cursor_mode.accepts_text()
        && !modifiers.alt_key()
        && !modifiers.super_key()
        && let Some(text) = text
        && !text.chars().all(char::is_control)
    {
        state.cancel_line_preview();
        state.cancel_move_lift();
        state.write_text(text);
        return Some(true);
    }
    None
}

#[cfg(test)]
fn handle_editor_key(
    state: &mut EditorState,
    key: &Key,
    text: Option<&str>,
    repeat: bool,
    modifiers: ModifiersState,
) -> Option<bool> {
    let mut ordered = input::OrderedModifierTracker::default();
    ordered.update(modifiers);
    handle_editor_key_with_order(state, key, text, repeat, modifiers, &ordered)
}

fn apply_edit_command(state: &mut EditorState, command: EditCommand) -> bool {
    match command {
        EditCommand::Move(direction) => state.move_cursor(direction),
        EditCommand::Draw(direction) => {
            state.move_or_draw(direction, true);
            true
        }
        EditCommand::DrawStamp(direction) => {
            state.draw_stamp(direction);
            true
        }
        EditCommand::ApplyUtility(direction) => state.apply_utility(direction),
        EditCommand::ExtendSelection(direction) => state.extend_selection(direction),
        EditCommand::Erase(direction) => state.erase(direction),
        EditCommand::Clear => {
            state.clear_selection();
            true
        }
        EditCommand::ClearAndBack => {
            state.clear_selection();
            state.move_cursor(model::Direction::Left);
            true
        }
        EditCommand::ToggleTextEntry => {
            state.toggle_text_entry();
            false
        }
        EditCommand::ToggleReplaceMode => {
            state.toggle_replace_mode();
            false
        }
        EditCommand::ConfirmOrTextEntry => {
            if state.has_shape_preview() {
                state.start_shape_or_confirm()
            } else {
                state.toggle_text_entry();
                false
            }
        }
        EditCommand::ConfirmOrReplace => {
            if state.has_shape_preview() {
                state.start_shape_or_confirm()
            } else {
                state.toggle_replace_mode();
                false
            }
        }
        EditCommand::BeginSingleReplace => {
            state.begin_single_replace();
            false
        }
        EditCommand::CancelTextEntry => {
            state.cancel_text_entry();
            false
        }
        EditCommand::PlaceStamp => {
            state.place_stamp();
            true
        }
        EditCommand::StartOrConfirmShape => state.start_shape_or_confirm(),
        EditCommand::Home => {
            state.move_home();
            false
        }
        EditCommand::End => {
            state.move_end();
            false
        }
        EditCommand::Backspace => {
            state.backspace();
            true
        }
        EditCommand::Delete => {
            state.delete();
            true
        }
        EditCommand::Newline => {
            state.newline();
            true
        }
        EditCommand::InsertTab => {
            state.insert("    ");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppConfig, CursorMode};
    use crate::model::{Coord, Direction};
    use crate::toolbar::{MainMode, PendingShortcut, ToolbarAction};
    use winit::keyboard::NamedKey;

    #[derive(Default)]
    struct ClipboardPlatform {
        text: String,
        save: Option<std::path::PathBuf>,
        open: Option<std::path::PathBuf>,
        fail_clipboard_write: bool,
    }

    impl export::ExportPlatform for ClipboardPlatform {
        fn set_clipboard_text(&mut self, text: &str) -> Result<()> {
            if self.fail_clipboard_write {
                anyhow::bail!("mock clipboard write failed");
            }
            self.text = text.to_string();
            Ok(())
        }

        fn clipboard_text(&mut self) -> Result<String> {
            Ok(self.text.clone())
        }

        fn choose_save_path(&mut self, _kind: export::FileKind) -> Option<std::path::PathBuf> {
            self.save.take()
        }

        fn choose_open_path(&mut self, _kind: export::FileKind) -> Option<std::path::PathBuf> {
            self.open.take()
        }

        fn render_canvas_image(
            &mut self,
            lines: &[Vec<crate::model::Atom>],
            default_face: &crate::model::Face,
        ) -> Result<render::CanvasImage> {
            let config = AppConfig::default();
            render::render_canvas_image(
                &render::load_renderer(&config),
                lines,
                default_face,
                1.0,
                config.macos.color_space,
            )
        }

        fn set_clipboard_image(&mut self, _image: &render::CanvasImage) -> Result<()> {
            if self.fail_clipboard_write {
                anyhow::bail!("mock clipboard image write failed");
            }
            Ok(())
        }
    }

    #[test]
    fn line_preview_anchors_orthogonal_segments_and_zero_length_space_commits() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        let before = history::HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: layout::ViewportOffset::default(),
        };

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Space),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert!(state.has_line_preview());
        assert_eq!(state.tooltip(), crate::toolbar::Tooltip::LinePreview);

        for _ in 0..2 {
            assert_eq!(
                handle_editor_key(
                    &mut state,
                    &Key::Named(NamedKey::ArrowRight),
                    None,
                    false,
                    ModifiersState::empty(),
                ),
                Some(false)
            );
        }
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 2 });
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::ArrowDown),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 2 });
        assert!(state.content_cells().is_empty());
        assert_eq!(
            state
                .lines_with_shape_preview()
                .expect("line preview is composited")
                .iter()
                .flatten()
                .filter(|atom| drawing::is_line_glyph(&atom.contents))
                .count(),
            3
        );

        for key in [
            Key::Named(NamedKey::Space),
            Key::Named(NamedKey::ArrowDown),
            Key::Named(NamedKey::ArrowDown),
            Key::Named(NamedKey::Space),
        ] {
            assert_eq!(
                handle_editor_key(&mut state, &key, None, false, ModifiersState::empty(),),
                Some(false)
            );
        }
        assert!(state.content_cells().is_empty());
        assert_eq!(state.edit_snapshot(), before.edit);
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Space),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert!(!state.has_line_preview());
        assert_eq!(
            state.content_cells(),
            [
                Coord { line: 0, column: 0 },
                Coord { line: 0, column: 1 },
                Coord { line: 0, column: 2 },
                Coord { line: 1, column: 2 },
                Coord { line: 2, column: 2 },
            ]
        );

        let after = history::HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: layout::ViewportOffset::default(),
        };
        let mut history = history::EditHistory::default();
        assert!(history.record_change(before.clone(), &after));
        assert_eq!(history.undo(after), Some(before));
    }

    #[test]
    fn zero_length_line_preview_confirms_without_a_document_change() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        let before = state.edit_snapshot();

        for _ in 0..2 {
            assert_eq!(
                handle_editor_key(
                    &mut state,
                    &Key::Named(NamedKey::Space),
                    None,
                    false,
                    ModifiersState::empty(),
                ),
                Some(false)
            );
        }
        assert!(!state.has_line_preview());
        assert_eq!(state.edit_snapshot(), before);
    }

    #[test]
    fn line_preview_origin_prepend_remains_transient_until_commit() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        let before = state.edit_snapshot();

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Space),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::ArrowLeft),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(state.edit_snapshot(), before);
        assert_eq!(state.take_pending_prepend(), (1, 0));

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Escape),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert!(!state.has_line_preview());
        assert_eq!(state.edit_snapshot(), before);
        assert_eq!(state.grid.cursor_pos, Coord::default());

        for key in [Key::Named(NamedKey::Space), Key::Named(NamedKey::ArrowLeft)] {
            assert!(
                handle_editor_key(&mut state, &key, None, false, ModifiersState::empty()).is_some()
            );
        }
        assert_eq!(state.take_pending_prepend(), (1, 0));

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Space),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Space),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert!(!state.has_line_preview());
        assert_eq!(
            state.content_cells(),
            [Coord { line: 0, column: 0 }, Coord { line: 0, column: 1 }]
        );
    }

    #[test]
    fn line_preview_backspace_removes_the_last_anchor() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));

        for key in [
            Key::Named(NamedKey::Space),
            Key::Named(NamedKey::ArrowRight),
            Key::Named(NamedKey::ArrowRight),
            Key::Named(NamedKey::Space),
            Key::Named(NamedKey::ArrowDown),
            Key::Named(NamedKey::ArrowDown),
            Key::Named(NamedKey::Space),
            Key::Named(NamedKey::Backspace),
        ] {
            assert_eq!(
                handle_editor_key(&mut state, &key, None, false, ModifiersState::empty(),),
                Some(false)
            );
        }
        assert!(state.has_line_preview());
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 2 });
        assert_eq!(
            state
                .lines_with_shape_preview()
                .expect("earlier preview segment remains active")
                .iter()
                .flatten()
                .filter(|atom| drawing::is_line_glyph(&atom.contents))
                .count(),
            3
        );
        assert!(state.content_cells().is_empty());

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Space),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert!(!state.has_line_preview());
        assert_eq!(
            state.content_cells(),
            [
                Coord { line: 0, column: 0 },
                Coord { line: 0, column: 1 },
                Coord { line: 0, column: 2 },
            ]
        );
    }

    #[test]
    fn shape_commands_start_preview_move_and_commit_a_rectangle() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
        let before = history::HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: layout::ViewportOffset::default(),
        };

        assert!(!apply_edit_command(
            &mut state,
            EditCommand::StartOrConfirmShape
        ));
        for command in [
            EditCommand::Move(Direction::Right),
            EditCommand::Move(Direction::Right),
            EditCommand::Move(Direction::Right),
            EditCommand::Move(Direction::Down),
            EditCommand::Move(Direction::Down),
        ] {
            assert!(!apply_edit_command(&mut state, command));
        }

        let preview = state
            .lines_with_shape_preview()
            .expect("preview is visible");
        assert_eq!(line_contents(&preview[0]), "┌──┐");
        assert!(apply_edit_command(
            &mut state,
            EditCommand::StartOrConfirmShape
        ));
        assert!(state.lines_with_shape_preview().is_none());
        assert_eq!(line_contents(&state.grid.lines[2]), "└──┘");
        let placed = history::HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: layout::ViewportOffset::default(),
        };
        let mut shape_history = history::EditHistory::default();
        assert!(shape_history.record_change(before.clone(), &placed));
        assert_eq!(shape_history.undo(placed), Some(before));
    }

    #[test]
    fn history_grouping_routes_only_line_strokes_and_text_accepting_sessions() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        let mut ordered = input::OrderedModifierTracker::default();
        ordered.update(ModifiersState::SHIFT);
        assert_eq!(
            history_group_for_key(
                &state,
                &Key::Named(NamedKey::ArrowRight),
                ModifiersState::SHIFT,
                &ordered,
            ),
            Some(HistoryGroup::LineStroke)
        );
        assert_eq!(
            history_group_for_key(
                &state,
                &Key::Named(NamedKey::ArrowRight),
                ModifiersState::empty(),
                &input::OrderedModifierTracker::default(),
            ),
            None
        );

        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            state.cursor_mode = mode;
            assert_eq!(
                history_group_for_key(
                    &state,
                    &Key::Character("u".into()),
                    ModifiersState::empty(),
                    &input::OrderedModifierTracker::default(),
                ),
                Some(HistoryGroup::TextSession),
                "mode {mode:?}"
            );
        }
    }

    #[test]
    fn plain_and_selection_movement_use_navigation_policy() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        state.insert("abc");
        state.move_home();
        let key = Key::Named(NamedKey::ArrowRight);
        let plain = input::OrderedModifierTracker::default();

        assert_eq!(
            change_policy_for_key(&state, &key, false, ModifiersState::empty(), &plain),
            ChangePolicy::Navigation {
                command: EditCommand::Move(Direction::Right),
                steps: 1,
            }
        );
        assert_eq!(
            navigation_target(&state, EditCommand::Move(Direction::Right), 1),
            Some(Coord { line: 0, column: 1 })
        );
        apply_navigation_command(&mut state, EditCommand::Move(Direction::Right), 1);
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });

        let mut control = input::OrderedModifierTracker::default();
        control.update(ModifiersState::CONTROL);
        assert_eq!(
            change_policy_for_key(&state, &key, false, ModifiersState::CONTROL, &control),
            ChangePolicy::Navigation {
                command: EditCommand::ExtendSelection(Direction::Right),
                steps: 1,
            }
        );
    }

    #[test]
    fn plain_history_shortcuts_undo_and_redo_in_every_canvas_mode() {
        let config = AppConfig::default();
        for main_mode in [
            MainMode::Line,
            MainMode::Stamp,
            MainMode::Shapes,
            MainMode::Utilities,
        ] {
            let mut state = EditorState::new(&config.theme, "test");
            state.apply_toolbar_action(ToolbarAction::SelectMain(main_mode));
            let before = history::HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: layout::ViewportOffset::default(),
            };
            state.insert("x");
            let edited = history::HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: layout::ViewportOffset::default(),
            };
            let mut edit_history = history::EditHistory::default();
            assert!(edit_history.record_change(before.clone(), &edited));

            assert_eq!(
                history_command(
                    &Key::Character("u".into()),
                    ModifiersState::empty(),
                    state.cursor_mode,
                ),
                Some(HistoryCommand::Undo),
                "mode {main_mode:?}"
            );
            state.prepare_history_command();
            let undone = edit_history.undo(edited).expect("undo entry");
            state.restore_edit_snapshot(undone.edit.clone());
            assert_eq!(line_contents(&state.grid.lines[0]), "");

            assert_eq!(
                history_command(
                    &Key::Character("U".into()),
                    ModifiersState::SHIFT,
                    state.cursor_mode,
                ),
                Some(HistoryCommand::Redo),
                "mode {main_mode:?}"
            );
            state.prepare_history_command();
            let redone = edit_history.redo(undone).expect("redo entry");
            state.restore_edit_snapshot(redone.edit);
            assert_eq!(line_contents(&state.grid.lines[0]), "x");
        }
    }

    #[test]
    fn plain_u_and_uppercase_u_remain_text_and_single_replacements() {
        let config = AppConfig::default();
        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            let mut state = EditorState::new(&config.theme, "test");
            state.cursor_mode = mode;
            for (key, modifiers) in [("u", ModifiersState::empty()), ("U", ModifiersState::SHIFT)] {
                assert_eq!(
                    history_command(&Key::Character(key.into()), modifiers, state.cursor_mode),
                    None,
                    "mode {mode:?}, key {key}"
                );
                assert_eq!(
                    handle_editor_key(
                        &mut state,
                        &Key::Character(key.into()),
                        Some(key),
                        false,
                        modifiers,
                    ),
                    Some(true)
                );
            }
            assert_eq!(line_contents(&state.grid.lines[0]), "uU", "mode {mode:?}");
        }

        for (key, modifiers) in [("u", ModifiersState::empty()), ("U", ModifiersState::SHIFT)] {
            let mut state = EditorState::new(&config.theme, "test");
            state.insert("x");
            state.move_to(Coord::default());
            assert!(state.begin_single_replace());
            assert_eq!(
                history_command(&Key::Character(key.into()), modifiers, state.cursor_mode),
                None
            );
            assert_eq!(
                handle_editor_key(
                    &mut state,
                    &Key::Character(key.into()),
                    Some(key),
                    false,
                    modifiers,
                ),
                Some(true)
            );
            assert_eq!(line_contents(&state.grid.lines[0]), key);
            assert_eq!(state.cursor_mode, CursorMode::Stamp);
        }
    }

    #[test]
    fn plain_history_precedes_and_cancels_pending_toolbar_prefixes() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "ascdraw");
        assert!(
            state.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty(),)
        );
        assert!(state.toolbar.pending_shortcut().is_some());
        assert_eq!(
            history_command(
                &Key::Character("u".into()),
                ModifiersState::empty(),
                state.cursor_mode,
            ),
            Some(HistoryCommand::Undo)
        );
        assert!(state.prepare_history_command());
        assert!(state.toolbar.pending_shortcut().is_none());
    }

    #[test]
    fn utils_move_routes_space_arrows_enter_and_escape_without_stealing_other_enter_behavior() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        state.insert("abcd");
        state.move_home();
        state.extend_selection(Direction::Right);
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 0,
        });
        let unchanged = state.grid.lines.clone();

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Space),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::ArrowRight),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert_eq!(state.grid.lines, unchanged);
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Escape),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert!(!state.move_lift_active());
        assert_eq!(state.grid.lines, unchanged);

        handle_editor_key(
            &mut state,
            &Key::Named(NamedKey::Space),
            None,
            false,
            ModifiersState::empty(),
        );
        handle_editor_key(
            &mut state,
            &Key::Named(NamedKey::ArrowRight),
            None,
            false,
            ModifiersState::empty(),
        );
        assert!(state.move_lift_active());
        assert!(!state.selection.is_collapsed());
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Enter),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(line_contents(&state.grid.lines[0]), " abd");

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Enter),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert_eq!(state.cursor_mode, app::CursorMode::Replace);
    }

    #[test]
    fn alt_direction_lifts_an_expanded_selection_in_every_mode() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Text,
            CursorMode::Insert,
            CursorMode::Replace,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            let mut state = EditorState::new(&AppConfig::default().theme, "test");
            state.insert("abcd");
            state.move_home();
            state.extend_selection(Direction::Right);
            state.cursor_mode = mode;
            let unchanged = state.grid.lines.clone();

            assert_eq!(
                handle_editor_key(
                    &mut state,
                    &Key::Named(NamedKey::ArrowRight),
                    None,
                    false,
                    ModifiersState::ALT,
                ),
                Some(false),
                "mode={mode:?}"
            );
            assert!(state.move_lift_active(), "mode={mode:?}");
            assert_eq!(state.selection_bounds().left, 1, "mode={mode:?}");
            assert_eq!(state.grid.lines, unchanged, "mode={mode:?}");

            assert_eq!(
                handle_editor_key(
                    &mut state,
                    &Key::Named(NamedKey::ArrowRight),
                    None,
                    false,
                    ModifiersState::ALT,
                ),
                Some(false),
                "mode={mode:?}"
            );
            assert_eq!(state.selection_bounds().left, 2, "mode={mode:?}");

            assert_eq!(
                handle_editor_key(
                    &mut state,
                    &Key::Named(NamedKey::ArrowLeft),
                    None,
                    false,
                    ModifiersState::empty(),
                ),
                Some(true),
                "mode={mode:?}"
            );
            assert!(!state.move_lift_active(), "mode={mode:?}");
            assert!(state.selection.is_collapsed(), "mode={mode:?}");
            assert_eq!(state.grid.cursor_pos.column, 2, "mode={mode:?}");
            assert_eq!(line_contents(&state.grid.lines[0]), "  ab", "mode={mode:?}");
        }
    }

    #[test]
    fn changing_selection_ends_an_active_lift() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.insert("abcd");
        state.move_home();
        state.extend_selection(Direction::Right);
        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Right));

        state.extend_selection(Direction::Right);
        assert!(!state.move_lift_active());
        assert_eq!(state.selection_bounds().left, 0);
        assert_eq!(state.selection_bounds().right, 2);

        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Right));
        state.move_to(model::Coord::default());
        assert!(!state.move_lift_active());
        assert!(state.selection.is_collapsed());
    }

    #[test]
    fn structural_edge_movement_reports_a_document_change() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "ascdraw");
        assert!(apply_edit_command(
            &mut state,
            EditCommand::Move(model::Direction::Up)
        ));
        assert_eq!(state.grid.lines.len(), 2);
    }

    #[test]
    fn erase_command_reports_only_real_canvas_erasure() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "ascdraw");
        assert!(!apply_edit_command(
            &mut state,
            EditCommand::Erase(model::Direction::Right)
        ));
        state.move_to(model::Coord::default());
        state.insert("x");
        state.move_to(model::Coord::default());
        assert!(apply_edit_command(
            &mut state,
            EditCommand::Erase(model::Direction::Right)
        ));
    }

    #[test]
    fn backspace_still_clears_in_line_mode() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        state.insert("│\n│\n│");
        state.move_to(Coord { line: 1, column: 0 });

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Backspace),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(line_contents(&state.grid.lines[0]), "│");
        assert_eq!(line_contents(&state.grid.lines[1]), " ");
        assert_eq!(line_contents(&state.grid.lines[2]), "│");
    }

    #[test]
    fn modified_directions_precede_and_cancel_pending_toolbar_prefixes() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "ascdraw");
        assert!(
            state.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty())
        );
        assert!(state.toolbar.pending_shortcut().is_some());

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Character("l".into()),
                None,
                false,
                ModifiersState::CONTROL,
            ),
            Some(false)
        );
        assert_eq!(state.selection.active(), Coord { line: 0, column: 1 });
        assert!(state.toolbar.pending_shortcut().is_none());
    }

    fn ordered_modifiers(states: &[ModifiersState]) -> input::OrderedModifierTracker {
        let mut ordered = input::OrderedModifierTracker::default();
        for state in states {
            ordered.update(*state);
        }
        ordered
    }

    fn dispatch_ordered(
        state: &mut EditorState,
        key: Key,
        states: &[ModifiersState],
    ) -> Option<bool> {
        let modifiers = *states.last().expect("at least one modifier state");
        let ordered = ordered_modifiers(states);
        handle_editor_key_with_order(state, &key, None, false, modifiers, &ordered)
    }

    #[test]
    fn ordered_shift_draws_connected_five_and_ten_cell_paths() {
        for (secondary, steps) in [(ModifiersState::CONTROL, 5), (ModifiersState::ALT, 10)] {
            let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
            let combined = ModifiersState::SHIFT | secondary;
            assert_eq!(
                dispatch_ordered(
                    &mut state,
                    Key::Named(NamedKey::ArrowRight),
                    &[ModifiersState::SHIFT, combined],
                ),
                Some(true)
            );
            assert_eq!(state.grid.cursor_pos.column, steps);
            assert_eq!(state.grid.lines[0].len(), steps + 1);
            assert!(
                state.grid.lines[0]
                    .iter()
                    .all(|atom| { !atom.contents.chars().all(char::is_whitespace) })
            );
        }
    }

    #[test]
    fn ordered_alt_erases_every_intermediate_cell_even_after_a_blank() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
        state.insert(" abcdef");
        state.move_to(Coord::default());
        assert_eq!(
            dispatch_ordered(
                &mut state,
                Key::Character("l".into()),
                &[
                    ModifiersState::ALT,
                    ModifiersState::ALT | ModifiersState::CONTROL,
                ],
            ),
            Some(true)
        );
        assert_eq!(state.grid.cursor_pos.column, 5);
        assert_eq!(line_contents(&state.grid.lines[0]), "      f");
    }

    #[test]
    fn ordered_control_grows_from_the_anchor_by_five_and_ten_without_document_change() {
        for (secondary, steps) in [(ModifiersState::ALT, 5), (ModifiersState::SHIFT, 10)] {
            let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
            let combined = ModifiersState::CONTROL | secondary;
            assert_eq!(
                dispatch_ordered(
                    &mut state,
                    Key::Character("l".into()),
                    &[ModifiersState::CONTROL, combined],
                ),
                Some(false)
            );
            assert_eq!(state.selection.anchor(), Coord::default());
            assert_eq!(
                state.selection.active(),
                Coord {
                    line: 0,
                    column: steps
                }
            );
        }
    }

    #[test]
    fn ordered_shift_preserves_stamp_shape_and_utility_routing() {
        let states = [
            ModifiersState::SHIFT,
            ModifiersState::SHIFT | ModifiersState::CONTROL,
        ];

        let mut stamp = EditorState::new(&app::ThemeConfig::default(), "test");
        assert!(stamp.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
        assert_eq!(
            dispatch_ordered(&mut stamp, Key::Character("l".into()), &states),
            Some(true)
        );
        assert_eq!(stamp.grid.cursor_pos.column, 5);
        assert!(stamp.grid.lines[0].len() >= 6);

        let mut shape = EditorState::new(&app::ThemeConfig::default(), "test");
        assert!(shape.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
        assert!(!apply_edit_command(
            &mut shape,
            EditCommand::StartOrConfirmShape
        ));
        assert_eq!(
            dispatch_ordered(&mut shape, Key::Character("l".into()), &states),
            Some(false)
        );
        assert_eq!(shape.grid.cursor_pos.column, 5);
        assert!(shape.lines_with_shape_preview().is_some());

        let mut utility = EditorState::new(&app::ThemeConfig::default(), "test");
        utility.insert("x");
        utility.move_to(Coord::default());
        assert!(utility.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
        assert!(utility.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 1,
        }));
        assert_eq!(
            dispatch_ordered(&mut utility, Key::Character("l".into()), &states),
            Some(true)
        );
        assert_eq!(utility.grid.lines[0].len(), 6);
        assert_eq!(line_contents(&utility.grid.lines[0]), "x     ");
    }

    #[test]
    fn ordered_shift_pull_all_repeats_five_or_ten_times_as_one_document_change() {
        for (secondary, steps) in [(ModifiersState::CONTROL, 5), (ModifiersState::ALT, 10)] {
            let source = "abcdefghijkl";
            let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
            state.insert(source);
            state.move_to(Coord::default());
            assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
            assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
                submenu: 0,
                option: 2,
            }));
            let before = history::HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: layout::ViewportOffset::default(),
            };
            let combined = ModifiersState::SHIFT | secondary;

            assert_eq!(
                dispatch_ordered(
                    &mut state,
                    Key::Character("h".into()),
                    &[ModifiersState::SHIFT, combined],
                ),
                Some(true)
            );
            let expected = format!("a{}", &source[steps + 1..]);
            assert_eq!(line_contents(&state.grid.lines[0]), expected);

            let after = history::HistorySnapshot {
                edit: state.edit_snapshot(),
                viewport: layout::ViewportOffset::default(),
            };
            let mut history = history::EditHistory::default();
            assert!(history.record_change(before.clone(), &after));
            assert_eq!(history.undo(after), Some(before));
        }
    }

    #[test]
    fn one_ordered_keypress_is_one_history_record_and_origin_prepends_aggregate() {
        let mut state = EditorState::new(&app::ThemeConfig::default(), "test");
        let before = history::HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: layout::ViewportOffset::default(),
        };
        assert_eq!(
            dispatch_ordered(
                &mut state,
                Key::Named(NamedKey::ArrowLeft),
                &[
                    ModifiersState::SHIFT,
                    ModifiersState::SHIFT | ModifiersState::CONTROL,
                ],
            ),
            Some(true)
        );
        assert_eq!(state.take_pending_prepend(), (5, 0));
        let after = history::HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: layout::ViewportOffset::default(),
        };
        let mut history = history::EditHistory::default();
        assert!(history.record_change(before.clone(), &after));
        assert_eq!(history.undo(after), Some(before));
    }

    #[test]
    fn cancel_keys_route_out_of_text_replace_and_single_replace() {
        let config = AppConfig::default();
        for (key, modifiers) in [
            (Key::Named(NamedKey::Escape), ModifiersState::empty()),
            (Key::Character("g".into()), ModifiersState::CONTROL),
        ] {
            for mode in [CursorMode::Text, CursorMode::Replace] {
                let mut state = EditorState::new(&config.theme, "test");
                assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
                state.cursor_mode = mode;

                assert_eq!(
                    handle_editor_key(&mut state, &key, None, false, modifiers),
                    Some(false)
                );
                assert_eq!(state.cursor_mode, CursorMode::Stamp);
            }

            let mut state = EditorState::new(&config.theme, "test");
            assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
            assert!(state.begin_single_replace());

            assert_eq!(
                handle_editor_key(&mut state, &key, None, false, modifiers),
                Some(false)
            );
            assert_eq!(state.cursor_mode, CursorMode::Shapes);
        }
    }

    #[test]
    fn clipboard_shortcuts_precede_all_modes_and_pending_toolbar_paths() {
        let config = AppConfig::default();
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
            CursorMode::Text,
            CursorMode::Insert,
            CursorMode::Replace,
        ] {
            for modifiers in [ModifiersState::CONTROL, ModifiersState::SUPER] {
                let mut state = EditorState::new(&config.theme, "test");
                state.cursor_mode = mode;
                let mut platform = ClipboardPlatform {
                    text: "v".into(),
                    ..ClipboardPlatform::default()
                };
                assert!(
                    handle_clipboard_shortcut(
                        &mut state,
                        &Key::Character("V".into()),
                        modifiers,
                        &mut platform,
                    )
                    .unwrap()
                    .unwrap(),
                    "mode={mode:?}"
                );
                assert_eq!(state.selected_text(), "v");
            }
        }

        let mut one_shot = EditorState::new(&config.theme, "test");
        assert!(one_shot.begin_single_replace());
        let mut platform = ClipboardPlatform {
            text: "paste".into(),
            ..ClipboardPlatform::default()
        };
        assert!(
            handle_clipboard_shortcut(
                &mut one_shot,
                &Key::Character("v".into()),
                ModifiersState::CONTROL,
                &mut platform,
            )
            .unwrap()
            .unwrap()
        );
        assert_eq!(one_shot.selected_text(), "paste");
        assert_eq!(one_shot.cursor_mode, CursorMode::Replace);

        for prefix in ["2", "0"] {
            let mut state = EditorState::new(&config.theme, "test");
            assert!(
                state.handle_toolbar_shortcut(
                    &Key::Character(prefix.into()),
                    ModifiersState::empty()
                )
            );
            let mut platform = ClipboardPlatform {
                text: "x".into(),
                ..ClipboardPlatform::default()
            };
            assert!(
                handle_clipboard_shortcut(
                    &mut state,
                    &Key::Character("v".into()),
                    ModifiersState::CONTROL,
                    &mut platform,
                )
                .unwrap()
                .unwrap()
            );
            assert_eq!(state.selected_text(), "x");
        }

        let mut copy = EditorState::new(&config.theme, "test");
        copy.insert("copy");
        copy.move_to(Coord::default());
        copy.extend_selection(Direction::Right);
        copy.extend_selection(Direction::Right);
        copy.extend_selection(Direction::Right);
        let before = copy.clone();
        let mut platform = ClipboardPlatform::default();
        assert!(
            !handle_clipboard_shortcut(
                &mut copy,
                &Key::Character("C".into()),
                ModifiersState::SUPER,
                &mut platform,
            )
            .unwrap()
            .unwrap()
        );
        assert_eq!(platform.text, "copy");
        assert_eq!(copy.grid.lines, before.grid.lines);
        assert_eq!(copy.selection, before.selection);
    }

    #[test]
    fn cut_shortcuts_precede_every_mode_and_single_replace() {
        let config = AppConfig::default();
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
            CursorMode::Text,
            CursorMode::Insert,
            CursorMode::Replace,
        ] {
            for (key, modifiers) in [
                (Key::Character("x".into()), ModifiersState::CONTROL),
                (Key::Character("X".into()), ModifiersState::SUPER),
            ] {
                let mut state = EditorState::new(&config.theme, "test");
                state.insert("cut");
                state.move_to(Coord::default());
                state.extend_selection(Direction::Right);
                state.extend_selection(Direction::Right);
                state.cursor_mode = mode;
                let mut platform = ClipboardPlatform::default();

                assert!(
                    handle_clipboard_shortcut(&mut state, &key, modifiers, &mut platform)
                        .unwrap()
                        .unwrap(),
                    "mode={mode:?}"
                );
                assert_eq!(platform.text, "cut", "mode={mode:?}");
                assert_eq!(state.selected_text(), "   ", "mode={mode:?}");
                assert_eq!(state.cursor_mode, mode, "mode={mode:?}");
            }
        }

        let mut one_shot = EditorState::new(&config.theme, "test");
        one_shot.insert("x");
        one_shot.move_to(Coord::default());
        assert!(one_shot.begin_single_replace());
        let mut platform = ClipboardPlatform::default();
        assert!(
            handle_clipboard_shortcut(
                &mut one_shot,
                &Key::Character("x".into()),
                ModifiersState::CONTROL,
                &mut platform,
            )
            .unwrap()
            .unwrap()
        );
        assert_eq!(platform.text, "x");
        assert_eq!(one_shot.selected_text(), " ");
        assert_eq!(one_shot.cursor_mode, CursorMode::Replace);
    }

    #[test]
    fn digits_route_to_insert_replace_and_single_replace() {
        let config = AppConfig::default();
        let digit = Key::Character("2".into());

        let mut insert = EditorState::new(&config.theme, "test");
        insert.toggle_text_entry();
        assert_eq!(
            handle_editor_key(
                &mut insert,
                &digit,
                Some("2"),
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(line_contents(&insert.grid.lines[0]), "2");

        let mut replace = EditorState::new(&config.theme, "test");
        replace.insert("a");
        replace.move_to(Coord::default());
        replace.toggle_replace_mode();
        assert_eq!(
            handle_editor_key(
                &mut replace,
                &digit,
                Some("2"),
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(line_contents(&replace.grid.lines[0]), "2");
        assert_eq!(replace.cursor_mode, CursorMode::Replace);

        let mut single_replace = EditorState::new(&config.theme, "test");
        single_replace.insert("a");
        single_replace.move_to(Coord::default());
        assert!(single_replace.begin_single_replace());
        assert_eq!(
            handle_editor_key(
                &mut single_replace,
                &digit,
                Some("2"),
                false,
                ModifiersState::empty(),
            ),
            Some(true)
        );
        assert_eq!(line_contents(&single_replace.grid.lines[0]), "2");
        assert_eq!(single_replace.grid.cursor_pos, Coord::default());
        assert_eq!(single_replace.cursor_mode, CursorMode::Stamp);
    }

    #[test]
    fn escape_closes_export_menu_without_collapsing_canvas_selection() {
        let config = AppConfig::default();
        let mut state = EditorState::new(&config.theme, "test");
        state.extend_selection(Direction::Right);
        let bounds = state.selection_bounds();

        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Character("0".into()),
                Some("0"),
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert!(state.toolbar.export_menu_open());
        assert_eq!(
            handle_editor_key(
                &mut state,
                &Key::Named(NamedKey::Escape),
                None,
                false,
                ModifiersState::empty(),
            ),
            Some(false)
        );
        assert!(!state.toolbar.export_menu_open());
        assert_eq!(state.selection_bounds(), bounds);
        assert_eq!(state.cursor_mode, CursorMode::Stamp);
    }

    #[test]
    fn every_export_outcome_keeps_its_transient_peer_mode_ready() {
        let config = AppConfig::default();
        for (action, pending) in [
            (
                export::ExportAction::ClipboardTxt,
                PendingShortcut::ExportOption(0),
            ),
            (
                export::ExportAction::ClipboardPng,
                PendingShortcut::ExportOption(0),
            ),
            (
                export::ExportAction::SaveTxt,
                PendingShortcut::ExportOption(1),
            ),
            (
                export::ExportAction::SaveJson,
                PendingShortcut::ExportOption(1),
            ),
            (
                export::ExportAction::SavePng,
                PendingShortcut::ExportOption(1),
            ),
            (
                export::ExportAction::LoadTxt,
                PendingShortcut::ExportOption(2),
            ),
            (
                export::ExportAction::LoadJson,
                PendingShortcut::ExportOption(2),
            ),
            (export::ExportAction::Clear, PendingShortcut::ExportFlat(3)),
        ] {
            let mut state = EditorState::new(&config.theme, "test");
            let durable = state.toolbar.durable_selections();
            assert!(state.apply_toolbar_action(ToolbarAction::RunExport(action)));
            assert_eq!(state.toolbar.take_export_action(), Some(action));
            let mut platform = ClipboardPlatform::default();
            let outcome = perform_export_action(
                action,
                &mut state,
                &mut layout::ViewportOffset::default(),
                layout::VisibleCanvasCells {
                    origin: (0, 0),
                    columns: 80,
                    rows: 24,
                },
                &mut platform,
            )
            .unwrap();

            assert_eq!(state.toolbar.take_export_action(), None);
            assert!(state.toolbar.export_menu_open(), "action={action:?}");
            assert_eq!(state.toolbar.pending_shortcut(), Some(pending));
            assert_eq!(state.toolbar.tooltip(), crate::toolbar::Tooltip::Export);
            assert_eq!(state.toolbar.durable_selections(), durable);
            assert_eq!(
                outcome,
                match action {
                    export::ExportAction::Clear => ExportOutcome::CanvasCleared,
                    export::ExportAction::ClipboardTxt | export::ExportAction::ClipboardPng => {
                        ExportOutcome::Unchanged
                    }
                    _ => ExportOutcome::Cancelled,
                }
            );
        }
    }

    #[test]
    fn export_error_consumes_once_but_keeps_the_action_prefix_active() {
        let config = AppConfig::default();
        for action in [
            export::ExportAction::ClipboardTxt,
            export::ExportAction::ClipboardPng,
        ] {
            let mut state = EditorState::new(&config.theme, "test");
            assert!(state.apply_toolbar_action(ToolbarAction::RunExport(action)));
            assert_eq!(state.toolbar.take_export_action(), Some(action));
            let mut platform = ClipboardPlatform {
                fail_clipboard_write: true,
                ..ClipboardPlatform::default()
            };

            assert!(
                perform_export_action(
                    action,
                    &mut state,
                    &mut layout::ViewportOffset::default(),
                    layout::VisibleCanvasCells {
                        origin: (0, 0),
                        columns: 80,
                        rows: 24,
                    },
                    &mut platform,
                )
                .is_err()
            );
            assert_eq!(state.toolbar.take_export_action(), None);
            assert!(state.toolbar.export_menu_open());
            assert_eq!(
                state.toolbar.pending_shortcut(),
                Some(PendingShortcut::ExportOption(0))
            );
        }
    }

    #[test]
    fn project_load_restores_durable_menus_behind_active_export_load() {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "ascdraw-export-mode-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let config = AppConfig::default();
        let mut source = EditorState::new(&config.theme, "source");
        source.insert("saved canvas");
        assert!(source.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
        assert!(source.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 4,
        }));
        let saved_menus = source.toolbar.durable_selections();
        let saved_viewport = layout::ViewportOffset { x: -19, y: 23 };
        let mut source_viewport = saved_viewport;
        let mut save = ClipboardPlatform {
            save: Some(path.clone()),
            ..ClipboardPlatform::default()
        };
        perform_export_action(
            export::ExportAction::SaveJson,
            &mut source,
            &mut source_viewport,
            layout::VisibleCanvasCells {
                origin: (0, 0),
                columns: 80,
                rows: 24,
            },
            &mut save,
        )
        .unwrap();

        let mut target = EditorState::new(&config.theme, "target");
        assert!(target.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
        assert!(
            target.apply_toolbar_action(ToolbarAction::RunExport(export::ExportAction::LoadJson,))
        );
        assert_eq!(
            target.toolbar.take_export_action(),
            Some(export::ExportAction::LoadJson)
        );
        let mut target_viewport = layout::ViewportOffset::default();
        let mut load = ClipboardPlatform {
            open: Some(path.clone()),
            ..ClipboardPlatform::default()
        };
        assert_eq!(
            perform_export_action(
                export::ExportAction::LoadJson,
                &mut target,
                &mut target_viewport,
                layout::VisibleCanvasCells {
                    origin: (0, 0),
                    columns: 80,
                    rows: 24,
                },
                &mut load,
            )
            .unwrap(),
            ExportOutcome::ProjectLoaded
        );
        assert_eq!(target.toolbar.durable_selections(), saved_menus);
        assert_eq!(target_viewport, saved_viewport);
        assert!(target.toolbar.export_menu_open());
        assert_eq!(
            target.toolbar.pending_shortcut(),
            Some(PendingShortcut::ExportOption(2))
        );
        assert!(target.apply_toolbar_action(ToolbarAction::ToggleExportMenu));
        assert!(!target.toolbar.export_menu_open());
        assert_eq!(target.toolbar.main_mode(), MainMode::Stamp);
        let _ = std::fs::remove_file(path);
    }

    fn line_contents(line: &[crate::model::Atom]) -> String {
        line.iter().map(|atom| atom.contents.as_str()).collect()
    }
}
