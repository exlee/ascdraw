use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[cfg(test)]
use anyhow::anyhow;
use anyhow::{Context, Result};
use winit::event::{MouseScrollDelta, TouchPhase};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{AppCommand, AppConfig};
use crate::diagnostics::log_error;
use crate::document;
use crate::editor::Editor;
use crate::export::{
    FileKind, lines_from_text, load_project_json, plain_text, project_json_contents,
    save_project_json,
};
use crate::history::{EditHistory, HistoryGroup, HistorySnapshot};
use crate::input::EditCommand;
use crate::input::{OrderedModifierTracker, ViewCommand};
use crate::jump::JumpViewportPan;
use crate::layout::{
    LayoutMetrics, PADDING, ViewportOffset, VisibleCanvasCells, constrained_origin,
    content_intersects_inner_screen, content_top_padding, cursor_is_visible, cursor_origin,
    layout_metrics, navigation_origin, normalized_cursor_and_origin,
};
#[cfg(target_os = "macos")]
use crate::macos;
use crate::model::{Coord, Direction};
use crate::perf::{FrameTiming, PerfDiagnostics, PerfSnapshot};
use crate::render::{Renderer, WindowSurface, load_renderer, render_canvas_layers_image};
use crate::runtime::background::{BackgroundSender, BackgroundWorker};
use crate::title_policy::window_attributes;
use crate::toolbar_stamp::toolbar_hotspot_at;
use crate::user_keys::FontSizeAction;

const EXPORT_SUCCESS_HIGHLIGHT_DURATION: Duration = Duration::from_millis(650);

#[derive(Clone, Debug)]
pub enum DocumentSession {
    Scratchpad(PathBuf),
    File(PathBuf),
    TextFile(PathBuf),
    JsonFile(PathBuf),
    Stdin(String),
}

impl DocumentSession {
    pub fn file(path: PathBuf) -> Self {
        Self::File(path)
    }

    pub fn scratchpad(path: PathBuf) -> Self {
        Self::Scratchpad(path)
    }

    fn is_stdin(&self) -> bool {
        matches!(self, Self::Stdin(_))
    }

    pub(crate) fn allows_document_history(&self) -> bool {
        !self.is_stdin()
    }

    fn path(&self) -> Option<&Path> {
        match self {
            Self::Scratchpad(path)
            | Self::File(path)
            | Self::TextFile(path)
            | Self::JsonFile(path) => Some(path),
            Self::Stdin(_) => None,
        }
    }

    pub(crate) fn window_title(&self) -> String {
        match self {
            Self::Scratchpad(_) => "ascdraw - scratchpad".to_owned(),
            Self::File(path) | Self::TextFile(path) | Self::JsonFile(path) => format!(
                "ascdraw - {}",
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
            ),
            Self::Stdin(_) => "ascdraw - stdin".to_owned(),
        }
    }

