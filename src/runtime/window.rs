use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::Result;
#[cfg(test)]
use anyhow::anyhow;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{AppCommand, AppConfig, DEFAULT_WINDOW_TITLE};
use crate::diagnostics::log_error;
use crate::document;
use crate::editor::{ContentIndex, EditorState};
use crate::history::{EditHistory, HistoryGroup, HistorySnapshot};
use crate::input::EditCommand;
use crate::input::{OrderedModifierTracker, ViewCommand};
use crate::layout::{
    LayoutMetrics, PADDING, ViewportOffset, VisibleCanvasCells, content_intersects_inner_screen,
    content_top_padding, cursor_is_visible, cursor_origin, layout_metrics, navigation_origin,
    normalized_cursor_and_origin,
};
#[cfg(target_os = "macos")]
use crate::macos;
use crate::model::{Atom, Coord, Direction};
use crate::perf::{FrameTiming, PerfDiagnostics};
use crate::render::{Renderer, WindowSurface, load_renderer};
use crate::title_policy::window_attributes;
use crate::user_keys::FontSizeAction;

#[cfg(target_os = "macos")]
const CURSOR_IDLE_TIMEOUT: Duration = Duration::from_secs(2);
const EXPORT_SUCCESS_HIGHLIGHT_DURATION: Duration = Duration::from_millis(650);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewCursorAnchor {
    x: i64,
    y: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateChangeViewportPolicy {
    CursorAndContent,
    CursorOnly,
}

impl ViewCursorAnchor {
    fn capture(
        cursor: Coord,
        viewport: ViewportOffset,
        cell_size: (usize, usize),
        grid_top: usize,
    ) -> Self {
        Self {
            x: (PADDING as i64)
                .saturating_add(
                    i64::try_from(cursor.column)
                        .unwrap_or(i64::MAX)
                        .saturating_mul(i64::try_from(cell_size.0).unwrap_or(i64::MAX)),
                )
                .saturating_add(viewport.x),
            y: i64::try_from(grid_top)
                .unwrap_or(i64::MAX)
                .saturating_add(
                    i64::try_from(cursor.line)
                        .unwrap_or(i64::MAX)
                        .saturating_mul(i64::try_from(cell_size.1).unwrap_or(i64::MAX)),
                )
                .saturating_add(viewport.y),
        }
    }

    fn cursor_for_viewport(
        self,
        viewport: ViewportOffset,
        cell_size: (usize, usize),
        grid_top: usize,
    ) -> (i64, i64) {
        let width = i64::try_from(cell_size.0.max(1)).unwrap_or(i64::MAX);
        let height = i64::try_from(cell_size.1.max(1)).unwrap_or(i64::MAX);
        (
            self.y
                .saturating_sub(i64::try_from(grid_top).unwrap_or(i64::MAX))
                .saturating_sub(viewport.y)
                .div_euclid(height),
            self.x
                .saturating_sub(PADDING as i64)
                .saturating_sub(viewport.x)
                .div_euclid(width),
        )
    }

    fn restore_for_cursor(
        self,
        viewport: &mut ViewportOffset,
        cursor: Coord,
        cell_size: (usize, usize),
        grid_top: usize,
    ) {
        viewport.x = self.x.saturating_sub(PADDING as i64).saturating_sub(
            i64::try_from(cursor.column)
                .unwrap_or(i64::MAX)
                .saturating_mul(i64::try_from(cell_size.0).unwrap_or(i64::MAX)),
        );
        viewport.y = self
            .y
            .saturating_sub(i64::try_from(grid_top).unwrap_or(i64::MAX))
            .saturating_sub(
                i64::try_from(cursor.line)
                    .unwrap_or(i64::MAX)
                    .saturating_mul(i64::try_from(cell_size.1).unwrap_or(i64::MAX)),
            );
    }
}

pub struct EditorWindow {
    pub window: Rc<Window>,
    pub surface: WindowSurface,
    pub modifiers: ModifiersState,
    pub ordered_modifiers: OrderedModifierTracker,
    pub mouse_cell: Option<Coord>,
    pub mouse_toolbar_position: Option<(usize, usize, usize)>,
    mouse_drag: Option<MouseDrag>,
    pub state: EditorState,
    pub renderer: Renderer,
    pub viewport: ViewportOffset,
    view_cursor_anchor: Option<ViewCursorAnchor>,
    history: EditHistory,
    content_index: ContentIndex,
    perf: PerfDiagnostics,
    transparent_menubar: bool,
    document_path: PathBuf,
    document_dirty: bool,
    menu_selections_dirty: bool,
    last_keypress: Instant,
    export_success_deadline: Option<Instant>,
    #[cfg(target_os = "macos")]
    last_cursor_activity: Instant,
    #[cfg(target_os = "macos")]
    cursor_hidden: bool,
}

#[derive(Debug, Clone)]
struct MouseDrag {
    previous_state: EditorState,
    previous_viewport: ViewportOffset,
    last_pointer: Coord,
    active: bool,
    document_changed: bool,
    input_override: Option<MouseDragOverride>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseDragOverride {
    Control,
    Space,
}

fn finish_mouse_drag_state(
    state: &mut EditorState,
    input_override: Option<MouseDragOverride>,
) -> bool {
    let changed = if state.move_lift_active() {
        state.confirm_move_lift()
    } else if input_override == Some(MouseDragOverride::Space)
        && state.cursor_mode == crate::app::CursorMode::Shapes
        && state.has_shape_preview()
    {
        crate::apply_edit_command(state, EditCommand::StartOrConfirmShape)
    } else {
        false
    };
    state.end_stroke();
    changed
}

impl EditorWindow {
    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn begin_mouse_drag(&mut self, coord: Coord) {
        let input_override = if self.modifiers == ModifiersState::empty() {
            match self.state.cursor_mode {
                crate::app::CursorMode::MoveDraw => Some(MouseDragOverride::Control),
                crate::app::CursorMode::Stamp | crate::app::CursorMode::Shapes => {
                    Some(MouseDragOverride::Space)
                }
                _ => None,
            }
        } else {
            None
        };
        let target = self.state.cursor_target_for_coord(coord);
        let preserve_selection =
            self.modifiers == ModifiersState::ALT && !self.state.selection.is_collapsed();
        if !preserve_selection && let Some(origin) = self.navigation_origin_for(target) {
            self.finish_history_transaction();
            self.state.move_to(coord);
            self.finish_navigation(origin);
            self.request_redraw();
        }
        self.mouse_drag = Some(MouseDrag {
            previous_state: self.state.clone(),
            previous_viewport: self.viewport,
            last_pointer: target,
            active: false,
            document_changed: false,
            input_override,
        });
    }

    pub fn continue_mouse_drag(&mut self) {
        let Some(coord) = self.mouse_cell else {
            return;
        };
        let target = self.state.cursor_target_for_coord(coord);
        let Some(mut drag) = self.mouse_drag.take() else {
            return;
        };
        if target == drag.last_pointer {
            self.mouse_drag = Some(drag);
            return;
        }
        if !drag.active {
            if drag.input_override == Some(MouseDragOverride::Space) {
                let command = match self.state.cursor_mode {
                    crate::app::CursorMode::Stamp => Some(EditCommand::PlaceStamp),
                    crate::app::CursorMode::Shapes => Some(EditCommand::StartOrConfirmShape),
                    _ => None,
                };
                if let Some(command) = command {
                    drag.document_changed |= crate::apply_edit_command(&mut self.state, command);
                }
            }
            drag.active = true;
        }
        let (modifiers, space_held) = match drag.input_override {
            Some(MouseDragOverride::Control) => (ModifiersState::CONTROL, false),
            Some(MouseDragOverride::Space) => (ModifiersState::empty(), true),
            None => (self.modifiers, false),
        };
        while drag.last_pointer.column != target.column {
            let direction = if drag.last_pointer.column < target.column {
                Direction::Right
            } else {
                Direction::Left
            };
            drag.document_changed |= crate::handle_cursor_direction(
                &mut self.state,
                direction,
                modifiers,
                None,
                space_held,
            )
            .unwrap_or(false);
            drag.last_pointer.column = match direction {
                Direction::Right => drag.last_pointer.column.saturating_add(1),
                Direction::Left => drag.last_pointer.column.saturating_sub(1),
                _ => unreachable!(),
            };
        }
        while drag.last_pointer.line != target.line {
            let direction = if drag.last_pointer.line < target.line {
                Direction::Down
            } else {
                Direction::Up
            };
            drag.document_changed |= crate::handle_cursor_direction(
                &mut self.state,
                direction,
                modifiers,
                None,
                space_held,
            )
            .unwrap_or(false);
            drag.last_pointer.line = match direction {
                Direction::Down => drag.last_pointer.line.saturating_add(1),
                Direction::Up => drag.last_pointer.line.saturating_sub(1),
                _ => unreachable!(),
            };
        }
        if drag.document_changed {
            self.content_index.invalidate();
        }
        self.mouse_drag = Some(drag);
        self.request_redraw();
    }

    pub fn finish_mouse_drag(&mut self) {
        let Some(drag) = self.mouse_drag.take() else {
            return;
        };
        if !drag.active {
            return;
        }
        let finished_document = finish_mouse_drag_state(&mut self.state, drag.input_override);
        let document_changed = drag.document_changed || finished_document;
        let recorded = self.finish_state_change(
            drag.previous_state,
            drag.previous_viewport,
            document_changed,
        );
        if recorded {
            self.mark_document_dirty();
        }
        self.request_redraw();
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn apply_config(&mut self, config: &AppConfig) {
        let scale_factor = self.window.scale_factor();
        let old_metrics = self.renderer.metrics(scale_factor);
        let old_toolbar_metrics = self.renderer.title_metrics(scale_factor);
        let old_grid_top = grid_top(
            scale_factor,
            self.transparent_menubar,
            old_toolbar_metrics.cell_height,
            &self.state.toolbar,
        );
        self.renderer.apply_config(config);
        let new_metrics = self.renderer.metrics(scale_factor);
        let new_toolbar_metrics = self.renderer.title_metrics(scale_factor);
        let new_grid_top = grid_top(
            scale_factor,
            config.transparent_menubar,
            new_toolbar_metrics.cell_height,
            &self.state.toolbar,
        );
        self.viewport.reanchor_cursor(
            self.state.grid.cursor_pos,
            (old_metrics.cell_width, old_metrics.cell_height),
            (new_metrics.cell_width, new_metrics.cell_height),
            old_grid_top,
            new_grid_top,
        );
        self.transparent_menubar = config.transparent_menubar;
        self.state.apply_theme(&config.theme);
        self.ensure_cursor_in_viewport();
        #[cfg(target_os = "macos")]
        if let Err(error) = macos::apply_window_color_space(self.window.as_ref(), &config.macos) {
            log_error(format!("macOS color space setup failed: {error:#}"));
        }
        if let Err(error) = self.surface.apply_config(config) {
            log_error(format!("renderer configuration failed: {error:#}"));
        }
        self.request_redraw();
    }

    pub fn note_keypress(&mut self, now: Instant) {
        self.last_keypress = now;
        self.perf.begin_keypress(now);
    }

    pub fn record_state_history_time(&mut self, started: Instant) {
        self.perf.record_state_history(started.elapsed());
    }

    pub fn finish_keypress(&mut self, now: Instant) {
        self.perf.finish_keypress(now);
    }

    pub fn record_present(&mut self, timing: FrameTiming, now: Instant) {
        self.perf.record_present(timing, now);
    }

    pub fn show_export_success(&mut self, action: crate::export::ExportAction, now: Instant) {
        self.state.toolbar.mark_export_successful(action);
        self.export_success_deadline = Some(now + EXPORT_SUCCESS_HIGHLIGHT_DURATION);
    }

    pub fn clear_export_success_if_elapsed(&mut self, now: Instant) {
        if self
            .export_success_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.export_success_deadline = None;
            if self.state.toolbar.clear_export_success() {
                self.request_redraw();
            }
        }
    }

    #[cfg(target_os = "macos")]
    pub fn note_cursor_activity(&mut self, now: Instant) {
        self.last_cursor_activity = now;
        if self.cursor_hidden {
            self.window.set_cursor_visible(true);
            self.cursor_hidden = false;
        }
    }

    #[cfg(target_os = "macos")]
    pub fn hide_cursor_if_idle(&mut self, now: Instant) {
        if !self.cursor_hidden
            && now.saturating_duration_since(self.last_cursor_activity) >= CURSOR_IDLE_TIMEOUT
        {
            self.window.set_cursor_visible(false);
            self.cursor_hidden = true;
        }
    }

    pub fn mark_document_dirty(&mut self) {
        self.document_dirty = true;
    }

    pub fn history_snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            edit: self.state.edit_snapshot(),
            viewport: self.viewport,
        }
    }

    pub fn undo(&mut self) -> bool {
        let transient_changed = self.state.prepare_history_command();
        let current = self.history_snapshot();
        let Some(snapshot) = self.history.undo(current) else {
            if transient_changed {
                self.request_redraw();
            }
            return false;
        };
        self.restore_history_snapshot(snapshot);
        true
    }

    pub fn finish_history_transaction(&mut self) -> bool {
        if !self.history.has_pending_transaction() {
            return false;
        }
        let current = self.history_snapshot();
        self.history.finish_transaction(&current)
    }

    pub fn navigation_origin_for(&mut self, cursor: Coord) -> Option<(i64, i64)> {
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let layout = self.current_layout();
        let content = self.content_index.cells(&self.state.grid.lines).to_vec();
        resolve_navigation_origin(
            self.viewport.origin(cell_size),
            cursor,
            (layout.cols.max(1), layout.rows.max(1)),
            &content,
        )
    }

    pub fn finish_navigation(&mut self, origin: (i64, i64)) {
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        if self.viewport.origin(cell_size) != origin {
            self.viewport.set_origin(origin, cell_size);
        }
    }

    pub fn redo(&mut self) -> bool {
        let transient_changed = self.state.prepare_history_command();
        let current = self.history_snapshot();
        let Some(snapshot) = self.history.redo(current) else {
            if transient_changed {
                self.request_redraw();
            }
            return false;
        };
        self.restore_history_snapshot(snapshot);
        true
    }

    fn restore_history_snapshot(&mut self, snapshot: HistorySnapshot) {
        self.state.restore_edit_snapshot(snapshot.edit);
        self.content_index.invalidate();
        self.viewport = snapshot.viewport;
        self.ensure_cursor_in_viewport();
        self.mark_document_dirty();
        self.request_redraw();
    }

    pub fn finish_state_change(
        &mut self,
        previous_state: EditorState,
        previous_viewport: ViewportOffset,
        document_changed: bool,
    ) -> bool {
        self.finish_state_change_in_group(
            previous_state,
            previous_viewport,
            document_changed,
            None,
            StateChangeViewportPolicy::CursorAndContent,
        )
    }

    pub fn finish_selection_clear(
        &mut self,
        previous_state: EditorState,
        previous_viewport: ViewportOffset,
    ) -> bool {
        self.finish_state_change_in_group(
            previous_state,
            previous_viewport,
            true,
            None,
            StateChangeViewportPolicy::CursorOnly,
        )
    }

    pub fn finish_grouped_state_change(
        &mut self,
        previous_state: EditorState,
        previous_viewport: ViewportOffset,
        document_changed: bool,
        group: HistoryGroup,
    ) -> bool {
        self.finish_state_change_in_group(
            previous_state,
            previous_viewport,
            document_changed,
            Some(group),
            StateChangeViewportPolicy::CursorAndContent,
        )
    }

    fn finish_state_change_in_group(
        &mut self,
        previous_state: EditorState,
        previous_viewport: ViewportOffset,
        document_changed: bool,
        group: Option<HistoryGroup>,
        viewport_policy: StateChangeViewportPolicy,
    ) -> bool {
        let previous = HistorySnapshot {
            edit: previous_state.edit_snapshot(),
            viewport: previous_viewport,
        };
        if group.is_none() {
            self.history.finish_transaction(&previous);
        }
        let menu_selections_changed =
            durable_menu_selections_changed(&previous_state.toolbar, &self.state.toolbar);
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let toolbar_metrics = self.renderer.title_metrics(scale_factor);
        reanchor_toolbar_transition(
            &mut self.viewport,
            scale_factor,
            self.transparent_menubar,
            toolbar_metrics.cell_height,
            &previous_state.toolbar,
            &self.state.toolbar,
        );
        let layout = self.current_layout();
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let view_mode_changed = reconcile_view_cursor(
            &mut self.view_cursor_anchor,
            &mut self.viewport,
            &previous_state,
            &mut self.state,
            cell_size,
            layout.grid_top,
        );
        let prepend = self.state.take_pending_prepend();
        if document_changed || prepend != (0, 0) {
            self.content_index.invalidate();
        }
        self.viewport
            .compensate_for_prepend(prepend.0, prepend.1, cell_size);

        if view_mode_changed {
            self.menu_selections_dirty |= menu_selections_changed;
            return false;
        }

        // A toolbar-only transition can temporarily clip anchored cells. Do
        // not let viewport normalization turn the exact pixel compensation
        // above into an unrelated canvas pan. Navigation and document edits
        // still take the normal constrained path below.
        if !document_changed
            && prepend == (0, 0)
            && self.state.grid.cursor_pos == previous_state.grid.cursor_pos
        {
            self.menu_selections_dirty |= menu_selections_changed;
            return false;
        }

        let current = self.viewport.origin(cell_size);
        let viewport_cells = (layout.cols.max(1), layout.rows.max(1));
        let content = self.content_index.cells(&self.state.grid.lines).to_vec();

        if let Some(origin) = resolve_state_change_origin(
            viewport_policy,
            current,
            self.state.grid.cursor_pos,
            viewport_cells,
            &content,
        ) {
            self.menu_selections_dirty |= menu_selections_changed;
            if origin != current {
                self.viewport.set_origin(origin, cell_size);
            }
            debug_assert!(cursor_is_visible(
                origin,
                self.state.grid.cursor_pos,
                viewport_cells
            ));
            debug_assert!(
                viewport_policy == StateChangeViewportPolicy::CursorOnly
                    || content.is_empty()
                    || content_intersects_inner_screen(origin, viewport_cells, &content)
            );
            if !document_changed {
                return false;
            }
            let current = self.history_snapshot();
            return match group {
                Some(group) => self
                    .history
                    .record_grouped_change(group, previous, &current),
                None => self.history.record_change(previous, &current),
            };
        }

        self.state = previous_state;
        self.content_index.invalidate();
        self.viewport = previous_viewport;
        false
    }

    pub fn finish_project_load(
        &mut self,
        previous_state: EditorState,
        previous_viewport: ViewportOffset,
    ) -> bool {
        self.menu_selections_dirty |=
            durable_menu_selections_changed(&previous_state.toolbar, &self.state.toolbar);
        self.state.compact_blank_runs_preserving_cursor();
        self.content_index.invalidate();
        self.ensure_cursor_in_viewport();
        let previous = HistorySnapshot {
            edit: previous_state.edit_snapshot(),
            viewport: previous_viewport,
        };
        let current = self.history_snapshot();
        let changed = self.history.record_project_load(previous, &current);
        if changed {
            self.mark_document_dirty();
        }
        self.request_redraw();
        changed
    }

    pub fn ensure_cursor_in_viewport(&mut self) {
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let layout = self.current_layout();
        let viewport_cells = (layout.cols.max(1), layout.rows.max(1));
        let current = self.viewport.origin(cell_size);
        let content = self.content_index.cells(&self.state.grid.lines).to_vec();
        let old_cursor = self.state.grid.cursor_pos;
        let (cursor, origin) =
            normalized_cursor_and_origin(current, old_cursor, viewport_cells, &content);
        if cursor != old_cursor {
            self.state.clamp_cursor_to_content(cursor);
        }
        if origin != current {
            self.viewport.set_origin(origin, cell_size);
        }
        debug_assert!(cursor_is_visible(
            origin,
            self.state.grid.cursor_pos,
            viewport_cells
        ));
        debug_assert!(
            content.is_empty() || content_intersects_inner_screen(origin, viewport_cells, &content)
        );
    }

    pub fn apply_view_command(&mut self, command: ViewCommand) -> bool {
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let layout = self.current_layout();
        let viewport_cells = (layout.cols.max(1), layout.rows.max(1));
        let content = self.content_index.cells(&self.state.grid.lines).to_vec();
        self.view_cursor_anchor.get_or_insert_with(|| {
            ViewCursorAnchor::capture(
                self.state.grid.cursor_pos,
                self.viewport,
                cell_size,
                layout.grid_top,
            )
        });
        let changed = match command {
            ViewCommand::Pan(direction) => pan_viewport(
                &mut self.viewport,
                direction,
                cell_size,
                viewport_cells,
                &content,
            ),
            ViewCommand::Center => {
                center_viewport(&mut self.viewport, cell_size, viewport_cells, &content)
            }
        };
        if changed {
            self.request_redraw();
        }
        changed
    }

    fn current_layout(&self) -> LayoutMetrics {
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let toolbar_metrics = self.renderer.title_metrics(scale_factor);
        let size = self.window.inner_size();
        layout_metrics(
            size.width as usize,
            size.height as usize,
            &metrics,
            toolbar_metrics.cell_height,
            &self.state.toolbar,
            self.transparent_menubar,
            scale_factor,
        )
    }

    pub fn visible_canvas_cells(&self) -> VisibleCanvasCells {
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        VisibleCanvasCells::from_layout(
            self.current_layout(),
            self.viewport,
            (metrics.cell_width, metrics.cell_height),
        )
    }

    pub fn autosave_if_idle(&mut self, now: Instant) -> Result<bool> {
        if !should_autosave(
            self.document_dirty || self.menu_selections_dirty,
            self.last_keypress,
            now,
        ) {
            return Ok(false);
        }
        self.save_document()?;
        Ok(true)
    }

    pub fn save_document(&mut self) -> Result<bool> {
        save_document_if_dirty(
            &mut self.document_dirty,
            &mut self.menu_selections_dirty,
            &self.document_path,
            &self.state.grid.lines,
            &self.state.toolbar.durable_selections(),
            document::save,
        )
    }
}

