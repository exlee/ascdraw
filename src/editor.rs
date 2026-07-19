#[cfg(test)]
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::{CursorMode, ThemeConfig};
#[cfg(test)]
use crate::drawing::LineEnding;
use crate::drawing::is_line_glyph;
use crate::jump::JumpMode;
use crate::model::{
    Atom, Coord, Direction, Face, LayerId, LayerSummary, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH,
};
use crate::selection::{
    CanvasSelection, SelectionBounds, TextRectangle, overwrite_rectangle, replace_range,
    selected_text,
};
use crate::toolbar::{
    DurableMenuSelections, LayerOperation, MainMode, ToolbarAction, ToolbarSpan, ToolbarState,
    Tooltip, UtilityKind,
};

mod color_tool;
mod grid;
mod jump_mode;
mod layers;
mod line_preview;
mod line_tool;
mod move_tool;
mod routing;
mod shape_tool;
mod state;
mod text_tool;
mod utility;
use crate::canvas::LineMarker as PlacedLineMarker;
pub(super) use grid::{adjacent_coord, edited_content_origin};
pub use layers::{LayerView, PersistedLayer};
use line_preview::LinePreview;
use line_tool::ActiveStroke;
use move_tool::MoveLift;

#[derive(Debug, Clone)]
pub struct GridState {
    pub lines: Vec<Vec<Atom>>,
    pub cursor_pos: Coord,
    pub default_face: Face,
    pub cursor_face: Face,
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub grid: GridState,
    pub theme: ThemeConfig,
    pub window_title: String,
    pub cursor_mode: CursorMode,
    pub toolbar: ToolbarState,
    pub selection: CanvasSelection,
    cursor_index: usize,
    active_stroke: Option<ActiveStroke>,
    canvas: crate::canvas::LayerStack,
    line_preview: Option<LinePreview>,
    shape_preview: Option<ShapePreview>,
    move_lift: Option<MoveLift>,
    jump_mode: Option<JumpMode>,
    single_replace_pending: bool,
    pending_prepend: (i64, i64),
    canvas_origin: Coord,
    toolbar_document_changed: bool,
    toolbar_viewport_stable: bool,
    transient_tip: Option<(String, std::time::Instant)>,
}

#[derive(Debug, Clone, Copy)]
struct ShapePreview {
    anchor: Coord,
    end: Coord,
}

fn reverse_theme_colors(theme: &mut ThemeConfig) {
    std::mem::swap(&mut theme.default.fg, &mut theme.default.bg);
}

#[derive(Debug, Clone)]
pub struct EditSnapshot {
    lines: Vec<Vec<Atom>>,
    cursor_pos: Coord,
    cursor_index: usize,
    selection: CanvasSelection,
    active_stroke: Option<ActiveStroke>,
    canvas_origin: Coord,
    canvas: crate::canvas::LayerStack,
}

impl PartialEq for EditSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.lines == other.lines
            && self.cursor_pos == other.cursor_pos
            && self.cursor_index == other.cursor_index
            && self.selection == other.selection
            && self.active_stroke == other.active_stroke
            && self.canvas_origin == other.canvas_origin
            && self.canvas == other.canvas
    }
}

impl Eq for EditSnapshot {}

impl EditSnapshot {
    pub fn same_document(&self, other: &Self) -> bool {
        self.lines == other.lines
            && self.canvas_origin == other.canvas_origin
            && self.canvas == other.canvas
    }

    #[cfg(test)]
    pub fn set_cursor_for_test(&mut self, line: usize, column: usize) {
        self.cursor_pos = Coord { line, column };
        self.selection.collapse(self.cursor_pos);
    }
}

#[cfg(test)]
impl Editor {
    fn line_markers_for_test(&self) -> Vec<PlacedLineMarker> {
        self.canvas.active_line_markers()
    }

    fn set_line_markers_for_test(&mut self, markers: Vec<PlacedLineMarker>) {
        self.canvas
            .commit_active_with_markers(&self.grid.lines, &markers)
            .expect("test line markers must address valid cells");
    }

    fn push_line_marker_for_test(&mut self, marker: PlacedLineMarker) {
        let mut markers = self.line_markers_for_test();
        markers.push(marker);
        self.set_line_markers_for_test(markers);
    }

    fn extend_line_markers_for_test(
        &mut self,
        markers: impl IntoIterator<Item = PlacedLineMarker>,
    ) {
        let mut combined = self.line_markers_for_test();
        combined.extend(markers);
        self.set_line_markers_for_test(combined);
    }
}

impl Editor {
    pub(crate) fn commit_canvas_mutations(&mut self) -> anyhow::Result<()> {
        if self.canvas.commit_active(&self.grid.lines).is_err() {
            // Legacy dense documents can still contain wide atoms. They remain
            // on the compatibility path until the document is edited into
            // one-cell atoms; sparse storage itself never accepts them.
            return Ok(());
        };
        self.canvas.set_enabled(self.toolbar.multi_layer_mode());
        Ok(())
    }

    fn commit_canvas(&mut self) {
        self.commit_canvas_mutations()
            .expect("editor mutations preserve one-cell sparse canvas atoms");
    }

    fn commit_canvas_with_remapped_line_data(
        &mut self,
        mut map: impl FnMut(Coord) -> Option<Coord>,
    ) {
        let markers = self
            .canvas
            .active_line_markers()
            .into_iter()
            .filter_map(|marker| {
                map(marker.coord).map(|coord| PlacedLineMarker { coord, ..marker })
            })
            .collect::<Vec<_>>();
        self.canvas
            .commit_active_with_markers(&self.grid.lines, &markers)
            .expect("line metadata remapping preserves valid sparse cells");
    }

    fn sync_sparse_cell_from_dense(&mut self, coord: Coord) {
        let Some(line) = self.grid.lines.get(coord.line) else {
            self.canvas.delete_at(coord);
            return;
        };
        let (index, column) = index_and_column_for_coord(line, coord.column);
        let Some(atom) = line.get(index).filter(|_| column == coord.column) else {
            self.canvas.delete_at(coord);
            return;
        };
        if atom_width(atom) != 1 {
            return;
        }
        self.canvas
            .set_at(coord, atom.clone(), &atom.face)
            .expect("edited cells contain one display-width-1 grapheme");
    }

    fn refresh_active_dense_view(&mut self) {
        self.grid.lines = self.canvas.active_dense_lines();
        while self.grid.lines.len() <= self.grid.cursor_pos.line {
            self.grid.lines.push(Vec::new());
        }
        self.cursor_index = index_for_column(
            &self.grid.lines[self.grid.cursor_pos.line],
            self.grid.cursor_pos.column,
        );
    }

    pub fn canvas(&self) -> &crate::canvas::LayerStack {
        &self.canvas
    }

    pub fn canvas_is_current(&self) -> bool {
        self.canvas.layers()[self.canvas.active_index()].matches_dense(&self.grid.lines)
            && self.canvas.enabled() == self.toolbar.multi_layer_mode()
            && !self.canvas.has_legacy_wide_atoms()
    }

    pub fn layer_summaries(&self) -> Vec<LayerSummary> {
        self.canvas.summaries()
    }

    pub fn layer_views(&self) -> Vec<LayerView> {
        self.canvas
            .layers()
            .iter()
            .enumerate()
            .map(|(index, layer)| LayerView {
                id: layer.id,
                visible: layer.visible,
                lines: if index == self.canvas.active_index() {
                    self.grid.lines.clone()
                } else {
                    layer.to_dense()
                },
            })
            .collect()
    }

    fn layer_contents(&self) -> Vec<(LayerId, Vec<Vec<Atom>>, Vec<PlacedLineMarker>)> {
        self.canvas
            .layers()
            .iter()
            .enumerate()
            .map(|(index, layer)| {
                if index == self.canvas.active_index() {
                    (layer.id, self.grid.lines.clone(), layer.line_markers())
                } else {
                    (layer.id, layer.to_dense(), layer.line_markers().to_vec())
                }
            })
            .collect()
    }

    pub fn active_layer_id(&self) -> LayerId {
        self.canvas.active_id()
    }

    pub fn persisted_layers(&self) -> Vec<PersistedLayer> {
        self.layer_views()
            .into_iter()
            .map(|layer| PersistedLayer {
                id: layer.id,
                visible: layer.visible,
                lines: layer.lines,
            })
            .collect()
    }

