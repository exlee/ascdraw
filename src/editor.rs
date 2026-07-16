#[cfg(test)]
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::{CursorMode, ThemeConfig};
#[cfg(test)]
use crate::drawing::LineEnding;
use crate::drawing::is_line_glyph;
use crate::model::{Atom, Coord, Direction, Face, LayerId, LayerSummary};
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
mod layers;
mod line_preview;
mod line_tool;
mod move_tool;
mod shape_tool;
mod text_tool;
mod utility;
pub(super) use grid::{adjacent_coord, edited_content_origin};
pub(crate) use grid::{compact_blank_runs, compacted_blank_runs};
pub use layers::{LayerStack, LayerView, PersistedLayer};
use line_preview::LinePreview;
use line_tool::{ActiveStroke, PlacedLineMarker};
use move_tool::MoveLift;

#[derive(Debug, Clone)]
pub struct GridState {
    pub lines: Vec<Vec<Atom>>,
    pub cursor_pos: Coord,
    pub default_face: Face,
    pub cursor_face: Face,
}

#[derive(Debug, Clone)]
pub struct EditorState {
    pub grid: GridState,
    pub theme: ThemeConfig,
    pub window_title: String,
    pub cursor_mode: CursorMode,
    pub toolbar: ToolbarState,
    pub selection: CanvasSelection,
    cursor_index: usize,
    active_stroke: Option<ActiveStroke>,
    line_markers: Vec<PlacedLineMarker>,
    layers: LayerStack,
    line_preview: Option<LinePreview>,
    shape_preview: Option<ShapePreview>,
    move_lift: Option<MoveLift>,
    single_replace_pending: bool,
    pending_prepend: (usize, usize),
    canvas_origin: Coord,
    toolbar_document_changed: bool,
}

#[derive(Debug, Clone, Copy)]
struct ShapePreview {
    anchor: Coord,
    end: Coord,
}