fn durable_menu_selections_changed(
    previous: &crate::toolbar::ToolbarState,
    current: &crate::toolbar::ToolbarState,
) -> bool {
    previous.durable_selections() != current.durable_selections()
}

fn reconcile_view_cursor(
    anchor: &mut Option<ViewCursorAnchor>,
    viewport: &mut ViewportOffset,
    previous: &EditorState,
    current: &mut EditorState,
    cell_size: (usize, usize),
    grid_top: usize,
) -> bool {
    let was_viewing = previous.view_active();
    let is_viewing = current.view_active();
    match (was_viewing, is_viewing) {
        (false, true) => {
            *anchor = Some(ViewCursorAnchor::capture(
                current.grid.cursor_pos,
                *viewport,
                cell_size,
                grid_top,
            ));
        }
        (true, false) => {
            if let Some(saved) = anchor.take() {
                let (line, column) = saved.cursor_for_viewport(*viewport, cell_size, grid_top);
                current.restore_cursor_after_view(line, column);
                saved.restore_for_cursor(viewport, current.grid.cursor_pos, cell_size, grid_top);
            }
        }
        (false, false) => {
            *anchor = None;
        }
        (true, true) => {}
    }
    was_viewing != is_viewing
}

fn pan_viewport(
    viewport: &mut ViewportOffset,
    direction: Direction,
    cell_size: (usize, usize),
    viewport_cells: (usize, usize),
    content: &[Coord],
) -> bool {
    let mut candidate = *viewport;
    let cell_width = i64::try_from(cell_size.0.max(1)).unwrap_or(i64::MAX);
    let cell_height = i64::try_from(cell_size.1.max(1)).unwrap_or(i64::MAX);
    match direction {
        Direction::Left => candidate.x = candidate.x.saturating_add(cell_width),
        Direction::Right => candidate.x = candidate.x.saturating_sub(cell_width),
        Direction::Up => candidate.y = candidate.y.saturating_add(cell_height),
        Direction::Down => candidate.y = candidate.y.saturating_sub(cell_height),
    }
    let origin = candidate.origin(cell_size);
    if !content.is_empty() && !content_intersects_inner_screen(origin, viewport_cells, content) {
        return false;
    }
    let changed = candidate != *viewport;
    *viewport = candidate;
    changed
}