    pub fn restore_layers(
        &mut self,
        layers: Vec<PersistedLayer>,
        active_layer: LayerId,
    ) -> anyhow::Result<()> {
        let maps = layers
            .into_iter()
            .map(|layer| {
                crate::canvas::LayerMap::from_dense_with_markers(
                    layer.id,
                    layer.visible,
                    &layer.lines,
                    &[],
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.canvas = crate::canvas::LayerStack::with_active(
            maps,
            active_layer,
            self.toolbar.multi_layer_mode(),
        )?;
        self.toolbar.sync_layer_count(self.canvas.layers().len());
        let active = &self.canvas.layers()[self.canvas.active_index()];
        self.grid.lines = active.to_dense();
        Ok(())
    }

    pub fn restore_canvas(&mut self, mut canvas: crate::canvas::LayerStack) {
        canvas.set_enabled(self.toolbar.multi_layer_mode());
        self.toolbar.sync_layer_count(canvas.layers().len());
        let active = &canvas.layers()[canvas.active_index()];
        self.grid.lines = active.to_dense();
        self.canvas = canvas;
    }

    pub fn canvas_origin(&self) -> Coord {
        self.canvas_origin
    }

    pub fn restore_canvas_position(&mut self, cursor: Coord, canvas_origin: Coord) {
        let cursor = clamp_canvas_coord(cursor);
        self.canvas_origin = clamp_canvas_coord(canvas_origin);
        self.grid.cursor_pos = cursor;
        self.sync_cursor_to_active_layer();
        self.selection.collapse(cursor);
    }

    pub fn select_layer(&mut self, id: LayerId) -> bool {
        let Some(index) = self.canvas.index_of(id) else {
            return false;
        };
        let changed = self
            .canvas
            .activate(index, &mut self.grid.lines)
            .expect("editor layers contain valid sparse cells");
        if changed {
            self.toolbar.sync_layer_count(self.canvas.layers().len());
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
            self.commit_canvas();
        }
        changed
    }

    pub fn add_layer_above(&mut self, id: LayerId) -> bool {
        let Some(index) = self.canvas.index_of(id) else {
            return false;
        };
        let changed = self
            .canvas
            .add_above(index, &mut self.grid.lines)
            .expect("editor layers contain valid sparse cells")
            .is_some();
        if changed {
            self.toolbar.sync_layer_count(self.canvas.layers().len());
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
            self.commit_canvas();
        }
        changed
    }

    pub fn toggle_layer_visibility(&mut self, id: LayerId) -> bool {
        let changed = self
            .canvas
            .index_of(id)
            .is_some_and(|index| self.canvas.toggle_visibility(index));
        if changed {
            self.commit_canvas();
        }
        changed
    }

    pub fn move_layer_up(&mut self, id: LayerId) -> bool {
        let changed = self
            .canvas
            .index_of(id)
            .is_some_and(|index| self.canvas.move_up(index));
        if changed {
            self.commit_canvas();
        }
        changed
    }

    pub fn move_layer_down(&mut self, id: LayerId) -> bool {
        let changed = self
            .canvas
            .index_of(id)
            .is_some_and(|index| self.canvas.move_down(index));
        if changed {
            self.commit_canvas();
        }
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
        let changed = self
            .canvas
            .merge_into(index, target, &mut self.grid.lines)
            .expect("editor layers contain valid sparse cells");
        if changed {
            self.toolbar.sync_layer_count(self.canvas.layers().len());
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
            self.commit_canvas();
        }
        changed
    }

    pub fn delete_layer(&mut self, id: LayerId) -> bool {
        let Some(index) = self.canvas.index_of(id) else {
            return false;
        };
        let changed = self
            .canvas
            .delete(index, &mut self.grid.lines)
            .expect("editor layers contain valid sparse cells");
        if changed {
            self.toolbar.sync_layer_count(self.canvas.layers().len());
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
            self.commit_canvas();
        }
        changed
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
        while self.grid.lines.len() <= self.grid.cursor_pos.line {
            self.grid.lines.push(Vec::new());
        }
        self.grid.cursor_pos.column = self
            .grid
            .cursor_pos
            .column
            .min(display_width(&self.grid.lines[self.grid.cursor_pos.line]));
        grid::expose_cursor_cells(
            &mut self.grid.lines[self.grid.cursor_pos.line],
            self.grid.cursor_pos.column,
        );
        self.cursor_index = index_for_column(
            &self.grid.lines[self.grid.cursor_pos.line],
            self.grid.cursor_pos.column,
        );
    }

    pub fn has_shape_preview(&self) -> bool {
        self.shape_preview.is_some()
    }
    pub fn edit_snapshot(&self) -> EditSnapshot {
        if let Some(lift) = &self.move_lift {
            return lift.source_snapshot.clone();
        }
        let (lines, cursor_pos, cursor_index, selection, line_markers, canvas_origin) =
            if let Some(preview) = self
                .line_preview
                .as_ref()
                .filter(|preview| !preview.has_committed_segments())
            {
                (
                    preview.source_lines.clone(),
                    preview.source_cursor,
                    preview.source_cursor_index,
                    preview.source_selection,
                    preview.source_canvas.active_line_markers(),
                    preview.source_canvas_origin,
                )
            } else {
                (
                    self.grid.lines.clone(),
                    self.grid.cursor_pos,
                    self.cursor_index,
                    self.selection,
                    self.canvas.active_line_markers(),
                    self.canvas_origin,
                )
            };
        let mut canvas = self.canvas.clone();
        canvas
            .commit_active_with_markers(&lines, &line_markers)
            .expect("editor layers contain valid sparse cells");
        EditSnapshot {
            lines,
            cursor_pos,
            cursor_index,
            selection,
            active_stroke: self.active_stroke.clone(),
            canvas_origin,
            canvas,
        }
    }

    pub fn restore_edit_snapshot(&mut self, snapshot: EditSnapshot) {
        self.grid.lines = snapshot.lines;
        self.grid.cursor_pos = snapshot.cursor_pos;
        self.cursor_index = snapshot.cursor_index;
        self.selection = snapshot.selection;
        self.active_stroke = snapshot.active_stroke;
        self.canvas_origin = snapshot.canvas_origin;
        self.canvas = snapshot.canvas;
        self.toolbar.sync_layer_count(self.canvas.layers().len());
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.pending_prepend = (0, 0);
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
                lines: vec![Vec::new()],
                cursor_pos: Coord::default(),
                default_face: theme.default.clone(),
                cursor_face: theme.cursor_block.clone(),
            },
            theme: theme.clone(),
            window_title: window_title.into(),
            cursor_mode: CursorMode::Stamp,
            toolbar,
            selection: CanvasSelection::collapsed_at(Coord::default()),
            cursor_index: 0,
            active_stroke: None,
            canvas,
            line_preview: None,
            shape_preview: None,
            move_lift: None,
            jump_mode: None,
            single_replace_pending: false,
            pending_prepend: (0, 0),
            canvas_origin: Coord::default(),
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

    pub fn restore_menu_selections(&mut self, selections: &DurableMenuSelections) {
        self.end_stroke();
        self.cancel_line_preview();
        self.shape_preview = None;
        self.move_lift = None;
        self.jump_mode = None;
        self.single_replace_pending = false;
        self.collapse_selection();
        self.toolbar.restore_durable_selections(selections);
        self.toolbar.sync_layer_count(self.canvas.layers().len());
        self.sync_cursor_mode_with_toolbar();
        self.commit_canvas();
    }

    pub fn tooltip(&self) -> Tooltip {
        if self.jump_mode.is_some() {
            return Tooltip::Jump;
        }
        if self.toolbar.export_menu_open() {
            return self.toolbar.tooltip();
        }
        if self.move_lift.is_some() {
            return Tooltip::SelectionMoveLift;
        }
        if self.line_preview.is_some() {
            return Tooltip::LinePreview;
        }
        if self.shape_preview.is_some() {
            return Tooltip::ShapePreview;
        }
        if self.single_replace_pending {
            return Tooltip::SingleReplace;
        }
        if !self.selection.is_collapsed() {
            return Tooltip::Selection;
        }
        if self.active_stroke.is_some() {
            return Tooltip::LineStroke;
        }
        match self.cursor_mode {
            CursorMode::Text => Tooltip::Text,
            CursorMode::Replace => Tooltip::Replace,
            _ => self.toolbar.tooltip(),
        }
    }

    #[cfg(test)]
    pub fn toolbar_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        self.toolbar_spans_for_width(row, usize::MAX)
    }

    #[cfg(test)]
    pub fn toolbar_spans_for_width(&self, row: usize, box_width: usize) -> Vec<ToolbarSpan> {
        self.toolbar
            .toolbar_spans_with_layers_for_width(row, box_width, &self.layer_summaries())
    }

    pub fn boxed_toolbar_spans_for_width(&self, row: usize, box_width: usize) -> Vec<ToolbarSpan> {
        self.toolbar
            .boxed_spans_with_layers_for_width(row, box_width, &self.layer_summaries())
    }

    pub fn select_custom_stamp(&mut self, text: &str) -> bool {
        let Some(rectangle) = TextRectangle::from_text(text) else {
            return false;
        };
        let [row] = rectangle.rows.as_slice() else {
            return false;
        };
        let [atom] = row.as_slice() else {
            return false;
        };
        if rectangle.width != 1 || UnicodeWidthStr::width(atom.contents.as_str()) != 1 {
            return false;
        }

        self.end_stroke();
        self.cancel_line_preview();
        self.cancel_move_lift();
        self.shape_preview = None;
        self.toolbar.select_custom_stamp(atom.contents.clone());
        self.sync_cursor_mode_with_toolbar();
        self.commit_canvas();
        true
    }

    pub fn cursor_coordinates(&self) -> (i128, i128) {
        (
            self.grid.cursor_pos.column as i128 - self.canvas_origin.column as i128,
            self.grid.cursor_pos.line as i128 - self.canvas_origin.line as i128,
        )
    }

    pub fn toolbar_action_at(
        &self,
        row: usize,
        column: usize,
        box_width: usize,
    ) -> Option<ToolbarAction> {
        self.toolbar
            .action_at_with_layers(row, column, box_width, &self.layer_summaries())
    }

    pub fn view_active(&self) -> bool {
        self.cursor_mode == CursorMode::Utilities
            && self.toolbar.main_mode() == MainMode::Utilities
            && self.toolbar.utility_kind() == UtilityKind::View
            && !self.toolbar.export_menu_open()
    }

    pub fn handle_toolbar_shortcut(&mut self, key: &Key, modifiers: ModifiersState) -> bool {
        self.toolbar_document_changed = false;
        self.toolbar_viewport_stable = false;
        if self.cursor_mode.accepts_text() {
            self.toolbar.cancel_shortcut();
            return false;
        }
        let export_was_open = self.toolbar.export_menu_open();
        let dark_was_enabled = self.toolbar.dark_mode();
        let old_mode = self.toolbar.main_mode();
        let old_utility = self.toolbar.utility_kind();
        let old_routing = self.toolbar.routing_mode();
        if !self
            .toolbar
            .handle_shortcut_with_layers(key, modifiers, &self.layer_summaries())
        {
            return false;
        }
        self.apply_pending_layer_action();
        if matches!(key, Key::Named(NamedKey::Escape)) && !export_was_open {
            self.collapse_selection();
        }
        if self.toolbar.dark_mode() != dark_was_enabled {
            reverse_theme_colors(&mut self.theme);
            self.sync_theme_faces();
        }
        if self.toolbar.main_mode() != old_mode {
            self.end_stroke();
            self.cancel_line_preview();
            self.shape_preview = None;
            self.sync_cursor_mode_with_toolbar();
        }
        if self.has_line_preview() && self.toolbar.routing_mode() != old_routing {
            self.refresh_line_preview_render();
        }
        if self.move_lift.is_some()
            && (self.toolbar.export_menu_open()
                || self.toolbar.main_mode() != old_mode
                || self.toolbar.utility_kind() != old_utility)
        {
            self.cancel_move_lift();
        }
        true
    }

    pub fn apply_toolbar_action(&mut self, action: ToolbarAction) -> bool {
        self.toolbar_document_changed = false;
        self.toolbar_viewport_stable = false;
        self.cancel_jump();
        let updates_live_route = self.has_line_preview()
            && matches!(action, ToolbarAction::SelectSubmenu { submenu: 3, .. })
            && self.toolbar.main_mode() == MainMode::Line;
        if !updates_live_route {
            self.cancel_line_preview();
        }
        if self.move_lift.is_some() {
            self.cancel_move_lift();
        }
        let dark_was_enabled = self.toolbar.dark_mode();
        let old_mode = self.toolbar.main_mode();
        if !self.toolbar.apply_action(action) {
            return false;
        }
        self.apply_pending_layer_action();
        if self.toolbar.dark_mode() != dark_was_enabled {
            reverse_theme_colors(&mut self.theme);
            self.sync_theme_faces();
        }
        if matches!(action, ToolbarAction::Toggle(_)) && self.toolbar.main_mode() != old_mode {
            self.end_stroke();
            self.shape_preview = None;
            self.sync_cursor_mode_with_toolbar();
        }
        if matches!(
            action,
            ToolbarAction::ToggleExportMenu
                | ToolbarAction::Toggle(_)
                | ToolbarAction::BeginLayersPath
                | ToolbarAction::BeginLayerPath(_)
                | ToolbarAction::BeginColorsPath
                | ToolbarAction::BeginColorPath(_)
                | ToolbarAction::RunExport(_)
        ) {
            return true;
        }
        self.end_stroke();
        self.shape_preview = None;
        self.move_lift = None;
        self.sync_cursor_mode_with_toolbar();
        if updates_live_route {
            self.refresh_line_preview_render();
        }
        true
    }

    pub fn take_toolbar_document_change(&mut self) -> bool {
        std::mem::take(&mut self.toolbar_document_changed)
    }

    pub fn take_toolbar_viewport_stable(&mut self) -> bool {
        std::mem::take(&mut self.toolbar_viewport_stable)
    }

    fn apply_pending_layer_action(&mut self) {
        let Some((layer, operation)) = self.toolbar.take_layer_action() else {
            return;
        };
        self.toolbar_viewport_stable = operation == LayerOperation::Show;
        self.toolbar_document_changed = match operation {
            LayerOperation::Select => {
                self.select_layer(layer);
                false
            }
            LayerOperation::Show => self.toggle_layer_visibility(layer),
            LayerOperation::MoveUp => self.move_layer_up(layer),
            LayerOperation::MoveDown => self.move_layer_down(layer),
            LayerOperation::MergeUp => self.merge_layer_up(layer),
            LayerOperation::MergeDown => self.merge_layer_down(layer),
            LayerOperation::New => self.add_layer_above(layer),
            LayerOperation::Delete => self.delete_layer(layer),
        };
    }

    fn sync_theme_faces(&mut self) {
        self.grid.default_face = self.theme.default.clone();
        self.grid.cursor_face = self.theme.cursor_block.clone();
    }

    pub fn toggle_text_entry(&mut self) {
        self.end_stroke();
        self.cancel_line_preview();
        self.toolbar.cancel_shortcut();
        self.single_replace_pending = false;
        if matches!(self.cursor_mode, CursorMode::Text | CursorMode::Replace) {
            self.sync_cursor_mode_with_toolbar();
        } else {
            self.cursor_mode = CursorMode::Text;
        }
    }

    pub fn toggle_replace_mode(&mut self) {
        self.end_stroke();
        self.cancel_line_preview();
        self.toolbar.cancel_shortcut();
        self.single_replace_pending = false;
        if matches!(self.cursor_mode, CursorMode::Text | CursorMode::Replace) {
            self.sync_cursor_mode_with_toolbar();
        } else {
            self.cursor_mode = CursorMode::Replace;
        }
    }

    pub fn begin_single_replace(&mut self) -> bool {
        if self.selection.is_collapsed()
            && matches!(
                self.cursor_mode,
                CursorMode::Text | CursorMode::Insert | CursorMode::Replace
            )
        {
            return false;
        }
        self.end_stroke();
        self.cancel_line_preview();
        self.toolbar.cancel_shortcut();
        self.single_replace_pending = true;
        self.cursor_mode = CursorMode::Replace;
        true
    }

    pub fn cancel_text_entry(&mut self) -> bool {
        if !self.cursor_mode.accepts_text() {
            return false;
        }
        self.end_stroke();
        self.cancel_line_preview();
        self.shape_preview = None;
        self.move_lift = None;
        self.collapse_selection();
        self.sync_cursor_mode_with_toolbar();
        true
    }

    pub fn prepare_history_command(&mut self) -> bool {
        let changed = self.active_stroke.is_some()
            || self.line_preview.is_some()
            || self.shape_preview.is_some()
            || self.move_lift.is_some()
            || self.jump_mode.is_some()
            || self.toolbar.pending_shortcut().is_some();
        self.end_stroke();
        self.cancel_line_preview();
        self.shape_preview = None;
        self.cancel_move_lift();
        self.cancel_jump();
        self.toolbar.cancel_shortcut();
        changed
    }

    fn sync_cursor_mode_with_toolbar(&mut self) {
        self.toolbar.cancel_shortcut();
        self.single_replace_pending = false;
        self.cursor_mode = match self.toolbar.main_mode() {
            MainMode::Line => CursorMode::MoveDraw,
            MainMode::Stamp => CursorMode::Stamp,
            MainMode::Shapes => CursorMode::Shapes,
            MainMode::Utilities => CursorMode::Utilities,
        };
    }

    pub fn move_home(&mut self) {
        self.end_stroke();
        grid::expose_cursor_cells(&mut self.grid.lines[self.grid.cursor_pos.line], 0);
        self.cursor_index = 0;
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn move_end(&mut self) {
        self.end_stroke();
        let width = display_width(&self.grid.lines[self.grid.cursor_pos.line]);
        grid::expose_cursor_cells(&mut self.grid.lines[self.grid.cursor_pos.line], width);
        self.cursor_index = self.grid.lines[self.grid.cursor_pos.line].len();
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn move_to(&mut self, coord: Coord) -> bool {
        let coord = clamp_canvas_coord(coord);
        self.cancel_line_preview();
        self.cancel_move_lift();
        let old_line_count = self.grid.lines.len();
        let old_width = self
            .grid
            .lines
            .get(coord.line)
            .map_or(0, |line| display_width(line));
        self.end_stroke();
        self.move_to_without_ending_stroke(coord);
        self.collapse_selection();
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.end = self.grid.cursor_pos;
        }
        self.grid.lines.len() != old_line_count || coord.column > old_width
    }

    pub fn resolve_pointer_coord(&mut self, line: i64, column: i64) -> Coord {
        let prepend_lines = (line < 0)
            .then(|| usize::try_from(line.saturating_neg()).unwrap_or(usize::MAX))
            .unwrap_or(0);
        let prepend_columns = (column < 0)
            .then(|| usize::try_from(column.saturating_neg()).unwrap_or(usize::MAX))
            .unwrap_or(0);
        for _ in 0..prepend_lines {
            if !self.prepend_line() {
                break;
            }
        }
        for _ in 0..prepend_columns {
            if !self.prepend_column() {
                break;
            }
        }
        clamp_canvas_coord(Coord {
            line: usize::try_from(line.max(0)).unwrap_or(usize::MAX),
            column: usize::try_from(column.max(0)).unwrap_or(usize::MAX),
        })
    }

    pub fn extend_selection_to(&mut self, coord: Coord) {
        let anchor = self.selection.anchor();
        self.cancel_line_preview();
        self.cancel_move_lift();
        self.end_stroke();
        self.move_to_without_ending_stroke(coord);
        self.selection.select(anchor, self.grid.cursor_pos);
    }

    /// Used when a smaller viewport can no longer contain both the active
    /// cursor and the drawing. The target is an existing content cell, so this
    /// does not allocate blank rows or columns merely because the window was
    /// resized.
    pub fn clamp_cursor_to_content(&mut self, coord: Coord) {
        self.end_stroke();
        self.cancel_line_preview();
        self.shape_preview = None;
        self.move_lift = None;
        self.grid.cursor_pos = coord;
        self.cursor_index = index_for_column(&self.grid.lines[coord.line], coord.column);
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn restore_cursor_after_view(&mut self, line: i64, column: i64) {
        self.end_stroke();
        self.cancel_line_preview();
        self.cancel_move_lift();
        self.move_to_without_ending_stroke(clamp_canvas_coord(Coord {
            line: usize::try_from(line.max(0)).unwrap_or(usize::MAX),
            column: usize::try_from(column.max(0)).unwrap_or(usize::MAX),
        }));
        self.collapse_selection();
    }

    fn move_to_without_ending_stroke(&mut self, coord: Coord) {
        let coord = clamp_canvas_coord(coord);
        while self.grid.lines.len() <= coord.line {
            self.grid.lines.push(Vec::new());
        }
        self.grid.cursor_pos.line = coord.line;
        grid::expose_cursor_cells(&mut self.grid.lines[coord.line], coord.column);
        self.cursor_index = index_for_column(&self.grid.lines[coord.line], coord.column);
        let current_width = display_width(&self.grid.lines[coord.line][..self.cursor_index]);
        if current_width < coord.column && self.cursor_index == self.grid.lines[coord.line].len() {
            if let Some(blank) = grid::blank_run(coord.column - current_width) {
                self.grid.lines[coord.line].push(blank);
            }
            self.cursor_index = self.grid.lines[coord.line].len();
        }
        self.sync_cursor_column();
    }

    pub fn move_cursor(&mut self, direction: Direction) -> bool {
        if self.line_preview.is_some() {
            return self.move_line_preview(direction);
        }
        let changed = self.move_or_draw(direction, false);
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.end = self.grid.cursor_pos;
        }
        changed
    }

    /// Resolves the cursor coordinate produced by a non-prepending move
    /// without mutating the grid. Runtime navigation uses this to validate the
    /// viewport before applying a lightweight cursor/selection update.
    pub fn navigation_target(
        &self,
        direction: Direction,
        extend_selection: bool,
        steps: usize,
    ) -> Option<Coord> {
        let mut cursor = self.grid.cursor_pos;
        for _ in 0..steps {
            cursor = self.navigation_target_from(cursor, direction, extend_selection)?;
        }
        Some(cursor)
    }

    pub fn cursor_target_for_coord(&self, target: Coord) -> Coord {
        let Some(line) = self.grid.lines.get(target.line) else {
            return target;
        };
        let index = index_for_column(line, target.column);
        if index < line.len() && !grid::is_blank_run(&line[index]) {
            Coord {
                line: target.line,
                column: display_width(&line[..index]),
            }
        } else {
            target
        }
    }

    fn navigation_target_from(
        &self,
        cursor: Coord,
        direction: Direction,
        extend_selection: bool,
    ) -> Option<Coord> {
        if matches!(direction, Direction::Up) && cursor.line == 0
            || matches!(direction, Direction::Left) && cursor.column == 0
        {
            return None;
        }
        let target = adjacent_coord(cursor, direction)?;
        if extend_selection {
            return Some(target);
        }
        let Some(line) = self.grid.lines.get(target.line) else {
            return Some(target);
        };
        let index = index_for_column(line, target.column);
        let column = if index < line.len() && !grid::is_blank_run(&line[index]) {
            display_width(&line[..index])
        } else {
            target.column
        };
        Some(Coord {
            line: target.line,
            column,
        })
    }

    pub fn extend_selection(&mut self, direction: Direction) -> bool {
        self.cancel_line_preview();
        self.cancel_move_lift();
        let Some(prepended) = self.prepare_adjacent(direction) else {
            return false;
        };
        let to = adjacent_coord(self.grid.cursor_pos, direction)
            .expect("canvas edge was structurally extended");
        self.end_stroke();
        self.shape_preview = None;
        self.move_selection_to_without_ending_stroke(to);
        self.selection.set_active(self.grid.cursor_pos);
        prepended
    }

    /// Moves one cell while erasing the traversed edge. Connected line cells
    /// lose only that edge; every other non-blank atom is replaced by
    /// display-width-preserving blank cells.
    pub fn erase(&mut self, direction: Direction) -> bool {
        self.cancel_line_preview();
        self.cancel_move_lift();
        if self.prepare_adjacent(direction).is_none() {
            return false;
        }
        self.end_stroke();
        self.shape_preview = None;
        let from = self.grid.cursor_pos;
        let to = adjacent_coord(from, direction).expect("canvas edge was structurally extended");
        let erased_from = self.erase_connection_or_atom(from, direction);
        self.move_to_without_ending_stroke(to);
        let erased_to = self.erase_connection_or_atom(to, direction.opposite());
        self.collapse_selection();
        erased_from || erased_to
    }

    fn erase_connection_or_atom(&mut self, coord: Coord, direction: Direction) -> bool {
        let is_line = self.canvas.line_at(coord).is_some()
            || self.cell_contents(coord).is_some_and(is_line_glyph);
        if is_line {
            let before_contents = self.cell_contents(coord).map(str::to_owned);
            let had_marker = self.canvas.line_at(coord).is_some();
            self.remove_connection(coord, direction);
            return had_marker || self.cell_contents(coord).map(str::to_owned) != before_contents;
        }
        self.clear_atom_at(coord)
    }

    fn clear_atom_at(&mut self, coord: Coord) -> bool {
        let Some(line) = self.grid.lines.get_mut(coord.line) else {
            return false;
        };
        let (index, start_column) = index_and_column_for_coord(line, coord.column);
        let Some(atom) = line.get(index) else {
            return false;
        };
        if atom.contents.chars().all(char::is_whitespace) {
            return false;
        }
        let width = atom_width(atom);
        self.canvas.remap_active_line_data(|marker| {
            (marker.line != coord.line
                || marker.column < start_column
                || marker.column >= start_column.saturating_add(width))
            .then_some(marker)
        });
        line.splice(index..=index, grid::blank_run(width));
        for column in start_column..start_column.saturating_add(width) {
            self.sync_sparse_cell_from_dense(Coord {
                line: coord.line,
                column,
            });
        }
        true
    }

    fn move_selection_to_without_ending_stroke(&mut self, coord: Coord) {
        while self.grid.lines.len() <= coord.line {
            self.grid.lines.push(Vec::new());
        }
        let line = &mut self.grid.lines[coord.line];
        let current_width = display_width(line);
        if current_width < coord.column
            && let Some(blank) = grid::blank_run(coord.column - current_width)
        {
            line.push(blank);
        }
        grid::expose_cursor_cells(line, coord.column);
        self.grid.cursor_pos = coord;
        self.cursor_index = index_for_column(line, coord.column);
    }

    pub fn clear_selection(&mut self) {
        self.end_stroke();
        if !self.selection_contains_nonblank() {
            return;
        }
        let bounds = self.selection.bounds();
        self.canvas
            .for_each_layer_dense_mut(&mut self.grid.lines, |_, lines, markers| {
                markers.retain(|marker| !bounds.contains(marker.coord));
                replace_range(lines, bounds, None);
            })
            .expect("editor layers contain valid sparse cells");
        self.restore_active_cursor_index();
    }

    fn selection_contains_nonblank(&self) -> bool {
        let bounds = self.selection.bounds();
        self.layer_views().into_iter().any(|layer| {
            (bounds.top..=bounds.bottom).any(|line_index| {
                let Some(line) = layer.lines.get(line_index) else {
                    return false;
                };
                let mut column: usize = 0;
                line.iter().any(|atom| {
                    let end = column.saturating_add(atom_width(atom));
                    let overlaps = column <= bounds.right && end > bounds.left;
                    column = end;
                    overlaps && !atom.contents.chars().all(char::is_whitespace)
                })
            })
        })
    }

    pub fn place_stamp(&mut self) {
        self.end_stroke();
        let stamp = self.toolbar.stamp().to_string();
        self.replace_selection_literal(Some(&stamp));
    }

    pub fn draw_stamp(&mut self, direction: Direction) {
        if self.prepare_adjacent(direction).is_none() {
            return;
        }
        let to = adjacent_coord(self.grid.cursor_pos, direction)
            .expect("canvas edge was structurally extended");
        self.shape_preview = None;
        self.place_stamp();
        self.move_to_without_ending_stroke(to);
        self.collapse_selection();
        self.place_stamp();
    }

    pub fn selection_bounds(&self) -> SelectionBounds {
        self.selection.bounds()
    }

    #[allow(dead_code)] // Public extraction hook for the queued export implementation.
    pub fn selected_text(&self) -> String {
        selected_text(&self.grid.lines, self.selection.bounds())
    }

    pub fn paste_text_rectangle(&mut self, text: &str) -> bool {
        self.cancel_line_preview();
        let Some(mut rectangle) = TextRectangle::from_text(text) else {
            return false;
        };
        if rectangle
            .rows
            .iter()
            .flatten()
            .any(|atom| atom.validate_cell().is_err())
        {
            self.invalid_text_tip();
            return false;
        }
        if !self.selection.is_collapsed()
            && rectangle.width == 1
            && let [row] = rectangle.rows.as_slice()
            && let [atom] = row.as_slice()
        {
            self.end_stroke();
            self.shape_preview = None;
            self.replace_selection_literal(Some(&atom.contents));
            return true;
        }
        let origin = Coord {
            line: self.selection.bounds().top,
            column: self.selection.bounds().left,
        };
        let bounds = rectangle.bounds_at(origin);
        if bounds.right >= MAX_CANVAS_WIDTH || bounds.bottom >= MAX_CANVAS_HEIGHT {
            return false;
        }
        self.end_stroke();
        self.shape_preview = None;
        let face = self.write_face();
        for atom in rectangle.rows.iter_mut().flatten() {
            if !atom.contents.chars().all(char::is_whitespace) {
                atom.face = face.clone();
            }
        }
        self.commit_canvas();
        self.canvas
            .overwrite_active_rectangle(origin, &rectangle)
            .expect("pasted text contains one-cell atoms");
        self.refresh_active_dense_view();
        let active = Coord {
            line: bounds.bottom,
            column: bounds.right,
        };
        self.selection.select(origin, active);
        self.grid.cursor_pos = active;
        self.cursor_index = index_for_column(&self.grid.lines[active.line], active.column);
        true
    }

    pub fn paste_styled_rectangle_at_cursor(&mut self, rectangle: &TextRectangle) -> bool {
        if rectangle.width == 0
            || rectangle.rows.is_empty()
            || rectangle.rows.iter().any(|row| {
                display_width(row) != rectangle.width
                    || row
                        .iter()
                        .any(|atom| atom.contents.contains('\n') || atom.validate_cell().is_err())
            })
        {
            return false;
        }
        self.cancel_line_preview();
        self.end_stroke();
        self.shape_preview = None;
        self.move_lift = None;

        let origin = self.grid.cursor_pos;
        let bounds = rectangle.bounds_at(origin);
        if bounds.right >= MAX_CANVAS_WIDTH || bounds.bottom >= MAX_CANVAS_HEIGHT {
            return false;
        }
        let mut lines = self.grid.lines.clone();
        overwrite_rectangle(&mut lines, origin, rectangle);
        let removed_marker = self
            .canvas
            .active_line_markers()
            .iter()
            .any(|marker| bounds.contains(marker.coord));
        if lines == self.grid.lines && !removed_marker {
            return false;
        }

        self.commit_canvas();
        self.canvas
            .overwrite_active_rectangle(origin, rectangle)
            .expect("styled rectangle contains one-cell atoms");
        self.refresh_active_dense_view();
        self.selection.collapse(origin);
        self.grid.cursor_pos = origin;
        self.cursor_index = index_for_column(&self.grid.lines[origin.line], origin.column);
        true
    }

    pub fn replace_canvas(&mut self, mut lines: Vec<Vec<Atom>>) {
        truncate_canvas_lines(&mut lines);
        self.canvas.reset();
        self.toolbar.sync_layer_count(self.canvas.layers().len());
        self.grid.lines = if lines.is_empty() {
            vec![Vec::new()]
        } else {
            lines
        };
        self.canvas_origin = edited_content_origin(&self.grid.lines).unwrap_or_default();
        self.grid.cursor_pos = Coord::default();
        self.cursor_index = 0;
        self.active_stroke = None;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.single_replace_pending = false;
        self.pending_prepend = (0, 0);
        self.toolbar.cancel_shortcut();
        self.selection.collapse(Coord::default());
        self.sync_cursor_mode_with_toolbar();
        self.commit_canvas();
    }

    pub fn restore_project(
        &mut self,
        layers: Vec<PersistedLayer>,
        active_layer: LayerId,
        cursor: Coord,
        selection: CanvasSelection,
        menu_selections: &DurableMenuSelections,
    ) -> anyhow::Result<()> {
        self.restore_layers(layers, active_layer)?;
        self.canvas_origin = self
            .layer_views()
            .into_iter()
            .filter_map(|layer| edited_content_origin(&layer.lines))
            .reduce(|origin, candidate| Coord {
                line: origin.line.min(candidate.line),
                column: origin.column.min(candidate.column),
            })
            .unwrap_or_default();
        self.restore_menu_selections(menu_selections);
        let cursor = clamp_canvas_coord(cursor);
        self.grid.cursor_pos = cursor;
        self.cursor_index = index_for_column(&self.grid.lines[cursor.line], cursor.column);
        self.selection.select(
            clamp_canvas_coord(selection.anchor()),
            clamp_canvas_coord(selection.active()),
        );
        self.active_stroke = None;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.single_replace_pending = false;
        self.pending_prepend = (0, 0);
        self.sync_cursor_mode_with_toolbar();
        Ok(())
    }

    pub fn clear_canvas(&mut self) {
        self.cancel_line_preview();
        let cursor = self.grid.cursor_pos;
        self.canvas.clear_contents(&mut self.grid.lines, cursor);

        self.grid.cursor_pos = cursor;
        self.cursor_index = index_for_column(&self.grid.lines[cursor.line], cursor.column);
        self.active_stroke = None;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.single_replace_pending = false;
        self.pending_prepend = (0, 0);
        self.toolbar.cancel_shortcut();
        self.selection.collapse(cursor);
        self.sync_cursor_mode_with_toolbar();
        self.commit_canvas();
    }

    pub fn preview_render_lines(&self) -> Option<&[Vec<Atom>]> {
        self.move_lift_render_lines()
            .or_else(|| self.line_preview_render_lines())
    }

    fn sync_cursor_column(&mut self) {
        self.grid.cursor_pos.column =
            display_width(&self.grid.lines[self.grid.cursor_pos.line][..self.cursor_index]);
        if self.grid.cursor_pos.column >= MAX_CANVAS_WIDTH {
            self.cursor_index = index_for_column(
                &self.grid.lines[self.grid.cursor_pos.line],
                MAX_CANVAS_WIDTH - 1,
            );
            self.grid.cursor_pos.column =
                display_width(&self.grid.lines[self.grid.cursor_pos.line][..self.cursor_index]);
        }
    }

    fn collapse_selection(&mut self) {
        self.cancel_move_lift();
        self.selection.collapse(self.grid.cursor_pos);
    }

    fn expose_cursor_cells(&mut self) {
        let line = self.grid.cursor_pos.line;
        grid::expose_cursor_cells(&mut self.grid.lines[line], self.grid.cursor_pos.column);
        self.cursor_index = index_for_column(&self.grid.lines[line], self.grid.cursor_pos.column);
    }

    fn restore_active_cursor_index(&mut self) {
        let active = self.selection.active();
        self.grid.cursor_pos = active;
        self.cursor_index = index_for_column(&self.grid.lines[active.line], active.column);
    }

    fn replace_selection_literal(&mut self, replacement: Option<&str>) {
        let bounds = self.selection.bounds();
        let legacy_wide = self.canvas.has_legacy_wide_atoms()
            || self
                .grid
                .lines
                .iter()
                .flatten()
                .any(|atom| atom_width(atom) != 1);
        if legacy_wide {
            self.canvas
                .remap_active_line_data(|coord| (!bounds.contains(coord)).then_some(coord));
            replace_range(&mut self.grid.lines, bounds, replacement);
            if replacement.is_some() {
                self.color_written_bounds(bounds);
            }
            self.restore_active_cursor_index();
            self.commit_canvas();
            return;
        }
        let replacement = replacement.map(|contents| {
            let face = if contents.chars().all(char::is_whitespace) {
                Face::default()
            } else {
                self.write_face()
            };
            (
                Atom {
                    face: face.clone(),
                    contents: contents.to_owned(),
                },
                face,
            )
        });
        self.canvas
            .replace_active_bounds(bounds, replacement)
            .expect("literal selection replacements contain one-cell atoms");
        self.refresh_active_dense_view();
        self.restore_active_cursor_index();
    }

    /// Positive values compensate for prepended cells; negative values undo
    /// that compensation when a transient edit restores its source snapshot.
    pub fn take_pending_prepend(&mut self) -> (i64, i64) {
        std::mem::take(&mut self.pending_prepend)
    }

    pub fn content_cells(&self) -> Vec<Coord> {
        let mut cells = self
            .layer_views()
            .into_iter()
            .filter(|layer| layer.visible)
            .flat_map(|layer| grid::content_cells(&layer.lines))
            .collect::<Vec<_>>();
        cells.sort_unstable_by_key(|coord| (coord.line, coord.column));
        cells.dedup();
        cells
    }

    pub fn content_cells_including_hidden(&self) -> Vec<Coord> {
        let mut cells = self
            .layer_views()
            .into_iter()
            .flat_map(|layer| grid::content_cells(&layer.lines))
            .collect::<Vec<_>>();
        cells.sort_unstable_by_key(|coord| (coord.line, coord.column));
        cells.dedup();
        cells
    }

    pub fn compact_blank_runs_preserving_cursor(&mut self) {
        grid::compact_blank_runs(&mut self.grid.lines);
        self.expose_cursor_cells();
    }

    fn prepare_adjacent(&mut self, direction: Direction) -> Option<bool> {
        match direction {
            Direction::Up if self.grid.cursor_pos.line == 0 => {
                if !self.prepend_line() {
                    return None;
                }
                self.canvas_origin.line = self.canvas_origin.line.saturating_add(1);
                Some(true)
            }
            Direction::Left if self.grid.cursor_pos.column == 0 => {
                if !self.prepend_column() {
                    return None;
                }
                self.canvas_origin.column = self.canvas_origin.column.saturating_add(1);
                Some(true)
            }
            _ => adjacent_coord(self.grid.cursor_pos, direction).map(|_| false),
        }
    }

    fn prepend_line(&mut self) -> bool {
        if self.canvas_height() >= MAX_CANVAS_HEIGHT {
            return false;
        }
        self.canvas.prepend_line_to_inactive();
        self.grid.lines.insert(0, Vec::new());
        self.grid.cursor_pos.line = self.grid.cursor_pos.line.saturating_add(1);
        self.selection.shift(0, 1);
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end.line = stroke.end.line.saturating_add(1);
        }
        self.commit_canvas_with_remapped_line_data(|mut coord| {
            coord.line = coord.line.saturating_add(1);
            Some(coord)
        });
        self.shift_line_preview(0, 1);
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.anchor.line = preview.anchor.line.saturating_add(1);
            preview.end.line = preview.end.line.saturating_add(1);
        }
        self.pending_prepend.1 = self.pending_prepend.1.saturating_add(1);
        true
    }

    fn prepend_column(&mut self) -> bool {
        if self.canvas_width() >= MAX_CANVAS_WIDTH {
            return false;
        }
        self.canvas.prepend_column_to_inactive();
        for line in &mut self.grid.lines {
            line.insert(0, blank_atom());
        }
        self.grid.cursor_pos.column = self.grid.cursor_pos.column.saturating_add(1);
        self.cursor_index = self.cursor_index.saturating_add(1);
        self.selection.shift(1, 0);
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end.column = stroke.end.column.saturating_add(1);
        }
        self.commit_canvas_with_remapped_line_data(|mut coord| {
            coord.column = coord.column.saturating_add(1);
            Some(coord)
        });
        self.shift_line_preview(1, 0);
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.anchor.column = preview.anchor.column.saturating_add(1);
            preview.end.column = preview.end.column.saturating_add(1);
        }
        self.pending_prepend.0 = self.pending_prepend.0.saturating_add(1);
        true
    }

    fn canvas_height(&self) -> usize {
        let stored_height = self
            .layer_views()
            .into_iter()
            .map(|layer| layer.lines.len())
            .max()
            .unwrap_or(1);
        stored_height.max(
            self.selection
                .bounds()
                .bottom
                .max(self.grid.cursor_pos.line)
                .saturating_add(1),
        )
    }

    fn canvas_width(&self) -> usize {
        let stored_width = self
            .layer_views()
            .into_iter()
            .flat_map(|layer| layer.lines)
            .map(|line| display_width(&line))
            .max()
            .unwrap_or(0);
        stored_width.max(
            self.selection
                .bounds()
                .right
                .max(self.grid.cursor_pos.column)
                .saturating_add(1),
        )
    }
}

