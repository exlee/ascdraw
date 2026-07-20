use super::*;

impl Editor {
    pub fn history_state(&self) -> HistoryEditorState {
        HistoryEditorState {
            cursor_pos: self.grid.cursor_pos,
            selection: self.selection,
            active_stroke: self.active_stroke.clone(),
        }
    }

    pub fn begin_history_capture(&self) {
        self.canvas.begin_history_capture();
    }

    pub fn cancel_history_capture(&self) {
        crate::canvas::LayerStack::cancel_history_capture();
    }

    pub fn finish_history_capture(&self) -> crate::canvas::HistoryCanvasDelta {
        self.canvas.finish_history_capture()
    }

    pub fn restore_history_state(&mut self, state: HistoryEditorState) {
        self.grid.cursor_pos = state.cursor_pos;
        self.selection = state.selection;
        self.active_stroke = state.active_stroke;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
    }

    pub fn apply_history_delta(
        &mut self,
        delta: &crate::canvas::HistoryCanvasDelta,
        forward: bool,
    ) {
        self.canvas.apply_history_delta(delta, forward);
        self.toolbar.sync_layer_count(self.canvas.layers().len());
    }

    pub fn has_shape_preview(&self) -> bool {
        self.shape_preview.is_some()
    }
    pub fn edit_snapshot(&self) -> EditSnapshot {
        if let Some(lift) = &self.move_lift {
            return lift.source_snapshot.clone();
        }
        let (canvas, cursor_pos, selection) = if let Some(preview) = self
            .line_preview
            .as_ref()
            .filter(|preview| !preview.has_committed_segments())
        {
            (
                preview.source_canvas.clone(),
                preview.source_cursor,
                preview.source_selection,
            )
        } else {
            let canvas = self.canvas.clone();
            (canvas, self.grid.cursor_pos, self.selection)
        };
        EditSnapshot {
            cursor_pos,
            selection,
            active_stroke: self.active_stroke.clone(),
            canvas,
        }
    }

    pub fn restore_edit_snapshot(&mut self, snapshot: EditSnapshot) {
        self.grid.cursor_pos = snapshot.cursor_pos;
        self.selection = snapshot.selection;
        self.active_stroke = snapshot.active_stroke;
        self.canvas = snapshot.canvas;
        self.toolbar.sync_layer_count(self.canvas.layers().len());
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
    }

    pub fn new(theme: &ThemeConfig, window_title: impl Into<String>) -> Self {
        let canvas = crate::canvas::LayerStack::new(
            vec![crate::canvas::LayerMap::new(LayerId(0), true)],
            false,
        )
        .expect("the initial canvas has a base layer");
        let mut toolbar = ToolbarState::default();
        toolbar.sync_layer_count(canvas.layers().len());
        Self {
            grid: GridState {
                cursor_pos: Coord::default(),
                default_face: theme.default.clone(),
                cursor_face: theme.cursor_block.clone(),
            },
            theme: theme.clone(),
            window_title: window_title.into(),
            cursor_mode: CursorMode::Stamp,
            toolbar,
            selection: CanvasSelection::collapsed_at(Coord::default()),
            active_stroke: None,
            canvas,
            line_preview: None,
            shape_preview: None,
            move_lift: None,
            jump_mode: None,
            single_replace_pending: false,
            toolbar_document_changed: false,
            toolbar_viewport_stable: false,
            transient_tip: None,
        }
    }

    pub fn transient_tip(&self) -> Option<&str> {
        self.transient_tip
            .as_ref()
            .filter(|(_, until)| std::time::Instant::now() < *until)
            .map(|(tip, _)| tip.as_str())
    }

    pub(super) fn invalid_text_tip(&mut self) {
        self.transient_tip = Some((
            "Invalid text: every cell must be one display-width-1 grapheme".to_owned(),
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        ));
    }

    pub fn apply_theme(&mut self, theme: &ThemeConfig) {
        self.theme = theme.clone();
        if self.toolbar.dark_mode() {
            reverse_theme_colors(&mut self.theme);
        }
        self.sync_theme_faces();
    }
}