fn center_viewport(
    viewport: &mut ViewportOffset,
    cell_size: (usize, usize),
    viewport_cells: (usize, usize),
    content: &[Coord],
) -> bool {
    let Some((min, max)) = content_bounds(content) else {
        return false;
    };
    let center = Coord {
        line: max.line - (max.line - min.line) / 2,
        column: max.column - (max.column - min.column) / 2,
    };
    let origin = (
        i64::try_from(center.column)
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(viewport_cells.0 / 2).unwrap_or(i64::MAX)),
        i64::try_from(center.line)
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(viewport_cells.1 / 2).unwrap_or(i64::MAX)),
    );
    let old_viewport = *viewport;
    viewport.set_origin(origin, cell_size);
    *viewport != old_viewport
}

fn content_bounds(content: &[Coord]) -> Option<(Coord, Coord)> {
    let first = *content.first()?;
    Some(
        content
            .iter()
            .copied()
            .fold((first, first), |(min, max), coord| {
                (
                    Coord {
                        line: min.line.min(coord.line),
                        column: min.column.min(coord.column),
                    },
                    Coord {
                        line: max.line.max(coord.line),
                        column: max.column.max(coord.column),
                    },
                )
            }),
    )
}

fn save_document_if_dirty(
    document_dirty: &mut bool,
    menu_selections_dirty: &mut bool,
    path: &Path,
    lines: &[Vec<Atom>],
    menu_selections: &crate::toolbar::DurableMenuSelections,
    save: impl FnOnce(&Path, &[Vec<Atom>], &crate::toolbar::DurableMenuSelections) -> Result<()>,
) -> Result<bool> {
    if !*document_dirty && !*menu_selections_dirty {
        return Ok(false);
    }
    save(path, lines, menu_selections)?;
    *document_dirty = false;
    *menu_selections_dirty = false;
    Ok(true)
}

#[derive(Debug, Default, Eq, PartialEq)]
struct ShutdownSaveSummary {
    saved: usize,
    failed: usize,
}

fn save_on_shutdown<'a, T: 'a>(
    documents: impl IntoIterator<Item = &'a mut T>,
    mut save: impl FnMut(&mut T) -> Result<bool>,
    mut report_failure: impl FnMut(anyhow::Error),
) -> ShutdownSaveSummary {
    let mut summary = ShutdownSaveSummary::default();
    for document in documents {
        match save(document) {
            Ok(true) => summary.saved += 1,
            Ok(false) => {}
            Err(error) => {
                summary.failed += 1;
                report_failure(error);
            }
        }
    }
    summary
}

fn save_editor_documents<'a>(
    editors: impl IntoIterator<Item = &'a mut EditorWindow>,
    lifecycle: &str,
) -> ShutdownSaveSummary {
    save_on_shutdown(editors, EditorWindow::save_document, |error| {
        log_error(format!("document save on {lifecycle} failed: {error:#}"));
    })
}

fn resolve_navigation_origin(
    current: (i64, i64),
    cursor: Coord,
    viewport: (usize, usize),
    content: &[Coord],
) -> Option<(i64, i64)> {
    navigation_origin(current, cursor, viewport, content)
}