fn reverse_theme_colors(theme: &mut ThemeConfig) {
    std::mem::swap(&mut theme.default.fg, &mut theme.default.bg);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSnapshot {
    lines: Vec<Vec<Atom>>,
    cursor_pos: Coord,
    cursor_index: usize,
    selection: CanvasSelection,
    active_stroke: Option<ActiveStroke>,
    line_markers: Vec<PlacedLineMarker>,
    canvas_origin: Coord,
    layers: LayerStack,
}

impl EditSnapshot {
    pub fn same_document(&self, other: &Self) -> bool {
        self.lines == other.lines
            && self.line_markers == other.line_markers
            && self.canvas_origin == other.canvas_origin
            && self.layers == other.layers
    }

    #[cfg(test)]
    pub fn set_cursor_for_test(&mut self, line: usize, column: usize) {
        self.cursor_pos = Coord { line, column };
        self.selection.collapse(self.cursor_pos);
    }
}

impl EditorState {
    pub fn layer_summaries(&self) -> Vec<LayerSummary> {
        self.layers.summaries()
    }

    pub fn layer_views(&self) -> Vec<LayerView<'_>> {
        self.layers
            .layers()
            .iter()
            .enumerate()
            .map(|(index, layer)| LayerView {
                id: layer.id,
                visible: layer.visible,
                lines: if index == self.layers.active_index() {
                    &self.grid.lines
                } else {
                    &layer.lines
                },
            })
            .collect()
    }

    pub fn active_layer_id(&self) -> LayerId {
        self.layers.active_id()
    }

    pub fn persisted_layers(&self) -> Vec<PersistedLayer> {
        self.layer_views()
            .into_iter()
            .map(|layer| PersistedLayer {
                id: layer.id,
                visible: layer.visible,
                lines: layer.lines.to_vec(),
            })
            .collect()
    }

    pub fn restore_layers(
        &mut self,
        layers: Vec<PersistedLayer>,
        active_layer: LayerId,
    ) -> anyhow::Result<()> {
        let (stack, lines) = LayerStack::from_persisted(layers, active_layer)?;
        self.layers = stack;
        self.grid.lines = lines;
        self.line_markers.clear();
        Ok(())
    }

    pub fn select_layer(&mut self, id: LayerId) -> bool {
        let Some(index) = self.layers.index_of(id) else {
            return false;
        };
        let changed = self
            .layers
            .activate(index, &mut self.grid.lines, &mut self.line_markers);
        if changed {
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
        }
        changed
    }

    pub fn add_layer_above(&mut self, id: LayerId) -> bool {
        let Some(index) = self.layers.index_of(id) else {
            return false;
        };
        let changed = self
            .layers
            .add_above(index, &mut self.grid.lines, &mut self.line_markers)
            .is_some();
        if changed {
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
        }
        changed
    }

    pub fn toggle_layer_visibility(&mut self, id: LayerId) -> bool {
        self.layers
            .index_of(id)
            .is_some_and(|index| self.layers.toggle_visibility(index))
    }

    pub fn move_layer_up(&mut self, id: LayerId) -> bool {
        self.layers
            .index_of(id)
            .is_some_and(|index| self.layers.move_up(index))
    }

    pub fn move_layer_down(&mut self, id: LayerId) -> bool {
        self.layers
            .index_of(id)
            .is_some_and(|index| self.layers.move_down(index))
    }

    pub fn delete_layer(&mut self, id: LayerId) -> bool {
        let Some(index) = self.layers.index_of(id) else {
            return false;
        };
        let changed = self
            .layers
            .delete(index, &mut self.grid.lines, &mut self.line_markers);
        if changed {
            self.cancel_layer_transients();
            self.sync_cursor_to_active_layer();
        }
        changed
    }

    fn cancel_layer_transients(&mut self) {
        self.active_stroke = None;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.single_replace_pending = false;
    }

    fn sync_cursor_to_active_layer(&mut self) {
        while self.grid.lines.len() <= self.grid.cursor_pos.line {
            self.grid.lines.push(Vec::new());
        }
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
        let (lines, cursor_pos, cursor_index, selection, line_markers, canvas_origin) =
            if let Some(preview) = self.line_preview.as_ref() {
                (
                    preview.source_lines.clone(),
                    preview.source_cursor,
                    preview.source_cursor_index,
                    preview.source_selection,
                    preview.source_markers.clone(),
                    preview.source_canvas_origin,
                )
            } else {
                let (cursor_pos, cursor_index, selection) = self.move_lift.as_ref().map_or(
                    (self.grid.cursor_pos, self.cursor_index, self.selection),
                    |lift| {
                        (
                            lift.source_cursor,
                            lift.source_cursor_index,
                            lift.source_selection,
                        )
                    },
                );
                (
                    self.grid.lines.clone(),
                    cursor_pos,
                    cursor_index,
                    selection,
                    self.line_markers.clone(),
                    self.canvas_origin,
                )
            };
        EditSnapshot {
            lines,
            cursor_pos,
            cursor_index,
            selection,
            active_stroke: self.active_stroke.clone(),
            line_markers,
            canvas_origin,
            layers: self.layers.clone(),
        }
    }

    pub fn restore_edit_snapshot(&mut self, snapshot: EditSnapshot) {
        self.grid.lines = snapshot.lines;
        self.grid.cursor_pos = snapshot.cursor_pos;
        self.cursor_index = snapshot.cursor_index;
        self.selection = snapshot.selection;
        self.active_stroke = snapshot.active_stroke;
        self.line_markers = snapshot.line_markers;
        self.canvas_origin = snapshot.canvas_origin;
        self.layers = snapshot.layers;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.pending_prepend = (0, 0);
    }

    pub fn new(theme: &ThemeConfig, window_title: impl Into<String>) -> Self {
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
            toolbar: ToolbarState::default(),
            selection: CanvasSelection::collapsed_at(Coord::default()),
            cursor_index: 0,
            active_stroke: None,
            line_markers: Vec::new(),
            layers: LayerStack::default(),
            line_preview: None,
            shape_preview: None,
            move_lift: None,
            single_replace_pending: false,
            pending_prepend: (0, 0),
            canvas_origin: Coord::default(),
            toolbar_document_changed: false,
        }
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
        self.single_replace_pending = false;
        self.collapse_selection();
        self.toolbar.restore_durable_selections(selections);
        self.sync_cursor_mode_with_toolbar();
    }

    pub fn tooltip(&self) -> Tooltip {
        if self.toolbar.export_menu_open() || self.toolbar.toggles_menu_open() {
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

    pub fn toolbar_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        let mut spans = self
            .toolbar
            .toolbar_spans_with_layers(row, &self.layer_summaries());
        if row + 1 == self.toolbar.content_rows() {
            let (x, y) = self.cursor_coordinates();
            spans.push(ToolbarSpan {
                contents: format!("  ({x},{y})"),
                bold_prefix: 0,
                selected: false,
                highlighted: false,
                tooltip: false,
                action: None,
                right_aligned: true,
                foreground: None,
            });
        }
        spans
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
            && !self.toolbar.toggles_menu_open()
    }

    pub fn handle_toolbar_shortcut(&mut self, key: &Key, modifiers: ModifiersState) -> bool {
        self.toolbar_document_changed = false;
        if self.cursor_mode.accepts_text() {
            self.toolbar.cancel_shortcut();
            return false;
        }
        let export_was_open = self.toolbar.export_menu_open();
        let toggles_was_open = self.toolbar.toggles_menu_open();
        let dark_was_enabled = self.toolbar.dark_mode();
        let old_mode = self.toolbar.main_mode();
        let old_utility = self.toolbar.utility_kind();
        if !self
            .toolbar
            .handle_shortcut_with_layers(key, modifiers, &self.layer_summaries())
        {
            return false;
        }
        self.apply_pending_layer_action();
        if matches!(key, Key::Named(NamedKey::Escape)) && !export_was_open && !toggles_was_open {
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
        if self.move_lift.is_some()
            && (self.toolbar.export_menu_open()
                || self.toolbar.toggles_menu_open()
                || self.toolbar.main_mode() != old_mode
                || self.toolbar.utility_kind() != old_utility)
        {
            self.cancel_move_lift();
        }
        true
    }

    pub fn apply_toolbar_action(&mut self, action: ToolbarAction) -> bool {
        self.toolbar_document_changed = false;
        self.cancel_line_preview();
        if self.move_lift.is_some() {
            self.cancel_move_lift();
        }
        let dark_was_enabled = self.toolbar.dark_mode();
        if !self.toolbar.apply_action(action) {
            return false;
        }
        self.apply_pending_layer_action();
        if self.toolbar.dark_mode() != dark_was_enabled {
            reverse_theme_colors(&mut self.theme);
            self.sync_theme_faces();
        }
        if matches!(
            action,
            ToolbarAction::ToggleExportMenu
                | ToolbarAction::ToggleTogglesMenu
                | ToolbarAction::Toggle(_)
                | ToolbarAction::RunExport(_)
        ) {
            return true;
        }
        self.end_stroke();
        self.shape_preview = None;
        self.move_lift = None;
        self.sync_cursor_mode_with_toolbar();
        true
    }

    pub fn take_toolbar_document_change(&mut self) -> bool {
        std::mem::take(&mut self.toolbar_document_changed)
    }

    fn apply_pending_layer_action(&mut self) {
        let Some((layer, operation)) = self.toolbar.take_layer_action() else {
            return;
        };
        self.toolbar_document_changed = match operation {
            LayerOperation::Select => {
                self.select_layer(layer);
                false
            }
            LayerOperation::Show => self.toggle_layer_visibility(layer),
            LayerOperation::MoveUp => self.move_layer_up(layer),
            LayerOperation::MoveDown => self.move_layer_down(layer),
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
        if matches!(
            self.cursor_mode,
            CursorMode::Text | CursorMode::Insert | CursorMode::Replace
        ) {
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
            || self.toolbar.pending_shortcut().is_some();
        self.end_stroke();
        self.cancel_line_preview();
        self.shape_preview = None;
        self.cancel_move_lift();
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
            MainMode::Layers | MainMode::Colors => CursorMode::Navigation,
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
        self.move_to_without_ending_stroke(Coord {
            line: usize::try_from(line.max(0)).unwrap_or(usize::MAX),
            column: usize::try_from(column.max(0)).unwrap_or(usize::MAX),
        });
        self.collapse_selection();
    }

    fn move_to_without_ending_stroke(&mut self, coord: Coord) {
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
        let prepended = self.prepare_adjacent(direction);
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
        self.prepare_adjacent(direction);
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
        let is_line = self.line_markers.iter().any(|marker| marker.coord == coord)
            || self.cell_contents(coord).is_some_and(is_line_glyph);
        if is_line {
            let before_contents = self.cell_contents(coord).map(str::to_owned);
            let had_marker = self.line_markers.iter().any(|marker| marker.coord == coord);
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
        self.line_markers.retain(|marker| {
            marker.coord.line != coord.line
                || marker.coord.column < start_column
                || marker.coord.column >= start_column.saturating_add(width)
        });
        line.splice(index..=index, grid::blank_run(width));
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
        self.line_markers
            .retain(|marker| !bounds.contains(marker.coord));
        replace_range(&mut self.grid.lines, bounds, None);
        self.restore_active_cursor_index();
    }

    fn selection_contains_nonblank(&self) -> bool {
        let bounds = self.selection.bounds();
        (bounds.top..=bounds.bottom).any(|line_index| {
            let Some(line) = self.grid.lines.get(line_index) else {
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
    }

    pub fn place_stamp(&mut self) {
        self.end_stroke();
        let stamp = self.toolbar.stamp().to_string();
        self.replace_selection_literal(Some(&stamp));
    }

    pub fn draw_stamp(&mut self, direction: Direction) {
        self.prepare_adjacent(direction);
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
        let Some(rectangle) = TextRectangle::from_text(text) else {
            return false;
        };
        let origin = Coord {
            line: self.selection.bounds().top,
            column: self.selection.bounds().left,
        };
        let bounds = rectangle.bounds_at(origin);
        self.end_stroke();
        self.shape_preview = None;
        self.line_markers
            .retain(|marker| !bounds.contains(marker.coord));
        overwrite_rectangle(&mut self.grid.lines, origin, &rectangle);
        self.color_written_bounds(bounds);
        let active = Coord {
            line: bounds.bottom,
            column: bounds.right,
        };
        self.selection.select(origin, active);
        self.grid.cursor_pos = active;
        self.cursor_index = index_for_column(&self.grid.lines[active.line], active.column);
        true
    }

    pub fn replace_canvas(&mut self, lines: Vec<Vec<Atom>>) {
        self.layers.reset();
        self.grid.lines = if lines.is_empty() {
            vec![Vec::new()]
        } else {
            lines
        };
        self.canvas_origin = edited_content_origin(&self.grid.lines).unwrap_or_default();
        self.grid.cursor_pos = Coord::default();
        self.cursor_index = 0;
        self.active_stroke = None;
        self.line_markers.clear();
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.single_replace_pending = false;
        self.pending_prepend = (0, 0);
        self.toolbar.cancel_shortcut();
        self.selection.collapse(Coord::default());
        self.sync_cursor_mode_with_toolbar();
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
            .filter_map(|layer| edited_content_origin(layer.lines))
            .reduce(|origin, candidate| Coord {
                line: origin.line.min(candidate.line),
                column: origin.column.min(candidate.column),
            })
            .unwrap_or_default();
        self.restore_menu_selections(menu_selections);
        self.grid.cursor_pos = cursor;
        self.cursor_index = index_for_column(&self.grid.lines[cursor.line], cursor.column);
        self.selection = selection;
        self.active_stroke = None;
        self.line_markers.clear();
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
        let already_blank = self.layers.layers().len() == 1
            && self.content_cells().is_empty()
            && self.line_markers.is_empty()
            && self
                .grid
                .lines
                .iter()
                .flatten()
                .all(|atom| atom.face == Face::default());

        if !already_blank {
            self.layers.reset();
            self.grid.lines = (0..=cursor.line).map(|_| Vec::new()).collect();
            self.grid.lines[cursor.line] = (0..cursor.column)
                .map(|_| Atom {
                    face: Face::default(),
                    contents: " ".to_string(),
                })
                .collect();
        }

        self.grid.cursor_pos = cursor;
        self.cursor_index = index_for_column(&self.grid.lines[cursor.line], cursor.column);
        self.active_stroke = None;
        self.line_markers.clear();
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.single_replace_pending = false;
        self.pending_prepend = (0, 0);
        self.toolbar.cancel_shortcut();
        self.selection.collapse(cursor);
        self.sync_cursor_mode_with_toolbar();
    }

    pub fn preview_render_lines(&self) -> Option<&[Vec<Atom>]> {
        self.move_lift_render_lines()
            .or_else(|| self.line_preview_render_lines())
    }

    fn sync_cursor_column(&mut self) {
        self.grid.cursor_pos.column =
            display_width(&self.grid.lines[self.grid.cursor_pos.line][..self.cursor_index]);
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
        self.line_markers
            .retain(|marker| !bounds.contains(marker.coord));
        replace_range(&mut self.grid.lines, bounds, replacement);
        if replacement.is_some() {
            self.color_written_bounds(bounds);
        }
        self.restore_active_cursor_index();
    }

    pub fn take_pending_prepend(&mut self) -> (usize, usize) {
        std::mem::take(&mut self.pending_prepend)
    }

    pub fn content_cells(&self) -> Vec<Coord> {
        let mut cells = self
            .layer_views()
            .into_iter()
            .filter(|layer| layer.visible)
            .flat_map(|layer| grid::content_cells(layer.lines))
            .collect::<Vec<_>>();
        cells.sort_unstable_by_key(|coord| (coord.line, coord.column));
        cells.dedup();
        cells
    }

    pub fn compact_blank_runs_preserving_cursor(&mut self) {
        grid::compact_blank_runs(&mut self.grid.lines);
        self.expose_cursor_cells();
    }

    fn prepare_adjacent(&mut self, direction: Direction) -> bool {
        match direction {
            Direction::Up if self.grid.cursor_pos.line == 0 => {
                self.prepend_line();
                self.canvas_origin.line = self.canvas_origin.line.saturating_add(1);
                true
            }
            Direction::Left if self.grid.cursor_pos.column == 0 => {
                self.prepend_column();
                self.canvas_origin.column = self.canvas_origin.column.saturating_add(1);
                true
            }
            _ => false,
        }
    }

    fn prepend_line(&mut self) {
        self.layers.prepend_line_to_inactive();
        self.grid.lines.insert(0, Vec::new());
        self.grid.cursor_pos.line = self.grid.cursor_pos.line.saturating_add(1);
        self.selection.shift(0, 1);
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end.line = stroke.end.line.saturating_add(1);
        }
        for marker in &mut self.line_markers {
            marker.coord.line = marker.coord.line.saturating_add(1);
        }
        self.shift_line_preview(0, 1);
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.anchor.line = preview.anchor.line.saturating_add(1);
            preview.end.line = preview.end.line.saturating_add(1);
        }
        self.pending_prepend.1 = self.pending_prepend.1.saturating_add(1);
    }

    fn prepend_column(&mut self) {
        self.layers.prepend_column_to_inactive();
        for line in &mut self.grid.lines {
            line.insert(0, blank_atom());
        }
        self.grid.cursor_pos.column = self.grid.cursor_pos.column.saturating_add(1);
        self.cursor_index = self.cursor_index.saturating_add(1);
        self.selection.shift(1, 0);
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end.column = stroke.end.column.saturating_add(1);
        }
        for marker in &mut self.line_markers {
            marker.coord.column = marker.coord.column.saturating_add(1);
        }
        self.shift_line_preview(1, 0);
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.anchor.column = preview.anchor.column.saturating_add(1);
            preview.end.column = preview.end.column.saturating_add(1);
        }
        self.pending_prepend.0 = self.pending_prepend.0.saturating_add(1);
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
    use crate::toolbar::{ToggleKind, UtilityKind};

    fn state() -> EditorState {
        EditorState::new(&ThemeConfig::default(), "ascdraw")
    }

    #[test]
    fn dark_mode_reverses_root_and_preserves_explicit_ui_accent_colors() {
        let source = ThemeConfig::default();
        let mut reversed = source.clone();
        reverse_theme_colors(&mut reversed);
        let mut state = EditorState::new(&source, "ascdraw");

        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::DarkMode)));
        assert_eq!(state.theme, reversed);
        assert_eq!(state.grid.default_face, reversed.default);
        assert_eq!(state.grid.cursor_face, reversed.cursor_block);
        assert_eq!(state.theme.selection, source.selection);
        assert_eq!(state.theme.selection_highlight, source.selection_highlight);
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
            (Direction::Right, "(1,0)"),
            (Direction::Down, "(1,1)"),
            (Direction::Left, "(0,1)"),
            (Direction::Up, "(0,0)"),
            (Direction::Left, "(-1,0)"),
            (Direction::Up, "(-1,-1)"),
        ] {
            state.move_cursor(direction);
            let last_row = state.toolbar.content_rows() - 1;
            let coordinate = state
                .toolbar_spans(last_row)
                .into_iter()
                .find(|span| span.right_aligned && span.contents.contains('('))
                .expect("last toolbar row contains cursor coordinates");
            assert_eq!(coordinate.contents.trim(), expected);
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
    fn cursor_coordinates_are_right_aligned_on_the_last_toolbar_content_row() {
        let mut state = state();
        state.move_to(Coord {
            line: 8,
            column: 10,
        });

        for action in [None, Some(ToolbarAction::ToggleExportMenu)] {
            if let Some(action) = action {
                assert!(state.apply_toolbar_action(action));
            }
            let last_row = state.toolbar.content_rows() - 1;
            let coordinate = state
                .toolbar_spans(last_row)
                .into_iter()
                .find(|span| span.right_aligned && span.contents.contains('('))
                .expect("last toolbar row contains cursor coordinates");
            assert_eq!(coordinate.contents.trim(), "(10,8)");
            assert!(
                state
                    .toolbar_spans(last_row - 1)
                    .iter()
                    .all(|span| !span.contents.contains("(10,8)"))
            );
        }
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
        state.line_markers.push(PlacedLineMarker {
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
        assert!(state.line_markers.is_empty());
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
        let mut state = EditorState::new(&theme, "ascdraw");
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
    fn erasing_either_display_cell_blanks_a_whole_wide_grapheme() {
        let mut state = state();
        state.insert("A界B");
        state.move_to(Coord { line: 0, column: 3 });

        assert!(state.erase(Direction::Left));

        assert_eq!(contents(&state.grid.lines[0]), "A   ");
        assert_eq!(display_width(&state.grid.lines[0]), 4);
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
    fn insert_shifts_line_markers_by_display_width() {
        let mut state = state();
        state.insert("a◆");
        state.line_markers.push(PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "╴".into(),
        });
        state.move_to(Coord::default());
        state.toggle_text_entry();

        state.write_text("界");

        assert_eq!(contents(&state.grid.lines[0]), "界a◆");
        assert_eq!(state.line_markers[0].coord, Coord { line: 0, column: 3 });
    }

    #[test]
    fn replace_removes_overwritten_markers_and_shifts_following_markers() {
        let mut state = state();
        state.insert("a◆");
        state.line_markers.push(PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "╴".into(),
        });
        state.move_to(Coord::default());
        state.toggle_replace_mode();

        state.write_text("界");

        assert_eq!(contents(&state.grid.lines[0]), "界◆");
        assert_eq!(state.line_markers[0].coord, Coord { line: 0, column: 2 });

        state.write_text("x");

        assert_eq!(contents(&state.grid.lines[0]), "界x");
        assert!(state.line_markers.is_empty());
    }

    #[test]
    fn newline_backspace_and_delete_remap_line_markers() {
        let mut state = state();
        state.insert("a◆\nb◆");
        state.line_markers.extend([
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
        state.move_to(Coord { line: 0, column: 1 });

        state.newline();

        assert_eq!(state.line_markers[0].coord, Coord { line: 1, column: 0 });
        assert_eq!(state.line_markers[1].coord, Coord { line: 2, column: 1 });

        state.backspace();

        assert_eq!(state.line_markers[0].coord, Coord { line: 0, column: 1 });
        assert_eq!(state.line_markers[1].coord, Coord { line: 1, column: 1 });

        state.delete();

        assert_eq!(contents(&state.grid.lines[0]), "a");
        assert_eq!(state.line_markers.len(), 1);
        assert_eq!(state.line_markers[0].coord, Coord { line: 1, column: 1 });

        state.delete();

        assert_eq!(contents(&state.grid.lines[0]), "ab◆");
        assert_eq!(state.line_markers[0].coord, Coord { line: 0, column: 2 });
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
        state.line_markers = vec![inside, outside.clone()];
        state.move_to(Coord::default());

        state.clear_selection();

        assert_eq!(contents(&state.grid.lines[0]), " ─◆");
        assert_eq!(state.line_markers, vec![outside]);
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
    fn paste_expands_grid_and_preserves_wide_source_graphemes() {
        let mut state = state();
        state.move_to(Coord { line: 2, column: 3 });
        assert!(state.paste_text_rectangle("😀\r\nq"));
        assert_eq!(state.grid.lines.len(), 4);
        assert_eq!(state.selected_text(), "😀\nq ");
        assert_eq!(state.selection_bounds().width(), 2);
        assert_eq!(state.selection_bounds().height(), 2);
        assert_eq!(state.grid.cursor_pos, Coord { line: 3, column: 4 });
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
        state.write_text("界ignored");

        assert_eq!(state.selected_text(), "界 \n界 ");
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
    fn cursor_column_tracks_wide_graphemes() {
        let mut state = state();
        state.insert("😀x");
        assert_eq!(state.grid.cursor_pos.column, 3);
        state.move_cursor(Direction::Left);
        assert_eq!(state.grid.cursor_pos.column, 2);
        state.move_cursor(Direction::Left);
        assert_eq!(state.grid.cursor_pos.column, 0);
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
    fn moving_up_at_zero_prepends_and_shifts_coordinate_state() {
        let mut state = state();
        state.insert("ab");
        state.move_to(Coord { line: 0, column: 1 });
        state.line_markers.push(PlacedLineMarker {
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
        assert_eq!(state.line_markers[0].coord.line, 1);
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
        state.line_markers.push(PlacedLineMarker {
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
        assert_eq!(state.line_markers[0].coord.column, 1);
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
        assert_eq!(state.line_markers.len(), 2);
        assert_eq!(state.line_markers[0].ending, LineEnding::Fixed('◆'));
        assert_eq!(
            state.line_markers[1].ending,
            LineEnding::Directional(crate::drawing::DirectionalEnding::Arrow)
        );

        let snapshot = state.edit_snapshot();
        state.clear_selection();
        state.restore_edit_snapshot(snapshot);
        assert_eq!(contents(&state.grid.lines[0]), "◆─╮");
        assert_eq!(contents(&state.grid.lines[1]), "  ↓");
        assert_eq!(
            state.line_markers[1].ending,
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
        state.insert("😀x");
        state.move_to(Coord { line: 0, column: 0 });

        state.clear_selection();

        assert_eq!(contents(&state.grid.lines[0]), "  x");
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
        let mut state = EditorState::new(&ThemeConfig::default(), "test");
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
        let mut state = EditorState::new(&ThemeConfig::default(), "test");
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
        state.line_markers.extend([
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
        assert_eq!(state.line_markers.len(), 1);
        assert_eq!(state.line_markers[0].coord.column, 2);
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
        state.line_markers.extend([
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
        assert_eq!(state.line_markers.len(), 1);
        assert_eq!(state.line_markers[0].coord.line, 2);
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
        state.line_markers.push(PlacedLineMarker {
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
        assert_eq!(state.line_markers[0].coord.column, 3);
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
    fn move_lift_treats_unedited_cells_as_transparent() {
        let mut state = utility_state(&["A C", "x─z"], UtilityKind::Push, Coord::default());
        state
            .selection
            .select(Coord::default(), Coord { line: 0, column: 2 });
        state.line_markers.push(PlacedLineMarker {
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
        assert_eq!(state.line_markers.len(), 1);
        assert_eq!(state.line_markers[0].coord, Coord { line: 1, column: 1 });
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
        wide.line_markers.push(PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "界".into(),
        });
        assert!(wide.begin_selected_move_lift());
        assert!(wide.move_lift(Direction::Down));
        assert!(wide.confirm_move_lift());
        assert_eq!(line_contents(&wide), vec!["a  z", " 界"]);
        assert_eq!(wide.line_markers.len(), 1);
        assert_eq!(wide.line_markers[0].coord, Coord { line: 1, column: 1 });
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

    fn utility_state(rows: &[&str], utility: UtilityKind, cursor: Coord) -> EditorState {
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

    fn line_contents(state: &EditorState) -> Vec<String> {
        state.grid.lines.iter().map(|line| contents(line)).collect()
    }

    fn select_toolbar_option(state: &mut EditorState, key: &str, count: usize) {
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
