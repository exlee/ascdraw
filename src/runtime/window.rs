use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use softbuffer::{Context as SoftContext, Surface};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{AppCommand, AppConfig, DEFAULT_WINDOW_TITLE};
use crate::diagnostics::log_error;
use crate::document;
use crate::editor::EditorState;
use crate::layout::{
    LayoutMetrics, ViewportOffset, content_intersects_inner_screen, content_top_padding,
    cursor_is_visible, layout_metrics, navigation_origin, normalized_cursor_and_origin,
};
#[cfg(target_os = "macos")]
use crate::macos;
use crate::model::Coord;
use crate::render::{Renderer, load_renderer, resize_surface};
use crate::title_policy::window_attributes;
use crate::user_keys::FontSizeAction;

pub struct EditorWindow {
    pub window: Rc<Window>,
    pub surface: Surface<Rc<Window>, Rc<Window>>,
    pub modifiers: ModifiersState,
    pub mouse_cell: Option<Coord>,
    pub mouse_toolbar_position: Option<(usize, usize, usize)>,
    pub state: EditorState,
    pub renderer: Renderer,
    pub viewport: ViewportOffset,
    transparent_menubar: bool,
    document_path: PathBuf,
    document_dirty: bool,
    last_keypress: Instant,
}

impl EditorWindow {
    pub fn window_id(&self) -> WindowId {
        self.window.id()
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
        self.request_redraw();
    }

    pub fn note_keypress(&mut self, now: Instant) {
        self.last_keypress = now;
    }

    pub fn mark_document_dirty(&mut self) {
        self.document_dirty = true;
    }

    pub fn finish_state_change(
        &mut self,
        previous_state: EditorState,
        previous_viewport: ViewportOffset,
        document_changed: bool,
    ) -> bool {
        let prepend = self.state.take_pending_prepend();
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let layout = self.current_layout();
        let cell_size = (metrics.cell_width, metrics.cell_height);
        self.viewport
            .compensate_for_prepend(prepend.0, prepend.1, cell_size);
        let current = self.viewport.origin(cell_size);
        let viewport_cells = (layout.cols.max(1), layout.rows.max(1));
        let content = self.state.content_cells();

        if let Some(origin) = resolve_navigation_origin(
            current,
            self.state.grid.cursor_pos,
            viewport_cells,
            &content,
        ) {
            if origin != current {
                self.viewport.set_origin(origin, cell_size);
            }
            debug_assert!(cursor_is_visible(
                origin,
                self.state.grid.cursor_pos,
                viewport_cells
            ));
            debug_assert!(
                content.is_empty()
                    || content_intersects_inner_screen(origin, viewport_cells, &content)
            );
            return document_changed;
        }

        self.state = previous_state;
        self.viewport = previous_viewport;
        false
    }

    pub fn ensure_cursor_in_viewport(&mut self) {
        let scale_factor = self.window.scale_factor();
        let metrics = self.renderer.metrics(scale_factor);
        let cell_size = (metrics.cell_width, metrics.cell_height);
        let layout = self.current_layout();
        let viewport_cells = (layout.cols.max(1), layout.rows.max(1));
        let current = self.viewport.origin(cell_size);
        let content = self.state.content_cells();
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

    pub fn autosave_if_idle(&mut self, now: Instant) -> Result<bool> {
        if !should_autosave(self.document_dirty, self.last_keypress, now) {
            return Ok(false);
        }
        self.save_document()?;
        Ok(true)
    }

    pub fn save_document(&mut self) -> Result<bool> {
        if !self.document_dirty {
            return Ok(false);
        }
        document::save(&self.document_path, &self.state.grid.lines)?;
        self.document_dirty = false;
        Ok(true)
    }
}

fn resolve_navigation_origin(
    current: (i64, i64),
    cursor: Coord,
    viewport: (usize, usize),
    content: &[Coord],
) -> Option<(i64, i64)> {
    navigation_origin(current, cursor, viewport, content)
}

pub fn create_editor_window(
    elwt: &ActiveEventLoop,
    config: &AppConfig,
    document_path: &Path,
) -> Result<EditorWindow> {
    let window = Rc::new(elwt.create_window(window_attributes(config))?);
    let context = SoftContext::new(window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    let mut surface =
        Surface::new(&context, window.clone()).map_err(|error| anyhow!(error.to_string()))?;
    resize_surface(&mut surface, window.inner_size())?;

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
    }
    let mut editor = EditorWindow {
        window,
        surface,
        modifiers: ModifiersState::empty(),
        mouse_cell: Some(Coord::default()),
        mouse_toolbar_position: None,
        state,
        renderer: load_renderer(config),
        viewport: ViewportOffset::default(),
        transparent_menubar: config.transparent_menubar,
        document_path: document_path.to_path_buf(),
        document_dirty: false,
        last_keypress: Instant::now(),
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
    if let Some(mut editor) = windows.remove(&window_id)
        && let Err(error) = editor.save_document()
    {
        log_error(format!("document save on close failed: {error:#}"));
    }
    if windows.is_empty() {
        elwt.exit();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppConfig;
    use crate::model::Direction;

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
}