fn resolve_state_change_origin(
    policy: StateChangeViewportPolicy,
    current: (i64, i64),
    cursor: Coord,
    viewport: (usize, usize),
    content: &[Coord],
) -> Option<(i64, i64)> {
    match policy {
        StateChangeViewportPolicy::CursorAndContent => {
            resolve_navigation_origin(current, cursor, viewport, content)
        }
        StateChangeViewportPolicy::CursorOnly => Some((
            cursor_origin(current.0, cursor.column, viewport.0),
            cursor_origin(current.1, cursor.line, viewport.1),
        )),
    }
}

pub fn create_editor_window(
    elwt: &ActiveEventLoop,
    config: &AppConfig,
    document_path: &Path,
) -> Result<EditorWindow> {
    let window = Rc::new(elwt.create_window(window_attributes(config))?);
    let surface = WindowSurface::new(&window, config)?;

    #[cfg(target_os = "macos")]
    {
        if let Err(error) = macos::apply_window_color_space(window.as_ref(), &config.macos) {
            log_error(format!("macOS color space setup failed: {error:#}"));
        }
        window.focus_window();
    }

    let mut state = EditorState::new(&config.theme, DEFAULT_WINDOW_TITLE);
    if let Some(document) = document::load(document_path)? {
        state.grid.lines = if document.lines.is_empty() {
            vec![Vec::new()]
        } else {
            document.lines
        };
        if let Some(menu_selections) = document.menu_selections {
            state.restore_menu_selections(&menu_selections);
        }
    }
    let content_index = ContentIndex::new(&state.grid.lines);
    let mut editor = EditorWindow {
        window,
        surface,
        modifiers: ModifiersState::empty(),
        ordered_modifiers: OrderedModifierTracker::default(),
        mouse_cell: Some(Coord::default()),
        mouse_toolbar_position: None,
        mouse_drag: None,
        state,
        renderer: load_renderer(config),
        viewport: ViewportOffset::default(),
        view_cursor_anchor: None,
        history: EditHistory::default(),
        content_index,
        perf: PerfDiagnostics::from_env(),
        transparent_menubar: config.transparent_menubar,
        document_path: document_path.to_path_buf(),
        document_dirty: false,
        menu_selections_dirty: false,
        last_keypress: Instant::now(),
        export_success_deadline: None,
        #[cfg(target_os = "macos")]
        last_cursor_activity: Instant::now(),
        #[cfg(target_os = "macos")]
        cursor_hidden: false,
    };
    editor.ensure_cursor_in_viewport();
    editor.request_redraw();
    Ok(editor)
}

pub fn focused_window_id(windows: &HashMap<WindowId, EditorWindow>) -> Option<WindowId> {
    windows
        .iter()
        .find_map(|(window_id, editor)| editor.window.has_focus().then_some(*window_id))
        .or_else(|| (windows.len() == 1).then(|| *windows.keys().next().expect("one window")))
}

pub fn close_window(
    windows: &mut HashMap<WindowId, EditorWindow>,
    window_id: WindowId,
    elwt: &ActiveEventLoop,
) {
    if let Some(mut editor) = windows.remove(&window_id) {
        editor.state.end_stroke();
        editor.finish_history_transaction();
        save_editor_documents(std::iter::once(&mut editor), "close");
    }
    if windows.is_empty() {
        elwt.exit();
    }
}

pub fn save_windows_on_exit(windows: &mut HashMap<WindowId, EditorWindow>) {
    for editor in windows.values_mut() {
        editor.state.end_stroke();
        editor.finish_history_transaction();
    }
    save_editor_documents(windows.values_mut(), "application exit");
}

fn should_autosave(dirty: bool, last_keypress: Instant, now: Instant) -> bool {
    dirty && now.saturating_duration_since(last_keypress) > Duration::from_secs(5)
}

pub fn handle_command(
    command: AppCommand,
    source_window_id: Option<WindowId>,
    windows: &mut HashMap<WindowId, EditorWindow>,
    elwt: &ActiveEventLoop,
    config: &AppConfig,
    document_path: &Path,
) {
    let target = source_window_id
        .filter(|window_id| windows.contains_key(window_id))
        .or_else(|| focused_window_id(windows));

    match command {
        AppCommand::WindowNew => match create_editor_window(elwt, config, document_path) {
            Ok(editor) => {
                windows.insert(editor.window_id(), editor);
            }
            Err(error) => log_error(format!("new window creation failed: {error:#}")),
        },
        AppCommand::WindowClose => {
            if let Some(window_id) = target {
                close_window(windows, window_id, elwt);
            }
        }
        AppCommand::FontScaleUp => {
            adjust_font_size(windows, target, FontSizeAction::Increase, config)
        }
        AppCommand::FontScaleDown => {
            adjust_font_size(windows, target, FontSizeAction::Decrease, config)
        }
        AppCommand::FontScaleReset => {
            adjust_font_size(windows, target, FontSizeAction::Reset, config)
        }
    }
}

fn adjust_font_size(
    windows: &mut HashMap<WindowId, EditorWindow>,
    target: Option<WindowId>,
    action: FontSizeAction,
    config: &AppConfig,
) {
    let Some(editor) = target.and_then(|window_id| windows.get_mut(&window_id)) else {
        return;
    };
    let scale_factor = editor.window.scale_factor();
    let old_metrics = editor.renderer.metrics(scale_factor);
    let toolbar_metrics = editor.renderer.title_metrics(scale_factor);
    let grid_top = grid_top(
        scale_factor,
        config.transparent_menubar,
        toolbar_metrics.cell_height,
        &editor.state.toolbar,
    );
    let changed = match action {
        FontSizeAction::Increase => editor.renderer.adjust_font_size(1.0),
        FontSizeAction::Decrease => editor.renderer.adjust_font_size(-1.0),
        FontSizeAction::Reset => editor.renderer.reset_font_size(),
    };
    if changed {
        let new_metrics = editor.renderer.metrics(scale_factor);
        editor.viewport.reanchor_cursor(
            editor.state.grid.cursor_pos,
            (old_metrics.cell_width, old_metrics.cell_height),
            (new_metrics.cell_width, new_metrics.cell_height),
            grid_top,
            grid_top,
        );
        editor.ensure_cursor_in_viewport();
        editor.request_redraw();
    }
}

fn grid_top(
    scale_factor: f64,
    transparent_menubar: bool,
    toolbar_cell_height: usize,
    toolbar: &crate::toolbar::ToolbarState,
) -> usize {
    content_top_padding(scale_factor, transparent_menubar)
        + crate::toolbar::toolbar_height(toolbar, toolbar_cell_height)
}

