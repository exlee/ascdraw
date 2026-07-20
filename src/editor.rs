use unicode_width::UnicodeWidthStr;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::{CursorMode, ThemeConfig};
use crate::drawing::is_line_glyph;
use crate::jump::JumpMode;
use crate::model::{
    Atom, Coord, Direction, Face, LayerId, LayerSummary, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH,
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
mod lifecycle;
mod line_preview;
mod line_tool;
mod move_tool;
mod routing;
mod shape_tool;
mod state;
mod text_tool;
mod utility;
pub(super) use grid::adjacent_coord;
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
    canvas: crate::canvas::LayerStack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEditorState {
    cursor_pos: Coord,
    selection: CanvasSelection,
    active_stroke: Option<ActiveStroke>,
}

impl PartialEq for EditSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.cursor_pos == other.cursor_pos
            && self.selection == other.selection
            && self.active_stroke == other.active_stroke
            && self.canvas == other.canvas
    }
}

impl Eq for EditSnapshot {}

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

    pub fn boxed_toolbar_spans_for_width(&self, row: usize, box_width: usize) -> Vec<ToolbarSpan> {
        self.toolbar
            .boxed_spans_with_layers_for_width(row, box_width, &self.layer_summaries())
    }

    pub fn cursor_coordinates(&self) -> (i128, i128) {
        (
            self.grid.cursor_pos.column as i128,
            self.grid.cursor_pos.line as i128,
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
        self.grid.cursor_pos.column = i16::try_from(width.min(MAX_CANVAS_WIDTH - 1))
            .expect("canvas width fits signed coordinate range");
        self.collapse_selection();
    }

    pub fn move_to(&mut self, coord: Coord) -> bool {
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

    pub fn resolve_pointer_coord(&self, line: i64, column: i64) -> Coord {
        Coord {
            line: i16::try_from(line).unwrap_or(if line < 0 { i16::MIN } else { i16::MAX }),
            column: i16::try_from(column).unwrap_or(if column < 0 { i16::MIN } else { i16::MAX }),
        }
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

    fn move_to_without_ending_stroke(&mut self, coord: Coord) {
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

    /// Resolves a cursor coordinate without mutating editor state. Runtime
    /// navigation validates the viewport before applying the lightweight move.
    pub fn navigation_target(&self, direction: Direction, steps: usize) -> Option<Coord> {
        let mut cursor = self.grid.cursor_pos;
        for _ in 0..steps {
            cursor = adjacent_coord(cursor, direction)?;
        }
        Some(cursor)
    }

    pub fn extend_selection(&mut self, direction: Direction) -> bool {
        self.cancel_line_preview();
        self.cancel_move_lift();
        let Some(to) = self.prepared_adjacent_coord(direction) else {
            return false;
        };
        self.end_stroke();
        self.shape_preview = None;
        self.move_selection_to_without_ending_stroke(to);
        self.selection.set_active(self.grid.cursor_pos);
        false
    }

    /// Moves one cell while erasing the traversed edge. Connected line cells
    /// lose only that edge; every other non-blank atom is replaced by
    /// display-width-preserving blank cells.
    pub fn erase(&mut self, direction: Direction) -> bool {
        self.commit_canvas();
        self.cancel_line_preview();
        self.cancel_move_lift();
        let Some(to) = self.prepared_adjacent_coord(direction) else {
            return false;
        };
        self.end_stroke();
        self.shape_preview = None;
        let from = self.grid.cursor_pos;
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

    pub fn clear_selection(&mut self) -> bool {
        self.end_stroke();
        if !self.selection_contains_nonblank() {
            return false;
        }
        let bounds = self.selection.bounds();
        self.commit_canvas();
        self.canvas
            .clear_bounds_in_all_layers(bounds)
            .expect("selection bounds fit the sparse canvas");
        self.restore_active_cursor();
        true
    }

    fn selection_contains_nonblank(&self) -> bool {
        let bounds = self.selection.bounds();
        self.canvas.layers().iter().any(|layer| {
            layer.rows().iter().any(|(&line, cells)| {
                line >= bounds.top
                    && line <= bounds.bottom
                    && cells.iter().any(|(&column, data)| {
                        column >= bounds.left
                            && column <= bounds.right
                            && !data.atom.contents().chars().all(char::is_whitespace)
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
        let Some(to) = self.prepared_adjacent_coord(direction) else {
            return;
        };
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
        crate::dense_exchange::selected_atoms(
            &self.canvas.layers()[self.canvas.active_index()],
            self.selection.bounds(),
        )
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
        if rectangle.width > MAX_CANVAS_WIDTH || rectangle.rows.len() > MAX_CANVAS_HEIGHT {
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
        crate::dense_exchange::overwrite_active_rectangle(&mut self.canvas, origin, &rectangle)
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
                row.iter()
                    .map(|atom| {
                        UnicodeWidthStr::width(atom.contents.as_str())
                            .max(usize::from(!atom.contents.is_empty()))
                    })
                    .sum::<usize>()
                    != rectangle.width
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
        if rectangle.width > MAX_CANVAS_WIDTH || rectangle.rows.len() > MAX_CANVAS_HEIGHT {
            return false;
        }
        let removed_marker = self
            .canvas
            .active_line_markers()
            .iter()
            .any(|marker| bounds.contains(marker.coord));
        let unchanged = crate::dense_exchange::selected_atoms(
            &self.canvas.layers()[self.canvas.active_index()],
            bounds,
        ) == rectangle.rows;
        if unchanged && !removed_marker {
            return false;
        }

        self.commit_canvas();
        crate::dense_exchange::overwrite_active_rectangle(&mut self.canvas, origin, rectangle)
            .expect("styled rectangle contains one-cell atoms");
        self.selection.collapse(origin);
        self.grid.cursor_pos = origin;
        true
    }

    pub fn replace_canvas(&mut self, mut replacement: crate::canvas::LayerStack) {
        replacement.set_enabled(self.toolbar.multi_layer_mode());
        self.canvas.record_history_replacement(&replacement);
        self.canvas = replacement;
        self.toolbar.sync_layer_count(self.canvas.layers().len());
        self.grid.cursor_pos = Coord::default();
        self.active_stroke = None;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.single_replace_pending = false;
        self.toolbar.cancel_shortcut();
        self.selection.collapse(Coord::default());
        self.sync_cursor_mode_with_toolbar();
    }

    pub fn restore_project(
        &mut self,
        canvas: crate::canvas::LayerStack,
        cursor: Coord,
        selection: CanvasSelection,
        menu_selections: &DurableMenuSelections,
    ) -> anyhow::Result<()> {
        self.restore_canvas(canvas);
        self.restore_menu_selections(menu_selections);
        self.grid.cursor_pos = cursor;
        self.selection
            .select(selection.anchor(), selection.active());
        self.active_stroke = None;
        self.line_preview = None;
        self.shape_preview = None;
        self.move_lift = None;
        self.single_replace_pending = false;
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

    pub fn content_cells(&self) -> Vec<Coord> {
        self.sparse_content_cells(false)
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
                for (&column, data) in row {
                    if data.atom.contents().chars().all(char::is_whitespace) {
                        continue;
                    }
                    cells.push(Coord { line, column });
                }
            }
        }
        cells.sort_unstable_by_key(|coord| (coord.line, coord.column));
        cells.dedup();
        cells
    }

    fn prepared_adjacent_coord(&mut self, direction: Direction) -> Option<Coord> {
        self.commit_canvas();
        adjacent_coord(self.grid.cursor_pos, direction)
    }

    fn canvas_height(&self) -> usize {
        let selection = self.selection.bounds();
        let (mut top, mut bottom) = (selection.top, selection.bottom);
        top = top.min(self.grid.cursor_pos.line);
        bottom = bottom.max(self.grid.cursor_pos.line);
        if let Some(bounds) = self.canvas.bounds() {
            top = top.min(bounds.min_y);
            bottom = bottom.max(bounds.max_y);
        }
        usize::try_from(i32::from(bottom) - i32::from(top) + 1).unwrap_or(usize::MAX)
    }

    fn canvas_width(&self) -> usize {
        let selection = self.selection.bounds();
        let (mut left, mut right) = (selection.left, selection.right);
        left = left.min(self.grid.cursor_pos.column);
        right = right.max(self.grid.cursor_pos.column);
        if let Some(bounds) = self.canvas.bounds() {
            left = left.min(bounds.min_x);
            right = right.max(bounds.max_x);
        }
        usize::try_from(i32::from(right) - i32::from(left) + 1).unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
#[path = "inline_tests/editor_tests.rs"]
mod tests;
