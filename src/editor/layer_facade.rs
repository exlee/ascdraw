use super::*;

impl Editor {
    pub fn layer_summaries(&self) -> Vec<LayerSummary> {
        self.canvas.summaries()
    }

    pub fn active_layer_id(&self) -> LayerId {
        self.canvas.active_id()
    }

    pub fn restore_canvas(&mut self, mut canvas: crate::canvas::LayerStack) {
        canvas.set_enabled(self.toolbar.multi_layer_mode());
        self.toolbar.sync_layer_count(canvas.layers().len());
        self.canvas.record_history_replacement(&canvas);
        self.canvas = canvas;
    }

    pub fn restore_canvas_position(&mut self, cursor: Coord) {
        self.grid.cursor_pos = cursor;
        self.selection.collapse(cursor);
    }

    pub fn select_layer(&mut self, id: LayerId) -> bool {
        let Some(index) = self.canvas.index_of(id) else {
            return false;
        };
        self.sync_layer_mode_before_operation();
        let changed = self.canvas.activate(index);
        if changed {
            self.toolbar.sync_layer_count(self.canvas.layers().len());
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
        }
        changed
    }

    pub fn add_layer_above(&mut self, id: LayerId) -> bool {
        let Some(index) = self.canvas.index_of(id) else {
            return false;
        };
        self.sync_layer_mode_before_operation();
        let changed = self
            .canvas
            .add_above(index)
            .expect("editor layers contain valid sparse cells")
            .is_some();
        if changed {
            self.toolbar.sync_layer_count(self.canvas.layers().len());
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
        }
        changed
    }

    pub fn toggle_layer_visibility(&mut self, id: LayerId) -> bool {
        self.sync_layer_mode_before_operation();
        let changed = self
            .canvas
            .index_of(id)
            .is_some_and(|index| self.canvas.toggle_visibility(index));
        changed
    }

    pub fn move_layer_up(&mut self, id: LayerId) -> bool {
        self.sync_layer_mode_before_operation();
        let changed = self
            .canvas
            .index_of(id)
            .is_some_and(|index| self.canvas.move_up(index));
        changed
    }

    pub fn move_layer_down(&mut self, id: LayerId) -> bool {
        self.sync_layer_mode_before_operation();
        let changed = self
            .canvas
            .index_of(id)
            .is_some_and(|index| self.canvas.move_down(index));
        changed
    }

    pub fn merge_layer_up(&mut self, id: LayerId) -> bool {
        let Some(index) = self.canvas.index_of(id) else {
            return false;
        };
        self.merge_layer_into(index, index.saturating_sub(1))
    }

    pub fn merge_layer_down(&mut self, id: LayerId) -> bool {
        let Some(index) = self.canvas.index_of(id) else {
            return false;
        };
        self.merge_layer_into(index, index.saturating_add(1))
    }

    fn merge_layer_into(&mut self, index: usize, target: usize) -> bool {
        self.sync_layer_mode_before_operation();
        let changed = self
            .canvas
            .merge_into(index, target)
            .expect("editor layers contain valid sparse cells");
        if changed {
            self.toolbar.sync_layer_count(self.canvas.layers().len());
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
        }
        changed
    }

    pub fn delete_layer(&mut self, id: LayerId) -> bool {
        let Some(index) = self.canvas.index_of(id) else {
            return false;
        };
        self.sync_layer_mode_before_operation();
        let changed = self.canvas.delete(index);
        if changed {
            self.toolbar.sync_layer_count(self.canvas.layers().len());
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
        }
        changed
    }

    fn sync_layer_mode_before_operation(&mut self) {
        self.commit_canvas();
    }

    fn cancel_layer_transients(&mut self) {
        self.active_stroke = None;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.jump_mode = None;
        self.single_replace_pending = false;
    }

    fn sync_cursor_to_active_layer(&mut self) {
        let row_width = i16::try_from(self.canvas.active_row_width(self.grid.cursor_pos.line))
            .unwrap_or(i16::MAX);
        self.grid.cursor_pos.column = self.grid.cursor_pos.column.min(row_width);
    }
}