    pub(crate) fn explicit_file_path(&self) -> Option<&Path> {
        match self {
            Self::File(path) | Self::TextFile(path) | Self::JsonFile(path) => Some(path),
            Self::Scratchpad(_) | Self::Stdin(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ViewCursorAnchor {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateChangeViewportPolicy {
    CursorAndContent,
    CursorOnly,
    Stable,
}

impl ViewCursorAnchor {
    fn capture(
        cursor: Coord,
        viewport: ViewportOffset,
        cell_size: (f32, f32),
        grid_top: f32,
    ) -> Self {
        Self {
            x: PADDING as f64 + cursor.column as f64 * cell_size.0 as f64 + viewport.x as f64,
            y: grid_top as f64 + cursor.line as f64 * cell_size.1 as f64 + viewport.y as f64,
        }
    }

    fn cursor_for_viewport(
        self,
        viewport: ViewportOffset,
        cell_size: (f32, f32),
        grid_top: f32,
    ) -> (i64, i64) {
        (
            ((self.y - grid_top as f64 - viewport.y as f64) / cell_size.1.max(1.0) as f64).floor()
                as i64,
            ((self.x - PADDING as f64 - viewport.x as f64) / cell_size.0.max(1.0) as f64).floor()
                as i64,
        )
    }

    fn restore_for_cursor(
        self,
        viewport: &mut ViewportOffset,
        cursor: Coord,
        cell_size: (f32, f32),
        grid_top: f32,
    ) {
        viewport.x =
            (self.x - PADDING as f64 - cursor.column as f64 * cell_size.0 as f64).round() as i64;
        viewport.y =
            (self.y - grid_top as f64 - cursor.line as f64 * cell_size.1 as f64).round() as i64;
    }
}

pub struct EditorWindow {
    pub window: Rc<Window>,
    pub surface: WindowSurface,
    pub modifiers: ModifiersState,
    pub ordered_modifiers: OrderedModifierTracker,
    pub mouse_position: Option<(f64, f64)>,
    pub mouse_cell: Option<(i64, i64)>,
    pub mouse_toolbar_position: Option<(usize, usize, usize)>,
    mouse_toolbar_hotspot: Option<usize>,
    mouse_drag: Option<MouseDrag>,
    last_line_click: Option<(Instant, Coord)>,
    scroll_pan: ScrollPan,
    wheel_zoom_remainder: f64,
    #[cfg(debug_assertions)]
    scroll_stats: ScrollStats,
    background: BackgroundSender,
    pub state: Editor,
    pub renderer: Renderer,
    pub viewport: ViewportOffset,
    view_cursor_anchor: Option<ViewCursorAnchor>,
    history: EditHistory,
    content_index: ContentIndex,
    perf: PerfDiagnostics,
    transparent_menubar: bool,
    document_session: DocumentSession,
    document_dirty: bool,
    menu_selections_dirty: bool,
    saved_canvas_position: document::CanvasPosition,
    last_keypress: Instant,
    export_success_deadline: Option<Instant>,
    autosave_in_flight: Option<PendingAutosave>,
}

struct PendingAutosave {
    position: document::CanvasPosition,
    document_dirty: bool,
    menu_selections_dirty: bool,
}

#[derive(Debug)]
struct ContentIndex {
    cells: Vec<Coord>,
    cells_including_hidden: Vec<Coord>,
    dirty: bool,
    #[cfg(test)]
    rebuilds: usize,
}

impl ContentIndex {
    fn new(state: &Editor) -> Self {
        Self {
            cells: state.content_cells(),
            cells_including_hidden: state.content_cells_including_hidden(),
            dirty: false,
            #[cfg(test)]
            rebuilds: 1,
        }
    }

    fn invalidate(&mut self) {
        self.dirty = true;
    }

    fn refresh(&mut self, state: &Editor) {
        if self.dirty {
            self.cells = state.content_cells();
            self.cells_including_hidden = state.content_cells_including_hidden();
            self.dirty = false;
            #[cfg(test)]
            {
                self.rebuilds += 1;
            }
        }
    }

    fn cells(&self) -> &[Coord] {
        &self.cells
    }

    fn cells_including_hidden(&self) -> &[Coord] {
        &self.cells_including_hidden
    }

    #[cfg(test)]
    fn rebuilds(&self) -> usize {
        self.rebuilds
    }
}

#[cfg(debug_assertions)]
#[derive(Debug)]
struct ScrollStats {
    enabled: bool,
    scroll_events: u64,
    input_events: u64,
    redraws: u64,
    input_time: Duration,
    buffer_time: Duration,
    raster_time: Duration,
    presentation_time: Duration,
    toolbar_time: Duration,
    grid_time: Duration,
    minimap_time: Duration,
    started: Instant,
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug)]
struct ScrollStatsReport {
    scroll_events: u64,
    input_events: u64,
    redraws: u64,
    input_time: Duration,
    buffer_time: Duration,
    raster_time: Duration,
    presentation_time: Duration,
    toolbar_time: Duration,
    grid_time: Duration,
    minimap_time: Duration,
}

#[cfg(debug_assertions)]
impl ScrollStats {
    fn new(enabled: bool, now: Instant) -> Self {
        Self {
            enabled,
            scroll_events: 0,
            input_events: 0,
            redraws: 0,
            input_time: Duration::ZERO,
            buffer_time: Duration::ZERO,
            raster_time: Duration::ZERO,
            presentation_time: Duration::ZERO,
            toolbar_time: Duration::ZERO,
            grid_time: Duration::ZERO,
            minimap_time: Duration::ZERO,
            started: now,
        }
    }

    fn note_scroll_event(&mut self, now: Instant) -> Option<ScrollStatsReport> {
        let report = self.advance(now);
        if self.enabled {
            self.scroll_events = self.scroll_events.saturating_add(1);
        }
        report
    }

    fn note_redraw(&mut self, now: Instant) -> Option<ScrollStatsReport> {
        let report = self.advance(now);
        if self.enabled {
            self.redraws = self.redraws.saturating_add(1);
        }
        report
    }

    fn note_input_event(&mut self, duration: Duration) {
        if self.enabled {
            self.input_events = self.input_events.saturating_add(1);
            self.input_time += duration;
        }
    }

    fn note_frame(&mut self, timing: FrameTiming, now: Instant) -> Option<ScrollStatsReport> {
        if self.enabled {
            self.buffer_time += timing.buffer_acquisition;
            self.raster_time += timing.rasterization;
            self.presentation_time += timing.presentation;
            self.toolbar_time += timing.toolbar;
            self.grid_time += timing.grid;
            self.minimap_time += timing.minimap;
        }
        self.advance(now)
    }

    fn advance(&mut self, now: Instant) -> Option<ScrollStatsReport> {
        if !self.enabled || now.saturating_duration_since(self.started) < Duration::from_secs(1) {
            return None;
        }
        let report = ScrollStatsReport {
            scroll_events: std::mem::take(&mut self.scroll_events),
            input_events: std::mem::take(&mut self.input_events),
            redraws: std::mem::take(&mut self.redraws),
            input_time: std::mem::take(&mut self.input_time),
            buffer_time: std::mem::take(&mut self.buffer_time),
            raster_time: std::mem::take(&mut self.raster_time),
            presentation_time: std::mem::take(&mut self.presentation_time),
            toolbar_time: std::mem::take(&mut self.toolbar_time),
            grid_time: std::mem::take(&mut self.grid_time),
            minimap_time: std::mem::take(&mut self.minimap_time),
        };
        self.started = now;
        (report.redraws > 1).then_some(report)
    }
}

#[cfg(debug_assertions)]
fn format_scroll_stats(report: ScrollStatsReport) -> String {
    let frames = report.redraws as f64;
    let milliseconds = |duration: Duration| duration.as_secs_f64() * 1_000.0 / frames;
    let input_milliseconds =
        report.input_time.as_secs_f64() * 1_000.0 / report.input_events.max(1) as f64;
    let measured = report.toolbar_time + report.grid_time + report.minimap_time;
    let other = report.raster_time.saturating_sub(measured);
    format!(
        "debug: scroll={}/s input={}/s input_handler={:.2}ms redraw={}/s frame={:.2}ms [buffer={:.2} raster={:.2} present={:.2}] raster=[toolbar={:.2} grid={:.2} minimap={:.2} other={:.2}]",
        report.scroll_events,
        report.input_events,
        input_milliseconds,
        report.redraws,
        milliseconds(report.buffer_time + report.raster_time + report.presentation_time),
        milliseconds(report.buffer_time),
        milliseconds(report.raster_time),
        milliseconds(report.presentation_time),
        milliseconds(report.toolbar_time),
        milliseconds(report.grid_time),
        milliseconds(report.minimap_time),
        milliseconds(other),
    )
}

#[derive(Debug, Clone)]
struct MouseDrag {
    previous_state: Editor,
    previous_viewport: ViewportOffset,
    last_pointer: Coord,
    active: bool,
    document_changed: bool,
    input_override: Option<MouseDragOverride>,
    line_preview_was_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseDragOverride {
    Control,
    Line,
    Space,
}

const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(400);

fn is_line_double_click(
    previous: Option<(Instant, Coord)>,
    now: Instant,
    coord: Coord,
    moved: bool,
) -> bool {
    !moved
        && previous.is_some_and(|(at, previous_coord)| {
            previous_coord == coord && now.saturating_duration_since(at) <= DOUBLE_CLICK_INTERVAL
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineMousePress {
    preview_was_active: bool,
    cursor_moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineMouseFinish {
    document_changed: bool,
    next_click: Option<(Instant, Coord)>,
}

fn begin_line_mouse_state(
    state: &mut Editor,
    coord: Coord,
    target: Coord,
    allow_cursor_move: bool,
) -> LineMousePress {
    let preview_was_active = state.has_line_preview();
    let wants_cursor_move = !preview_was_active && state.grid.cursor_pos != target;
    let cursor_moved = wants_cursor_move && allow_cursor_move;
    if preview_was_active {
        state.move_line_preview_to(target);
    } else if cursor_moved {
        state.move_to(coord);
    } else if !wants_cursor_move {
        state.start_or_advance_line_preview();
    }
    LineMousePress {
        preview_was_active,
        cursor_moved,
    }
}

fn continue_line_mouse_state(state: &mut Editor, target: Coord) {
    if !state.has_line_preview() {
        state.start_or_advance_line_preview();
    }
    state.move_line_preview_to(target);
}

fn finish_line_mouse_state(
    state: &mut Editor,
    coord: Coord,
    drag_active: bool,
    preview_was_active: bool,
    previous_click: Option<(Instant, Coord)>,
    now: Instant,
) -> LineMouseFinish {
    let moved = state
        .line_preview_anchor()
        .is_some_and(|anchor| anchor != coord);
    let double_click =
        preview_was_active && is_line_double_click(previous_click, now, coord, moved);
    let document_changed = if moved {
        state.start_or_advance_line_preview()
    } else if double_click {
        state.finish_line_preview()
    } else {
        false
    };
    if drag_active && moved {
        state.finish_line_preview();
    }
    let next_click = (state.has_line_preview() && preview_was_active).then_some((now, coord));
    LineMouseFinish {
        document_changed,
        next_click,
    }
}

#[derive(Debug, Default)]
struct ScrollPan {
    x: f64,
    y: f64,
}

impl ScrollPan {
    fn queue(&mut self, delta: (f64, f64)) {
        self.x += delta.0;
        self.y += delta.1;
    }

    fn next_step(&self, _cell_size: (f32, f32)) -> (i64, i64) {
        (self.x.trunc() as i64, self.y.trunc() as i64)
    }

    fn consume(&mut self, requested: (i64, i64), applied: (i64, i64)) {
        consume_scroll_axis(&mut self.x, requested.0, applied.0);
        consume_scroll_axis(&mut self.y, requested.1, applied.1);
    }

    fn is_active(&self) -> bool {
        self.x.abs() >= 1.0 || self.y.abs() >= 1.0
    }
}

fn finish_mouse_drag_state(state: &mut Editor, input_override: Option<MouseDragOverride>) -> bool {
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

    pub fn begin_mouse_drag(&mut self, pointer: (i64, i64)) {
        self.state.cancel_jump();
        let input_override = if self.modifiers == ModifiersState::empty() {
            match (self.state.toolbar.main_mode(), self.state.cursor_mode) {
                (crate::toolbar::MainMode::Line, crate::app::CursorMode::MoveDraw) => {
                    Some(MouseDragOverride::Line)
                }
                (_, crate::app::CursorMode::MoveDraw) => Some(MouseDragOverride::Control),
                (_, crate::app::CursorMode::Stamp | crate::app::CursorMode::Shapes) => {
                    Some(MouseDragOverride::Space)
                }
                _ => None,
            }
        } else {
            None
        };
        let previous_state = self.state.clone();
        let previous_viewport = self.viewport;
        let coord = self.state.resolve_pointer_coord(pointer.0, pointer.1);
        let target = self.state.cursor_target_for_coord(coord);
        let mut line_preview_was_active = false;
        let mut confirmed_move = false;
        let extending_selection = self.modifiers.shift_key();
        if input_override == Some(MouseDragOverride::Line) {
            let moves_cursor =
                !self.state.has_line_preview() && self.state.grid.cursor_pos != target;
            let origin = if moves_cursor {
                self.navigation_origin_for(target)
            } else {
                None
            };
            if origin.is_some() {
                self.finish_history_transaction();
            }
            let press = begin_line_mouse_state(
                &mut self.state,
                coord,
                target,
                !moves_cursor || origin.is_some(),
            );
            line_preview_was_active = press.preview_was_active;
            debug_assert_eq!(press.cursor_moved, moves_cursor && origin.is_some());
            if let Some(origin) = origin {
                self.finish_navigation(origin);
            }
            self.request_redraw();
        } else {
            let preserve_selection =
                self.modifiers == ModifiersState::ALT && !self.state.selection.is_collapsed();
            confirmed_move = self
                .state
                .move_lift_bounds()
                .is_some_and(|bounds| !bounds.contains(target))
                && self.state.confirm_move_lift();
            if extending_selection {
                self.state.move_to(target);
                self.request_redraw();
            } else if !preserve_selection && let Some(origin) = self.navigation_origin_for(target) {
                self.finish_history_transaction();
                self.state.move_to(coord);
                self.finish_navigation(origin);
                self.request_redraw();
            }
        }
        self.mouse_drag = Some(MouseDrag {
            previous_state,
            previous_viewport,
            last_pointer: target,
            active: extending_selection,
            document_changed: confirmed_move,
            input_override,
            line_preview_was_active,
        });
    }

    pub fn continue_mouse_drag(&mut self) {
        let Some(pointer) = self.mouse_cell else {
            return;
        };
        let coord = self.state.resolve_pointer_coord(pointer.0, pointer.1);
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
        if drag.input_override == Some(MouseDragOverride::Line) {
            continue_line_mouse_state(&mut self.state, target);
            drag.last_pointer = target;
            self.mouse_drag = Some(drag);
            self.request_redraw();
            return;
        }
        let (modifiers, space_held) = match drag.input_override {
            Some(MouseDragOverride::Control) => (ModifiersState::CONTROL, false),
            Some(MouseDragOverride::Space) => (ModifiersState::empty(), true),
            Some(MouseDragOverride::Line) => unreachable!("line drags return above"),
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
        if drag.input_override == Some(MouseDragOverride::Line) {
            self.finish_line_mouse_gesture(drag);
            return;
        }
        if !drag.active && !drag.document_changed {
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

    fn finish_line_mouse_gesture(&mut self, drag: MouseDrag) {
        let now = Instant::now();
        let coord = drag.last_pointer;
        let finish = finish_line_mouse_state(
            &mut self.state,
            coord,
            drag.active,
            drag.line_preview_was_active,
            self.last_line_click,
            now,
        );
        let recorded = self.finish_grouped_state_change(
            drag.previous_state,
            drag.previous_viewport,
            finish.document_changed,
            HistoryGroup::LineRoute,
        );
        if !self.state.has_line_preview() {
            self.finish_history_transaction();
        }
        self.last_line_click = finish.next_click;
        if recorded {
            self.mark_document_dirty();
        }
        self.request_redraw();
    }

    pub fn continue_passive_line_preview(&mut self) {
        if self.mouse_drag.is_none()
            && self.modifiers == ModifiersState::empty()
            && let Some((line, column)) = self.mouse_cell
            && let Some(coord) = usize::try_from(line)
                .ok()
                .zip(usize::try_from(column).ok())
                .map(|(line, column)| Coord { line, column })
            && self.state.move_line_preview_to(coord)
        {
            self.request_redraw();
        }
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub fn note_scroll_event(&mut self) {
        #[cfg(debug_assertions)]
        if let Some(report) = self.scroll_stats.note_scroll_event(Instant::now()) {
            self.background.debug_output(format_scroll_stats(report));
        }
    }

    pub fn note_redraw(&mut self) {
        #[cfg(debug_assertions)]
        if let Some(report) = self.scroll_stats.note_redraw(Instant::now()) {
            self.background.debug_output(format_scroll_stats(report));
        }
    }

    pub fn report_scroll_event_stats(&mut self, _now: Instant) {
        #[cfg(debug_assertions)]
        if let Some(report) = self.scroll_stats.advance(_now) {
            self.background.debug_output(format_scroll_stats(report));
        }
    }

    #[cfg(debug_assertions)]
    pub fn report_render_cache_usage(&self) {
        let (bytes, used, capacity) = self.renderer.rendered_atom_cache_usage();
        self.background.debug_output(format!(
            "debug: rendered atom cache bytes={bytes} slots={used}/{capacity}"
        ));
    }

    pub fn set_mouse_toolbar_hotspot(&mut self, hotspot: Option<usize>) {
        if self.mouse_toolbar_hotspot != hotspot {
            self.mouse_toolbar_hotspot = hotspot;
            self.request_redraw();
        }
    }

    pub fn mouse_toolbar_hotspot(&self) -> Option<usize> {
        self.mouse_toolbar_hotspot
    }

    pub fn toolbar_hotspot_hovered(&self) -> bool {
        self.mouse_toolbar_hotspot.is_some()
    }

    pub fn apply_config(&mut self, config: &AppConfig) {
        let scale_factor = self.window.scale_factor();
        let old_metrics = self.renderer.metrics(scale_factor);
        let old_toolbar_metrics = self.renderer.title_metrics(scale_factor);
        let viewport_width = self.window.inner_size().width as usize;
        let old_grid_top = grid_top_for_width(
            scale_factor,
            self.transparent_menubar,
            viewport_width,
            (
                old_toolbar_metrics.cell_width,
                old_toolbar_metrics.cell_height,
            ),
            &self.state.toolbar,
        );
        self.renderer.apply_config(config);
        let new_metrics = self.renderer.metrics(scale_factor);
        let new_toolbar_metrics = self.renderer.title_metrics(scale_factor);
        let new_grid_top = grid_top_for_width(
            scale_factor,
            config.transparent_menubar,
            viewport_width,
            (
                new_toolbar_metrics.cell_width,
                new_toolbar_metrics.cell_height,
            ),
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
        if let Some((x, y)) = self.mouse_position {
            let metrics = self.renderer.title_metrics(scale_factor);
            self.mouse_toolbar_hotspot = toolbar_hotspot_at(
                x,
                y,
                self.window.inner_size().width as usize,
                metrics.cell_width,
                metrics.cell_height,
                content_top_padding(scale_factor, config.transparent_menubar),
            );
        }
        self.request_redraw();
    }

    pub fn note_keypress(&mut self, now: Instant) {
        self.last_keypress = now;
        self.perf.begin_keypress(now);
    }

    pub fn note_input_event(&mut self, duration: Duration) {
        #[cfg(debug_assertions)]
        self.scroll_stats.note_input_event(duration);
        #[cfg(not(debug_assertions))]
        let _ = duration;
    }

    pub fn record_state_history_time(&mut self, started: Instant) {
        self.perf.record_state_history(started.elapsed());
    }

    pub fn finish_keypress(&mut self, now: Instant) {
        self.perf.finish_keypress(now);
    }

    pub fn record_present(&mut self, timing: FrameTiming, now: Instant) {
        #[cfg(debug_assertions)]
        if let Some(report) = self.scroll_stats.note_frame(timing, now) {
            self.background.debug_output(format_scroll_stats(report));
        }
        self.perf.record_present(timing, now);
    }

    pub fn enable_automation_metrics(&mut self) {
        self.perf.enable();
    }

    pub fn perf_snapshot(&mut self, reset: bool) -> PerfSnapshot {
        let snapshot = self.perf.snapshot();
        if reset {
            self.perf.reset();
        }
        snapshot
    }

    pub fn automation_state(&self) -> serde_json::Value {
        let size = self.window.inner_size();
        let selection = self.state.selection.bounds();
        serde_json::json!({
            "window_id": format!("{:?}", self.window_id()),
            "window": {
                "width": size.width,
                "height": size.height,
                "scale_factor": self.window.scale_factor(),
                "renderer": self.surface.backend_name(),
            },
            "cursor": self.state.grid.cursor_pos,
            "selection": selection,
            "selection_collapsed": self.state.selection.is_collapsed(),
            "viewport": { "x": self.viewport.x, "y": self.viewport.y },
            "cursor_mode": format!("{:?}", self.state.cursor_mode),
            "editor_state": format!("{:?}", self.state.state()),
            "active_layer": self.state.active_layer_id().0,
            "layers": self.state.layer_summaries().len(),
            "content_cells": self.state.content_cells().len(),
            "document_dirty": self.document_dirty,
        })
    }

    pub fn capture_canvas(&self, path: &Path, config: &AppConfig) -> Result<(usize, usize)> {
        let layers = self
            .state
            .canvas()
            .effective_layers()
            .iter()
            .filter(|layer| layer.visible)
            .map(crate::canvas::LayerMap::to_dense)
            .collect::<Vec<_>>();
        let image = render_canvas_layers_image(
            &self.renderer,
            &layers,
            &self.state.grid.default_face,
            self.window.scale_factor(),
            config.macos.color_space,
        )?;
        std::fs::write(path, &image.png)
            .with_context(|| format!("failed to write screenshot {}", path.display()))?;
        Ok((image.width, image.height))
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

    pub fn mark_document_dirty(&mut self) {
        self.document_dirty = true;
    }

    fn refresh_content_index(&mut self) {
        self.content_index.refresh(&self.state);
    }

    pub fn render(&mut self, config: &AppConfig) -> Result<FrameTiming> {
        self.refresh_content_index();
        let toolbar_hotspot_hovered = self.toolbar_hotspot_hovered();
        self.surface.render(
            &self.window,
            &self.state,
            &self.renderer,
            config,
            crate::render::RenderContext {
                content: crate::render::CanvasContent {
                    visible: self.content_index.cells(),
                    including_hidden: self.content_index.cells_including_hidden(),
                },
                viewport: self.viewport,
                toolbar_hotspot_hovered,
            },
        )
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
        self.refresh_content_index();
        let content = self.content_index.cells();
        resolve_navigation_origin(
            self.viewport.origin(cell_size),
            cursor,
            (layout.cols.max(1), layout.rows.max(1)),
            content,
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
        previous_state: Editor,
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
        previous_state: Editor,
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

    pub fn finish_state_change_with_stable_viewport(
        &mut self,
        previous_state: Editor,
        previous_viewport: ViewportOffset,
        document_changed: bool,
    ) -> bool {
        self.finish_state_change_in_group(
            previous_state,
            previous_viewport,
            document_changed,
            None,
            StateChangeViewportPolicy::Stable,
        )
    }

    pub fn finish_grouped_state_change(
        &mut self,
        previous_state: Editor,
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
        mut previous_state: Editor,
        previous_viewport: ViewportOffset,
        document_changed: bool,
        group: Option<HistoryGroup>,
        viewport_policy: StateChangeViewportPolicy,
    ) -> bool {
        previous_state
            .commit_canvas_mutations()
            .expect("editor cells remain valid at history boundaries");
        self.state
            .commit_canvas_mutations()
            .expect("editor cells remain valid at history boundaries");
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
            self.window.inner_size().width as usize,
            (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
            &previous_state.toolbar,
            &self.state.toolbar,
        );
        if document_changed {
            self.content_index.invalidate();
        }
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
        if prepend != (0, 0) {
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
        self.refresh_content_index();
        let content = self.content_index.cells();

        if let Some(origin) = resolve_state_change_origin(
            viewport_policy,
            current,
            self.state.grid.cursor_pos,
            viewport_cells,
            content,
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
                matches!(
                    viewport_policy,
                    StateChangeViewportPolicy::CursorOnly | StateChangeViewportPolicy::Stable
                ) || content.is_empty()
                    || content_intersects_inner_screen(origin, viewport_cells, content)
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
        previous_state: Editor,
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
        self.cancel_scroll_pan();
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let layout = self.current_layout();
        let viewport_cells = (layout.cols.max(1), layout.rows.max(1));
        let current = self.viewport.origin(cell_size);
        self.refresh_content_index();
        let content = self.content_index.cells();
        let old_cursor = self.state.grid.cursor_pos;
        let (cursor, origin) =
            normalized_cursor_and_origin(current, old_cursor, viewport_cells, content);
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
            content.is_empty() || content_intersects_inner_screen(origin, viewport_cells, content)
        );
    }

    pub fn apply_view_command(&mut self, command: ViewCommand) -> bool {
        self.cancel_scroll_pan();
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let layout = self.current_layout();
        let viewport_cells = (layout.cols.max(1), layout.rows.max(1));
        self.refresh_content_index();
        let content = self.content_index.cells();
        self.view_cursor_anchor.get_or_insert_with(|| {
            ViewCursorAnchor::capture(
                self.state.grid.cursor_pos,
                self.viewport,
                cell_size,
                layout.grid_top,
            )
        });
        let changed = match command {
            ViewCommand::Pan(direction) => pan_viewport(&mut self.viewport, direction, cell_size),
            ViewCommand::Center => {
                center_viewport(&mut self.viewport, cell_size, viewport_cells, content)
            }
        };
        if changed {
            self.request_redraw();
        }
        changed
    }

    pub fn queue_scroll_pan(&mut self, delta: MouseScrollDelta) -> bool {
        self.wheel_zoom_remainder = 0.0;
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let pixel_delta = scroll_delta_in_pixels(delta, cell_size);
        if pixel_delta == (0.0, 0.0) {
            return false;
        }
        self.scroll_pan.queue(pixel_delta);
        self.request_redraw();
        true
    }

    pub fn advance_scroll_pan(&mut self) -> bool {
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let pixel_delta = self.scroll_pan.next_step(cell_size);
        if pixel_delta == (0, 0) {
            return false;
        }
        let layout = self.current_layout();
        self.view_cursor_anchor.get_or_insert_with(|| {
            ViewCursorAnchor::capture(
                self.state.grid.cursor_pos,
                self.viewport,
                cell_size,
                layout.grid_top,
            )
        });

        let changed = pan_viewport_by_pixels(&mut self.viewport, pixel_delta);
        self.scroll_pan.consume(pixel_delta, pixel_delta);
        changed
    }

    pub fn scroll_pan_active(&self) -> bool {
        self.scroll_pan.is_active()
    }

    pub fn cancel_scroll_pan(&mut self) {
        self.scroll_pan = ScrollPan::default();
    }

    pub fn zoom_from_pinch(&mut self, delta: f64, phase: TouchPhase) -> bool {
        self.cancel_scroll_pan();
        self.wheel_zoom_remainder = 0.0;
        let _ = phase;
        self.zoom_canvas_by(pinch_zoom_units(delta) as f32)
    }

    pub fn zoom_from_mouse_wheel(&mut self, delta: MouseScrollDelta) -> bool {
        self.cancel_scroll_pan();
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let units = wheel_zoom_units(delta, (metrics.cell_width, metrics.cell_height));
        let steps = take_zoom_steps(&mut self.wheel_zoom_remainder, units);
        self.zoom_canvas_by(steps as f32)
    }

    pub(crate) fn zoom_canvas_by(&mut self, delta: f32) -> bool {
        if delta.abs() < f32::EPSILON {
            return false;
        }
        let scale_factor = self.window.scale_factor();
        let old_metrics = self.renderer.metrics(scale_factor);
        let old_cell_size = (old_metrics.cell_width, old_metrics.cell_height);
        let old_layout = self.current_layout();
        let anchor = self.mouse_position.unwrap_or_else(|| {
            canvas_cell_center(
                self.state.grid.cursor_pos,
                old_layout.grid_top,
                old_cell_size,
                self.viewport,
            )
        });
        if !self.renderer.adjust_font_size(delta) {
            return false;
        }

        let new_metrics = self.renderer.metrics(scale_factor);
        let new_cell_size = (new_metrics.cell_width, new_metrics.cell_height);
        let new_layout = self.current_layout();
        self.viewport = zoom_anchored_viewport(
            self.viewport,
            anchor,
            old_cell_size,
            new_cell_size,
            old_layout.grid_top,
            new_layout.grid_top,
        );

        let viewport_cells = (new_layout.cols.max(1), new_layout.rows.max(1));
        self.refresh_content_index();
        let content = self.content_index.cells();
        let origin = self.viewport.origin(new_cell_size);
        if !content.is_empty() && !content_intersects_inner_screen(origin, viewport_cells, content)
        {
            if let Some(origin) =
                constrained_origin(origin, self.state.grid.cursor_pos, viewport_cells, content)
            {
                self.viewport.set_origin(origin, new_cell_size);
            } else {
                let (cursor, origin) = normalized_cursor_and_origin(
                    origin,
                    self.state.grid.cursor_pos,
                    viewport_cells,
                    content,
                );
                self.state.clamp_cursor_to_content(cursor);
                self.viewport.set_origin(origin, new_cell_size);
            }
        }
        self.request_redraw();
        true
    }

    pub fn apply_jump_viewport_pan(&mut self) -> bool {
        let pan = self.state.take_jump_viewport_pan();
        if pan == JumpViewportPan::default() {
            return false;
        }
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        pan_viewport_by_cells(
            &mut self.viewport,
            pan,
            (metrics.cell_width, metrics.cell_height),
        )
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
            (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
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
        if self.document_session.is_stdin() {
            return Ok(false);
        }
        if self.autosave_in_flight.is_some() {
            return Ok(false);
        }
        if !should_autosave(
            self.document_dirty
                || self.menu_selections_dirty
                || self.canvas_position() != self.saved_canvas_position,
            self.last_keypress,
            now,
        ) {
            return Ok(false);
        }
        let path = self
            .document_session
            .path()
            .expect("stdin sessions returned before path persistence")
            .to_path_buf();
        let format = match self.document_session {
            DocumentSession::TextFile(_) => Some(FileKind::Txt),
            DocumentSession::JsonFile(_) => Some(FileKind::Json),
            DocumentSession::Scratchpad(_) | DocumentSession::File(_) => None,
            DocumentSession::Stdin(_) => {
                unreachable!("stdin sessions returned before path persistence")
            }
        };
        let position = self.canvas_position();
        self.state.commit_canvas_mutations()?;
        let contents = match format {
            Some(FileKind::Txt) => plain_text(&self.state),
            Some(FileKind::Json) => project_json_contents(&self.state, self.viewport)?,
            Some(FileKind::Png) => unreachable!("PNG cannot be a document session"),
            None => document::contents(
                self.state.canvas(),
                &self.state.toolbar.durable_selections(),
                position,
            )?,
        };
        let pending = PendingAutosave {
            position,
            document_dirty: self.document_dirty,
            menu_selections_dirty: self.menu_selections_dirty,
        };
        self.document_dirty = false;
        self.menu_selections_dirty = false;
        if let Err(error) = self
            .background
            .write_autosave(self.window_id(), path, contents)
        {
            self.document_dirty |= pending.document_dirty;
            self.menu_selections_dirty |= pending.menu_selections_dirty;
            return Err(error);
        }
        self.autosave_in_flight = Some(pending);
        Ok(true)
    }

    pub fn finish_autosave(&mut self, result: std::result::Result<(), String>) {
        let Some(pending) = self.autosave_in_flight.take() else {
            return;
        };
        match result {
            Ok(()) => self.saved_canvas_position = pending.position,
            Err(error) => {
                self.document_dirty |= pending.document_dirty;
                self.menu_selections_dirty |= pending.menu_selections_dirty;
                log_error(format!("document autosave failed: {error}"));
            }
        }
    }

    pub fn save_document(&mut self) -> Result<bool> {
        self.background.flush();
        if let Some(pending) = self.autosave_in_flight.take() {
            self.document_dirty |= pending.document_dirty;
            self.menu_selections_dirty |= pending.menu_selections_dirty;
        }
        if self.document_session.is_stdin() {
            let text = plain_text(&self.state);
            let stdout = std::io::stdout();
            return write_plain_text_if_dirty(
                &mut self.document_dirty,
                &mut self.menu_selections_dirty,
                &text,
                &mut stdout.lock(),
            );
        }
        self.state.commit_canvas_mutations()?;
        let layers = self.state.persisted_layers();
        let native_canvas = self.state.canvas().clone();
        let position = self.canvas_position();
        self.document_dirty |= position != self.saved_canvas_position;
        let path = self
            .document_session
            .path()
            .expect("stdin sessions returned before path persistence")
            .to_path_buf();
        let format = match self.document_session {
            DocumentSession::TextFile(_) => Some(FileKind::Txt),
            DocumentSession::JsonFile(_) => Some(FileKind::Json),
            DocumentSession::Scratchpad(_) | DocumentSession::File(_) => None,
            DocumentSession::Stdin(_) => {
                unreachable!("stdin sessions returned before path persistence")
            }
        };
        let text = (format == Some(FileKind::Txt)).then(|| plain_text(&self.state));
        let state = (format == Some(FileKind::Json)).then(|| self.state.clone());
        let viewport = self.viewport;
        let saved = save_document_if_dirty(
            &mut self.document_dirty,
            &mut self.menu_selections_dirty,
            &path,
            &layers,
            self.state.active_layer_id(),
            &self.state.toolbar.durable_selections(),
            |path, _layers, _active_layer, menu_selections| match format {
                Some(FileKind::Txt) => std::fs::write(path, text.as_deref().unwrap_or_default())
                    .with_context(|| format!("failed to write {}", path.display())),
                Some(FileKind::Json) => save_project_json(
                    path,
                    state
                        .as_ref()
                        .expect("JSON session cloned its editor state"),
                    viewport,
                ),
                Some(FileKind::Png) => unreachable!("PNG cannot be a document session"),
                None => document::save(path, &native_canvas, menu_selections, position),
            },
        )?;
        if saved {
            self.saved_canvas_position = position;
        }
        Ok(saved)
    }

    pub fn activate_export_file(&mut self, path: PathBuf, format: FileKind) {
        self.document_session = match format {
            FileKind::Txt => DocumentSession::TextFile(path),
            FileKind::Json => DocumentSession::JsonFile(path),
            FileKind::Png => return,
        };
        self.window.set_title(&self.document_session.window_title());
        self.state.window_title = self.document_session.window_title();
        self.document_dirty = false;
        self.menu_selections_dirty = false;
        self.saved_canvas_position = self.canvas_position();
    }

    pub fn set_recent_documents(&mut self, files: &[PathBuf]) {
        self.state.toolbar.set_recent_documents(files);
    }

    pub fn set_document_history_enabled(&mut self, enabled: bool) {
        self.state.toolbar.set_document_history_enabled(enabled);
    }

    pub fn open_document(&mut self, path: PathBuf, scratchpad: bool) -> Result<()> {
        self.save_document()?;
        let session = if scratchpad {
            DocumentSession::scratchpad(path.clone())
        } else {
            DocumentSession::file(path.clone())
        };
        let title = session.window_title();
        let mut state = Editor::new(&self.state.theme, title.clone());
        let mut viewport = ViewportOffset::default();
        let mut restored_position = false;
        self.renderer.restore_zoom(0);
        if let Some(document) = document::load(&path)? {
            if let Some(selections) = document.menu_selections {
                state.restore_menu_selections(&selections);
            }
            state.restore_canvas(document.canvas);
            if let Some(position) = document.position {
                state.restore_canvas_position(position.cursor, position.canvas_origin);
                self.renderer.restore_zoom(position.zoom);
                viewport = position.viewport;
                restored_position = true;
            }
        }
        self.window.set_title(&title);
        self.state = state;
        self.document_session = session;
        self.document_dirty = false;
        self.menu_selections_dirty = false;
        self.history = EditHistory::default();
        self.viewport = viewport;
        if !restored_position {
            self.ensure_cursor_in_viewport();
        }
        self.saved_canvas_position = self.canvas_position();
        self.request_redraw();
        Ok(())
    }

    fn canvas_position(&self) -> document::CanvasPosition {
        document::CanvasPosition {
            cursor: self.state.grid.cursor_pos,
            canvas_origin: self.state.canvas_origin(),
            viewport: self.viewport,
            zoom: self.renderer.zoom(),
        }
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
    previous: &Editor,
    current: &mut Editor,
    cell_size: (f32, f32),
    grid_top: f32,
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
    cell_size: (f32, f32),
) -> bool {
    let cell_width = cell_size.0.max(1.0).round() as i64;
    let cell_height = cell_size.1.max(1.0).round() as i64;
    let delta = match direction {
        Direction::Left => (cell_width, 0),
        Direction::Right => (-cell_width, 0),
        Direction::Up => (0, cell_height),
        Direction::Down => (0, -cell_height),
    };
    pan_viewport_by_pixels(viewport, delta)
}

fn scroll_delta_in_pixels(delta: MouseScrollDelta, cell_size: (f32, f32)) -> (f64, f64) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => (
            f64::from(x) * cell_size.0.max(1.0) as f64,
            f64::from(y) * cell_size.1.max(1.0) as f64,
        ),
        MouseScrollDelta::PixelDelta(position) => (position.x, position.y),
    }
}

pub(crate) fn modified_wheel_zooms(modifiers: ModifiersState) -> bool {
    !modifiers.shift_key()
        && !modifiers.alt_key()
        && (modifiers.control_key() || modifiers.super_key())
}

fn pinch_zoom_units(delta: f64) -> f64 {
    if delta.is_finite() {
        (delta * 20.0).clamp(-4.0, 4.0)
    } else {
        0.0
    }
}

fn wheel_zoom_units(delta: MouseScrollDelta, cell_size: (f32, f32)) -> f64 {
    let units = match delta {
        MouseScrollDelta::LineDelta(_, y) => f64::from(y),
        MouseScrollDelta::PixelDelta(position) => position.y / cell_size.1.max(1.0) as f64,
    };
    units.clamp(-4.0, 4.0)
}

fn take_zoom_steps(remainder: &mut f64, units: f64) -> i64 {
    *remainder += units;
    let steps = remainder.trunc() as i64;
    *remainder -= steps as f64;
    steps
}

fn canvas_cell_center(
    coord: Coord,
    grid_top: f32,
    cell_size: (f32, f32),
    viewport: ViewportOffset,
) -> (f64, f64) {
    (
        PADDING as f64
            + coord.column as f64 * cell_size.0 as f64
            + cell_size.0 as f64 / 2.0
            + viewport.x as f64,
        grid_top as f64
            + coord.line as f64 * cell_size.1 as f64
            + cell_size.1 as f64 / 2.0
            + viewport.y as f64,
    )
}

fn zoom_anchored_viewport(
    viewport: ViewportOffset,
    anchor: (f64, f64),
    old_cell_size: (f32, f32),
    new_cell_size: (f32, f32),
    old_grid_top: f32,
    new_grid_top: f32,
) -> ViewportOffset {
    let canvas_x =
        (anchor.0 - PADDING as f64 - viewport.x as f64) / old_cell_size.0.max(1.0) as f64;
    let canvas_y =
        (anchor.1 - old_grid_top as f64 - viewport.y as f64) / old_cell_size.1.max(1.0) as f64;
    ViewportOffset {
        x: (anchor.0 - PADDING as f64 - canvas_x * new_cell_size.0.max(1.0) as f64).round() as i64,
        y: (anchor.1 - new_grid_top as f64 - canvas_y * new_cell_size.1.max(1.0) as f64).round()
            as i64,
    }
}

fn consume_scroll_axis(pending: &mut f64, requested: i64, applied: i64) {
    if requested == 0 {
        return;
    }
    if requested == applied {
        *pending -= applied as f64;
    } else {
        *pending = 0.0;
    }
}

fn pan_viewport_by_pixels(viewport: &mut ViewportOffset, delta: (i64, i64)) -> bool {
    let candidate = ViewportOffset {
        x: viewport.x.saturating_add(delta.0),
        y: viewport.y.saturating_add(delta.1),
    };
    let changed = candidate != *viewport;
    *viewport = candidate;
    changed
}

fn pan_viewport_by_cells(
    viewport: &mut ViewportOffset,
    pan: JumpViewportPan,
    cell_size: (f32, f32),
) -> bool {
    let old = *viewport;
    viewport.x = viewport
        .x
        .saturating_sub((pan.columns as f64 * cell_size.0.max(1.0) as f64).round() as i64);
    viewport.y = viewport
        .y
        .saturating_sub((pan.rows as f64 * cell_size.1.max(1.0) as f64).round() as i64);
    *viewport != old
}

fn center_viewport(
    viewport: &mut ViewportOffset,
    cell_size: (f32, f32),
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
    layers: &[crate::editor::PersistedLayer],
    active_layer: crate::model::LayerId,
    menu_selections: &crate::toolbar::DurableMenuSelections,
    save: impl FnOnce(
        &Path,
        &[crate::editor::PersistedLayer],
        crate::model::LayerId,
        &crate::toolbar::DurableMenuSelections,
    ) -> Result<()>,
) -> Result<bool> {
    if !*document_dirty && !*menu_selections_dirty {
        return Ok(false);
    }
    save(path, layers, active_layer, menu_selections)?;
    *document_dirty = false;
    *menu_selections_dirty = false;
    Ok(true)
}

fn write_plain_text_if_dirty(
    document_dirty: &mut bool,
    menu_selections_dirty: &mut bool,
    text: &str,
    output: &mut impl Write,
) -> Result<bool> {
    if !*document_dirty && !*menu_selections_dirty {
        return Ok(false);
    }
    output.write_all(text.as_bytes())?;
    output.flush()?;
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
        StateChangeViewportPolicy::Stable => Some(current),
    }
}

pub fn create_editor_window(
    elwt: &ActiveEventLoop,
    config: &AppConfig,
    document_session: &DocumentSession,
    _debug: bool,
    background: BackgroundSender,
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

    let title = document_session.window_title();
    window.set_title(&title);
    let mut state = Editor::new(&config.theme, title);
    let renderer = load_renderer(config);
    let mut viewport = ViewportOffset::default();
    let mut restored_position = false;
    match document_session {
        DocumentSession::Scratchpad(document_path) | DocumentSession::File(document_path) => {
            if let Some(document) = document::load(document_path)? {
                if let Some(menu_selections) = document.menu_selections {
                    state.restore_menu_selections(&menu_selections);
                }
                state.restore_canvas(document.canvas);
                if let Some(position) = document.position {
                    state.restore_canvas_position(position.cursor, position.canvas_origin);
                    renderer.restore_zoom(position.zoom);
                    viewport = position.viewport;
                    restored_position = true;
                }
            }
        }
        DocumentSession::TextFile(document_path) => {
            let text = std::fs::read_to_string(document_path)
                .with_context(|| format!("failed to read {}", document_path.display()))?;
            state.replace_canvas(lines_from_text(&text));
        }
        DocumentSession::JsonFile(document_path) => {
            let mut loaded = state.clone();
            load_project_json(document_path, &mut loaded, &mut viewport)?;
            state = loaded;
            restored_position = true;
        }
        DocumentSession::Stdin(text) => state.replace_canvas(lines_from_text(text)),
    }
    let content_index = ContentIndex::new(&state);
    let mut editor = EditorWindow {
        window,
        surface,
        modifiers: ModifiersState::empty(),
        ordered_modifiers: OrderedModifierTracker::default(),
        mouse_position: None,
        mouse_cell: Some((0, 0)),
        mouse_toolbar_position: None,
        mouse_toolbar_hotspot: None,
        mouse_drag: None,
        last_line_click: None,
        scroll_pan: ScrollPan::default(),
        wheel_zoom_remainder: 0.0,
        #[cfg(debug_assertions)]
        scroll_stats: ScrollStats::new(_debug, Instant::now()),
        background,
        state,
        renderer,
        viewport,
        view_cursor_anchor: None,
        history: EditHistory::default(),
        content_index,
        perf: PerfDiagnostics::from_env(),
        transparent_menubar: config.transparent_menubar,
        document_session: document_session.clone(),
        document_dirty: document_session.is_stdin(),
        menu_selections_dirty: false,
        saved_canvas_position: document::CanvasPosition {
            cursor: Coord::default(),
            canvas_origin: Coord::default(),
            viewport: ViewportOffset::default(),
            zoom: 0,
        },
        last_keypress: Instant::now(),
        export_success_deadline: None,
        autosave_in_flight: None,
    };
    if !restored_position {
        editor.ensure_cursor_in_viewport();
    }
    editor.saved_canvas_position = editor.canvas_position();
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
    background: &BackgroundWorker,
) {
    background.flush();
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

pub struct CommandContext<'a> {
    pub elwt: &'a ActiveEventLoop,
    pub config: &'a AppConfig,
    pub document_session: &'a DocumentSession,
    pub recent_documents: &'a [PathBuf],
    pub debug: bool,
    pub background: &'a BackgroundWorker,
}

fn should_autosave(dirty: bool, last_keypress: Instant, now: Instant) -> bool {
    dirty && now.saturating_duration_since(last_keypress) > Duration::from_secs(5)
}

pub fn handle_command(
    command: AppCommand,
    source_window_id: Option<WindowId>,
    windows: &mut HashMap<WindowId, EditorWindow>,
    context: CommandContext<'_>,
) {
    let target = source_window_id
        .filter(|window_id| windows.contains_key(window_id))
        .or_else(|| focused_window_id(windows));

    match command {
        AppCommand::WindowNew if context.document_session.is_stdin() => {}
        AppCommand::WindowNew => {
            match create_editor_window(
                context.elwt,
                context.config,
                context.document_session,
                context.debug,
                context.background.sender(),
            ) {
                Ok(mut editor) => {
                    editor.set_document_history_enabled(
                        context.document_session.allows_document_history(),
                    );
                    editor.set_recent_documents(context.recent_documents);
                    windows.insert(editor.window_id(), editor);
                }
                Err(error) => log_error(format!("new window creation failed: {error:#}")),
            }
        }
        AppCommand::WindowClose => {
            if let Some(window_id) = target {
                close_window(windows, window_id, context.elwt, context.background);
            }
        }
        AppCommand::FontScaleUp => {
            adjust_font_size(windows, target, FontSizeAction::Increase, context.config)
        }
        AppCommand::FontScaleDown => {
            adjust_font_size(windows, target, FontSizeAction::Decrease, context.config)
        }
        AppCommand::FontScaleReset => {
            adjust_font_size(windows, target, FontSizeAction::Reset, context.config)
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
    let grid_top = grid_top_for_width(
        scale_factor,
        config.transparent_menubar,
        editor.window.inner_size().width as usize,
        (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
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

#[cfg(test)]
fn grid_top(
    scale_factor: f64,
    transparent_menubar: bool,
    toolbar_cell_height: f32,
    toolbar: &crate::toolbar::ToolbarState,
) -> f32 {
    (content_top_padding(scale_factor, transparent_menubar)
        + crate::toolbar::toolbar_height(toolbar, toolbar_cell_height))
    .round()
}

fn grid_top_for_width(
    scale_factor: f64,
    transparent_menubar: bool,
    viewport_width: usize,
    toolbar_cell_size: (f32, f32),
    toolbar: &crate::toolbar::ToolbarState,
) -> f32 {
    let box_width =
        (viewport_width.saturating_sub(PADDING * 2) as f32 / toolbar_cell_size.0.max(1.0)) as usize;
    (content_top_padding(scale_factor, transparent_menubar)
        + crate::toolbar::toolbar_height_for_width(toolbar, box_width, toolbar_cell_size.1))
    .round()
}

fn reanchor_toolbar_transition(
    viewport: &mut ViewportOffset,
    scale_factor: f64,
    transparent_menubar: bool,
    viewport_width: usize,
    toolbar_cell_size: (f32, f32),
    old_toolbar: &crate::toolbar::ToolbarState,
    new_toolbar: &crate::toolbar::ToolbarState,
) {
    let old_grid_top = grid_top_for_width(
        scale_factor,
        transparent_menubar,
        viewport_width,
        toolbar_cell_size,
        old_toolbar,
    );
    let new_grid_top = grid_top_for_width(
        scale_factor,
        transparent_menubar,
        viewport_width,
        toolbar_cell_size,
        new_toolbar,
    );
    viewport.reanchor_grid_top(old_grid_top, new_grid_top);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppConfig, DEFAULT_WINDOW_TITLE};
    use crate::export::{self, ExportAction, ExportOutcome, ExportPlatform, FileKind};
    use crate::model::{Atom, Direction, Face};
    use crate::toolbar::{MainMode, ToolbarAction};
    use winit::dpi::PhysicalPosition;
    use winit::keyboard::{Key, ModifiersState, NamedKey};

    #[test]
    fn line_double_click_requires_same_stationary_cell_within_interval() {
        let first = Instant::now();
        let coord = Coord { line: 2, column: 3 };
        assert!(is_line_double_click(
            Some((first, coord)),
            first + DOUBLE_CLICK_INTERVAL,
            coord,
            false,
        ));
        assert!(!is_line_double_click(
            Some((first, coord)),
            first + DOUBLE_CLICK_INTERVAL + Duration::from_millis(1),
            coord,
            false,
        ));
        assert!(!is_line_double_click(
            Some((first, coord)),
            first + Duration::from_millis(10),
            Coord { line: 2, column: 4 },
            false,
        ));
        assert!(!is_line_double_click(
            Some((first, coord)),
            first + Duration::from_millis(10),
            coord,
            true,
        ));
    }

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

    fn toolbar_test_metrics(config: &AppConfig) -> (f32, (f32, f32)) {
        let renderer = load_renderer(config);
        let toolbar = renderer.title_metrics(1.0);
        let canvas = renderer.metrics(1.0);
        (toolbar.cell_height, (canvas.cell_width, canvas.cell_height))
    }

    fn canvas_screen_position(
        coord: Coord,
        grid_top: f32,
        cell_size: (f32, f32),
        viewport: ViewportOffset,
    ) -> (f32, f32) {
        (
            PADDING as f32 + coord.column as f32 * cell_size.0 + viewport.x as f32,
            grid_top + coord.line as f32 * cell_size.1 + viewport.y as f32,
        )
    }

    fn state_with_rows(rows: &[&str]) -> Editor {
        let mut state = Editor::new(&AppConfig::default().theme, DEFAULT_WINDOW_TITLE);
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
    fn content_index_rebuilds_only_after_document_invalidation() {
        let mut state = state_with_rows(&["a"]);
        let mut index = ContentIndex::new(&state);

        index.refresh(&state);
        index.refresh(&state);
        assert_eq!(index.rebuilds(), 1);
        assert_eq!(index.cells(), &[Coord::default()]);

        state.grid.lines[0].push(Atom {
            face: Face::default(),
            contents: "b".to_owned(),
        });
        index.invalidate();
        index.refresh(&state);

        assert_eq!(index.rebuilds(), 2);
        assert_eq!(
            index.cells(),
            &[Coord::default(), Coord { line: 0, column: 1 }]
        );
    }

    #[test]
    fn scroll_stats_reports_each_active_second_and_skips_idle_seconds() {
        let started = Instant::now();
        let mut stats = ScrollStats::new(true, started);
        for _ in 0..3 {
            assert!(
                stats
                    .note_scroll_event(started + Duration::from_millis(100))
                    .is_none()
            );
        }
        for _ in 0..2 {
            assert!(
                stats
                    .note_redraw(started + Duration::from_millis(100))
                    .is_none()
            );
        }

        let report = stats.advance(started + Duration::from_secs(1)).unwrap();

        assert_eq!(report.scroll_events, 3);
        assert_eq!(report.redraws, 2);
        assert!(stats.advance(started + Duration::from_secs(2)).is_none());

        assert!(
            stats
                .note_redraw(started + Duration::from_secs(2))
                .is_none()
        );
        assert!(stats.advance(started + Duration::from_secs(3)).is_none());

        let mut disabled = ScrollStats::new(false, started);
        assert!(disabled.note_scroll_event(started).is_none());
        assert!(disabled.note_redraw(started).is_none());
        assert!(disabled.advance(started + Duration::from_secs(2)).is_none());
    }

    fn line_mouse_state() -> Editor {
        let mut state = state_with_rows(&["      "]);
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        state
    }

    #[test]
    fn first_line_click_on_a_new_cell_only_moves_cursor_without_history_or_document_change() {
        let mut state = line_mouse_state();
        let target = Coord { line: 0, column: 2 };
        let previous = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };

        let press = begin_line_mouse_state(&mut state, target, target, true);
        assert_eq!(
            press,
            LineMousePress {
                preview_was_active: false,
                cursor_moved: true,
            }
        );
        assert_eq!(state.grid.cursor_pos, target);
        assert!(!state.has_line_preview());

        let finish = finish_line_mouse_state(
            &mut state,
            target,
            false,
            press.preview_was_active,
            None,
            Instant::now(),
        );
        assert!(!finish.document_changed);
        assert_eq!(finish.next_click, None);
        let current = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        assert!(previous.edit.same_document(&current.edit));

        let history = EditHistory::default();
        assert!(!history.has_pending_transaction());
        assert_eq!(history.lengths(), (0, 0));
    }

    #[test]
    fn line_click_at_current_cursor_activates_preview_immediately() {
        let mut state = line_mouse_state();
        let cursor = state.grid.cursor_pos;

        let press = begin_line_mouse_state(&mut state, cursor, cursor, true);
        assert_eq!(
            press,
            LineMousePress {
                preview_was_active: false,
                cursor_moved: false,
            }
        );
        assert!(state.has_line_preview());
        assert_eq!(state.line_preview_anchor(), Some(cursor));

        let finish = finish_line_mouse_state(
            &mut state,
            cursor,
            false,
            press.preview_was_active,
            None,
            Instant::now(),
        );
        assert!(!finish.document_changed);
        assert!(state.has_line_preview());
        assert_eq!(finish.next_click, None);
    }

    #[test]
    fn second_click_that_activates_line_preview_cannot_also_double_click_finish() {
        let mut state = line_mouse_state();
        let target = Coord { line: 0, column: 2 };
        let first_click = Instant::now();

        let first_press = begin_line_mouse_state(&mut state, target, target, true);
        let first_finish = finish_line_mouse_state(
            &mut state,
            target,
            false,
            first_press.preview_was_active,
            None,
            first_click,
        );
        assert_eq!(first_finish.next_click, None);
        assert!(!state.has_line_preview());

        let activation = begin_line_mouse_state(&mut state, target, target, true);
        assert!(state.has_line_preview());
        let activation_finish = finish_line_mouse_state(
            &mut state,
            target,
            false,
            activation.preview_was_active,
            Some((first_click, target)),
            first_click + Duration::from_millis(10),
        );
        assert!(!activation_finish.document_changed);
        assert!(state.has_line_preview());
        assert_eq!(activation_finish.next_click, None);
    }

    #[test]
    fn active_line_preview_click_adds_anchor_then_double_click_finishes() {
        let mut state = line_mouse_state();
        let origin = state.grid.cursor_pos;
        begin_line_mouse_state(&mut state, origin, origin, true);
        let endpoint = Coord { line: 0, column: 3 };
        assert!(state.move_line_preview_to(endpoint));
        let anchor_click = Instant::now();

        let anchor_press = begin_line_mouse_state(&mut state, endpoint, endpoint, true);
        assert!(anchor_press.preview_was_active);
        let anchor_finish = finish_line_mouse_state(
            &mut state,
            endpoint,
            false,
            anchor_press.preview_was_active,
            None,
            anchor_click,
        );
        assert!(anchor_finish.document_changed);
        assert!(state.has_line_preview());
        assert_eq!(state.line_preview_anchor(), Some(endpoint));
        assert_eq!(anchor_finish.next_click, Some((anchor_click, endpoint)));

        let finish_press = begin_line_mouse_state(&mut state, endpoint, endpoint, true);
        let finish = finish_line_mouse_state(
            &mut state,
            endpoint,
            false,
            finish_press.preview_was_active,
            anchor_finish.next_click,
            anchor_click + Duration::from_millis(10),
        );
        assert!(finish.document_changed);
        assert!(!state.has_line_preview());
        assert_eq!(finish.next_click, None);
    }

    #[test]
    fn line_drag_from_a_new_cell_still_creates_and_finishes_one_segment() {
        let mut state = line_mouse_state();
        let start = Coord { line: 0, column: 1 };
        let endpoint = Coord { line: 0, column: 4 };

        let press = begin_line_mouse_state(&mut state, start, start, true);
        assert!(press.cursor_moved);
        assert!(!state.has_line_preview());
        continue_line_mouse_state(&mut state, endpoint);
        assert!(state.has_line_preview());
        assert_eq!(state.line_preview_anchor(), Some(start));

        let finish = finish_line_mouse_state(
            &mut state,
            endpoint,
            true,
            press.preview_was_active,
            None,
            Instant::now(),
        );
        assert!(finish.document_changed);
        assert!(!state.has_line_preview());
        assert_eq!(finish.next_click, None);
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
        let cell_size = (9.0, 13.0);
        let original = ViewportOffset { x: 7, y: -3 };
        for (direction, expected) in [
            (Direction::Left, ViewportOffset { x: 16, y: -3 }),
            (Direction::Right, ViewportOffset { x: -2, y: -3 }),
            (Direction::Up, ViewportOffset { x: 7, y: 10 }),
            (Direction::Down, ViewportOffset { x: 7, y: -16 }),
        ] {
            let mut viewport = original;
            assert!(pan_viewport(&mut viewport, direction, cell_size));
            assert_eq!(viewport, expected);
        }

        let mut viewport = original;
        for _ in 0..20 {
            assert!(pan_viewport(&mut viewport, Direction::Left, cell_size));
            assert!(pan_viewport(&mut viewport, Direction::Right, cell_size));
        }
        assert_eq!(viewport, original);
    }

    #[test]
    fn scroll_lines_map_to_canvas_cells_and_pixels_remain_precise() {
        assert_eq!(
            scroll_delta_in_pixels(MouseScrollDelta::LineDelta(2.0, -3.0), (9.0, 13.0)),
            (18.0, -39.0),
        );
        assert_eq!(
            scroll_delta_in_pixels(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(1.25, -2.75)),
                (9.0, 13.0),
            ),
            (1.25, -2.75),
        );

        let mut pan = ScrollPan::default();
        for _ in 0..3 {
            pan.queue((0.4, -0.4));
        }
        let step = pan.next_step((9.0, 13.0));
        assert_eq!(step, (1, -1));
        pan.consume(step, step);
        assert!(!pan.is_active());
    }

    #[test]
    fn queued_scroll_is_applied_directly_without_synthetic_motion() {
        let mut pan = ScrollPan::default();
        pan.queue((3.0, -5.0));
        pan.queue((4.0, -6.0));
        pan.queue((5.0, -7.0));

        let step = pan.next_step((8.0, 16.0));
        assert_eq!(step, (12, -18));
        pan.consume(step, step);
        assert!(!pan.is_active());
        assert_eq!(pan.next_step((8.0, 16.0)), (0, 0));
    }

    #[test]
    fn line_scroll_is_applied_without_easing() {
        let mut pan = ScrollPan::default();
        pan.queue((16.0, -32.0));

        assert_eq!(pan.next_step((8.0, 16.0)), (16, -32));
    }

    #[test]
    fn pinch_and_modified_wheel_route_zoom_without_stealing_plain_scroll() {
        assert_eq!(pinch_zoom_units(0.05), 1.0);
        assert_eq!(pinch_zoom_units(-0.05), -1.0);
        assert_eq!(pinch_zoom_units(f64::NAN), 0.0);
        let mut pinch_remainder = 0.0;
        assert_eq!(take_zoom_steps(&mut pinch_remainder, 0.4), 0);
        assert_eq!(take_zoom_steps(&mut pinch_remainder, 0.6), 1);
        assert_eq!(
            wheel_zoom_units(MouseScrollDelta::LineDelta(0.0, 1.0), (8.0, 16.0)),
            1.0,
        );
        assert_eq!(
            wheel_zoom_units(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -8.0)),
                (8.0, 16.0),
            ),
            -0.5,
        );

        assert!(!modified_wheel_zooms(ModifiersState::empty()));
        assert!(modified_wheel_zooms(ModifiersState::CONTROL));
        assert!(modified_wheel_zooms(ModifiersState::SUPER));
        assert!(!modified_wheel_zooms(
            ModifiersState::CONTROL | ModifiersState::SHIFT,
        ));
    }

    #[test]
    fn pointer_anchored_zoom_preserves_the_canvas_point_under_it() {
        let viewport = ViewportOffset { x: -4, y: 8 };
        let anchor = (40.0, 80.0);
        let zoomed =
            zoom_anchored_viewport(viewport, anchor, (8.0, 16.0), (10.0, 20.0), 40.0, 40.0);

        assert_eq!(zoomed, ViewportOffset { x: -10, y: 0 });
        let coord = Coord { line: 2, column: 3 };
        assert_eq!(
            canvas_screen_position(coord, 40.0, (8.0, 16.0), viewport),
            (40.0, 80.0)
        );
        assert_eq!(
            canvas_screen_position(coord, 40.0, (10.0, 20.0), zoomed),
            (40.0, 80.0)
        );
    }

    #[test]
    fn scroll_pan_moves_by_pixels_without_content_constraints() {
        let mut viewport = ViewportOffset::default();

        assert!(pan_viewport_by_pixels(&mut viewport, (5, 7)));
        assert_eq!(viewport, ViewportOffset { x: 5, y: 7 });
        assert!(pan_viewport_by_pixels(&mut viewport, (-80, 0)));
        assert_eq!(viewport, ViewportOffset { x: -75, y: 7 });
    }

    #[test]
    fn jump_pan_moves_the_viewport_by_whole_sector_cells_and_preserves_residuals() {
        let mut viewport = ViewportOffset { x: 7, y: -3 };
        assert!(pan_viewport_by_cells(
            &mut viewport,
            JumpViewportPan {
                columns: 21,
                rows: -15,
            },
            (9.0, 13.0),
        ));
        assert_eq!(
            viewport,
            ViewportOffset {
                x: 7 - 21 * 9,
                y: -3 + 15 * 13,
            }
        );
    }

    #[test]
    fn view_pan_is_unconstrained_by_cursor_or_content() {
        let mut cursor_boundary = ViewportOffset::default();
        assert!(pan_viewport(
            &mut cursor_boundary,
            Direction::Right,
            (8.0, 12.0)
        ));
        assert_eq!(cursor_boundary, ViewportOffset { x: -8, y: 0 });

        let mut content_boundary = ViewportOffset::default();
        assert!(pan_viewport(
            &mut content_boundary,
            Direction::Right,
            (8.0, 12.0)
        ));
        assert_eq!(content_boundary, ViewportOffset { x: -8, y: 0 });

        let mut empty = ViewportOffset::default();
        assert!(pan_viewport(&mut empty, Direction::Left, (8.0, 12.0)));
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
            (8.0, 12.0),
            40.0,
        ));
        assert_eq!(viewport, original_viewport);
        assert_ne!(state.grid.cursor_pos, previous.grid.cursor_pos);
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
    }

    #[test]
    fn view_restores_cursor_to_entry_screen_position_after_panning() {
        let cell_size = (8.0, 12.0);
        let grid_top = 40.0;
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
        let cell_size = (8.0, 12.0);
        let grid_top = 40.0;
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
        assert!(center_viewport(
            &mut viewport,
            (7.0, 11.0),
            (10, 8),
            &content,
        ));
        assert_eq!(viewport.origin((7.0, 11.0)), (0, -1));
        assert_eq!(state.grid.cursor_pos, Coord { line: 3, column: 5 });
        assert_eq!(state.selection, selection);
        assert_eq!(state.grid.lines, lines);
        assert!(!center_viewport(
            &mut viewport,
            (7.0, 11.0),
            (10, 8),
            &content,
        ));
    }

    #[test]
    fn view_center_is_blank_noop_and_leaves_hidden_cursor_unchanged() {
        let mut blank = state_with_rows(&["     "]);
        blank.grid.cursor_pos = Coord { line: 0, column: 4 };
        let blank_cursor = blank.grid.cursor_pos;
        let mut blank_viewport = ViewportOffset { x: 5, y: -9 };
        assert!(!center_viewport(
            &mut blank_viewport,
            (8.0, 12.0),
            (3, 3),
            &[],
        ));
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
        assert!(center_viewport(
            &mut viewport,
            (8.0, 12.0),
            (3, 1),
            &content,
        ));
        assert_eq!(viewport.origin((8.0, 12.0)), (4, 0));
        assert_eq!(wide.grid.cursor_pos, cursor);
        assert_eq!(wide.selection, selection);
        assert_eq!(wide.grid.lines, lines);
    }

    fn assert_toolbar_transition_is_anchored(
        config: &AppConfig,
        old_toolbar: &crate::toolbar::ToolbarState,
        new_toolbar: &crate::toolbar::ToolbarState,
        viewport: &mut ViewportOffset,
        toolbar_cell_height: f32,
        cell_size: (f32, f32),
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
            usize::MAX,
            (1.0, toolbar_cell_height),
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
        state: &mut Editor,
        action: ToolbarAction,
        viewport: &mut ViewportOffset,
        config: &AppConfig,
        toolbar_cell_height: f32,
        cell_size: (f32, f32),
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
        state: &mut Editor,
        key: Key,
        viewport: &mut ViewportOffset,
        config: &AppConfig,
        toolbar_cell_height: f32,
        cell_size: (f32, f32),
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
        state: &mut Editor,
        viewport: ViewportOffset,
        config: &AppConfig,
        toolbar_cell_height: f32,
        cell_size: (f32, f32),
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
        let lines = vec![vec![crate::model::Atom {
            face: crate::model::Face::default(),
            contents: "latest".into(),
        }]];
        let layers = vec![crate::editor::PersistedLayer {
            id: crate::model::LayerId(0),
            visible: true,
            lines,
        }];
        let menu = crate::toolbar::ToolbarState::default().durable_selections();
        let mut writes = 0;

        assert!(
            save_document_if_dirty(
                &mut document_dirty,
                &mut menu_dirty,
                path,
                &layers,
                crate::model::LayerId(0),
                &menu,
                |saved_path, saved_layers, active_layer, saved_menu| {
                    writes += 1;
                    assert_eq!(saved_path, path);
                    assert_eq!(saved_layers, layers);
                    assert_eq!(active_layer, crate::model::LayerId(0));
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
    fn stdin_shutdown_emits_unchanged_or_modified_text_exactly_once() {
        for text in ["unchanged\n", "modified text"] {
            let mut document_dirty = true;
            let mut menu_dirty = false;
            let mut output = Vec::new();

            assert!(
                write_plain_text_if_dirty(&mut document_dirty, &mut menu_dirty, text, &mut output,)
                    .unwrap()
            );
            assert_eq!(output, text.as_bytes());
            assert!(!document_dirty);
            assert!(!menu_dirty);
            assert!(
                !write_plain_text_if_dirty(
                    &mut document_dirty,
                    &mut menu_dirty,
                    text,
                    &mut output,
                )
                .unwrap()
            );
            assert_eq!(output, text.as_bytes());
        }
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
                crate::model::LayerId(0),
                &menu,
                |_, _, _, _| panic!("clean documents must not be written"),
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
            crate::model::LayerId(0),
            &menu,
            |_, _, _, _| Err(anyhow!("disk full")),
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
                crate::model::LayerId(0),
                &menu,
                |_, saved_layers, _, saved_menu| {
                    writes += 1;
                    assert!(saved_layers.is_empty());
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

        let mut state = Editor::new(&AppConfig::default().theme, "test");
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
        assert_eq!(
            resolve_state_change_origin(
                StateChangeViewportPolicy::Stable,
                current,
                cursor,
                viewport,
                &content,
            ),
            Some(current)
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
        let mut state = Editor::new(&AppConfig::default().theme, "test");
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
        let mut state = Editor::new(&AppConfig::default().theme, "test");
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
        let mut state = Editor::new(&AppConfig::default().theme, "test");
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
        let mut state = Editor::new(&AppConfig::default().theme, "test");
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
        let mut state = Editor::new(&AppConfig::default().theme, "test");
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
        let mut state = Editor::new(&config.theme, "test");
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
        let mut state = Editor::new(&config.theme, "test");
        state
            .toolbar
            .apply_action(ToolbarAction::SelectMain(MainMode::Line));
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
        let mut state = Editor::new(&config.theme, "test");
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
        let mut state = Editor::new(&config.theme, "test");
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
            let mut state = Editor::new(&config.theme, "test");
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
        let mut state = Editor::new(&config.theme, "test");
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