fn reanchor_toolbar_transition(
    viewport: &mut ViewportOffset,
    scale_factor: f64,
    transparent_menubar: bool,
    toolbar_cell_height: usize,
    old_toolbar: &crate::toolbar::ToolbarState,
    new_toolbar: &crate::toolbar::ToolbarState,
) {
    let old_grid_top = grid_top(
        scale_factor,
        transparent_menubar,
        toolbar_cell_height,
        old_toolbar,
    );
    let new_grid_top = grid_top(
        scale_factor,
        transparent_menubar,
        toolbar_cell_height,
        new_toolbar,
    );
    viewport.reanchor_grid_top(old_grid_top, new_grid_top);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppConfig;
    use crate::export::{self, ExportAction, ExportOutcome, ExportPlatform, FileKind};
    use crate::model::{Atom, Direction, Face};
    use crate::toolbar::{MainMode, ToolbarAction};
    use winit::keyboard::{Key, ModifiersState, NamedKey};

    #[derive(Default)]
    struct NoopExportPlatform;

    impl ExportPlatform for NoopExportPlatform {
        fn set_clipboard_text(&mut self, _: &str) -> Result<()> {
            unreachable!("Clear does not access the clipboard")
        }

        fn clipboard_text(&mut self) -> Result<String> {
            unreachable!("Clear does not access the clipboard")
        }

        fn choose_save_path(&mut self, _: FileKind) -> Option<PathBuf> {
            unreachable!("Clear does not open a save dialog")
        }

        fn choose_open_path(&mut self, _: FileKind) -> Option<PathBuf> {
            unreachable!("Clear does not open a load dialog")
        }
    }

    fn toolbar_test_metrics(config: &AppConfig) -> (usize, (usize, usize)) {
        let renderer = load_renderer(config);
        let toolbar = renderer.title_metrics(1.0);
        let canvas = renderer.metrics(1.0);
        (toolbar.cell_height, (canvas.cell_width, canvas.cell_height))
    }

    fn canvas_screen_position(
        coord: Coord,
        grid_top: usize,
        cell_size: (usize, usize),
        viewport: ViewportOffset,
    ) -> (i64, i64) {
        (
            PADDING as i64 + coord.column as i64 * cell_size.0 as i64 + viewport.x,
            grid_top as i64 + coord.line as i64 * cell_size.1 as i64 + viewport.y,
        )
    }

    fn state_with_rows(rows: &[&str]) -> EditorState {
        let mut state = EditorState::new(&AppConfig::default().theme, DEFAULT_WINDOW_TITLE);
        state.grid.lines = rows
            .iter()
            .map(|row| {
                unicode_segmentation::UnicodeSegmentation::graphemes(*row, true)
                    .map(|contents| Atom {
                        face: Face::default(),
                        contents: contents.to_string(),
                    })
                    .collect()
            })
            .collect();
        state
    }

    #[test]
    fn alt_drag_release_commits_the_move_before_a_later_click() {
        let mut state = state_with_rows(&["abcd"]);
        state.extend_selection(Direction::Right);
        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Right));
        assert!(state.move_lift_active());
        let original = state.grid.lines.clone();

        assert!(finish_mouse_drag_state(&mut state, None));
        assert!(!state.move_lift_active());
        assert_ne!(state.grid.lines, original);
        let committed = state.grid.lines.clone();

        state.move_to(Coord { line: 0, column: 3 });
        assert_eq!(state.grid.lines, committed);
    }

    #[test]
    fn view_pan_uses_camera_directions_exact_cells_and_preserves_pixel_residuals() {
        let cell_size = (9, 13);
        let original = ViewportOffset { x: 7, y: -3 };
        let content = [Coord { line: 5, column: 5 }];
        for (direction, expected) in [
            (Direction::Left, ViewportOffset { x: 16, y: -3 }),
            (Direction::Right, ViewportOffset { x: -2, y: -3 }),
            (Direction::Up, ViewportOffset { x: 7, y: 10 }),
            (Direction::Down, ViewportOffset { x: 7, y: -16 }),
        ] {
            let mut viewport = original;
            assert!(pan_viewport(
                &mut viewport,
                direction,
                cell_size,
                (10, 10),
                &content,
            ));
            assert_eq!(viewport, expected);
        }

        let mut viewport = original;
        for _ in 0..20 {
            assert!(pan_viewport(
                &mut viewport,
                Direction::Left,
                cell_size,
                (10, 10),
                &content,
            ));
            assert!(pan_viewport(
                &mut viewport,
                Direction::Right,
                cell_size,
                (10, 10),
                &content,
            ));
        }
        assert_eq!(viewport, original);
    }

    #[test]
    fn view_pan_allows_cursor_escape_but_rejects_actual_inner_content_escape() {
        let mut cursor_boundary = ViewportOffset::default();
        assert!(pan_viewport(
            &mut cursor_boundary,
            Direction::Right,
            (8, 12),
            (10, 10),
            &[],
        ));
        assert_eq!(cursor_boundary, ViewportOffset { x: -8, y: 0 });

        let mut content_boundary = ViewportOffset::default();
        let content = [Coord { line: 3, column: 3 }];
        assert!(!pan_viewport(
            &mut content_boundary,
            Direction::Right,
            (8, 12),
            (10, 10),
            &content,
        ));
        assert_eq!(content_boundary, ViewportOffset::default());

        let mut empty = ViewportOffset::default();
        assert!(pan_viewport(
            &mut empty,
            Direction::Left,
            (8, 12),
            (10, 10),
            &[],
        ));
    }

    #[test]
    fn move_lift_does_not_reanchor_the_viewport() {
        let mut state = state_with_rows(&["ab"]);
        state
            .selection
            .select(Coord::default(), Coord { line: 0, column: 1 });
        let previous = state.clone();
        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Right));
        let mut viewport = ViewportOffset { x: 5, y: -7 };
        let original_viewport = viewport;
        let mut anchor = None;

        assert!(!reconcile_view_cursor(
            &mut anchor,
            &mut viewport,
            &previous,
            &mut state,
            (8, 12),
            40,
        ));
        assert_eq!(viewport, original_viewport);
        assert_ne!(state.grid.cursor_pos, previous.grid.cursor_pos);
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
    }

    #[test]
    fn view_restores_cursor_to_entry_screen_position_after_panning() {
        let cell_size = (8, 12);
        let grid_top = 40;
        let initial_viewport = ViewportOffset { x: 5, y: -7 };
        let mut state = state_with_rows(&["", "", "", "drawing"]);
        state.move_to(Coord { line: 1, column: 2 });
        let initial_screen_position =
            canvas_screen_position(state.grid.cursor_pos, grid_top, cell_size, initial_viewport);
        let previous = state.clone();
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities,)));
        assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        }));

        let mut anchor = None;
        let mut view_viewport = initial_viewport;
        assert!(reconcile_view_cursor(
            &mut anchor,
            &mut view_viewport,
            &previous,
            &mut state,
            cell_size,
            grid_top,
        ));
        assert!(anchor.is_some());

        let mut panned_viewport = ViewportOffset {
            x: initial_viewport.x - 3 * cell_size.0 as i64,
            y: initial_viewport.y - 2 * cell_size.1 as i64,
        };
        let previous = state.clone();
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line,)));
        assert!(reconcile_view_cursor(
            &mut anchor,
            &mut panned_viewport,
            &previous,
            &mut state,
            cell_size,
            grid_top,
        ));
        assert!(anchor.is_none());
        assert_eq!(state.grid.cursor_pos, Coord { line: 3, column: 5 });
        assert_eq!(
            canvas_screen_position(state.grid.cursor_pos, grid_top, cell_size, panned_viewport,),
            initial_screen_position
        );
    }

    #[test]
    fn view_restore_reanchors_when_screen_position_maps_before_canvas_origin() {
        let cell_size = (8, 12);
        let grid_top = 40;
        let initial_viewport = ViewportOffset::default();
        let mut state = state_with_rows(&["x"]);
        let initial_screen_position =
            canvas_screen_position(state.grid.cursor_pos, grid_top, cell_size, initial_viewport);
        let previous = state.clone();
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities,)));
        assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        }));
        let mut anchor = None;
        let mut view_viewport = initial_viewport;
        assert!(reconcile_view_cursor(
            &mut anchor,
            &mut view_viewport,
            &previous,
            &mut state,
            cell_size,
            grid_top,
        ));

        let mut panned_viewport = ViewportOffset {
            x: cell_size.0 as i64,
            y: cell_size.1 as i64,
        };
        let previous = state.clone();
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line,)));
        assert!(reconcile_view_cursor(
            &mut anchor,
            &mut panned_viewport,
            &previous,
            &mut state,
            cell_size,
            grid_top,
        ));
        let prepend = state.take_pending_prepend();
        assert_eq!(prepend, (0, 0));

        assert_eq!(state.grid.cursor_pos, Coord::default());
        assert_eq!(state.content_cells(), vec![Coord::default()]);
        assert_eq!(
            canvas_screen_position(state.grid.cursor_pos, grid_top, cell_size, panned_viewport,),
            initial_screen_position
        );
    }

    #[test]
    fn view_center_uses_requested_asymmetric_content_and_display_midpoints() {
        let mut state = state_with_rows(&[
            "          ",
            "  X       ",
            "          ",
            "          ",
            "        Y ",
        ]);
        state.grid.cursor_pos = Coord { line: 3, column: 5 };
        state
            .selection
            .select(Coord { line: 2, column: 4 }, Coord { line: 3, column: 5 });
        let selection = state.selection;
        let lines = state.grid.lines.clone();
        let content = state.content_cells();
        let mut viewport = ViewportOffset { x: 3, y: 7 };

        // Bounds x=2..8 and y=1..4 use max - range/2, producing (5,3).
        // A 10x8 display uses cell midpoint (5,4), hence origin (0,-1).
        assert!(center_viewport(&mut viewport, (7, 11), (10, 8), &content,));
        assert_eq!(viewport.origin((7, 11)), (0, -1));
        assert_eq!(state.grid.cursor_pos, Coord { line: 3, column: 5 });
        assert_eq!(state.selection, selection);
        assert_eq!(state.grid.lines, lines);
        assert!(!center_viewport(&mut viewport, (7, 11), (10, 8), &content,));
    }

    #[test]
    fn view_center_is_blank_noop_and_leaves_hidden_cursor_unchanged() {
        let mut blank = state_with_rows(&["     "]);
        blank.grid.cursor_pos = Coord { line: 0, column: 4 };
        let blank_cursor = blank.grid.cursor_pos;
        let mut blank_viewport = ViewportOffset { x: 5, y: -9 };
        assert!(!center_viewport(&mut blank_viewport, (8, 12), (3, 3), &[],));
        assert_eq!(blank_viewport, ViewportOffset { x: 5, y: -9 });
        assert_eq!(blank.grid.cursor_pos, blank_cursor);

        let mut wide = state_with_rows(&["    界"]);
        wide.grid.cursor_pos = Coord::default();
        wide.selection
            .select(Coord::default(), Coord { line: 0, column: 1 });
        let lines = wide.grid.lines.clone();
        let selection = wide.selection;
        let cursor = wide.grid.cursor_pos;
        let content = wide.content_cells();
        let mut viewport = ViewportOffset::default();
        assert!(center_viewport(&mut viewport, (8, 12), (3, 1), &content,));
        assert_eq!(viewport.origin((8, 12)), (4, 0));
        assert_eq!(wide.grid.cursor_pos, cursor);
        assert_eq!(wide.selection, selection);
        assert_eq!(wide.grid.lines, lines);
    }

    fn assert_toolbar_transition_is_anchored(
        config: &AppConfig,
        old_toolbar: &crate::toolbar::ToolbarState,
        new_toolbar: &crate::toolbar::ToolbarState,
        viewport: &mut ViewportOffset,
        toolbar_cell_height: usize,
        cell_size: (usize, usize),
    ) {
        let old_grid_top = grid_top(
            1.0,
            config.transparent_menubar,
            toolbar_cell_height,
            old_toolbar,
        );
        let new_grid_top = grid_top(
            1.0,
            config.transparent_menubar,
            toolbar_cell_height,
            new_toolbar,
        );
        let before = [
            Coord::default(),
            Coord { line: 3, column: 7 },
            Coord {
                line: 19,
                column: 2,
            },
        ]
        .map(|coord| canvas_screen_position(coord, old_grid_top, cell_size, *viewport));
        let horizontal = viewport.x;

        reanchor_toolbar_transition(
            viewport,
            1.0,
            config.transparent_menubar,
            toolbar_cell_height,
            old_toolbar,
            new_toolbar,
        );

        assert_eq!(viewport.x, horizontal);
        assert_eq!(
            [
                Coord::default(),
                Coord { line: 3, column: 7 },
                Coord {
                    line: 19,
                    column: 2,
                },
            ]
            .map(|coord| { canvas_screen_position(coord, new_grid_top, cell_size, *viewport) }),
            before
        );
    }

    fn apply_mouse_toolbar_transition(
        state: &mut EditorState,
        action: ToolbarAction,
        viewport: &mut ViewportOffset,
        config: &AppConfig,
        toolbar_cell_height: usize,
        cell_size: (usize, usize),
    ) {
        let old_toolbar = state.toolbar.clone();
        assert!(state.apply_toolbar_action(action));
        assert_toolbar_transition_is_anchored(
            config,
            &old_toolbar,
            &state.toolbar,
            viewport,
            toolbar_cell_height,
            cell_size,
        );
    }

    fn apply_keyboard_toolbar_transition(
        state: &mut EditorState,
        key: Key,
        viewport: &mut ViewportOffset,
        config: &AppConfig,
        toolbar_cell_height: usize,
        cell_size: (usize, usize),
    ) {
        let old_toolbar = state.toolbar.clone();
        assert!(state.handle_toolbar_shortcut(&key, ModifiersState::empty()));
        assert_toolbar_transition_is_anchored(
            config,
            &old_toolbar,
            &state.toolbar,
            viewport,
            toolbar_cell_height,
            cell_size,
        );
    }

    fn clear_after_toolbar_action_preserves_cursor_screen_position(
        state: &mut EditorState,
        viewport: ViewportOffset,
        config: &AppConfig,
        toolbar_cell_height: usize,
        cell_size: (usize, usize),
    ) -> ViewportOffset {
        let cursor = state.grid.cursor_pos;
        let current_grid_top = grid_top(
            1.0,
            config.transparent_menubar,
            toolbar_cell_height,
            &state.toolbar,
        );
        let before = canvas_screen_position(cursor, current_grid_top, cell_size, viewport);
        let mut platform = NoopExportPlatform;

        assert_eq!(
            state.toolbar.take_export_action(),
            Some(ExportAction::Clear)
        );
        assert_eq!(
            export::perform(
                ExportAction::Clear,
                state,
                &mut ViewportOffset::default(),
                VisibleCanvasCells {
                    origin: (0, 0),
                    columns: 80,
                    rows: 24,
                },
                &mut platform,
            )
            .unwrap(),
            ExportOutcome::CanvasCleared
        );
        assert_eq!(state.grid.cursor_pos, cursor);
        assert!(state.content_cells().is_empty());
        assert_eq!(
            canvas_screen_position(cursor, current_grid_top, cell_size, viewport),
            before
        );

        viewport
    }

    #[test]
    fn autosave_requires_a_change_and_more_than_five_idle_seconds() {
        let keypress = Instant::now();
        assert!(!should_autosave(
            true,
            keypress,
            keypress + Duration::from_secs(5)
        ));
        assert!(should_autosave(
            true,
            keypress,
            keypress + Duration::from_millis(5_001)
        ));
        assert!(!should_autosave(
            false,
            keypress,
            keypress + Duration::from_secs(10)
        ));
    }

    #[test]
    fn only_actual_durable_keyboard_or_mouse_selections_mark_menu_state_changed() {
        let previous = crate::toolbar::ToolbarState::default();

        let mut keyboard_prefix = previous.clone();
        assert!(
            keyboard_prefix.handle_shortcut(&Key::Character("1".into()), ModifiersState::empty(),)
        );
        assert!(!durable_menu_selections_changed(
            &previous,
            &keyboard_prefix
        ));

        let mut keyboard_selection = keyboard_prefix;
        assert!(
            keyboard_selection
                .handle_shortcut(&Key::Character("2".into()), ModifiersState::empty(),)
        );
        assert!(durable_menu_selections_changed(
            &previous,
            &keyboard_selection
        ));

        let mut mouse_selection = previous.clone();
        assert!(mouse_selection.apply_action(ToolbarAction::SelectMain(MainMode::Line)));
        assert!(durable_menu_selections_changed(&previous, &mouse_selection));

        let mut unchanged = previous.clone();
        assert!(unchanged.apply_action(ToolbarAction::SelectMain(MainMode::Stamp)));
        assert!(!durable_menu_selections_changed(&previous, &unchanged));

        let mut export = previous.clone();
        assert!(export.apply_action(ToolbarAction::ToggleExportMenu));
        assert!(!durable_menu_selections_changed(&previous, &export));
    }

    #[test]
    fn dirty_shutdown_save_writes_latest_document_without_waiting_for_idle() {
        let mut document_dirty = true;
        let mut menu_dirty = false;
        let path = Path::new("latest-document.toml");
        let lines = vec![vec![Atom {
            face: Face::default(),
            contents: "latest".into(),
        }]];
        let menu = crate::toolbar::ToolbarState::default().durable_selections();
        let mut writes = 0;

        assert!(
            save_document_if_dirty(
                &mut document_dirty,
                &mut menu_dirty,
                path,
                &lines,
                &menu,
                |saved_path, saved_lines, saved_menu| {
                    writes += 1;
                    assert_eq!(saved_path, path);
                    assert_eq!(saved_lines, lines);
                    assert_eq!(saved_menu, &menu);
                    Ok(())
                }
            )
            .unwrap()
        );

        assert_eq!(writes, 1);
        assert!(!document_dirty);
        assert!(!menu_dirty);
    }

    #[test]
    fn clean_shutdown_save_does_not_write() {
        let mut document_dirty = false;
        let mut menu_dirty = false;
        let menu = crate::toolbar::ToolbarState::default().durable_selections();

        assert!(
            !save_document_if_dirty(
                &mut document_dirty,
                &mut menu_dirty,
                Path::new("clean-document.toml"),
                &[],
                &menu,
                |_, _, _| panic!("clean documents must not be written"),
            )
            .unwrap()
        );
        assert!(!document_dirty);
        assert!(!menu_dirty);
    }

    #[test]
    fn failed_shutdown_save_keeps_document_dirty() {
        let mut document_dirty = true;
        let mut menu_dirty = true;
        let menu = crate::toolbar::ToolbarState::default().durable_selections();
        let error = save_document_if_dirty(
            &mut document_dirty,
            &mut menu_dirty,
            Path::new("failed-document.toml"),
            &[],
            &menu,
            |_, _, _| Err(anyhow!("disk full")),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "disk full");
        assert!(document_dirty);
        assert!(menu_dirty);
    }

    #[test]
    fn menu_only_shutdown_save_writes_without_marking_the_canvas_dirty() {
        let mut document_dirty = false;
        let mut menu_dirty = true;
        let mut toolbar = crate::toolbar::ToolbarState::default();
        toolbar.apply_action(ToolbarAction::SelectMain(MainMode::Utilities));
        toolbar.apply_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        });
        let menu = toolbar.durable_selections();
        let mut writes = 0;

        assert!(
            save_document_if_dirty(
                &mut document_dirty,
                &mut menu_dirty,
                Path::new("menu-only.toml"),
                &[],
                &menu,
                |_, saved_lines, saved_menu| {
                    writes += 1;
                    assert!(saved_lines.is_empty());
                    assert_eq!(saved_menu, &menu);
                    Ok(())
                },
            )
            .unwrap()
        );
        assert_eq!(writes, 1);
        assert!(!document_dirty);
        assert!(!menu_dirty);
    }

    #[test]
    fn shutdown_saves_each_dirty_document_and_continues_after_failures() {
        struct FakeDocument {
            path: &'static str,
            contents: &'static str,
            dirty: bool,
        }

        let mut documents = [
            FakeDocument {
                path: "first.toml",
                contents: "first latest",
                dirty: true,
            },
            FakeDocument {
                path: "clean.toml",
                contents: "unchanged",
                dirty: false,
            },
            FakeDocument {
                path: "failed.toml",
                contents: "failed latest",
                dirty: true,
            },
            FakeDocument {
                path: "last.toml",
                contents: "last latest",
                dirty: true,
            },
        ];
        let mut writes = Vec::new();
        let mut failures = Vec::new();

        let summary = save_on_shutdown(
            documents.iter_mut(),
            |document| {
                if !document.dirty {
                    return Ok(false);
                }
                writes.push((document.path, document.contents));
                if document.path == "failed.toml" {
                    return Err(anyhow!("failed.toml is read-only"));
                }
                document.dirty = false;
                Ok(true)
            },
            |error| failures.push(error.to_string()),
        );

        assert_eq!(
            writes,
            vec![
                ("first.toml", "first latest"),
                ("failed.toml", "failed latest"),
                ("last.toml", "last latest"),
            ]
        );
        assert_eq!(failures, vec!["failed.toml is read-only"]);
        assert_eq!(
            summary,
            ShutdownSaveSummary {
                saved: 2,
                failed: 1,
            }
        );
        assert!(!documents[0].dirty);
        assert!(!documents[1].dirty);
        assert!(documents[2].dirty);
        assert!(!documents[3].dirty);
    }

    #[test]
    fn runtime_navigation_allows_far_blank_horizontal_and_vertical_positions() {
        let content = [Coord { line: 5, column: 5 }];
        assert_eq!(
            resolve_navigation_origin(
                (0, 0),
                Coord {
                    line: 1,
                    column: 20
                },
                (24, 24),
                &content,
            ),
            Some((0, 0))
        );
        assert_eq!(
            resolve_navigation_origin(
                (0, 0),
                Coord {
                    line: 20,
                    column: 1
                },
                (24, 24),
                &content,
            ),
            Some((0, 0))
        );
    }

    #[test]
    fn runtime_navigation_rejects_only_when_cursor_visibility_needs_illegal_panning() {
        let content = [Coord { line: 5, column: 5 }];
        assert_eq!(
            resolve_navigation_origin(
                (2, 2),
                Coord {
                    line: 12,
                    column: 12
                },
                (10, 10),
                &content,
            ),
            None
        );

        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.move_to(Coord {
            line: 11,
            column: 11,
        });
        state.extend_selection(Direction::Left);
        let previous = state.clone();
        state.extend_selection(Direction::Right);
        state.extend_selection(Direction::Right);
        assert_ne!(state.selection, previous.selection);
        state = previous.clone();
        assert_eq!(state.selection, previous.selection);
        assert_eq!(state.grid.cursor_pos, previous.grid.cursor_pos);
    }

    #[test]
    fn selection_clear_keeps_the_cursor_visible_without_requiring_remaining_content() {
        let current = (2, 2);
        let cursor = Coord {
            line: 12,
            column: 12,
        };
        let viewport = (10, 10);
        let content = [Coord { line: 5, column: 5 }];

        assert_eq!(
            resolve_state_change_origin(
                StateChangeViewportPolicy::CursorAndContent,
                current,
                cursor,
                viewport,
                &content,
            ),
            None
        );
        let origin = resolve_state_change_origin(
            StateChangeViewportPolicy::CursorOnly,
            current,
            cursor,
            viewport,
            &content,
        )
        .expect("selection clearing always has a cursor-visible origin");
        assert!(cursor_is_visible(origin, cursor, viewport));
        assert!(!content_intersects_inner_screen(origin, viewport, &content));
    }

    #[test]
    fn document_change_flag_cannot_bypass_cursor_visibility() {
        let content = [Coord { line: 5, column: 5 }];
        let invisible_cursor = Coord {
            line: 12,
            column: 12,
        };

        // finish_state_change uses this same result for drawing, stamping,
        // replacing, clearing, and ordinary movement. There is deliberately no
        // document_changed fallback anymore.
        for _operation in ["draw", "stamp", "replace", "clear"] {
            assert_eq!(
                resolve_navigation_origin((2, 2), invisible_cursor, (10, 10), &content),
                None
            );
        }
    }

    #[test]
    fn rejected_selection_extension_restores_anchor_active_and_cursor_together() {
        let content = [Coord { line: 5, column: 5 }];
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.move_to(Coord {
            line: 11,
            column: 11,
        });
        state.extend_selection(Direction::Left);
        let previous = state.clone();
        state.extend_selection(Direction::Right);
        state.extend_selection(Direction::Right);
        assert_eq!(
            resolve_navigation_origin((2, 2), state.grid.cursor_pos, (10, 10), &content),
            None
        );
        state = previous.clone();
        assert_eq!(state.selection, previous.selection);
        assert_eq!(state.grid.cursor_pos, previous.grid.cursor_pos);
    }

    #[test]
    fn rejected_erasure_can_restore_document_selection_and_cursor_atomically() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.move_to(Coord { line: 5, column: 5 });
        state.insert("x");
        state.move_to(Coord {
            line: 11,
            column: 11,
        });
        state.insert("y");
        state.move_to(Coord {
            line: 11,
            column: 11,
        });
        let previous = state.clone();

        assert!(state.erase(crate::model::Direction::Right));
        assert_eq!(
            resolve_navigation_origin(
                (2, 2),
                state.grid.cursor_pos,
                (10, 10),
                &state.content_cells(),
            ),
            None
        );

        state = previous.clone();
        assert_eq!(state.grid.lines, previous.grid.lines);
        assert_eq!(state.selection, previous.selection);
        assert_eq!(state.grid.cursor_pos, previous.grid.cursor_pos);
    }

    #[test]
    fn rejected_literal_clear_can_restore_document_selection_and_cursor_atomically() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.move_to(Coord { line: 5, column: 5 });
        state.insert("x");
        state.move_to(Coord {
            line: 12,
            column: 12,
        });
        state.insert("y");
        state.move_to(Coord {
            line: 12,
            column: 12,
        });
        let previous = state.clone();

        state.clear_selection();
        assert_eq!(
            resolve_navigation_origin(
                (2, 2),
                state.grid.cursor_pos,
                (10, 10),
                &state.content_cells(),
            ),
            None
        );

        state = previous.clone();
        assert_eq!(state.grid.lines, previous.grid.lines);
        assert_eq!(state.selection, previous.selection);
        assert_eq!(state.grid.cursor_pos, previous.grid.cursor_pos);
    }

    #[test]
    fn rejected_rectangular_paste_can_restore_grid_selection_and_cursor_atomically() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.grid.lines = vec![vec![crate::model::Atom {
            face: crate::model::Face::default(),
            contents: "x".to_string(),
        }]];
        state.move_to(Coord {
            line: 11,
            column: 11,
        });
        let previous = state.clone();
        assert!(state.paste_text_rectangle(" "));
        assert_eq!(
            resolve_navigation_origin(
                (2, 2),
                state.grid.cursor_pos,
                (10, 10),
                &state.content_cells(),
            ),
            None
        );

        state = previous.clone();
        assert_eq!(state.grid.lines, previous.grid.lines);
        assert_eq!(state.selection, previous.selection);
        assert_eq!(state.grid.cursor_pos, previous.grid.cursor_pos);
    }

    #[test]
    fn rejected_utility_transform_can_restore_document_and_coordinates_atomically() {
        let mut state = EditorState::new(&AppConfig::default().theme, "test");
        state.grid.lines.resize_with(6, Vec::new);
        state.grid.lines[5].resize_with(5, || Atom {
            face: Face::default(),
            contents: " ".into(),
        });
        state.grid.lines[5].push(Atom {
            face: Face::default(),
            contents: "x".into(),
        });
        state.move_to(Coord {
            line: 11,
            column: 11,
        });
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 0,
        });
        let previous = state.clone();
        assert!(state.apply_utility(Direction::Left));
        assert_eq!(
            resolve_navigation_origin(
                (2, 2),
                state.grid.cursor_pos,
                (10, 10),
                &state.content_cells(),
            ),
            None
        );

        state = previous.clone();
        assert_eq!(state.grid.lines, previous.grid.lines);
        assert_eq!(state.selection, previous.selection);
        assert_eq!(state.grid.cursor_pos, previous.grid.cursor_pos);
    }

    #[test]
    fn load_reset_origin_is_normalized_instead_of_blindly_kept_at_zero() {
        let content = [Coord {
            line: 40,
            column: 50,
        }];
        let (cursor, origin) =
            normalized_cursor_and_origin((0, 0), Coord::default(), (10, 10), &content);
        assert_eq!(cursor, content[0]);
        assert!(cursor_is_visible(origin, cursor, (10, 10)));
        assert!(content_intersects_inner_screen(origin, (10, 10), &content));
    }

    #[test]
    fn toolbar_height_or_zoom_row_reduction_reuses_resize_normalization() {
        let content = [Coord { line: 5, column: 5 }];
        let cursor = Coord {
            line: 20,
            column: 1,
        };
        let (cursor_before, _) = normalized_cursor_and_origin((0, 0), cursor, (24, 24), &content);
        let (cursor_after, origin_after) =
            normalized_cursor_and_origin((0, 0), cursor, (24, 10), &content);
        assert_eq!(cursor_before, cursor);
        assert_eq!(cursor_after, content[0]);
        assert!(cursor_is_visible(origin_after, cursor_after, (24, 10)));
    }

    #[test]
    fn keyboard_and_mouse_mode_height_changes_keep_every_canvas_cell_anchored() {
        let config = AppConfig::default();
        let (toolbar_cell_height, cell_size) = toolbar_test_metrics(&config);
        let mut state = EditorState::new(&config.theme, "test");
        let mut viewport = ViewportOffset { x: -13, y: 17 };

        for mode in [MainMode::Utilities, MainMode::Stamp, MainMode::Shapes] {
            apply_mouse_toolbar_transition(
                &mut state,
                ToolbarAction::SelectMain(mode),
                &mut viewport,
                &config,
                toolbar_cell_height,
                cell_size,
            );
            apply_mouse_toolbar_transition(
                &mut state,
                ToolbarAction::SelectMain(MainMode::Line),
                &mut viewport,
                &config,
                toolbar_cell_height,
                cell_size,
            );
        }

        for digit in ["2", "3", "4", "1"] {
            apply_keyboard_toolbar_transition(
                &mut state,
                Key::Character("1".into()),
                &mut viewport,
                &config,
                toolbar_cell_height,
                cell_size,
            );
            apply_keyboard_toolbar_transition(
                &mut state,
                Key::Character(digit.into()),
                &mut viewport,
                &config,
                toolbar_cell_height,
                cell_size,
            );
        }

        assert_eq!(viewport, ViewportOffset { x: -13, y: 17 });
    }

    #[test]
    fn large_line_and_compact_utils_menu_cycles_have_no_drift() {
        let config = AppConfig::default();
        let (toolbar_cell_height, cell_size) = toolbar_test_metrics(&config);
        let mut state = EditorState::new(&config.theme, "test");
        let initial = ViewportOffset { x: 21, y: -37 };
        let mut viewport = initial;

        assert!(
            state.toolbar.rows() > {
                let mut compact = state.toolbar.clone();
                compact.apply_action(ToolbarAction::SelectMain(MainMode::Utilities));
                compact.rows()
            }
        );
        for _ in 0..20 {
            apply_mouse_toolbar_transition(
                &mut state,
                ToolbarAction::SelectMain(MainMode::Utilities),
                &mut viewport,
                &config,
                toolbar_cell_height,
                cell_size,
            );
            apply_mouse_toolbar_transition(
                &mut state,
                ToolbarAction::SelectMain(MainMode::Line),
                &mut viewport,
                &config,
                toolbar_cell_height,
                cell_size,
            );
        }

        assert_eq!(viewport, initial);
    }

    #[test]
    fn export_actions_stay_open_without_drift_and_escape_round_trips() {
        let config = AppConfig::default();
        let (toolbar_cell_height, cell_size) = toolbar_test_metrics(&config);
        let mut state = EditorState::new(&config.theme, "test");
        let initial = ViewportOffset { x: -8, y: 29 };
        let mut viewport = initial;

        apply_keyboard_toolbar_transition(
            &mut state,
            Key::Character("0".into()),
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        apply_keyboard_toolbar_transition(
            &mut state,
            Key::Character("2".into()),
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        apply_keyboard_toolbar_transition(
            &mut state,
            Key::Named(NamedKey::Escape),
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        assert_eq!(viewport, initial);

        apply_mouse_toolbar_transition(
            &mut state,
            ToolbarAction::ToggleExportMenu,
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        let export_viewport = viewport;
        for _ in 0..12 {
            apply_mouse_toolbar_transition(
                &mut state,
                ToolbarAction::SelectExportCategory(3),
                &mut viewport,
                &config,
                toolbar_cell_height,
                cell_size,
            );
            assert_eq!(viewport, export_viewport);
        }
        apply_keyboard_toolbar_transition(
            &mut state,
            Key::Named(NamedKey::Escape),
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        assert_eq!(viewport, initial);
    }

    #[test]
    fn export_close_then_document_edit_records_one_coherent_anchored_viewport() {
        let config = AppConfig::default();
        let (toolbar_cell_height, cell_size) = toolbar_test_metrics(&config);
        let mut state = EditorState::new(&config.theme, "test");
        state.insert("ragged\nx\nfar drawing");
        let cursor = Coord { line: 2, column: 7 };
        state.move_to(cursor);
        let mut viewport = ViewportOffset {
            x: -(cursor.column as i64 * cell_size.0 as i64) + cell_size.0 as i64 * 4,
            y: -(cursor.line as i64 * cell_size.1 as i64) + cell_size.1 as i64 * 3,
        };

        apply_mouse_toolbar_transition(
            &mut state,
            ToolbarAction::ToggleExportMenu,
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        apply_mouse_toolbar_transition(
            &mut state,
            ToolbarAction::RunExport(ExportAction::Clear),
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        let closed_viewport = viewport;
        let previous = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport,
        };
        viewport = clear_after_toolbar_action_preserves_cursor_screen_position(
            &mut state,
            viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        let current = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport,
        };
        let mut history = EditHistory::default();

        assert!(history.record_change(previous.clone(), &current));
        assert_eq!(history.undo(current.clone()), Some(previous.clone()));
        assert_eq!(history.redo(previous), Some(current));
        assert_eq!(closed_viewport, viewport);
    }

    #[test]
    fn keyboard_and_repeated_mouse_clear_do_not_drift() {
        let config = AppConfig::default();
        let (toolbar_cell_height, cell_size) = toolbar_test_metrics(&config);

        for use_keyboard in [true, false] {
            let mut state = EditorState::new(&config.theme, "test");
            state.insert("drawing");
            let cursor = Coord { line: 4, column: 9 };
            state.move_to(cursor);
            let initial = ViewportOffset {
                x: -(cursor.column as i64 * cell_size.0 as i64) + cell_size.0 as i64 * 5,
                y: -(cursor.line as i64 * cell_size.1 as i64) + cell_size.1 as i64 * 4,
            };
            let initial_grid_top = grid_top(
                1.0,
                config.transparent_menubar,
                toolbar_cell_height,
                &state.toolbar,
            );
            let initial_screen =
                canvas_screen_position(cursor, initial_grid_top, cell_size, initial);
            let mut viewport = initial;

            if use_keyboard {
                apply_keyboard_toolbar_transition(
                    &mut state,
                    Key::Character("0".into()),
                    &mut viewport,
                    &config,
                    toolbar_cell_height,
                    cell_size,
                );
            } else {
                apply_mouse_toolbar_transition(
                    &mut state,
                    ToolbarAction::ToggleExportMenu,
                    &mut viewport,
                    &config,
                    toolbar_cell_height,
                    cell_size,
                );
            }
            let export_viewport = viewport;
            let export_grid_top = grid_top(
                1.0,
                config.transparent_menubar,
                toolbar_cell_height,
                &state.toolbar,
            );

            for _ in 0..4 {
                if use_keyboard {
                    apply_keyboard_toolbar_transition(
                        &mut state,
                        Key::Character("9".into()),
                        &mut viewport,
                        &config,
                        toolbar_cell_height,
                        cell_size,
                    );
                } else {
                    apply_mouse_toolbar_transition(
                        &mut state,
                        ToolbarAction::ToggleExportMenu,
                        &mut viewport,
                        &config,
                        toolbar_cell_height,
                        cell_size,
                    );
                    apply_mouse_toolbar_transition(
                        &mut state,
                        ToolbarAction::RunExport(ExportAction::Clear),
                        &mut viewport,
                        &config,
                        toolbar_cell_height,
                        cell_size,
                    );
                }
                viewport = clear_after_toolbar_action_preserves_cursor_screen_position(
                    &mut state,
                    viewport,
                    &config,
                    toolbar_cell_height,
                    cell_size,
                );
                assert_eq!(viewport, export_viewport);
                assert_eq!(
                    canvas_screen_position(cursor, export_grid_top, cell_size, viewport),
                    initial_screen
                );
            }
        }
    }

    #[test]
    fn clearing_styled_whitespace_removes_its_face_without_moving_cursor_or_viewport() {
        let config = AppConfig::default();
        let (toolbar_cell_height, cell_size) = toolbar_test_metrics(&config);
        let mut state = EditorState::new(&config.theme, "test");
        state.grid.lines = vec![vec![Atom {
            face: config.theme.selection.clone(),
            contents: " ".into(),
        }]];
        let cursor = Coord { line: 3, column: 6 };
        state.move_to(cursor);
        let initial = ViewportOffset {
            x: -(cursor.column as i64 * cell_size.0 as i64) + cell_size.0 as i64 * 4,
            y: -(cursor.line as i64 * cell_size.1 as i64) + cell_size.1 as i64 * 3,
        };
        let initial_grid_top = grid_top(
            1.0,
            config.transparent_menubar,
            toolbar_cell_height,
            &state.toolbar,
        );
        let screen = canvas_screen_position(cursor, initial_grid_top, cell_size, initial);
        let mut viewport = initial;

        apply_mouse_toolbar_transition(
            &mut state,
            ToolbarAction::ToggleExportMenu,
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        apply_mouse_toolbar_transition(
            &mut state,
            ToolbarAction::RunExport(ExportAction::Clear),
            &mut viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );
        viewport = clear_after_toolbar_action_preserves_cursor_screen_position(
            &mut state,
            viewport,
            &config,
            toolbar_cell_height,
            cell_size,
        );

        let export_grid_top = grid_top(
            1.0,
            config.transparent_menubar,
            toolbar_cell_height,
            &state.toolbar,
        );
        assert_eq!(
            canvas_screen_position(cursor, export_grid_top, cell_size, viewport),
            screen
        );
        assert!(
            state
                .grid
                .lines
                .iter()
                .flatten()
                .all(|atom| atom.face == Face::default())
        );
    }
}
