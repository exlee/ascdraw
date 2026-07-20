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
    StyledAtom,
};
use crate::selection::{CanvasSelection, SelectionBounds, TextRectangle};
use crate::toolbar::{
    DurableMenuSelections, LayerOperation, MainMode, ToolbarAction, ToolbarSpan, ToolbarState,
    Tooltip, UtilityKind,
};

mod color_tool;
mod grid;
mod jump_mode;
mod layer_facade;
mod layers;
mod lifecycle;
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
    cursor_pos: Coord,
    selection: CanvasSelection,
    active_stroke: Option<ActiveStroke>,
    canvas_origin: Coord,
    canvas: crate::canvas::LayerStack,
}

impl PartialEq for EditSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.cursor_pos == other.cursor_pos
            && self.selection == other.selection
            && self.active_stroke == other.active_stroke
            && self.canvas_origin == other.canvas_origin
            && self.canvas == other.canvas
    }
}

impl Eq for EditSnapshot {}

impl EditSnapshot {
    pub fn same_document(&self, other: &Self) -> bool {
        self.canvas_origin == other.canvas_origin && self.canvas == other.canvas
    }

    #[cfg(test)]
    pub fn set_cursor_for_test(&mut self, line: usize, column: usize) {
        self.cursor_pos = Coord { line, column };
        self.selection.collapse(self.cursor_pos);
    }
}

#[cfg(test)]
impl Editor {
    pub(crate) fn set_lines_for_test(&mut self, lines: Vec<Vec<StyledAtom>>) {
        let markers = self.canvas.active_line_markers();
        self.canvas
            .commit_active_with_markers(&lines, &markers)
            .expect("test canvas contains valid one-cell atoms");
    }

    pub(crate) fn lines_for_test(&self) -> Vec<Vec<StyledAtom>> {
        self.canvas.active_dense_lines()
    }

    pub(crate) fn set_cell_face_for_test(&mut self, coord: Coord, face: Face) {
        assert!(self.canvas.set_face_at(coord, face));
    }

    fn line_markers_for_test(&self) -> Vec<PlacedLineMarker> {
        self.canvas.active_line_markers()
    }

    fn set_line_markers_for_test(&mut self, markers: Vec<PlacedLineMarker>) {
        let lines = self.canvas.active_dense_lines();
        self.canvas
            .commit_active_with_markers(&lines, &markers)
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
        self.canvas.set_enabled(self.toolbar.multi_layer_mode());
        Ok(())
    }

    fn commit_canvas(&mut self) {
        self.commit_canvas_mutations()
            .expect("editor mutations preserve one-cell sparse canvas atoms");
    }

    pub fn canvas(&self) -> &crate::canvas::LayerStack {
        &self.canvas
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
        self.canvas.set_enabled(self.toolbar.multi_layer_mode());
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
        self.grid.cursor_pos.column = 0;
        self.collapse_selection();
    }

    pub fn move_end(&mut self) {
        self.end_stroke();
        let width = self.canvas.active_row_width(self.grid.cursor_pos.line);
        self.grid.cursor_pos.column = width.min(MAX_CANVAS_WIDTH - 1);
        self.collapse_selection();
    }