fn clamp_canvas_coord(coord: Coord) -> Coord {
    Coord {
        line: coord.line.min(MAX_CANVAS_HEIGHT - 1),
        column: coord.column.min(MAX_CANVAS_WIDTH - 1),
    }
}

fn truncate_canvas_lines(lines: &mut Vec<Vec<Atom>>) {
    lines.truncate(MAX_CANVAS_HEIGHT);
    for line in lines {
        let mut width: usize = 0;
        let keep = line
            .iter()
            .take_while(|atom| {
                let next = width.saturating_add(atom_width(atom));
                if next > MAX_CANVAS_WIDTH {
                    return false;
                }
                width = next;
                true
            })
            .count();
        line.truncate(keep);
    }
}

fn blank_atom() -> Atom {
    Atom {
        face: Face::default(),
        contents: " ".to_string(),
    }
}

pub(super) fn replace_cell(lines: &mut Vec<Vec<Atom>>, coord: Coord, contents: String) {
    while lines.len() <= coord.line {
        lines.push(Vec::new());
    }
    let line = &mut lines[coord.line];
    let boundary = coord.column.saturating_add(1);
    let mut prefix = Vec::new();
    let mut suffix = Vec::new();
    let mut replacement_face = Face::default();
    let mut column = 0usize;
    for atom in line.iter() {
        let width = atom_width(atom);
        let end = column.saturating_add(width);
        if end <= coord.column {
            prefix.push(atom.clone());
        } else if column < coord.column {
            if grid::is_blank_run(atom) {
                replacement_face = atom.face.clone();
                prefix.extend(grid::blank_run_with_face(
                    atom.face.clone(),
                    coord.column - column,
                ));
            } else {
                prefix.extend((column..coord.column).map(|_| blank_atom()));
            }
        } else if column == coord.column && grid::is_blank_run(atom) {
            replacement_face = atom.face.clone();
        }
        if column >= boundary {
            suffix.push(atom.clone());
        } else if end > boundary {
            if grid::is_blank_run(atom) {
                suffix.extend(grid::blank_run_with_face(atom.face.clone(), end - boundary));
            } else {
                suffix.extend((boundary..end).map(|_| blank_atom()));
            }
        }
        column = end;
    }
    let prefix_width = display_width(&prefix);
    prefix.extend(grid::blank_run(coord.column.saturating_sub(prefix_width)));
    prefix.push(Atom {
        face: replacement_face,
        contents,
    });
    prefix.extend(suffix);
    grid::compact_blank_line(&mut prefix);
    *line = prefix;
}

fn index_and_column_for_coord(atoms: &[Atom], target_column: usize) -> (usize, usize) {
    let mut column = 0;
    for (index, atom) in atoms.iter().enumerate() {
        if target_column < column + atom_width(atom) {
            return (index, column);
        }
        column += atom_width(atom);
    }
    (atoms.len(), column)
}

fn atom_width(atom: &Atom) -> usize {
    UnicodeWidthStr::width(atom.contents.as_str()).max(usize::from(!atom.contents.is_empty()))
}

fn display_width(atoms: &[Atom]) -> usize {
    atoms.iter().map(atom_width).sum()
}

