use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Result, anyhow};
use softbuffer::{Context as SoftContext, Surface};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::app::{AppCommand, AppConfig, DEFAULT_WINDOW_TITLE};
use crate::diagnostics::log_error;
use crate::editor::EditorState;
use crate::layout::{ViewportOffset, content_top_padding};
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
    pub state: EditorState,
    pub renderer: Renderer,
    pub viewport: ViewportOffset,
    transparent_menubar: bool,
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
        );
        self.renderer.apply_config(config);
        let new_metrics = self.renderer.metrics(scale_factor);
        let new_toolbar_metrics = self.renderer.title_metrics(scale_factor);
        let new_grid_top = grid_top(
            scale_factor,
            config.transparent_menubar,
            new_toolbar_metrics.cell_height,
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
        #[cfg(target_os = "macos")]
        if let Err(error) = macos::apply_window_color_space(self.window.as_ref(), &config.macos) {
            log_error(format!("macOS color space setup failed: {error:#}"));
        }
        self.request_redraw();
    }
}

pub fn create_editor_window(elwt: &ActiveEventLoop, config: &AppConfig) -> Result<EditorWindow> {
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

    let editor = EditorWindow {
        window,
        surface,
        modifiers: ModifiersState::empty(),
        mouse_cell: Some(Coord::default()),
        state: EditorState::new(&config.theme, DEFAULT_WINDOW_TITLE),
        renderer: load_renderer(config),
        viewport: ViewportOffset::default(),
        transparent_menubar: config.transparent_menubar,
    };
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
    windows.remove(&window_id);
    if windows.is_empty() {
        elwt.exit();
    }
}

pub fn handle_command(
    command: AppCommand,
    source_window_id: Option<WindowId>,
    windows: &mut HashMap<WindowId, EditorWindow>,
    elwt: &ActiveEventLoop,
    config: &AppConfig,
) {
    let target = source_window_id
        .filter(|window_id| windows.contains_key(window_id))
        .or_else(|| focused_window_id(windows));

    match command {
        AppCommand::WindowNew => match create_editor_window(elwt, config) {
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
        editor.request_redraw();
    }
}

fn grid_top(scale_factor: f64, transparent_menubar: bool, toolbar_cell_height: usize) -> usize {
    content_top_padding(scale_factor, transparent_menubar)
        + crate::toolbar::TOOLBAR_ROWS * toolbar_cell_height
}