    pub fn move_to(&mut self, coord: Coord) -> bool {
        let coord = clamp_canvas_coord(coord);
        self.cancel_line_preview();
        self.cancel_move_lift();
        self.end_stroke();
        self.move_to_without_ending_stroke(coord);
        self.collapse_selection();
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.end = self.grid.cursor_pos;
        }
        false
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
        self.grid.cursor_pos = coord;
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
        target
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
        Some(target)
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
        self.commit_canvas();
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
        let Some(data) = self.canvas.active_cell(coord) else {
            return false;
        };
        if data.atom.contents().chars().all(char::is_whitespace) {
            return false;
        }
        self.canvas.delete_at(coord)
    }

    fn move_selection_to_without_ending_stroke(&mut self, coord: Coord) {
        self.grid.cursor_pos = coord;
    }

    pub fn clear_selection(&mut self) {
        self.end_stroke();
        if !self.selection_contains_nonblank() {
            return;
        }
        let bounds = self.selection.bounds();
        self.commit_canvas();
        self.canvas
            .clear_bounds_in_all_layers(bounds)
            .expect("selection bounds fit the sparse canvas");
        self.restore_active_cursor();
    }

    fn selection_contains_nonblank(&self) -> bool {
        let bounds = self.selection.bounds();
        self.canvas.layers().iter().any(|layer| {
            layer.rows().iter().any(|(&line, cells)| {
                let Ok(line) = usize::try_from(line) else {
                    return false;
                };
                line >= bounds.top
                    && line <= bounds.bottom
                    && cells.iter().any(|(&column, data)| {
                        usize::try_from(column).is_ok_and(|column| {
                            column >= bounds.left
                                && column <= bounds.right
                                && !data.atom.contents().chars().all(char::is_whitespace)
                        })
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
        self.canvas.layers()[self.canvas.active_index()]
            .selected_atoms(self.selection.bounds())
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|atom| atom.contents)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        let active = Coord {
            line: bounds.bottom,
            column: bounds.right,
        };
        self.selection.select(origin, active);
        self.grid.cursor_pos = active;
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
        let removed_marker = self
            .canvas
            .active_line_markers()
            .iter()
            .any(|marker| bounds.contains(marker.coord));
        let unchanged = self.canvas.layers()[self.canvas.active_index()].selected_atoms(bounds)
            == rectangle.rows;
        if unchanged && !removed_marker {
            return false;
        }

        self.commit_canvas();
        self.canvas
            .overwrite_active_rectangle(origin, rectangle)
            .expect("styled rectangle contains one-cell atoms");
        self.selection.collapse(origin);
        self.grid.cursor_pos = origin;
        true
    }

    pub fn replace_canvas(&mut self, mut lines: Vec<Vec<StyledAtom>>) {
        truncate_canvas_lines(&mut lines);
        let map = crate::canvas::LayerMap::from_dense_with_markers(LayerId(0), true, &lines, &[])
            .expect("loaded canvas contains valid graphemes");
        self.canvas = crate::canvas::LayerStack::new(vec![map], self.toolbar.multi_layer_mode())
            .expect("replacement canvas has a base layer");
        self.toolbar.sync_layer_count(self.canvas.layers().len());
        self.canvas_origin = edited_content_origin(&lines).unwrap_or_default();
        self.grid.cursor_pos = Coord::default();
        self.active_stroke = None;
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
            .filter_map(|layer| edited_content_origin(&layer.lines))
            .reduce(|origin, candidate| Coord {
                line: origin.line.min(candidate.line),
                column: origin.column.min(candidate.column),
            })
            .unwrap_or_default();
        self.restore_menu_selections(menu_selections);
        let cursor = clamp_canvas_coord(cursor);
        self.grid.cursor_pos = cursor;
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
        self.commit_canvas();
        let cursor = self.grid.cursor_pos;
        self.canvas.clear_contents();

        self.grid.cursor_pos = cursor;
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

    fn collapse_selection(&mut self) {
        self.cancel_move_lift();
        self.selection.collapse(self.grid.cursor_pos);
    }

    fn restore_active_cursor(&mut self) {
        let active = self.selection.active();
        self.grid.cursor_pos = active;
    }

    fn replace_selection_literal(&mut self, replacement: Option<&str>) {
        let bounds = self.selection.bounds();
        let replacement = replacement.map(|contents| {
            let face = if contents.chars().all(char::is_whitespace) {
                Face::default()
            } else {
                self.write_face()
            };
            (
                Atom::new(contents).expect("replacement was validated as one cell"),
                face,
            )
        });
        self.canvas
            .replace_active_bounds(bounds, replacement)
            .expect("literal selection replacements contain one-cell atoms");
        self.restore_active_cursor();
    }

    /// Positive values compensate for prepended cells; negative values undo
    /// that compensation when a transient edit restores its source snapshot.
    pub fn take_pending_prepend(&mut self) -> (i64, i64) {
        std::mem::take(&mut self.pending_prepend)
    }

    pub fn content_cells(&self) -> Vec<Coord> {
        self.sparse_content_cells(false)
    }

    pub fn content_cells_including_hidden(&self) -> Vec<Coord> {
        self.sparse_content_cells(true)
    }

    fn sparse_content_cells(&self, include_hidden: bool) -> Vec<Coord> {
        let mut cells = Vec::new();
        for layer in self
            .canvas
            .layers()
            .iter()
            .filter(|layer| include_hidden || layer.visible)
        {
            for (&line, row) in layer.rows() {
                let Ok(line) = usize::try_from(line) else {
                    continue;
                };
                for (&column, data) in row {
                    if data.atom.contents().chars().all(char::is_whitespace) {
                        continue;
                    }
                    let Ok(column) = usize::try_from(column) else {
                        continue;
                    };
                    cells.push(Coord { line, column });
                }
            }
        }
        cells.sort_unstable_by_key(|coord| (coord.line, coord.column));
        cells.dedup();
        cells
    }

    pub fn compact_blank_runs_preserving_cursor(&mut self) {}

    fn prepare_adjacent(&mut self, direction: Direction) -> Option<bool> {
        self.commit_canvas();
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
        self.canvas.prepend_line_in_all_layers();
        self.grid.cursor_pos.line = self.grid.cursor_pos.line.saturating_add(1);
        self.selection.shift(0, 1);
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end.line = stroke.end.line.saturating_add(1);
        }
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
        self.canvas.prepend_column_in_all_layers();
        self.grid.cursor_pos.column = self.grid.cursor_pos.column.saturating_add(1);
        self.selection.shift(1, 0);
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end.column = stroke.end.column.saturating_add(1);
        }
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
            .canvas
            .bounds()
            .and_then(|bounds| usize::try_from(bounds.max_y).ok())
            .map_or(1, |bottom| bottom.saturating_add(1));
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
            .canvas
            .bounds()
            .and_then(|bounds| usize::try_from(bounds.max_x).ok())
            .map_or(0, |right| right.saturating_add(1));
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

fn truncate_canvas_lines(lines: &mut Vec<Vec<StyledAtom>>) {
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

fn atom_width(atom: &StyledAtom) -> usize {
    UnicodeWidthStr::width(atom.contents.as_str()).max(usize::from(!atom.contents.is_empty()))
}

fn display_width(atoms: &[StyledAtom]) -> usize {
    atoms.iter().map(atom_width).sum()
}

#[cfg(test)]
#[path = "inline_tests/editor_tests.rs"]
mod tests;