fn index_for_column(atoms: &[Atom], column: usize) -> usize {
    let mut width = 0;
    for (index, atom) in atoms.iter().enumerate() {
        let next = width + atom_width(atom);
        if column < next {
            return index;
        }
        width = next;
    }
    atoms.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_event::EditorState;
    use crate::export::lines_from_text;
    use crate::model::ColorId;
    use crate::toolbar::{ToggleKind, UtilityKind};

    fn state() -> Editor {
        Editor::new(&ThemeConfig::default(), "ascdraw")
    }

    #[test]
    fn editor_state_enum_tracks_modes_and_transient_interactions() {
        let mut editor = state();
        assert_eq!(editor.state(), EditorState::StampMode);

        assert!(
            editor.handle_toolbar_shortcut(&Key::Character("1".into()), ModifiersState::empty())
        );
        assert_eq!(editor.state(), EditorState::ToolbarMode);
        assert!(editor.cancel_current_state());
        assert_eq!(editor.state(), EditorState::StampMode);

        assert!(editor.apply_toolbar_action(ToolbarAction::ToggleExportMenu));
        assert_eq!(editor.state(), EditorState::ExportMode);
        assert!(editor.cancel_current_state());
        assert_eq!(editor.state(), EditorState::StampMode);

        assert!(editor.apply_toolbar_action(ToolbarAction::ToggleExportMenu));
        assert!(
            editor.apply_toolbar_action(ToolbarAction::SelectExportCategory(
                crate::toolbar::FILES_TOGGLE_CATEGORY,
            ))
        );
        assert_eq!(editor.state(), EditorState::ExportMode);
        assert!(editor.cancel_current_state());

        assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        assert_eq!(editor.state(), EditorState::LineMode);
        assert!(!editor.start_or_advance_line_preview());
        assert_eq!(editor.state(), EditorState::LinePreviewMode);
        assert!(editor.cancel_current_state());

        editor.extend_selection(Direction::Right);
        assert_eq!(
            editor.state(),
            EditorState::SelectionMode(CursorMode::MoveDraw)
        );
        assert!(editor.begin_selected_move_lift());
        assert_eq!(editor.state(), EditorState::MoveMode);
        assert!(editor.cancel_current_state());
        assert!(editor.cancel_current_state());
        assert_eq!(editor.state(), EditorState::LineMode);

        assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
        assert_eq!(editor.state(), EditorState::ShapeMode);
        editor.toggle_shape_preview();
        assert_eq!(editor.state(), EditorState::ShapePreviewMode);
        assert!(editor.cancel_current_state());

        editor.toggle_text_entry();
        assert_eq!(editor.state(), EditorState::TextMode);
        assert!(editor.cancel_current_state());
        editor.cursor_mode = CursorMode::Insert;
        assert_eq!(editor.state(), EditorState::InsertMode);
        assert!(editor.cancel_current_state());
        editor.toggle_replace_mode();
        assert_eq!(editor.state(), EditorState::ReplaceMode);
        assert!(editor.cancel_current_state());

        assert!(editor.begin_single_replace());
        assert_eq!(editor.state(), EditorState::ReplaceOneMode);
        assert!(editor.cancel_current_state());

        assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
        assert_eq!(editor.state(), EditorState::UtilityMode);
    }

    #[test]
    fn layer_panel_paths_and_disable_preserve_the_active_editor_mode() {
        let mut editor = state();
        assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        assert!(editor.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
        assert!(editor.apply_toolbar_action(ToolbarAction::BeginLayersPath));
        assert!(
            editor.handle_toolbar_shortcut(&Key::Character("1".into()), ModifiersState::empty())
        );
        assert!(
            editor.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty())
        );
        assert_eq!(editor.toolbar.main_mode(), MainMode::Line);
        assert_eq!(editor.cursor_mode, CursorMode::MoveDraw);

        assert!(editor.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
        assert_eq!(editor.toolbar.main_mode(), MainMode::Line);
        assert_eq!(editor.cursor_mode, CursorMode::MoveDraw);
    }

    #[test]
    fn color_panel_paths_and_disable_preserve_the_active_editor_mode() {
        let mut editor = state();
        assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
        assert!(editor.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode)));
        assert!(editor.apply_toolbar_action(ToolbarAction::BeginColorsPath));
        assert!(
            editor.handle_toolbar_shortcut(&Key::Character("1".into()), ModifiersState::empty())
        );
        assert!(
            editor.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty())
        );
        assert_eq!(editor.toolbar.active_color(), ColorId(1));
        assert_eq!(editor.toolbar.main_mode(), MainMode::Shapes);
        assert_eq!(editor.cursor_mode, CursorMode::Shapes);

        assert!(editor.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode)));
        assert_eq!(editor.toolbar.main_mode(), MainMode::Shapes);
        assert_eq!(editor.cursor_mode, CursorMode::Shapes);
    }

    #[test]
    fn dark_mode_reverses_root_and_preserves_explicit_ui_accent_colors() {
        let source = ThemeConfig::default();
        let mut reversed = source.clone();
        reverse_theme_colors(&mut reversed);
        let mut state = Editor::new(&source, "ascdraw");

        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::DarkMode)));
        assert_eq!(state.theme, reversed);
        assert_eq!(state.grid.default_face, reversed.default);
        assert_eq!(state.grid.cursor_face, reversed.cursor_block);
        assert_eq!(state.theme.selection, source.selection);
        assert_eq!(state.theme.selection_highlight, source.selection_highlight);
        assert_eq!(state.theme.color_selection, source.color_selection);
        assert_eq!(state.theme.jump_grid, source.jump_grid);
        assert_eq!(state.theme.cursor_drawing, source.cursor_drawing);
        assert_eq!(state.theme.tooltip, source.tooltip);

        let selection = crate::face_resolution::resolve_derived_face(
            &state.grid.default_face,
            &state.theme.selection,
            crate::face_resolution::Rgba::rgb(0, 0, 0),
            crate::face_resolution::Rgba::rgb(255, 255, 255),
        );
        let highlight = crate::face_resolution::resolve_derived_face(
            &state.grid.default_face,
            &state.theme.selection_highlight,
            crate::face_resolution::Rgba::rgb(0, 0, 0),
            crate::face_resolution::Rgba::rgb(255, 255, 255),
        );
        let tooltip = crate::face_resolution::resolve_derived_face(
            &state.grid.default_face,
            &state.theme.tooltip,
            crate::face_resolution::Rgba::rgb(0, 0, 0),
            crate::face_resolution::Rgba::rgb(255, 255, 255),
        );
        assert_eq!(selection.fg, crate::face_resolution::Rgba::rgb(0xff, 0, 0));
        assert_eq!(
            highlight.fg,
            crate::face_resolution::Rgba::rgb(0x00, 0x4d, 0xff)
        );
        assert_eq!(
            tooltip.fg,
            crate::face_resolution::Rgba::rgb(0x80, 0x80, 0x80)
        );
        assert_eq!(tooltip.bg, crate::face_resolution::Rgba::rgb(0, 0, 0));

        state.apply_theme(&source);
        assert_eq!(state.theme, reversed);

        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::DarkMode)));
        assert_eq!(state.theme, source);
    }

    #[test]
    fn cursor_coordinates_use_downward_positive_screen_coordinates() {
        let mut state = state();

        for (direction, expected) in [
            (Direction::Right, (1, 0)),
            (Direction::Down, (1, 1)),
            (Direction::Left, (0, 1)),
            (Direction::Up, (0, 0)),
            (Direction::Left, (-1, 0)),
            (Direction::Up, (-1, -1)),
        ] {
            state.move_cursor(direction);
            assert_eq!(state.cursor_coordinates(), expected);
        }
    }

    #[test]
    fn loaded_document_origin_is_its_top_left_edited_cell_and_stays_fixed() {
        let mut state = state();
        state.replace_canvas(vec![
            Vec::new(),
            vec![
                blank_atom(),
                Atom {
                    face: Face::default(),
                    contents: "x".to_owned(),
                },
            ],
        ]);

        assert_eq!(state.cursor_coordinates(), (-1, -1));
        state.move_to(Coord { line: 1, column: 1 });
        assert_eq!(state.cursor_coordinates(), (0, 0));

        state.move_cursor(Direction::Left);
        state.place_stamp();
        assert_eq!(state.cursor_coordinates(), (-1, 0));
        state.clear_selection();
        assert_eq!(state.cursor_coordinates(), (-1, 0));
    }

    #[test]
    fn cursor_coordinates_are_centered_on_the_minimap_border_comma() {
        let mut state = state();
        state.move_to(Coord {
            line: 8,
            column: 10,
        });

        let border = crate::toolbar::toolbar_minimap_border_spans(
            80,
            crate::layout::MINIMAP_COLUMNS,
            state.cursor_coordinates(),
        );
        let contents = border[0].contents.as_str();
        assert!(contents.contains("(10,8)"));
        assert_eq!(contents.chars().nth((59 + 78) / 2), Some(','));
    }

    #[test]
    fn layer_state_swaps_active_content_and_round_trips_in_edit_snapshots() {
        let mut state = state();
        state.insert("a");
        let base = state.active_layer_id();

        assert!(state.add_layer_above(base));
        let upper = state.active_layer_id();
        assert_ne!(upper, base);
        state.insert("b");

        let views = state.layer_views();
        assert_eq!(contents(&views[0].lines[0]), "a");
        assert_eq!(contents(&views[1].lines[0]), "b");

        let snapshot = state.edit_snapshot();
        assert!(state.select_layer(base));
        state.insert("c");
        assert_eq!(contents(&state.grid.lines[0]), "ac");

        state.restore_edit_snapshot(snapshot);
        assert_eq!(state.active_layer_id(), upper);
        assert_eq!(contents(&state.grid.lines[0]), "b");
        assert_eq!(contents(&state.layer_views()[0].lines[0]), "a");
    }

    #[test]
    fn layer_limits_base_rules_reordering_deletion_and_symbol_reuse_are_stable() {
        let mut state = state();
        let base = state.active_layer_id();
        let mut created = Vec::new();
        for _ in 1..crate::model::MAX_LAYERS {
            let active = state.active_layer_id();
            assert!(state.add_layer_above(active));
            created.push(state.active_layer_id());
        }
        assert_eq!(state.layer_summaries().len(), crate::model::MAX_LAYERS);
        assert!(!state.add_layer_above(state.active_layer_id()));
        assert!(!state.move_layer_up(base));
        assert!(!state.move_layer_down(base));
        assert!(!state.delete_layer(base));

        assert!(state.toggle_layer_visibility(base));
        assert!(!state.layer_summaries()[0].visible);
        assert!(state.select_layer(base));
        state.insert("base");
        assert_eq!(contents(&state.grid.lines[0]), "base");

        let removed = created[2];
        let preserved_active = *created.last().unwrap();
        assert!(state.select_layer(preserved_active));
        assert!(state.delete_layer(removed));
        assert_eq!(state.active_layer_id(), preserved_active);
        assert!(state.add_layer_above(base));
        assert_eq!(state.active_layer_id(), removed);

        let active = state.active_layer_id();
        assert!(state.delete_layer(active));
        assert_eq!(state.active_layer_id(), base);
    }

    #[test]
    fn minimap_projection_content_includes_hidden_layers() {
        let mut state = state();
        state.grid.lines = lines_from_text("x");
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let upper = state.active_layer_id();
        state.grid.lines = lines_from_text("   y");
        assert!(state.toggle_layer_visibility(upper));

        assert_eq!(state.content_cells(), vec![Coord::default()]);
        assert_eq!(
            state.content_cells_including_hidden(),
            vec![Coord::default(), Coord { line: 0, column: 3 }]
        );
    }

    #[test]
    fn layer_panel_arrows_move_toward_the_displayed_row_direction() {
        let mut state = state();
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let middle = state.active_layer_id();
        assert!(state.add_layer_above(middle));
        let top = state.active_layer_id();
        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));

        for key in ["8", "3", "4"] {
            assert!(
                state.handle_toolbar_shortcut(&Key::Character(key.into()), ModifiersState::empty())
            );
        }
        assert_eq!(
            state
                .layer_summaries()
                .iter()
                .map(|layer| layer.id)
                .collect::<Vec<_>>(),
            [base, top, middle]
        );

        for key in ["8", "2", "5"] {
            assert!(
                state.handle_toolbar_shortcut(&Key::Character(key.into()), ModifiersState::empty())
            );
        }
        assert_eq!(
            state
                .layer_summaries()
                .iter()
                .map(|layer| layer.id)
                .collect::<Vec<_>>(),
            [base, middle, top]
        );
    }

    #[test]
    fn layer_merge_consumes_source_and_overlays_nonblank_atoms_and_markers() {
        let mut state = state();
        state.grid.lines = lines_from_text("A界z");
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "界".into(),
        });
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let source = state.active_layer_id();
        state.grid.lines = lines_from_text(" B ");
        let source_face = state.theme.tooltip.clone();
        state.grid.lines[0][1].face = source_face.clone();
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('●'),
            base_glyph: "B".into(),
        });
        assert!(state.add_layer_above(source));
        let top = state.active_layer_id();
        state.grid.lines = lines_from_text("top");

        assert!(state.merge_layer_up(source));

        assert_eq!(state.active_layer_id(), base);
        assert_eq!(
            state
                .layer_summaries()
                .iter()
                .map(|layer| layer.id)
                .collect::<Vec<_>>(),
            [base, top]
        );
        assert_eq!(contents(&state.grid.lines[0]), "AB z");
        assert_eq!(state.grid.lines[0][1].face, source_face);
        assert_eq!(state.line_markers_for_test().len(), 1);
        assert_eq!(
            state.line_markers_for_test()[0].ending,
            LineEnding::Fixed('●')
        );
        assert_eq!(contents(&state.layer_views()[1].lines[0]), "top");
        assert!(!state.merge_layer_up(base));
        assert!(!state.merge_layer_down(base));
        assert!(!state.merge_layer_down(top));

        let mut down = Editor::new(&ThemeConfig::default(), "ascdraw");
        down.grid.lines = lines_from_text("base");
        let base = down.active_layer_id();
        assert!(down.add_layer_above(base));
        let source = down.active_layer_id();
        down.grid.lines = lines_from_text(" M");
        assert!(down.add_layer_above(source));
        let target = down.active_layer_id();
        down.grid.lines = lines_from_text("T");

        assert!(down.merge_layer_down(source));
        assert_eq!(down.active_layer_id(), target);
        assert_eq!(contents(&down.grid.lines[0]), "TM");
        assert_eq!(down.layer_summaries().len(), 2);
    }

    #[test]
    fn shifted_layer_shortcut_merges_and_consumes_the_selected_layer() {
        let mut state = Editor::new(&ThemeConfig::default(), "ascdraw");
        state.grid.lines = lines_from_text("base");
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let source = state.active_layer_id();
        state.grid.lines = lines_from_text(" top");
        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));

        for key in ["8", "2"] {
            assert!(
                state
                    .handle_toolbar_shortcut(&Key::Character(key.into()), ModifiersState::empty(),)
            );
        }
        assert!(state.handle_toolbar_shortcut(&Key::Character("4".into()), ModifiersState::SHIFT,));

        assert_eq!(state.layer_summaries().len(), 1);
        assert_eq!(state.active_layer_id(), base);
        assert_ne!(state.active_layer_id(), source);
        assert_eq!(contents(&state.grid.lines[0]), "btop");
    }

    #[test]
    fn clear_applies_to_every_layer_while_move_lift_only_applies_to_visible_layers() {
        let mut state = state();
        state.grid.lines = lines_from_text("A");
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let upper = state.active_layer_id();
        state.grid.lines = lines_from_text(" B");
        assert!(state.toggle_layer_visibility(base));
        let summaries = state.layer_summaries();

        state.move_to(Coord::default());
        state.extend_selection(Direction::Right);
        state.clear_selection();
        assert_eq!(state.layer_summaries(), summaries);
        assert!(state.layer_views().iter().all(|layer| {
            layer
                .lines
                .iter()
                .flatten()
                .all(|atom| atom.contents.chars().all(char::is_whitespace))
        }));

        assert!(state.select_layer(base));
        state.grid.lines = lines_from_text("A");
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord::default(),
            ending: LineEnding::Fixed('◆'),
            base_glyph: "A".into(),
        });
        assert!(state.select_layer(upper));
        state.grid.lines = lines_from_text(" B");
        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        state.color_written_bounds(SelectionBounds {
            left: 1,
            right: 1,
            top: 0,
            bottom: 0,
        });
        let upper_face = state.grid.lines[0][1].face.clone();
        assert_ne!(upper_face, Face::default());
        state.move_to(Coord::default());
        state.extend_selection(Direction::Right);
        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Right));
        assert!(state.move_lift_render_lines_for_layer(base).is_none());
        assert_eq!(
            contents(
                &state
                    .move_lift_render_lines_for_layer(upper)
                    .expect("upper layer preview")[0]
            ),
            "  B"
        );
        assert!(state.confirm_move_lift());

        let views = state.layer_views();
        assert_eq!(contents(&views[0].lines[0]), "A");
        assert_eq!(contents(&views[1].lines[0]), "  B");
        assert_eq!(views[1].lines[0][2].face, upper_face);
        assert_eq!(state.layer_summaries(), summaries);
        assert!(state.select_layer(base));
        assert_eq!(state.line_markers_for_test().len(), 1);
        assert_eq!(state.line_markers_for_test()[0].coord, Coord::default());
        assert!(state.select_layer(upper));

        state.clear_canvas();
        assert_eq!(state.layer_summaries(), summaries);
        assert!(
            state
                .layer_views()
                .iter()
                .all(|layer| layer.lines.iter().flatten().all(|atom| {
                    atom.contents.chars().all(char::is_whitespace) && atom.face == Face::default()
                }))
        );
    }

    #[test]
    fn push_and_pull_apply_the_same_structural_change_to_every_layer() {
        let mut state = utility_state(&["ABC"], UtilityKind::Push, Coord::default());
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let upper = state.active_layer_id();
        state.grid.lines = lines_from_text(" xyz");

        assert!(state.apply_utility(Direction::Right));
        let views = state.layer_views();
        assert_eq!(contents(&views[0].lines[0]), "A BC");
        assert_eq!(contents(&views[1].lines[0]), "  xyz");

        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 1,
        });
        assert!(state.apply_utility(Direction::Left));
        let views = state.layer_views();
        assert_eq!(contents(&views[0].lines[0]), "ABC");
        assert_eq!(contents(&views[1].lines[0]), " xyz");
        assert_eq!(state.active_layer_id(), upper);
    }

    #[test]
    fn selected_color_applies_only_to_future_nonblank_writes_in_every_editing_path() {
        let color = crate::model::ColorId(9);
        let foreground = color.hex().unwrap();

        let mut text = state();
        text.insert("a");
        text.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        text.apply_toolbar_action(ToolbarAction::SelectColor(color));
        text.insert("b");
        assert_eq!(text.grid.lines[0][0].face, Face::default());
        assert_eq!(text.grid.lines[0][1].face.fg, foreground);

        text.move_home();
        assert!(text.begin_single_replace());
        text.write_text("r");
        assert_eq!(text.grid.lines[0][0].face.fg, foreground);
        text.toggle_replace_mode();
        text.write_text("z");
        assert_eq!(text.grid.lines[0][0].face.fg, foreground);

        let mut stamp = state();
        stamp.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        stamp.apply_toolbar_action(ToolbarAction::SelectColor(color));
        stamp.place_stamp();
        assert_eq!(stamp.grid.lines[0][0].face.fg, foreground);

        let mut line = state();
        line.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        line.apply_toolbar_action(ToolbarAction::SelectColor(color));
        line.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        assert!(line.move_or_draw(Direction::Right, true));
        assert!(
            line.grid
                .lines
                .iter()
                .flatten()
                .filter(|atom| !atom.contents.chars().all(char::is_whitespace))
                .all(|atom| atom.face.fg == foreground)
        );

        let mut shape = state();
        shape.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        shape.apply_toolbar_action(ToolbarAction::SelectColor(color));
        shape.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        shape.toggle_shape_preview();
        shape.move_cursor(Direction::Right);
        shape.move_cursor(Direction::Down);
        shape.confirm_shape();
        assert!(
            shape
                .grid
                .lines
                .iter()
                .flatten()
                .filter(|atom| !atom.contents.chars().all(char::is_whitespace))
                .all(|atom| atom.face.fg == foreground)
        );

        let mut paste = state();
        paste.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        paste.apply_toolbar_action(ToolbarAction::SelectColor(color));
        assert!(paste.paste_text_rectangle("p q"));
        assert_eq!(paste.grid.lines[0][0].face.fg, foreground);
        assert_eq!(paste.grid.lines[0][1].face, Face::default());
        assert_eq!(paste.grid.lines[0][2].face.fg, foreground);
    }

    #[test]
    fn disabling_colors_stops_future_coloring_and_moves_preserve_existing_colors() {
        let color = crate::model::ColorId(10);
        let foreground = color.hex().unwrap();
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        state.apply_toolbar_action(ToolbarAction::SelectColor(color));
        state.insert("x");
        state.move_home();
        state.extend_selection(Direction::Right);
        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Right));
        assert!(state.confirm_move_lift());
        assert_eq!(state.grid.lines[0][1].face.fg, foreground);

        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        state.move_to(Coord { line: 0, column: 2 });
        state.insert("y");
        assert_eq!(state.grid.lines[0][2].face, Face::default());

        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::DarkMode));
        assert_eq!(state.grid.lines[0][1].face.fg, foreground);
    }

    #[test]
    fn line_connection_regeneration_uses_the_current_color() {
        let first = crate::model::ColorId(1);
        let second = crate::model::ColorId(6);
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        state.apply_toolbar_action(ToolbarAction::SelectColor(first));
        assert!(state.move_or_draw(Direction::Right, true));
        state.end_stroke();

        state.apply_toolbar_action(ToolbarAction::SelectColor(second));
        assert!(state.move_or_draw(Direction::Down, true));

        assert_eq!(state.grid.lines[0][0].face.fg, first.hex().unwrap());
        assert_eq!(state.grid.lines[0][1].face.fg, second.hex().unwrap());
        assert_eq!(
            state.grid.lines[1]
                .iter()
                .find(|atom| !atom.contents.chars().all(char::is_whitespace))
                .unwrap()
                .face
                .fg,
            second.hex().unwrap()
        );
        assert!(state.erase(Direction::Up));
        assert!(
            state
                .grid
                .lines
                .iter()
                .flatten()
                .filter(|atom| atom.contents.chars().all(char::is_whitespace))
                .all(|atom| atom.face == Face::default())
        );
    }

    #[test]
    fn edit_snapshot_restores_document_cursor_selection_and_line_continuation_only() {
        let mut state = state();
        state.move_or_draw(Direction::Right, true);
        state
            .selection
            .select(Coord::default(), state.grid.cursor_pos);
        let snapshot = state.edit_snapshot();

        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
        state.toggle_text_entry();
        state.window_title = "current title".into();
        state.theme.selection.fg = "#123456".into();
        state.shape_preview = Some(ShapePreview {
            anchor: Coord::default(),
            end: Coord { line: 1, column: 1 },
        });
        state.insert("changed");

        state.restore_edit_snapshot(snapshot.clone());

        assert_eq!(state.edit_snapshot(), snapshot);
        assert_eq!(state.cursor_mode, CursorMode::Text);
        assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
        assert_eq!(state.window_title, "current title");
        assert_eq!(state.theme.selection.fg, "#123456");
        assert!(state.shape_preview.is_none());
        assert_eq!(state.pending_prepend, (0, 0));
    }

    #[test]
    fn restoring_durable_menu_state_syncs_mode_and_clears_transient_editor_state() {
        let mut selected = ToolbarState::default();
        selected.apply_action(ToolbarAction::SelectMain(MainMode::Utilities));
        selected.apply_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        });
        let selections = selected.durable_selections();

        let mut state = state();
        state
            .selection
            .select(Coord::default(), Coord { line: 2, column: 2 });
        state.shape_preview = Some(ShapePreview {
            anchor: Coord::default(),
            end: Coord { line: 1, column: 1 },
        });
        state.cursor_mode = CursorMode::Replace;
        state.single_replace_pending = true;
        state.restore_menu_selections(&selections);

        assert_eq!(state.toolbar.durable_selections(), selections);
        assert_eq!(state.toolbar.main_mode(), MainMode::Utilities);
        assert_eq!(state.toolbar.utility_kind(), UtilityKind::View);
        assert_eq!(state.cursor_mode, CursorMode::Utilities);
        assert!(state.selection.is_collapsed());
        assert!(state.shape_preview.is_none());
        assert!(!state.single_replace_pending);
        assert!(!state.toolbar.export_menu_open());
        assert_eq!(state.toolbar.pending_shortcut(), None);
    }

    #[test]
    fn clearing_an_already_blank_selection_is_an_exact_document_no_op() {
        let mut state = state();
        state.extend_selection(Direction::Right);
        let before = state.edit_snapshot();

        state.clear_selection();

        assert_eq!(state.edit_snapshot(), before);
    }

    #[test]
    fn clear_canvas_resets_cells_faces_cursor_selection_and_drawing_transients() {
        let mut state = state();
        state.grid.lines = vec![vec![Atom {
            face: Face {
                fg: "#123456".into(),
                bg: "#abcdef".into(),
                underline: "#fedcba".into(),
                attributes: vec!["reverse".into()],
            },
            contents: "x".into(),
        }]];
        state.grid.cursor_pos = Coord { line: 3, column: 4 };
        state.cursor_index = 1;
        state
            .selection
            .select(Coord { line: 1, column: 2 }, Coord { line: 3, column: 4 });
        state.active_stroke = Some(ActiveStroke {
            end: Coord { line: 3, column: 4 },
            end_base_glyph: "─".into(),
            moving_ending: LineEnding::Directional(
                crate::drawing::DirectionalEnding::BlackTriangle,
            ),
            incoming_connection: Direction::Left,
            end_was_existing_line: false,
        });
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord::default(),
            ending: LineEnding::Directional(crate::drawing::DirectionalEnding::BlackTriangle),
            base_glyph: "─".into(),
        });
        state.shape_preview = Some(ShapePreview {
            anchor: Coord::default(),
            end: Coord { line: 3, column: 4 },
        });
        state.single_replace_pending = true;
        state.pending_prepend = (2, 3);

        state.clear_canvas();

        assert_eq!(state.grid.lines.len(), 4);
        assert!(state.grid.lines[..3].iter().all(Vec::is_empty));
        assert_eq!(display_width(&state.grid.lines[3]), 4);
        assert!(state.content_cells().is_empty());
        assert_eq!(state.grid.cursor_pos, Coord { line: 3, column: 4 });
        assert_eq!(state.cursor_index, 4);
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.active(), Coord { line: 3, column: 4 });
        assert!(state.active_stroke.is_none());
        assert!(state.line_markers_for_test().is_empty());
        assert!(state.shape_preview.is_none());
        assert!(!state.single_replace_pending);
        assert_eq!(state.pending_prepend, (0, 0));
        assert_eq!(state.cursor_mode, CursorMode::Stamp);
    }

    #[test]
    fn clear_canvas_preserves_a_far_cursor_and_later_inserts_there() {
        let mut state = state();
        state.grid.lines = vec![
            vec![Atom {
                face: Face::default(),
                contents: "drawing".into(),
            }],
            Vec::new(),
            vec![Atom {
                face: Face::default(),
                contents: "x".into(),
            }],
        ];
        let cursor = Coord {
            line: 5,
            column: 12,
        };
        state.move_to(cursor);

        state.clear_canvas();

        assert_eq!(state.grid.cursor_pos, cursor);
        assert_eq!(state.selection.active(), cursor);
        assert!(state.content_cells().is_empty());
        assert_eq!(state.grid.lines.len(), cursor.line + 1);
        assert_eq!(display_width(&state.grid.lines[cursor.line]), cursor.column);

        state.insert("x");
        assert_eq!(state.grid.lines[cursor.line][cursor.column].contents, "x");
        assert_eq!(
            state.grid.cursor_pos,
            Coord {
                line: cursor.line,
                column: cursor.column + 1,
            }
        );
    }

    #[test]
    fn clear_canvas_removes_faces_from_styled_whitespace() {
        let theme = ThemeConfig::default();
        let mut state = Editor::new(&theme, "ascdraw");
        let cursor = Coord { line: 2, column: 3 };
        state.grid.lines = vec![
            vec![Atom {
                face: theme.selection.clone(),
                contents: " ".into(),
            }],
            Vec::new(),
            vec![Atom {
                face: theme.tooltip.clone(),
                contents: "   ".into(),
            }],
        ];
        state.grid.cursor_pos = cursor;

        state.clear_canvas();

        assert_eq!(state.grid.cursor_pos, cursor);
        assert!(state.content_cells().is_empty());
        assert!(
            state
                .grid
                .lines
                .iter()
                .flatten()
                .all(|atom| atom.face == Face::default())
        );
    }

    #[test]
    fn clear_canvas_on_a_canonical_blank_is_an_exact_document_no_op() {
        let mut state = state();
        let before = state.edit_snapshot();

        state.clear_canvas();

        assert_eq!(state.edit_snapshot(), before);
    }

    #[test]
    fn erasing_moves_across_and_clears_general_non_line_content() {
        let mut state = state();
        state.insert("x●◆");
        state.move_to(Coord::default());

        assert!(state.erase(Direction::Right));
        assert_eq!(contents(&state.grid.lines[0]), "  ◆");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });

        assert!(state.erase(Direction::Right));
        assert_eq!(contents(&state.grid.lines[0]), "   ");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 2 });
    }

    #[test]
    fn erasing_a_traversed_line_edge_preserves_unrelated_connections() {
        let mut state = state();
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);

        assert!(state.erase(Direction::Left));

        assert_eq!(contents(&state.grid.lines[0]), "╶╴ ");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
    }

    #[test]
    fn erasing_a_display_cell_preserves_the_row_width() {
        let mut state = state();
        state.insert("ABC");
        state.move_to(Coord { line: 0, column: 2 });

        assert!(state.erase(Direction::Left));

        assert_eq!(contents(&state.grid.lines[0]), "A  ");
        assert_eq!(display_width(&state.grid.lines[0]), 3);
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
    }

    #[test]
    fn blank_origin_erasing_prepends_safely_but_reports_no_document_edit() {
        let mut state = state();

        assert!(!state.erase(Direction::Left));

        assert_eq!(state.grid.cursor_pos, Coord::default());
        assert_eq!(state.take_pending_prepend(), (1, 0));
        assert!(state.selection.is_collapsed());
    }

    #[test]
    fn inserts_and_edits_multiple_lines() {
        let mut state = state();
        state.insert("ab\ncd");
        assert_eq!(state.grid.lines.len(), 2);
        assert_eq!(state.grid.cursor_pos, Coord { line: 1, column: 2 });
        state.backspace();
        assert_eq!(state.grid.cursor_pos.column, 1);
    }

    #[test]
    fn replace_mode_overwrites_instead_of_inserting() {
        let mut state = state();
        state.insert("abc");
        state.move_to(Coord { line: 0, column: 1 });
        state.toggle_replace_mode();

        state.write_text("XY");

        assert_eq!(contents(&state.grid.lines[0]), "aXY");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 3 });
        state.toggle_replace_mode();
        assert_eq!(state.cursor_mode, CursorMode::Stamp);
    }

    #[test]
    fn insert_shifts_line_markers_by_one_cell() {
        let mut state = state();
        state.insert("a◆");
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "╴".into(),
        });
        state.move_to(Coord::default());
        state.toggle_text_entry();

        state.write_text("z");

        assert_eq!(contents(&state.grid.lines[0]), "za◆");
        assert_eq!(
            state.line_markers_for_test()[0].coord,
            Coord { line: 0, column: 2 }
        );
    }

    #[test]
    fn replace_removes_overwritten_markers_and_shifts_following_markers() {
        let mut state = state();
        state.insert("a◆");
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "╴".into(),
        });
        state.move_to(Coord::default());
        state.toggle_replace_mode();

        state.write_text("z");

        assert_eq!(contents(&state.grid.lines[0]), "z◆");
        assert_eq!(
            state.line_markers_for_test()[0].coord,
            Coord { line: 0, column: 1 }
        );

        state.write_text("x");

        assert_eq!(contents(&state.grid.lines[0]), "zx");
        assert!(state.line_markers_for_test().is_empty());
    }

    #[test]
    fn newline_backspace_and_delete_remap_line_markers() {
        let mut state = state();
        state.insert("a◆\nb◆");
        state.extend_line_markers_for_test([
            PlacedLineMarker {
                coord: Coord { line: 0, column: 1 },
                ending: LineEnding::Fixed('◆'),
                base_glyph: "╴".into(),
            },
            PlacedLineMarker {
                coord: Coord { line: 1, column: 1 },
                ending: LineEnding::Fixed('◆'),
                base_glyph: "╴".into(),
            },
        ]);
        assert_eq!(state.line_markers_for_test().len(), 2);
        state.move_to(Coord { line: 0, column: 1 });

        state.newline();

        assert_eq!(
            state.line_markers_for_test()[0].coord,
            Coord { line: 1, column: 0 }
        );
        assert_eq!(
            state.line_markers_for_test()[1].coord,
            Coord { line: 2, column: 1 }
        );

        state.backspace();

        assert_eq!(
            state.line_markers_for_test()[0].coord,
            Coord { line: 0, column: 1 }
        );
        assert_eq!(
            state.line_markers_for_test()[1].coord,
            Coord { line: 1, column: 1 }
        );

        state.delete();

        assert_eq!(contents(&state.grid.lines[0]), "a");
        assert_eq!(state.line_markers_for_test().len(), 1);
        assert_eq!(
            state.line_markers_for_test()[0].coord,
            Coord { line: 1, column: 1 }
        );

        state.delete();

        assert_eq!(contents(&state.grid.lines[0]), "ab◆");
        assert_eq!(
            state.line_markers_for_test()[0].coord,
            Coord { line: 0, column: 2 }
        );
    }

    #[test]
    fn single_replace_consumes_one_grapheme_without_moving_the_cursor() {
        let mut state = state();
        state.insert("abc");
        state.move_to(Coord { line: 0, column: 1 });

        assert!(state.begin_single_replace());
        assert_eq!(state.cursor_mode, CursorMode::Replace);
        state.write_text("XY");

        assert_eq!(contents(&state.grid.lines[0]), "aXc");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
        assert_eq!(state.cursor_mode, CursorMode::Stamp);
    }

    #[test]
    fn selection_extension_keeps_its_anchor_and_normal_movement_collapses() {
        let mut state = state();
        state.move_to(Coord { line: 2, column: 2 });

        state.extend_selection(Direction::Left);
        state.extend_selection(Direction::Left);
        state.extend_selection(Direction::Up);
        assert_eq!(
            state.selection_bounds(),
            SelectionBounds {
                left: 0,
                right: 2,
                top: 1,
                bottom: 2,
            }
        );
        assert_eq!(state.selection.active(), state.grid.cursor_pos);

        state.extend_selection(Direction::Right);
        state.extend_selection(Direction::Right);
        state.extend_selection(Direction::Right);
        assert_eq!(state.selection.anchor(), Coord { line: 2, column: 2 });
        assert_eq!(state.selection.active(), Coord { line: 1, column: 3 });
        assert_eq!(state.selection_bounds().left, 2);
        assert_eq!(state.selection_bounds().right, 3);

        state.move_cursor(Direction::Down);
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.active(), state.grid.cursor_pos);

        state.extend_selection(Direction::Right);
        state.move_to(Coord { line: 4, column: 7 });
        assert_eq!(
            state.selection_bounds(),
            SelectionBounds {
                left: 7,
                right: 7,
                top: 4,
                bottom: 4
            }
        );
    }

    #[test]
    fn top_and_left_prepend_shift_anchor_while_active_enters_new_cell() {
        let mut state = state();
        state.move_to(Coord { line: 0, column: 0 });

        assert!(state.extend_selection(Direction::Up));
        assert_eq!(state.selection.anchor(), Coord { line: 1, column: 0 });
        assert_eq!(state.selection.active(), Coord { line: 0, column: 0 });
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 0 });

        assert!(state.extend_selection(Direction::Left));
        assert_eq!(state.selection.anchor(), Coord { line: 1, column: 1 });
        assert_eq!(state.selection.active(), Coord { line: 0, column: 0 });
        assert_eq!(state.take_pending_prepend(), (1, 1));
    }

    #[test]
    fn extending_selection_to_a_blank_cell_above_content_preserves_its_anchor() {
        let mut state = state();
        state.grid.lines = lines_from_text("\n\n\ncontent");
        state.move_to(Coord { line: 3, column: 2 });

        state.extend_selection_to(Coord { line: 0, column: 1 });

        assert_eq!(state.selection.anchor(), Coord { line: 3, column: 2 });
        assert_eq!(state.selection.active(), Coord { line: 0, column: 1 });
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
    }

    #[test]
    fn moving_to_a_drag_start_collapses_a_previous_selection() {
        let mut state = state();
        state.grid.lines = lines_from_text("abcdef");
        state.move_to(Coord { line: 0, column: 5 });
        state.extend_selection_to(Coord { line: 0, column: 1 });

        state.move_to(Coord { line: 0, column: 3 });

        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.anchor(), Coord { line: 0, column: 3 });
        assert_eq!(state.selection.active(), Coord { line: 0, column: 3 });
    }

    #[test]
    fn range_clear_is_rectangular_across_short_rows_and_wide_graphemes() {
        let mut state = state();
        state.grid.lines = vec![
            vec![
                Atom {
                    face: Face::default(),
                    contents: "a".into(),
                },
                Atom {
                    face: Face::default(),
                    contents: "😀".into(),
                },
                Atom {
                    face: Face::default(),
                    contents: "z".into(),
                },
            ],
            Vec::new(),
        ];
        state.move_to(Coord { line: 0, column: 1 });
        state.extend_selection(Direction::Right);
        state.extend_selection(Direction::Down);

        state.clear_selection();

        assert_eq!(state.selected_text(), "  \n  ");
        assert_eq!(contents(&state.grid.lines[0]), "a  z");
        assert_eq!(display_width(&state.grid.lines[1]), 3);
        assert_eq!(state.grid.cursor_pos, Coord { line: 1, column: 2 });
        assert_eq!(
            state.selection_bounds(),
            SelectionBounds {
                left: 1,
                right: 2,
                top: 0,
                bottom: 1
            }
        );
    }

    #[test]
    fn clear_is_literal_and_does_not_cap_neighboring_line_cells() {
        let mut state = state();
        state.insert("│\n│\n│");
        state.move_to(Coord { line: 1, column: 0 });

        state.clear_selection();

        assert_eq!(contents(&state.grid.lines[0]), "│");
        assert_eq!(contents(&state.grid.lines[1]), " ");
        assert_eq!(contents(&state.grid.lines[2]), "│");
    }

    #[test]
    fn rectangular_clear_leaves_every_perimeter_atom_and_face_unchanged() {
        let mut state = state();
        let perimeter_face = state.theme.selection.clone();
        let center_face = state.theme.cursor_drawing.clone();
        state.grid.lines = ["┌┬┐", "├┼┤", "└┴┘"]
            .into_iter()
            .map(|row| {
                row.chars()
                    .map(|contents| Atom {
                        face: perimeter_face.clone(),
                        contents: contents.to_string(),
                    })
                    .collect()
            })
            .collect();
        state.grid.lines[1][1].face = center_face;
        let before = state.grid.lines.clone();
        state.move_to(Coord { line: 1, column: 1 });

        state.clear_selection();

        for coord in [
            Coord { line: 0, column: 0 },
            Coord { line: 0, column: 1 },
            Coord { line: 0, column: 2 },
            Coord { line: 1, column: 0 },
            Coord { line: 1, column: 2 },
            Coord { line: 2, column: 0 },
            Coord { line: 2, column: 1 },
            Coord { line: 2, column: 2 },
        ] {
            assert_eq!(
                state.grid.lines[coord.line][coord.column],
                before[coord.line][coord.column]
            );
        }
        assert_eq!(contents(&state.grid.lines[1]), "├ ┤");
    }

    #[test]
    fn clear_removes_only_markers_whose_cells_are_selected() {
        let mut state = state();
        state.insert("◆─◆");
        let inside = PlacedLineMarker {
            coord: Coord { line: 0, column: 0 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "─".into(),
        };
        let outside = PlacedLineMarker {
            coord: Coord { line: 0, column: 2 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "─".into(),
        };
        state.set_line_markers_for_test(vec![inside, outside.clone()]);
        state.move_to(Coord::default());

        state.clear_selection();

        assert_eq!(contents(&state.grid.lines[0]), " ─◆");
        assert_eq!(state.line_markers_for_test(), vec![outside]);
    }

    #[test]
    fn single_replacement_preserves_neighboring_line_segments() {
        let mut state = state();
        state.insert("╷\n│\n╵");
        state.move_to(Coord { line: 1, column: 0 });

        assert!(state.begin_single_replace());
        state.write_text("x");

        assert_eq!(contents(&state.grid.lines[0]), "╷");
        assert_eq!(contents(&state.grid.lines[1]), "x");
        assert_eq!(contents(&state.grid.lines[2]), "╵");
    }

    #[test]
    fn paste_rectangular_overwrite_uses_selection_origin_and_selects_result() {
        let mut state = state();
        let outside = Face {
            fg: "#123456".to_string(),
            ..Face::default()
        };
        state.grid.lines = vec![
            vec![
                Atom {
                    face: outside.clone(),
                    contents: "L".into(),
                },
                Atom {
                    face: outside.clone(),
                    contents: "a".into(),
                },
                Atom {
                    face: outside.clone(),
                    contents: "b".into(),
                },
                Atom {
                    face: outside.clone(),
                    contents: "R".into(),
                },
            ],
            vec![
                Atom {
                    face: outside.clone(),
                    contents: "l".into(),
                },
                Atom {
                    face: outside.clone(),
                    contents: "c".into(),
                },
                Atom {
                    face: outside.clone(),
                    contents: "d".into(),
                },
                Atom {
                    face: outside.clone(),
                    contents: "r".into(),
                },
            ],
        ];
        state.move_to(Coord { line: 1, column: 2 });
        state.extend_selection(Direction::Left);
        state.extend_selection(Direction::Up);

        assert!(state.paste_text_rectangle("x\nYZ"));

        assert_eq!(contents(&state.grid.lines[0]), "Lx R");
        assert_eq!(contents(&state.grid.lines[1]), "lYZr");
        assert_eq!(state.grid.lines[0][0].face, outside);
        assert_eq!(state.grid.lines[0][1].face, Face::default());
        assert_eq!(state.selection.anchor(), Coord { line: 0, column: 1 });
        assert_eq!(state.selection.active(), Coord { line: 1, column: 2 });
        assert_eq!(state.grid.cursor_pos, Coord { line: 1, column: 2 });
        assert_eq!(state.selected_text(), "x \nYZ");
    }

    #[test]
    fn single_cell_paste_fills_the_current_selection() {
        let mut state = state();
        state.insert("abcd\nefgh");
        state.move_to(Coord { line: 1, column: 2 });
        state.extend_selection(Direction::Left);
        state.extend_selection(Direction::Up);

        assert!(state.paste_text("x"));

        assert_eq!(contents(&state.grid.lines[0]), "axxd");
        assert_eq!(contents(&state.grid.lines[1]), "exxh");
        assert_eq!(state.selection_bounds().width(), 2);
        assert_eq!(state.selection_bounds().height(), 2);
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
    }

    #[test]
    fn paste_rejects_wide_source_graphemes_transactionally() {
        let mut state = state();
        state.move_to(Coord { line: 2, column: 3 });
        let before = state.edit_snapshot();

        assert!(!state.paste_text_rectangle("😀\r\nq"));

        assert_eq!(state.edit_snapshot(), before);
        assert!(state.transient_tip().is_some());
    }

    #[test]
    fn styled_toolbar_paste_uses_cursor_origin_active_layer_dimensions_and_one_undo() {
        use crate::history::{EditHistory, HistorySnapshot};
        use crate::layout::ViewportOffset;
        use crate::selection::{CanvasRegion, region_atoms};
        use crate::toolbar_stamp::styled_toolbar_snapshot;

        let mut editor = state();
        editor.insert("base");
        let base = editor.active_layer_id();
        assert!(editor.add_layer_above(base));
        let upper = editor.active_layer_id();
        let base_before = editor.layer_views()[0].lines.to_vec();
        editor.move_to(Coord { line: 2, column: 3 });
        editor.canvas_origin = Coord { line: 4, column: 5 };
        let origin = editor.grid.cursor_pos;
        let signed_origin = editor.cursor_coordinates();
        let rectangle = styled_toolbar_snapshot(&editor, 52).unwrap();
        let previous = HistorySnapshot {
            edit: editor.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };

        assert!(editor.paste_styled_rectangle_at_cursor(&rectangle));
        assert_eq!(
            editor.navigation_target(Direction::Right, false, 1),
            Some(Coord {
                line: origin.line,
                column: origin.column + 1,
            })
        );
        assert_eq!(editor.grid.cursor_pos, origin);
        assert_eq!(editor.cursor_coordinates(), signed_origin);
        assert!(editor.selection.is_collapsed());
        assert_eq!(editor.active_layer_id(), upper);
        assert_eq!(editor.layer_views()[0].lines, base_before);
        let pasted = region_atoms(
            &editor.grid.lines,
            CanvasRegion {
                left: origin.column as i64,
                top: origin.line as i64,
                width: rectangle.width,
                height: rectangle.rows.len(),
            },
        );
        assert_eq!(pasted, rectangle.rows);
        assert!(pasted.iter().any(|row| {
            row.iter().any(|atom| {
                atom.contents.bytes().all(|byte| byte == b' ') && atom.face.bg != "default"
            })
        }));

        let current = HistorySnapshot {
            edit: editor.edit_snapshot(),
            viewport: ViewportOffset::default(),
        };
        let mut history = EditHistory::default();
        assert!(history.record_change(previous.clone(), &current));
        assert_eq!(history.lengths(), (1, 0));
        let undone = history.undo(current).unwrap();
        editor.restore_edit_snapshot(undone.edit);
        assert_eq!(editor.edit_snapshot(), previous.edit);
        assert_eq!(history.lengths(), (0, 1));
    }

    #[test]
    fn plain_toolbar_hotspot_click_is_an_exact_editor_no_op() {
        use crate::toolbar_stamp::{HotspotClick, hotspot_click, styled_toolbar_snapshot};

        let mut editor = state();
        editor.insert("unchanged");
        let before = editor.edit_snapshot();
        let click = hotspot_click(Some(48), ModifiersState::empty());
        if let HotspotClick::Paste { box_width } = click {
            let rectangle = styled_toolbar_snapshot(&editor, box_width).unwrap();
            editor.paste_styled_rectangle_at_cursor(&rectangle);
        }

        assert_eq!(click, HotspotClick::Consume);
        assert_eq!(editor.edit_snapshot(), before);
    }

    #[test]
    fn single_replace_fills_the_range_and_restores_mode_without_moving_active_corner() {
        let mut state = state();
        state.grid.lines = vec![
            vec![blank_atom(), blank_atom(), blank_atom()],
            vec![blank_atom(), blank_atom(), blank_atom()],
        ];
        state.move_to(Coord { line: 0, column: 0 });
        state.extend_selection(Direction::Right);
        state.extend_selection(Direction::Right);
        state.extend_selection(Direction::Down);
        let active = state.grid.cursor_pos;

        assert!(state.begin_single_replace());
        state.write_text("zignored");

        assert_eq!(state.selected_text(), "zzz\nzzz");
        assert_eq!(state.grid.cursor_pos, active);
        assert_eq!(state.selection.active(), active);
        assert_eq!(state.cursor_mode, CursorMode::Stamp);
    }

    #[test]
    fn stamp_space_fills_every_selected_cell_and_keeps_the_range() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
        state.move_to(Coord { line: 0, column: 0 });
        state.extend_selection(Direction::Right);
        state.extend_selection(Direction::Down);

        state.place_stamp();

        assert_eq!(state.selected_text(), "□□\n□□");
        assert_eq!(state.grid.cursor_pos, Coord { line: 1, column: 1 });
        assert_eq!(
            state.selection_bounds(),
            SelectionBounds {
                left: 0,
                right: 1,
                top: 0,
                bottom: 1
            }
        );
    }

    #[test]
    fn selected_text_excludes_everything_outside_the_normalized_rectangle() {
        let mut state = state();
        state.insert("outside\n012345\noutside");
        state.move_to(Coord { line: 1, column: 4 });
        for _ in 0..3 {
            state.extend_selection(Direction::Left);
        }

        assert_eq!(state.selected_text(), "1234");
    }

    #[test]
    fn escape_and_text_cancellation_collapse_expanded_selection() {
        let mut state = state();
        state.extend_selection(Direction::Right);
        state.start_shape_or_confirm();
        assert!(state.selection.is_collapsed());

        state.extend_selection(Direction::Right);
        state.toggle_replace_mode();
        assert!(state.cancel_text_entry());
        assert!(state.selection.is_collapsed());
    }

    #[test]
    fn prefix_escape_also_collapses_selection_without_changing_toolbar_mode() {
        let mut state = state();
        state.extend_selection(Direction::Right);
        assert!(
            state.handle_toolbar_shortcut(&Key::Character("1".into()), ModifiersState::empty(),)
        );

        assert!(
            state.handle_toolbar_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty(),)
        );

        assert!(state.selection.is_collapsed());
        assert_eq!(state.toolbar.pending_shortcut(), None);
        assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
    }

    #[test]
    fn single_replace_cannot_start_in_text_insert_or_replace_modes() {
        let mut state = state();
        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            state.cursor_mode = mode;
            assert!(!state.begin_single_replace());
            assert_eq!(state.cursor_mode, mode);
        }
    }

    #[test]
    fn cancelling_text_replace_and_single_replace_restores_the_toolbar_mode() {
        for editing_mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            let mut state = state();
            assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
            state.cursor_mode = editing_mode;

            assert!(state.cancel_text_entry());

            assert_eq!(state.cursor_mode, CursorMode::Stamp);
        }

        let mut state = state();
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
        assert!(state.begin_single_replace());

        assert!(state.cancel_text_entry());

        assert_eq!(state.cursor_mode, CursorMode::Shapes);
    }

    #[test]
    fn cancelling_text_entry_clears_a_pending_toolbar_prefix() {
        let mut state = state();
        assert!(
            state
                .toolbar
                .handle_shortcut(&Key::Character("1".into()), ModifiersState::empty())
        );
        state.cursor_mode = CursorMode::Replace;

        assert!(state.cancel_text_entry());
        assert!(
            state.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty(),)
        );

        assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
    }

    #[test]
    fn invalid_wide_text_is_transactional_and_sets_a_tip() {
        let mut state = state();
        state.insert("x");
        let before = state.edit_snapshot();
        state.insert("😀x");
        assert_eq!(state.edit_snapshot(), before);
        assert!(state.transient_tip().is_some());
    }

    #[test]
    fn clicking_beyond_content_pads_the_canvas() {
        let mut state = state();
        state.move_to(Coord { line: 2, column: 4 });
        assert_eq!(state.grid.lines.len(), 3);
        assert_eq!(state.grid.lines[2].len(), 1);
        assert_eq!(contents(&state.grid.lines[2]), "    ");
        assert_eq!(state.grid.cursor_pos, Coord { line: 2, column: 4 });
    }

    #[test]
    fn move_draw_uses_grid_movement_without_wrapping() {
        let mut state = state();
        state.move_or_draw(Direction::Right, false);
        state.move_or_draw(Direction::Down, false);
        assert_eq!(state.grid.cursor_pos, Coord { line: 1, column: 1 });
        assert_eq!(state.grid.lines.len(), 2);
        assert_eq!(contents(&state.grid.lines[0]), " ");
        assert_eq!(contents(&state.grid.lines[1]), " ");
    }

    #[test]
    fn canvas_stops_at_maximum_width_and_height() {
        let mut state = state();
        state.move_to(Coord {
            line: MAX_CANVAS_HEIGHT - 1,
            column: MAX_CANVAS_WIDTH - 1,
        });
        state.insert("xy");

        assert_eq!(state.grid.lines.len(), MAX_CANVAS_HEIGHT);
        assert_eq!(
            display_width(state.grid.lines.last().unwrap()),
            MAX_CANVAS_WIDTH
        );
        assert_eq!(
            state.grid.cursor_pos,
            Coord {
                line: MAX_CANVAS_HEIGHT - 1,
                column: MAX_CANVAS_WIDTH - 1,
            }
        );
        assert!(!state.move_cursor(Direction::Right));
        assert!(!state.move_cursor(Direction::Down));

        state.move_to(Coord::default());
        assert!(!state.move_cursor(Direction::Left));
        assert!(!state.move_cursor(Direction::Up));
        assert_eq!(state.grid.lines.len(), MAX_CANVAS_HEIGHT);
        assert_eq!(
            display_width(state.grid.lines.last().unwrap()),
            MAX_CANVAS_WIDTH
        );
    }

    #[test]
    fn replacing_canvas_truncates_oversized_rows_and_columns() {
        let mut lines = vec![Vec::new(); MAX_CANVAS_HEIGHT + 1];
        lines[0].push(Atom {
            face: Face::default(),
            contents: "x".repeat(MAX_CANVAS_WIDTH + 1),
        });
        let mut state = state();

        state.replace_canvas(lines);

        assert_eq!(state.grid.lines.len(), MAX_CANVAS_HEIGHT);
        assert!(state.grid.lines[0].is_empty());
    }

    #[test]
    fn moving_up_at_zero_prepends_and_shifts_coordinate_state() {
        let mut state = state();
        state.insert("ab");
        state.move_to(Coord { line: 0, column: 1 });
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord::default(),
            ending: LineEnding::Directional(crate::drawing::DirectionalEnding::BlackTriangle),
            base_glyph: "╶".to_string(),
        });
        state.shape_preview = Some(ShapePreview {
            anchor: Coord::default(),
            end: state.grid.cursor_pos,
        });

        assert!(state.move_cursor(Direction::Up));

        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
        assert_eq!(contents(&state.grid.lines[0]), " ");
        assert_eq!(contents(&state.grid.lines[1]), "ab");
        assert_eq!(state.line_markers_for_test()[0].coord.line, 1);
        let preview = state.shape_preview.unwrap();
        assert_eq!(preview.anchor.line, 1);
        assert_eq!(preview.end, state.grid.cursor_pos);
        assert_eq!(state.take_pending_prepend(), (0, 1));
    }

    #[test]
    fn prepending_shifts_an_active_stroke_endpoint() {
        let mut state = state();
        state.active_stroke = Some(ActiveStroke {
            end: Coord::default(),
            end_base_glyph: "─".to_string(),
            moving_ending: LineEnding::None,
            incoming_connection: Direction::Left,
            end_was_existing_line: false,
        });

        state.prepend_line();

        assert_eq!(state.active_stroke.unwrap().end.line, 1);
    }

    #[test]
    fn moving_left_at_zero_prepends_every_line_and_shifts_coordinate_state() {
        let mut state = state();
        state.insert("a\nb");
        state.move_to(Coord::default());
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord { line: 1, column: 0 },
            ending: LineEnding::Directional(crate::drawing::DirectionalEnding::BlackTriangle),
            base_glyph: "╶".to_string(),
        });
        state.shape_preview = Some(ShapePreview {
            anchor: Coord::default(),
            end: Coord { line: 1, column: 0 },
        });

        assert!(state.move_cursor(Direction::Left));

        assert_eq!(state.grid.cursor_pos, Coord::default());
        assert_eq!(contents(&state.grid.lines[0]), " a");
        assert_eq!(contents(&state.grid.lines[1]), " b");
        assert_eq!(state.line_markers_for_test()[0].coord.column, 1);
        let preview = state.shape_preview.unwrap();
        assert_eq!(preview.anchor.column, 1);
        assert_eq!(preview.end, state.grid.cursor_pos);
        assert_eq!(state.take_pending_prepend(), (1, 0));
    }

    #[test]
    fn drawing_connects_across_newly_prepended_top_and_left_cells() {
        let mut top = state();
        top.move_or_draw(Direction::Right, true);
        top.move_or_draw(Direction::Up, true);
        assert_eq!(contents(&top.grid.lines[0]), " ╷");
        assert_eq!(contents(&top.grid.lines[1]), "╶╯");

        let mut left = state();
        left.move_or_draw(Direction::Down, true);
        left.move_or_draw(Direction::Left, true);
        assert_eq!(contents(&left.grid.lines[0]), " ╷");
        assert_eq!(contents(&left.grid.lines[1]), "╶╯");
    }

    #[test]
    fn content_cells_ignore_allocated_blank_padding() {
        let mut state = state();
        state.move_to(Coord {
            line: 8,
            column: 12,
        });
        assert!(state.content_cells().is_empty());
        state.write_text("x");
        assert_eq!(
            state.content_cells(),
            vec![Coord {
                line: 8,
                column: 12,
            }]
        );
    }

    #[test]
    fn viewport_clamp_moves_cursor_and_collapses_selection_without_changing_lines() {
        let mut state = state();
        state.move_to(Coord { line: 5, column: 5 });
        state.write_text("x");
        state.move_to(Coord { line: 1, column: 1 });
        state.extend_selection(Direction::Right);
        let lines = state.grid.lines.clone();

        state.clamp_cursor_to_content(Coord { line: 5, column: 5 });

        assert_eq!(state.grid.cursor_pos, Coord { line: 5, column: 5 });
        assert!(state.selection.is_collapsed());
        assert_eq!(state.selection.active(), state.grid.cursor_pos);
        assert_eq!(state.grid.lines, lines);
    }

    #[test]
    fn draw_connects_straights_and_rounded_corners() {
        let mut state = state();
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "╶─╮");
        assert_eq!(contents(&state.grid.lines[1]), "  ╵");
    }

    #[test]
    fn dashed_style_draws_repeated_half_segments() {
        let mut horizontal = state();
        horizontal.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        horizontal.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 3,
        });
        for _ in 0..4 {
            horizontal.move_or_draw(Direction::Right, true);
        }
        assert_eq!(contents(&horizontal.grid.lines[0]), "╴╴╴╴╴");

        let mut vertical = state();
        vertical.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        vertical.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 3,
        });
        for _ in 0..4 {
            vertical.move_or_draw(Direction::Down, true);
        }
        assert_eq!(contents(&vertical.grid.lines[0]), "╵");
        assert_eq!(contents(&vertical.grid.lines[1]), "╵");
        assert_eq!(contents(&vertical.grid.lines[2]), "╵");
        assert_eq!(contents(&vertical.grid.lines[3]), "╵");
        assert_eq!(contents(&vertical.grid.lines[4]), "╵");
    }

    #[test]
    fn dashed_style_honors_sharp_corner_selection() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 3,
        });
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 4,
            option: 1,
        });

        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "╴╴┐");
        assert_eq!(contents(&state.grid.lines[1]), "  ╵");
    }

    #[test]
    fn dashed_stroke_keeps_the_incoming_direction_when_turning_left_then_up() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 1,
        });
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 3,
        });
        state.move_to(Coord { line: 0, column: 3 });

        for direction in [
            Direction::Down,
            Direction::Down,
            Direction::Left,
            Direction::Left,
            Direction::Left,
            Direction::Up,
            Direction::Up,
        ] {
            state.move_or_draw(direction, true);
        }

        assert_eq!(contents(&state.grid.lines[0]), "╵  △");
        assert_eq!(contents(&state.grid.lines[1]), "╵  ╵");
        assert_eq!(contents(&state.grid.lines[2]), "╰╴╴╯");
    }

    #[test]
    fn draw_connects_crossing_lines() {
        let mut state = state();
        state.move_to(Coord { line: 0, column: 1 });
        state.move_or_draw(Direction::Down, true);
        state.move_or_draw(Direction::Down, true);
        state.move_to(Coord { line: 1, column: 0 });
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);

        assert_eq!(contents(&state.grid.lines[1]), "╶┼╴");
    }

    #[test]
    fn ending_a_stroke_on_an_existing_line_keeps_the_full_tee() {
        let mut state = state();
        state.move_to(Coord { line: 2, column: 1 });
        for direction in [
            Direction::Right,
            Direction::Right,
            Direction::Up,
            Direction::Left,
            Direction::Down,
        ] {
            state.move_or_draw(direction, true);
        }

        assert_eq!(contents(&state.grid.lines[2]), " ╶┴╯");
    }

    #[test]
    fn draw_preserves_non_line_text() {
        let mut state = state();
        state.insert("x");
        state.move_to(Coord { line: 0, column: 0 });
        state.move_or_draw(Direction::Right, true);

        assert_eq!(contents(&state.grid.lines[0]), "x╴");
    }

    #[test]
    fn selected_line_endings_stay_at_the_stroke_endpoints() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        });
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 2,
        });

        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "◀─╮");
        assert_eq!(contents(&state.grid.lines[1]), "  ▼");
    }

    #[test]
    fn fixed_start_and_directional_end_survive_turning_and_marker_history_state() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 11,
        });
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 3,
        });

        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "◆─╮");
        assert_eq!(contents(&state.grid.lines[1]), "  ↓");
        assert_eq!(state.line_markers_for_test().len(), 2);
        assert_eq!(
            state.line_markers_for_test()[0].ending,
            LineEnding::Fixed('◆')
        );
        assert_eq!(
            state.line_markers_for_test()[1].ending,
            LineEnding::Directional(crate::drawing::DirectionalEnding::Arrow)
        );

        let snapshot = state.edit_snapshot();
        state.clear_selection();
        state.restore_edit_snapshot(snapshot);
        assert_eq!(contents(&state.grid.lines[0]), "◆─╮");
        assert_eq!(contents(&state.grid.lines[1]), "  ↓");
        assert_eq!(
            state.line_markers_for_test()[1].ending,
            LineEnding::Directional(crate::drawing::DirectionalEnding::Arrow)
        );
    }

    #[test]
    fn unadorned_endings_use_the_selected_double_line_style() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 2,
        });

        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);

        assert_eq!(contents(&state.grid.lines[0]), "═══");
    }

    #[test]
    fn drawing_from_an_existing_line_keeps_the_full_tee() {
        let mut state = state();
        state.insert("│");
        state.move_to(Coord { line: 0, column: 0 });
        select_toolbar_option(&mut state, "2", 1);

        state.move_or_draw(Direction::Right, true);

        assert_eq!(contents(&state.grid.lines[0]), "├╴");
    }

    #[test]
    fn drawing_from_an_end_marker_moves_it_to_the_new_end() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        select_toolbar_option(&mut state, "3", 2);
        state.move_or_draw(Direction::Right, true);
        state.move_to(Coord { line: 0, column: 1 });

        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "╶╮");
        assert_eq!(contents(&state.grid.lines[1]), " ▼");
    }

    #[test]
    fn drawing_from_a_start_marker_moves_it_to_the_new_end() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        select_toolbar_option(&mut state, "2", 2);
        state.move_or_draw(Direction::Right, true);
        state.move_to(Coord { line: 0, column: 0 });

        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "╭╴");
        assert_eq!(contents(&state.grid.lines[1]), "▼");
    }

    #[test]
    fn clearing_a_cell_preserves_its_canvas_width() {
        let mut state = state();
        state.insert("abx");
        state.move_to(Coord { line: 0, column: 0 });

        state.clear_selection();

        assert_eq!(contents(&state.grid.lines[0]), " bx");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 0 });
    }

    #[test]
    fn toolbar_main_mode_controls_editor_mode() {
        let mut state = state();
        state.toggle_text_entry();
        assert_eq!(state.cursor_mode, CursorMode::Text);
        assert!(!state.handle_toolbar_shortcut(
            &winit::keyboard::Key::Character("1".into()),
            winit::keyboard::ModifiersState::empty(),
        ));
        state.move_cursor(Direction::Right);
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
        state.toggle_text_entry();
        assert_eq!(state.cursor_mode, CursorMode::Stamp);

        for key in ["1", "2"] {
            assert!(state.handle_toolbar_shortcut(
                &winit::keyboard::Key::Character(key.into()),
                winit::keyboard::ModifiersState::empty(),
            ));
        }
        assert_eq!(state.toolbar.main_mode(), MainMode::Line);
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
    }

    #[test]
    fn tooltip_tracks_editor_mode_and_export_override() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        assert_eq!(state.tooltip(), Tooltip::Stamp);

        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp)));
        assert_eq!(state.tooltip(), Tooltip::Stamp);
        state.toggle_text_entry();
        assert_eq!(state.tooltip(), Tooltip::Text);
        state.toggle_text_entry();
        state.toggle_replace_mode();
        assert_eq!(state.tooltip(), Tooltip::Replace);

        assert!(state.apply_toolbar_action(ToolbarAction::ToggleExportMenu));
        assert_eq!(state.tooltip(), Tooltip::Export);
    }

    #[test]
    fn tooltip_reacts_to_selection_and_transient_editor_states() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.insert("abcd");
        state.move_home();
        assert_eq!(state.tooltip(), Tooltip::Stamp);
        assert!(state.tooltip().text().starts_with("Stamp:"));

        state.extend_selection(Direction::Right);
        assert_eq!(state.tooltip(), Tooltip::Selection);
        assert!(
            state
                .tooltip()
                .text()
                .contains("Alt-direction lifts and moves")
        );

        assert!(state.begin_selected_move_lift());
        assert_eq!(state.tooltip(), Tooltip::SelectionMoveLift);
        assert!(
            state
                .tooltip()
                .text()
                .contains("direction confirms and moves")
        );
        assert!(state.cancel_move_lift());

        state.move_to(Coord::default());
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.toggle_shape_preview();
        assert_eq!(state.tooltip(), Tooltip::ShapePreview);
        assert!(state.tooltip().text().contains("Space confirms"));

        state.toggle_shape_preview();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
        assert!(state.begin_single_replace());
        assert_eq!(state.tooltip(), Tooltip::SingleReplace);
        assert!(
            state
                .tooltip()
                .text()
                .contains("type or paste one character")
        );

        state.cancel_text_entry();
        state.clear_canvas();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        state.move_or_draw(Direction::Right, true);
        assert_eq!(state.tooltip(), Tooltip::LineStroke);
        assert!(state.tooltip().text().contains("release Ctrl to finish"));
    }

    #[test]
    fn export_activation_is_transient_and_does_not_mutate_editor_state() {
        let mut state = state();
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
        assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        }));
        state.insert("canvas");
        let edit = state.edit_snapshot();
        let cursor_mode = state.cursor_mode;
        let durable = state.toolbar.durable_selections();

        assert!(
            state.handle_toolbar_shortcut(&Key::Character("0".into()), ModifiersState::empty(),)
        );
        assert!(state.toolbar.export_menu_open());
        assert_eq!(state.edit_snapshot(), edit);
        assert_eq!(state.cursor_mode, cursor_mode);
        assert_eq!(state.toolbar.durable_selections(), durable);

        assert!(
            state.handle_toolbar_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty(),)
        );
        assert!(!state.toolbar.export_menu_open());
        assert_eq!(state.edit_snapshot(), edit);
        assert_eq!(state.cursor_mode, cursor_mode);
        assert_eq!(state.toolbar.durable_selections(), durable);

        assert!(state.apply_toolbar_action(ToolbarAction::ToggleExportMenu));
        assert!(state.toolbar.export_menu_open());
        assert_eq!(state.edit_snapshot(), edit);
        assert_eq!(state.cursor_mode, cursor_mode);
        assert_eq!(state.toolbar.durable_selections(), durable);
    }

    #[test]
    fn toolbar_shortcuts_are_bypassed_in_every_text_accepting_mode() {
        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            let mut state = state();
            state.cursor_mode = mode;

            assert!(
                !state
                    .handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty(),)
            );
            assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
        }
    }

    #[test]
    fn text_transition_clears_a_pending_toolbar_prefix() {
        let mut state = state();
        assert!(state.handle_toolbar_shortcut(
            &winit::keyboard::Key::Character("1".into()),
            winit::keyboard::ModifiersState::empty(),
        ));

        state.toggle_text_entry();
        assert_eq!(state.toolbar.pending_shortcut(), None);
        assert!(
            state
                .toolbar
                .toolbar_spans(1)
                .iter()
                .all(|span| !span.highlighted)
        );
        state.toggle_text_entry();
        assert!(state.handle_toolbar_shortcut(
            &winit::keyboard::Key::Character("2".into()),
            winit::keyboard::ModifiersState::empty(),
        ));

        assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
    }

    #[test]
    fn stamp_mode_places_the_exclusively_selected_stamp() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 3,
        });

        state.place_stamp();

        assert_eq!(contents(&state.grid.lines[0]), "█");
        assert_eq!(state.grid.cursor_pos, Coord::default());
    }

    #[test]
    fn custom_stamp_fills_selection_until_a_bundled_stamp_is_selected() {
        let mut state = state();
        state.insert("abcd");
        state.move_to(Coord { line: 0, column: 1 });
        state.extend_selection(Direction::Right);

        assert!(state.select_custom_stamp("◇"));
        state.place_stamp();
        assert_eq!(contents(&state.grid.lines[0]), "a◇◇d");
        assert_eq!(state.toolbar.custom_stamp(), Some("◇"));

        assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 0,
        }));
        assert_eq!(state.toolbar.custom_stamp(), None);
        assert_eq!(state.toolbar.stamp(), "□");
        assert!(!state.select_custom_stamp("😀"));
        assert!(!state.select_custom_stamp("xy"));
    }

    #[test]
    fn stamp_in_middle_of_line_preserves_the_other_segments() {
        let mut state = state();
        state.insert("╷\n│\n╵");
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 0,
        });
        state.move_to(Coord { line: 1, column: 0 });

        state.place_stamp();

        assert_eq!(contents(&state.grid.lines[0]), "╷");
        assert_eq!(contents(&state.grid.lines[1]), "△");
        assert_eq!(contents(&state.grid.lines[2]), "╵");
    }

    #[test]
    fn shift_drawing_in_stamp_mode_stamps_both_ends_of_the_move() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));

        state.draw_stamp(Direction::Right);

        assert_eq!(contents(&state.grid.lines[0]), "□□");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
    }

    #[test]
    fn shape_preview_follows_movement_and_commits_only_on_confirmation() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.toggle_shape_preview();
        for direction in [
            Direction::Right,
            Direction::Right,
            Direction::Right,
            Direction::Down,
            Direction::Down,
        ] {
            state.move_cursor(direction);
        }

        let preview = state.lines_with_shape_preview().expect("preview is active");
        assert_eq!(contents(&preview[0]), "┌──┐");
        assert_eq!(contents(&preview[1]), "│  │");
        assert_eq!(contents(&preview[2]), "└──┘");
        assert!(
            state
                .grid
                .lines
                .iter()
                .flatten()
                .all(|atom| atom.contents.chars().all(char::is_whitespace))
        );

        state.confirm_shape();
        assert!(state.lines_with_shape_preview().is_none());
        assert_eq!(contents(&state.grid.lines[0]), "┌──┐");
        assert_eq!(contents(&state.grid.lines[2]), "└──┘");
    }

    #[test]
    fn shape_space_draws_one_cell_outside_a_selected_region() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state
            .selection
            .select(Coord { line: 3, column: 5 }, Coord { line: 2, column: 3 });

        assert!(state.start_shape_or_confirm());

        assert!(state.selection.is_collapsed());
        assert!(state.shape_preview.is_none());
        assert_eq!(state.take_pending_prepend(), (0, 0));
        assert_eq!(
            state
                .grid
                .lines
                .iter()
                .map(|line| contents(line))
                .collect::<Vec<_>>(),
            ["", "  ┌───┐", "  │   │", "  │   │", "  └───┘"]
        );
    }

    #[test]
    fn shape_space_prepends_to_surround_a_selection_at_the_origin() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state
            .selection
            .select(Coord { line: 1, column: 1 }, Coord { line: 0, column: 0 });

        assert!(state.start_shape_or_confirm());

        assert_eq!(state.take_pending_prepend(), (1, 1));
        assert_eq!(
            state
                .grid
                .lines
                .iter()
                .map(|line| contents(line))
                .collect::<Vec<_>>(),
            ["┌──┐", "│  │", "│  │", "└──┘"]
        );
    }

    #[test]
    fn shape_preview_and_commit_keep_right_edge_aligned_on_ragged_rows() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.grid.lines = [11, 7, 0, 7, 11]
            .into_iter()
            .map(|width| (0..width).map(|_| blank_atom()).collect())
            .collect();
        state.shape_preview = Some(ShapePreview {
            anchor: Coord { line: 0, column: 2 },
            end: Coord {
                line: 4,
                column: 10,
            },
        });

        let preview = state.lines_with_shape_preview().expect("preview is active");
        assert_eq!(
            preview
                .iter()
                .map(|line| contents(line))
                .collect::<Vec<_>>(),
            [
                "  ┌───────┐",
                "  │       │",
                "  │       │",
                "  │       │",
                "  └───────┘",
            ]
        );

        state.confirm_shape();
        assert_eq!(
            state
                .grid
                .lines
                .iter()
                .map(|line| contents(line))
                .collect::<Vec<_>>(),
            [
                "  ┌───────┐",
                "  │       │",
                "  │       │",
                "  │       │",
                "  └───────┘",
            ]
        );
    }

    #[test]
    fn reversed_rounded_shape_extends_one_cell_past_content_and_adds_missing_rows() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 1,
        });
        state.grid.lines = vec![(0..4).map(|_| blank_atom()).collect()];
        state.shape_preview = Some(ShapePreview {
            anchor: Coord { line: 4, column: 4 },
            end: Coord { line: 0, column: 0 },
        });

        let expected = ["╭───╮", "│   │", "│   │", "│   │", "╰───╯"];
        let preview = state.lines_with_shape_preview().expect("preview is active");
        assert_eq!(
            preview
                .iter()
                .map(|line| contents(line))
                .collect::<Vec<_>>(),
            expected
        );

        state.confirm_shape();
        assert_eq!(
            state
                .grid
                .lines
                .iter()
                .map(|line| contents(line))
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn shape_boundary_inside_wide_grapheme_blanks_it_without_moving_the_edge() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        let outside_face = Face {
            fg: "#123456".into(),
            ..Face::default()
        };
        state.grid.lines = vec![
            vec![blank_atom(), blank_atom(), blank_atom(), blank_atom()],
            vec![
                blank_atom(),
                Atom {
                    face: outside_face.clone(),
                    contents: "界".into(),
                },
                Atom {
                    face: outside_face.clone(),
                    contents: "Z".into(),
                },
            ],
            vec![blank_atom(), blank_atom(), blank_atom(), blank_atom()],
        ];
        state.shape_preview = Some(ShapePreview {
            anchor: Coord { line: 0, column: 0 },
            end: Coord { line: 2, column: 2 },
        });

        let preview = state.lines_with_shape_preview().expect("preview is active");
        assert_eq!(contents(&preview[1]), "│ │Z");
        assert_eq!(preview[1][3].face, outside_face);

        state.confirm_shape();
        assert_eq!(contents(&state.grid.lines[1]), "│ │Z");
        assert_eq!(state.grid.lines[1][3].face, outside_face);
    }

    #[test]
    fn escape_cancels_an_active_shape_preview() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.toggle_shape_preview();
        state.move_cursor(Direction::Right);
        assert!(state.lines_with_shape_preview().is_some());

        state.toggle_shape_preview();

        assert!(state.lines_with_shape_preview().is_none());
        assert!(state.grid.lines[0].iter().all(|atom| atom.contents == " "));
    }

    #[test]
    fn history_preparation_cancels_transients_without_closing_export_or_durable_tools() {
        let mut shape = state();
        shape.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        shape.toggle_shape_preview();
        assert!(shape.prepare_history_command());
        assert!(shape.shape_preview.is_none());

        let mut line = state();
        line.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        line.move_or_draw(Direction::Right, true);
        assert!(line.active_stroke.is_some());
        assert!(line.prepare_history_command());
        assert!(line.active_stroke.is_none());

        let mut lift = utility_state(&["abc"], UtilityKind::Push, Coord::default());
        lift.selection
            .select(Coord::default(), Coord { line: 0, column: 1 });
        let before_lift = lift.edit_snapshot();
        assert!(lift.begin_selected_move_lift());
        assert!(lift.move_lift(Direction::Right));
        assert!(lift.prepare_history_command());
        assert!(!lift.move_lift_active());
        assert_eq!(lift.edit_snapshot(), before_lift);

        let mut export = state();
        let durable = export.toolbar.durable_selections();
        export.apply_toolbar_action(ToolbarAction::ToggleExportMenu);
        assert!(export.toolbar.export_menu_open());
        assert!(export.toolbar.pending_shortcut().is_some());
        assert!(export.prepare_history_command());
        assert!(export.toolbar.export_menu_open());
        assert!(export.toolbar.pending_shortcut().is_none());
        assert_eq!(export.toolbar.durable_selections(), durable);
        assert!(!export.prepare_history_command());
    }

    #[test]
    fn rounded_shape_preview_uses_selected_fill() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 1,
        });
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 2,
            option: 1,
        });
        state.toggle_shape_preview();
        for direction in [
            Direction::Right,
            Direction::Right,
            Direction::Right,
            Direction::Down,
            Direction::Down,
        ] {
            state.move_cursor(direction);
        }

        let preview = state.lines_with_shape_preview().unwrap();
        assert_eq!(contents(&preview[0]), "╭──╮");
        assert_eq!(contents(&preview[1]), "│░░│");
        assert_eq!(contents(&preview[2]), "╰──╯");
    }

    #[test]
    fn push_inserts_each_requested_row_and_column() {
        let mut right = utility_state(
            &["ab", "cd"],
            UtilityKind::Push,
            Coord { line: 0, column: 1 },
        );
        assert!(right.apply_utility(Direction::Right));
        assert_eq!(line_contents(&right), vec!["ab ", "cd "]);

        let mut left = utility_state(
            &["ab", "cd"],
            UtilityKind::Push,
            Coord { line: 0, column: 1 },
        );
        assert!(left.apply_utility(Direction::Left));
        assert_eq!(line_contents(&left), vec![" ab", " cd"]);
        assert_eq!(left.grid.cursor_pos.column, 2);

        let mut up = utility_state(&["a", "b"], UtilityKind::Push, Coord { line: 1, column: 0 });
        assert!(up.apply_utility(Direction::Up));
        assert_eq!(line_contents(&up), vec!["", "a", "b"]);
        assert_eq!(up.grid.cursor_pos.line, 2);

        let mut down = utility_state(&["a", "b"], UtilityKind::Push, Coord { line: 0, column: 0 });
        assert!(down.apply_utility(Direction::Down));
        assert_eq!(line_contents(&down), vec!["a", "", "b"]);
        assert_eq!(down.grid.cursor_pos.line, 0);
    }

    #[test]
    fn pull_horizontal_directions_remove_all_content_with_literal_asymmetry() {
        let mut left = utility_state(
            &["abcd", "xy", ""],
            UtilityKind::Pull,
            Coord { line: 0, column: 1 },
        );
        assert!(left.apply_utility(Direction::Left));
        assert_eq!(line_contents(&left), vec!["abd", "xy", ""]);

        let mut right = utility_state(
            &["abcd", "xy", ""],
            UtilityKind::Pull,
            Coord { line: 0, column: 1 },
        );
        assert!(right.apply_utility(Direction::Right));
        assert_eq!(line_contents(&right), vec![" abd", " xy", ""]);
        assert_eq!(right.grid.cursor_pos.column, 2);
    }

    #[test]
    fn pull_left_compresses_every_row_in_the_supplied_overlapping_boxes() {
        let input = [
            "                     ╭────────╮",
            "                     │        │",
            "               ┌─────│─────┐  │",
            "               │     │     │  │",
            "               │     │     │  │",
            "               └─────│─────┘  │",
            "  ╭─────────────╮    │        │",
            "  │             │    ╰────────╯",
            "  │    X        │",
            "  │             │",
            "  │             │",
            "  │             │",
            "  │             │",
            "  ╰─────────────╯",
        ];
        let expected = vec![
            "                    ╭────────╮",
            "                    │        │",
            "              ┌─────│─────┐  │",
            "              │     │     │  │",
            "              │     │     │  │",
            "              └─────│─────┘  │",
            "  ╭────────────╮    │        │",
            "  │            │    ╰────────╯",
            "  │    X       │",
            "  │            │",
            "  │            │",
            "  │            │",
            "  │            │",
            "  ╰────────────╯",
        ];
        let mut state = utility_state(&input, UtilityKind::Pull, Coord { line: 8, column: 7 });

        assert!(state.apply_utility(Direction::Left));
        assert_eq!(line_contents(&state), expected);
    }

    #[test]
    fn pull_horizontal_rejects_the_whole_operation_for_either_wide_atom_cell() {
        for cursor_column in [0, 1] {
            let mut state = utility_state(
                &["abc", "a界z"],
                UtilityKind::Pull,
                Coord {
                    line: 0,
                    column: cursor_column,
                },
            );
            let before = state.edit_snapshot();

            assert!(!state.apply_utility(Direction::Left));
            assert_eq!(state.edit_snapshot(), before);

            assert!(!state.apply_utility(Direction::Right));
            assert_eq!(state.edit_snapshot(), before);
        }
    }

    #[test]
    fn pull_right_shifts_ragged_finite_prefixes_without_growing_empty_rows() {
        let mut state = utility_state(
            &["a", "abcd", "", "xy"],
            UtilityKind::Pull,
            Coord { line: 1, column: 2 },
        );

        assert!(state.apply_utility(Direction::Right));
        assert_eq!(line_contents(&state), vec![" a", " abc", "", " xy"]);
        assert_eq!(state.grid.cursor_pos.column, 3);
    }

    #[test]
    fn pull_preserves_shifted_faces_and_removes_or_remaps_line_metadata() {
        let mut state = utility_state(&["ABCD"], UtilityKind::Pull, Coord { line: 0, column: 0 });
        for (index, atom) in state.grid.lines[0].iter_mut().enumerate() {
            atom.face.fg = format!("#{index}{index}{index}{index}{index}{index}");
        }
        state
            .selection
            .select(Coord { line: 0, column: 0 }, Coord { line: 0, column: 3 });
        state.active_stroke = Some(ActiveStroke {
            end: Coord { line: 0, column: 1 },
            end_base_glyph: "─".into(),
            moving_ending: LineEnding::None,
            incoming_connection: Direction::Left,
            end_was_existing_line: false,
        });
        state.extend_line_markers_for_test([
            PlacedLineMarker {
                coord: Coord { line: 0, column: 1 },
                ending: LineEnding::None,
                base_glyph: "B".into(),
            },
            PlacedLineMarker {
                coord: Coord { line: 0, column: 3 },
                ending: LineEnding::None,
                base_glyph: "D".into(),
            },
        ]);
        state.shape_preview = Some(ShapePreview {
            anchor: Coord { line: 0, column: 0 },
            end: Coord { line: 0, column: 3 },
        });

        assert!(state.apply_utility(Direction::Left));
        assert_eq!(line_contents(&state), vec!["ACD"]);
        assert_eq!(state.grid.lines[0][0].face.fg, "#000000");
        assert_eq!(state.grid.lines[0][1].face.fg, "#222222");
        assert_eq!(state.grid.lines[0][2].face.fg, "#333333");
        assert_eq!(state.selection.active().column, 2);
        assert!(state.active_stroke.is_none());
        assert_eq!(state.line_markers_for_test().len(), 1);
        assert_eq!(state.line_markers_for_test()[0].coord.column, 2);
        assert!(state.shape_preview.is_none());
    }

    #[test]
    fn pull_vertical_directions_remove_entire_rows_with_nonblank_content() {
        let mut up = utility_state(
            &["AX", "BY", "界Z", "CX"],
            UtilityKind::Pull,
            Coord::default(),
        );
        assert!(up.apply_utility(Direction::Up));
        assert_eq!(line_contents(&up), vec!["AX", "界Z", "CX"]);

        let mut down = utility_state(
            &["AX", "BY", "界Z", "CX"],
            UtilityKind::Pull,
            Coord { line: 3, column: 0 },
        );
        assert!(down.apply_utility(Direction::Down));
        assert_eq!(line_contents(&down), vec!["", "AX", "BY", "CX"]);
        assert_eq!(down.grid.cursor_pos.line, 3);
    }

    #[test]
    fn pull_row_removes_target_metadata_and_remaps_every_lower_coordinate() {
        let mut state = utility_state(
            &["A", "B", "C", "D"],
            UtilityKind::Pull,
            Coord { line: 1, column: 0 },
        );
        state
            .selection
            .select(Coord { line: 1, column: 0 }, Coord { line: 3, column: 0 });
        state.active_stroke = Some(ActiveStroke {
            end: Coord { line: 3, column: 0 },
            end_base_glyph: "D".into(),
            moving_ending: LineEnding::None,
            incoming_connection: Direction::Up,
            end_was_existing_line: false,
        });
        state.extend_line_markers_for_test([
            PlacedLineMarker {
                coord: Coord { line: 2, column: 0 },
                ending: LineEnding::None,
                base_glyph: "C".into(),
            },
            PlacedLineMarker {
                coord: Coord { line: 3, column: 0 },
                ending: LineEnding::None,
                base_glyph: "D".into(),
            },
        ]);

        assert!(state.apply_utility(Direction::Up));
        assert_eq!(line_contents(&state), vec!["A", "B", "D"]);
        assert_eq!(state.selection.active().line, 2);
        assert_eq!(state.active_stroke.as_ref().unwrap().end.line, 2);
        assert_eq!(state.line_markers_for_test().len(), 1);
        assert_eq!(state.line_markers_for_test()[0].coord.line, 2);
    }

    #[test]
    fn pull_vertical_no_target_is_no_op_and_origin_down_prepends_safely() {
        let mut no_target = utility_state(&["x"], UtilityKind::Pull, Coord::default());
        let before = no_target.edit_snapshot();
        assert!(!no_target.apply_utility(Direction::Up));
        assert_eq!(no_target.edit_snapshot(), before);

        let mut down = utility_state(&["界", "z"], UtilityKind::Pull, Coord::default());
        assert!(down.apply_utility(Direction::Down));
        assert_eq!(line_contents(&down), vec!["", "界", "z"]);
        assert_eq!(down.grid.cursor_pos.line, 1);
        assert_eq!(down.take_pending_prepend(), (0, 1));

        let mut blank = utility_state(&[""], UtilityKind::Pull, Coord::default());
        assert!(!blank.apply_utility(Direction::Down));
        assert_eq!(line_contents(&blank), vec![""]);

        let mut unchanged =
            utility_state(&["", "x"], UtilityKind::Pull, Coord { line: 1, column: 0 });
        let before = unchanged.edit_snapshot();
        assert!(!unchanged.apply_utility(Direction::Down));
        assert_eq!(unchanged.edit_snapshot(), before);
    }

    #[test]
    fn utility_origin_prepend_and_wide_boundary_are_safe() {
        let mut left = utility_state(&["界x"], UtilityKind::Push, Coord::default());
        assert!(left.apply_utility(Direction::Left));
        assert_eq!(line_contents(&left), vec![" 界x"]);
        assert_eq!(left.grid.cursor_pos.column, 1);
        assert_eq!(left.take_pending_prepend(), (1, 0));

        let mut up = utility_state(&["x"], UtilityKind::Push, Coord::default());
        assert!(up.apply_utility(Direction::Up));
        assert_eq!(line_contents(&up), vec!["", "x"]);
        assert_eq!(up.take_pending_prepend(), (0, 1));

        let mut inside_wide = utility_state(&["界x"], UtilityKind::Push, Coord::default());
        assert!(!inside_wide.apply_utility(Direction::Right));
        assert_eq!(line_contents(&inside_wide), vec!["界x"]);

        let mut pull_down = utility_state(&["x"], UtilityKind::Pull, Coord::default());
        assert!(pull_down.apply_utility(Direction::Down));
        assert_eq!(line_contents(&pull_down), vec!["", "x"]);
        assert_eq!(pull_down.grid.cursor_pos.line, 1);
    }

    #[test]
    fn push_remaps_selection_markers_stroke_and_preview_coordinates() {
        let mut state = utility_state(&["abc"], UtilityKind::Push, Coord { line: 0, column: 2 });
        state
            .selection
            .select(Coord { line: 0, column: 1 }, Coord { line: 0, column: 2 });
        state.active_stroke = Some(ActiveStroke {
            end: Coord { line: 0, column: 2 },
            end_base_glyph: "─".into(),
            moving_ending: LineEnding::None,
            incoming_connection: Direction::Left,
            end_was_existing_line: false,
        });
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord { line: 0, column: 2 },
            ending: LineEnding::Directional(crate::drawing::DirectionalEnding::BlackTriangle),
            base_glyph: "─".into(),
        });
        state.shape_preview = Some(ShapePreview {
            anchor: Coord { line: 0, column: 1 },
            end: Coord { line: 0, column: 2 },
        });

        assert!(state.apply_utility(Direction::Left));
        assert_eq!(state.selection.anchor().column, 2);
        assert_eq!(state.selection.active().column, 3);
        assert_eq!(state.active_stroke.as_ref().unwrap().end.column, 3);
        assert_eq!(state.line_markers_for_test()[0].coord.column, 3);
        let preview = state.shape_preview.unwrap();
        assert_eq!((preview.anchor.column, preview.end.column), (2, 3));
    }

    #[test]
    fn move_lift_previews_without_mutation_then_composes_edited_cells() {
        let mut state = utility_state(&["abXX", "cdYY"], UtilityKind::Push, Coord::default());
        let configured_face = state.theme.tooltip.clone();
        state.grid.lines[0][0].face = configured_face.clone();
        state
            .selection
            .select(Coord::default(), Coord { line: 1, column: 1 });
        let before = state.edit_snapshot();

        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Right));
        assert!(state.move_lift(Direction::Right));
        assert_eq!(state.edit_snapshot().lines, before.lines);
        let preview = state
            .lines_with_shape_preview()
            .expect("lifted selection has a composited preview");
        assert_eq!(contents(&preview[0]), "  ab");
        assert_eq!(contents(&preview[1]), "  cd");
        assert_eq!(preview[0][2].face, configured_face);

        assert!(state.confirm_move_lift());
        assert!(!state.move_lift_active());
        assert_eq!(line_contents(&state), vec!["  ab", "  cd"]);
        assert_eq!(state.grid.lines[0][2].face, state.theme.tooltip);
        assert_eq!(
            state.selection_bounds(),
            SelectionBounds {
                left: 2,
                right: 3,
                top: 0,
                bottom: 1,
            }
        );
    }

    #[test]
    fn clone_move_lift_clones_once_per_shift_press_and_can_clone_after_moving() {
        let mut initial = utility_state(&["A"], UtilityKind::Push, Coord::default());
        initial
            .selection
            .select(Coord::default(), Coord { line: 0, column: 1 });
        let before = initial.edit_snapshot();

        assert!(initial.begin_selected_move_lift());
        assert!(initial.clone_move_lift(Direction::Right, 1));
        assert_eq!(initial.edit_snapshot(), before);
        assert_eq!(
            contents(&initial.lines_with_shape_preview().unwrap()[0]),
            "AA"
        );

        assert!(initial.clone_move_lift(Direction::Right, 1));
        assert_eq!(
            contents(&initial.lines_with_shape_preview().unwrap()[0]),
            "A A"
        );

        assert!(initial.clone_move_lift(Direction::Left, 2));
        assert_eq!(
            contents(&initial.lines_with_shape_preview().unwrap()[0]),
            "AAA"
        );
        assert!(initial.confirm_move_lift());
        assert_eq!(line_contents(&initial), vec!["AAA"]);

        let mut delayed = utility_state(&["A"], UtilityKind::Push, Coord::default());
        delayed
            .selection
            .select(Coord::default(), Coord { line: 0, column: 1 });
        assert!(delayed.begin_selected_move_lift());
        assert!(delayed.move_lift(Direction::Right));
        assert!(delayed.clone_move_lift(Direction::Right, 1));
        assert!(delayed.confirm_move_lift());
        assert_eq!(line_contents(&delayed), vec![" AA"]);
    }

    #[test]
    fn clone_move_lift_preserves_faces_and_line_markers_for_every_copy() {
        let mut state = utility_state(&["A"], UtilityKind::Push, Coord::default());
        let configured_face = state.theme.tooltip.clone();
        state.grid.lines[0][0].face = configured_face.clone();
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord::default(),
            ending: LineEnding::Fixed('◆'),
            base_glyph: "A".into(),
        });
        state
            .selection
            .select(Coord::default(), Coord { line: 0, column: 1 });

        assert!(state.begin_selected_move_lift());
        assert!(state.clone_move_lift(Direction::Right, 1));
        assert!(state.clone_move_lift(Direction::Right, 2));
        assert!(state.confirm_move_lift());

        assert_eq!(line_contents(&state), vec!["AAA"]);
        assert!(
            state.grid.lines[0]
                .iter()
                .all(|atom| atom.face == configured_face)
        );
        assert_eq!(
            state
                .line_markers_for_test()
                .iter()
                .map(|marker| marker.coord.column)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        let mut overlap = utility_state(&["AB"], UtilityKind::Push, Coord::default());
        overlap.extend_line_markers_for_test([
            PlacedLineMarker {
                coord: Coord::default(),
                ending: LineEnding::Fixed('◆'),
                base_glyph: "A".into(),
            },
            PlacedLineMarker {
                coord: Coord { line: 0, column: 1 },
                ending: LineEnding::Fixed('◆'),
                base_glyph: "B".into(),
            },
        ]);
        overlap
            .selection
            .select(Coord::default(), Coord { line: 0, column: 1 });
        assert!(overlap.begin_selected_move_lift());
        assert!(overlap.clone_move_lift(Direction::Right, 1));
        assert!(overlap.confirm_move_lift());
        assert_eq!(line_contents(&overlap), vec!["AAB"]);
        assert_eq!(overlap.line_markers_for_test().len(), 3);
        assert_eq!(
            overlap
                .line_markers_for_test()
                .iter()
                .map(|marker| marker.coord.column)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn move_lift_treats_unedited_cells_as_transparent() {
        let mut state = utility_state(&["A C", "x─z"], UtilityKind::Push, Coord::default());
        state
            .selection
            .select(Coord::default(), Coord { line: 0, column: 2 });
        state.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord { line: 1, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "─".into(),
        });

        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Down));
        let preview = state
            .lines_with_shape_preview()
            .expect("lifted selection has a composited preview");
        assert_eq!(contents(&preview[0]), "   ");
        assert_eq!(contents(&preview[1]), "A─C");

        assert!(state.confirm_move_lift());
        assert_eq!(line_contents(&state), vec!["   ", "A─C"]);
        assert_eq!(state.line_markers_for_test().len(), 1);
        assert_eq!(
            state.line_markers_for_test()[0].coord,
            Coord { line: 1, column: 1 }
        );
    }

    #[test]
    fn move_lift_cancel_restores_exact_cursor_selection_and_document() {
        let mut state = utility_state(&["abc"], UtilityKind::Push, Coord { line: 0, column: 2 });
        state
            .selection
            .select(Coord { line: 0, column: 2 }, Coord { line: 0, column: 1 });
        let before = state.edit_snapshot();

        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Down));
        assert!(state.move_lift(Direction::Right));
        assert!(state.cancel_move_lift());

        assert_eq!(state.edit_snapshot(), before);
        assert!(state.lines_with_shape_preview().is_none());
    }

    #[test]
    fn move_lift_extends_past_the_top_left_canvas_origin() {
        let mut left = utility_state(&["  AB"], UtilityKind::Push, Coord { line: 0, column: 3 });
        left.selection
            .select(Coord { line: 0, column: 2 }, Coord { line: 0, column: 3 });
        let before = left.edit_snapshot();
        assert!(left.begin_selected_move_lift());
        for _ in 0..5 {
            assert!(left.move_lift(Direction::Left));
        }
        assert_eq!(left.move_lift_bounds().unwrap().left, 0);
        assert_eq!(left.edit_snapshot(), before);
        assert!(left.cancel_move_lift());
        assert_eq!(left.edit_snapshot(), before);
        assert_eq!(left.take_pending_prepend(), (-3, 0));

        assert!(left.begin_selected_move_lift());
        for _ in 0..5 {
            assert!(left.move_lift(Direction::Left));
        }
        assert!(left.confirm_move_lift());
        assert_eq!(left.canvas_origin.column, 3);
        assert_eq!(left.selection_bounds().left, 0);
        assert_eq!(left.selected_text(), "AB");

        let mut up = utility_state(
            &["", "", "AB"],
            UtilityKind::Push,
            Coord { line: 2, column: 1 },
        );
        up.selection
            .select(Coord { line: 2, column: 0 }, Coord { line: 2, column: 1 });
        assert!(up.begin_selected_move_lift());
        for _ in 0..4 {
            assert!(up.move_lift(Direction::Up));
        }
        assert!(up.confirm_move_lift());
        assert_eq!(up.canvas_origin.line, 2);
        assert_eq!(up.selection_bounds().top, 0);
        assert_eq!(up.selected_text(), "AB");

        let mut stationary =
            utility_state(&["AB"], UtilityKind::Push, Coord { line: 0, column: 1 });
        stationary
            .selection
            .select(Coord::default(), Coord { line: 0, column: 1 });
        let before = stationary.edit_snapshot();
        assert!(stationary.begin_selected_move_lift());
        assert!(stationary.move_lift(Direction::Left));
        assert!(stationary.move_lift(Direction::Right));
        assert!(!stationary.confirm_move_lift());
        assert_eq!(stationary.edit_snapshot(), before);
        assert_eq!(stationary.take_pending_prepend(), (-1, 0));
    }

    #[test]
    fn move_lift_handles_overlap_wide_atoms_and_line_marker_metadata() {
        let mut overlap = utility_state(&["abcd"], UtilityKind::Push, Coord { line: 0, column: 2 });
        overlap
            .selection
            .select(Coord { line: 0, column: 1 }, Coord { line: 0, column: 2 });
        assert!(overlap.begin_selected_move_lift());
        assert!(overlap.move_lift(Direction::Right));
        assert!(overlap.confirm_move_lift());
        assert_eq!(line_contents(&overlap), vec!["a bc"]);

        let mut wide = utility_state(&["a界z"], UtilityKind::Push, Coord { line: 0, column: 1 });
        wide.selection
            .select(Coord { line: 0, column: 1 }, Coord { line: 0, column: 2 });
        wide.push_line_marker_for_test(PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "界".into(),
        });
        assert!(wide.begin_selected_move_lift());
        assert!(wide.move_lift(Direction::Down));
        assert!(wide.confirm_move_lift());
        assert_eq!(line_contents(&wide), vec!["a  z", " 界"]);
        assert_eq!(wide.line_markers_for_test().len(), 1);
        assert_eq!(
            wide.line_markers_for_test()[0].coord,
            Coord { line: 1, column: 1 }
        );
    }

    #[test]
    fn confirming_a_stationary_move_lift_is_an_exact_document_no_op() {
        let mut state = utility_state(&["abc"], UtilityKind::Push, Coord { line: 0, column: 1 });
        state
            .selection
            .select(Coord { line: 0, column: 1 }, Coord { line: 0, column: 2 });
        let before = state.edit_snapshot();

        assert!(state.begin_selected_move_lift());
        assert!(!state.confirm_move_lift());

        assert_eq!(state.edit_snapshot(), before);
    }

    fn utility_state(rows: &[&str], utility: UtilityKind, cursor: Coord) -> Editor {
        let mut state = state();
        state.grid.lines = rows
            .iter()
            .map(|row| {
                UnicodeSegmentation::graphemes(*row, true)
                    .map(|contents| Atom {
                        face: Face::default(),
                        contents: contents.to_string(),
                    })
                    .collect()
            })
            .collect();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: match utility {
                UtilityKind::Push => 0,
                UtilityKind::Pull => 1,
                UtilityKind::View => 2,
            },
        });
        state.grid.cursor_pos = cursor;
        state.cursor_index = index_for_column(&state.grid.lines[cursor.line], cursor.column);
        state.selection.collapse(cursor);
        state
    }

    fn line_contents(state: &Editor) -> Vec<String> {
        state.grid.lines.iter().map(|line| contents(line)).collect()
    }

    fn select_toolbar_option(state: &mut Editor, key: &str, count: usize) {
        let submenu = key.parse::<usize>().expect("numeric toolbar group") - 2;
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu,
            option: count,
        });
    }

    fn contents(line: &[Atom]) -> String {
        line.iter().map(|atom| atom.contents.as_str()).collect()
    }
}
