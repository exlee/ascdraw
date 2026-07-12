use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::{CursorMode, ThemeConfig};
use crate::drawing::{
    CornerStyle, LineEnding, LineStyle, glyph_with_connection, glyph_with_connection_and_corner,
    glyph_without_connection, is_line_glyph, line_ending_glyph,
};
use crate::model::{Atom, Coord, Direction, Face};
use crate::selection::{
    CanvasSelection, SelectionBounds, TextRectangle, overwrite_rectangle, replace_range,
    selected_text,
};
use crate::toolbar::{MainMode, ShapeKind, ToolbarAction, ToolbarState, Tooltip};

mod utility;

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
    shape_preview: Option<ShapePreview>,
    single_replace_pending: bool,
    pending_prepend: (usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveStroke {
    end: Coord,
    end_base_glyph: String,
    moving_ending: LineEnding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlacedLineMarker {
    coord: Coord,
    ending: LineEnding,
    base_glyph: String,
}

#[derive(Debug, Clone, Copy)]
struct ShapePreview {
    anchor: Coord,
    end: Coord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSnapshot {
    lines: Vec<Vec<Atom>>,
    cursor_pos: Coord,
    cursor_index: usize,
    selection: CanvasSelection,
    active_stroke: Option<ActiveStroke>,
    line_markers: Vec<PlacedLineMarker>,
}

impl EditSnapshot {
    pub fn same_document(&self, other: &Self) -> bool {
        self.lines == other.lines && self.line_markers == other.line_markers
    }

    #[cfg(test)]
    pub fn set_cursor_for_test(&mut self, line: usize, column: usize) {
        self.cursor_pos = Coord { line, column };
        self.selection.collapse(self.cursor_pos);
    }
}

impl EditorState {
    pub fn has_shape_preview(&self) -> bool {
        self.shape_preview.is_some()
    }
    pub fn edit_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            lines: self.grid.lines.clone(),
            cursor_pos: self.grid.cursor_pos,
            cursor_index: self.cursor_index,
            selection: self.selection,
            active_stroke: self.active_stroke.clone(),
            line_markers: self.line_markers.clone(),
        }
    }

    pub fn restore_edit_snapshot(&mut self, snapshot: EditSnapshot) {
        self.grid.lines = snapshot.lines;
        self.grid.cursor_pos = snapshot.cursor_pos;
        self.cursor_index = snapshot.cursor_index;
        self.selection = snapshot.selection;
        self.active_stroke = snapshot.active_stroke;
        self.line_markers = snapshot.line_markers;
        self.shape_preview = None;
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
            cursor_mode: CursorMode::MoveDraw,
            toolbar: ToolbarState::default(),
            selection: CanvasSelection::collapsed_at(Coord::default()),
            cursor_index: 0,
            active_stroke: None,
            line_markers: Vec::new(),
            shape_preview: None,
            single_replace_pending: false,
            pending_prepend: (0, 0),
        }
    }

    pub fn apply_theme(&mut self, theme: &ThemeConfig) {
        self.grid.default_face = theme.default.clone();
        self.grid.cursor_face = theme.cursor_block.clone();
        self.theme = theme.clone();
    }

    pub fn tooltip(&self) -> Tooltip {
        if self.toolbar.export_menu_open() {
            return Tooltip::Export;
        }
        if !self.selection.is_collapsed() {
            return Tooltip::Selection;
        }
        match self.cursor_mode {
            CursorMode::Text => Tooltip::Text,
            CursorMode::Replace => Tooltip::Replace,
            _ => self.toolbar.tooltip(),
        }
    }

    pub fn handle_toolbar_shortcut(&mut self, key: &Key, modifiers: ModifiersState) -> bool {
        if self.cursor_mode.accepts_text() {
            self.toolbar.cancel_shortcut();
            return false;
        }
        let export_was_open = self.toolbar.export_menu_open();
        let old_mode = self.toolbar.main_mode();
        if !self.toolbar.handle_shortcut(key, modifiers) {
            return false;
        }
        if matches!(key, Key::Named(NamedKey::Escape)) && !export_was_open {
            self.collapse_selection();
        }
        if self.toolbar.main_mode() != old_mode {
            self.end_stroke();
            self.shape_preview = None;
            self.sync_cursor_mode_with_toolbar();
        }
        true
    }

    pub fn apply_toolbar_action(&mut self, action: ToolbarAction) -> bool {
        if !self.toolbar.apply_action(action) {
            return false;
        }
        if matches!(
            action,
            ToolbarAction::ToggleExportMenu | ToolbarAction::RunExport(_)
        ) {
            return true;
        }
        self.end_stroke();
        self.shape_preview = None;
        self.sync_cursor_mode_with_toolbar();
        true
    }

    pub fn toggle_text_entry(&mut self) {
        self.end_stroke();
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
        self.shape_preview = None;
        self.collapse_selection();
        self.sync_cursor_mode_with_toolbar();
        true
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

    pub fn insert(&mut self, text: &str) {
        self.end_stroke();
        for part in text.split_inclusive('\n') {
            let content = part.strip_suffix('\n').unwrap_or(part);
            let atoms = UnicodeSegmentation::graphemes(content, true).map(|contents| Atom {
                face: Face::default(),
                contents: contents.to_string(),
            });
            self.grid.lines[self.grid.cursor_pos.line]
                .splice(self.cursor_index..self.cursor_index, atoms);
            self.cursor_index = self.grid.lines[self.grid.cursor_pos.line]
                .len()
                .min(self.cursor_index + UnicodeSegmentation::graphemes(content, true).count());
            if part.ends_with('\n') {
                self.newline();
            }
        }
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn write_text(&mut self, text: &str) {
        if self.single_replace_pending {
            self.replace_once(text);
        } else if self.cursor_mode == CursorMode::Replace {
            self.replace(text);
        } else {
            self.insert(text);
        }
    }

    fn replace_once(&mut self, text: &str) {
        let Some(grapheme) = UnicodeSegmentation::graphemes(text, true).next() else {
            return;
        };
        self.end_stroke();
        self.replace_selection(Some(grapheme));
        self.sync_cursor_mode_with_toolbar();
        self.restore_active_cursor_index();
    }

    fn replace(&mut self, text: &str) {
        self.end_stroke();
        for part in text.split_inclusive('\n') {
            let content = part.strip_suffix('\n').unwrap_or(part);
            for grapheme in UnicodeSegmentation::graphemes(content, true) {
                let line = &mut self.grid.lines[self.grid.cursor_pos.line];
                let atom = Atom {
                    face: Face::default(),
                    contents: grapheme.to_string(),
                };
                if self.cursor_index < line.len() {
                    line[self.cursor_index] = atom;
                } else {
                    line.push(atom);
                }
                self.cursor_index += 1;
            }
            if part.ends_with('\n') {
                self.newline();
            }
        }
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn newline(&mut self) {
        self.end_stroke();
        let remainder = self.grid.lines[self.grid.cursor_pos.line].split_off(self.cursor_index);
        self.grid.cursor_pos.line += 1;
        self.grid.lines.insert(self.grid.cursor_pos.line, remainder);
        self.cursor_index = 0;
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn backspace(&mut self) {
        self.end_stroke();
        if self.cursor_index > 0 {
            self.cursor_index -= 1;
            self.grid.lines[self.grid.cursor_pos.line].remove(self.cursor_index);
        } else if self.grid.cursor_pos.line > 0 {
            let current = self.grid.lines.remove(self.grid.cursor_pos.line);
            self.grid.cursor_pos.line -= 1;
            self.cursor_index = self.grid.lines[self.grid.cursor_pos.line].len();
            self.grid.lines[self.grid.cursor_pos.line].extend(current);
        }
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn delete(&mut self) {
        self.end_stroke();
        let line = self.grid.cursor_pos.line;
        if self.cursor_index < self.grid.lines[line].len() {
            self.grid.lines[line].remove(self.cursor_index);
        } else if line + 1 < self.grid.lines.len() {
            let next = self.grid.lines.remove(line + 1);
            self.grid.lines[line].extend(next);
        }
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn move_home(&mut self) {
        self.end_stroke();
        self.cursor_index = 0;
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn move_end(&mut self) {
        self.end_stroke();
        self.cursor_index = self.grid.lines[self.grid.cursor_pos.line].len();
        self.sync_cursor_column();
        self.collapse_selection();
    }

    pub fn move_to(&mut self, coord: Coord) -> bool {
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
        self.shape_preview = None;
        self.grid.cursor_pos = coord;
        self.cursor_index = index_for_column(&self.grid.lines[coord.line], coord.column);
        self.sync_cursor_column();
        self.collapse_selection();
    }

    fn move_to_without_ending_stroke(&mut self, coord: Coord) {
        while self.grid.lines.len() <= coord.line {
            self.grid.lines.push(Vec::new());
        }
        self.grid.cursor_pos.line = coord.line;
        self.cursor_index = index_for_column(&self.grid.lines[coord.line], coord.column);
        let current_width = display_width(&self.grid.lines[coord.line][..self.cursor_index]);
        if current_width < coord.column && self.cursor_index == self.grid.lines[coord.line].len() {
            self.grid.lines[coord.line].extend((current_width..coord.column).map(|_| Atom {
                face: Face::default(),
                contents: " ".to_string(),
            }));
            self.cursor_index = self.grid.lines[coord.line].len();
        }
        self.sync_cursor_column();
    }

    pub fn move_cursor(&mut self, direction: Direction) -> bool {
        let changed = self.move_or_draw(direction, false);
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.end = self.grid.cursor_pos;
        }
        changed
    }

    pub fn extend_selection(&mut self, direction: Direction) -> bool {
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
        line.splice(index..=index, (0..width).map(|_| blank_atom()));
        true
    }

    fn move_selection_to_without_ending_stroke(&mut self, coord: Coord) {
        while self.grid.lines.len() <= coord.line {
            self.grid.lines.push(Vec::new());
        }
        let line = &mut self.grid.lines[coord.line];
        let current_width = display_width(line);
        if current_width <= coord.column {
            line.extend((current_width..coord.column).map(|_| blank_atom()));
        }
        self.grid.cursor_pos = coord;
        self.cursor_index = index_for_column(line, coord.column);
    }

    pub fn move_or_draw(&mut self, direction: Direction, draw: bool) -> bool {
        let prepended = self.prepare_adjacent(direction);
        let from = self.grid.cursor_pos;
        let to = adjacent_coord(from, direction).expect("canvas edge was structurally extended");
        let line_style = self.toolbar.line_style();
        let corner_style = self.toolbar.line_corner();

        if !draw {
            self.end_stroke();
            self.move_to_without_ending_stroke(to);
            self.collapse_selection();
            return prepended;
        }

        let continuing_stroke = self
            .active_stroke
            .take()
            .filter(|stroke| stroke.end == from);
        let (from_was_existing_line, moving_ending) =
            if let Some(stroke) = continuing_stroke.as_ref() {
                self.take_line_marker(from);
                self.set_cell_contents(from, stroke.end_base_glyph.clone());
                (true, stroke.moving_ending)
            } else if let Some(marker) = self.take_line_marker(from) {
                self.set_cell_contents(from, marker.base_glyph);
                (true, marker.ending)
            } else {
                (
                    self.cell_contents(from).is_some_and(is_line_glyph),
                    self.toolbar.line_end(),
                )
            };

        let continuing_stroke = continuing_stroke.is_some();
        if !continuing_stroke {
            self.active_stroke = None;
        }

        let from_base = self.add_connection(from, direction, line_style, corner_style);
        self.move_to_without_ending_stroke(to);
        let to_was_existing_line = self.cell_contents(to).is_some_and(is_line_glyph);
        let Some(end_base_glyph) =
            self.add_connection(to, direction.opposite(), line_style, corner_style)
        else {
            self.active_stroke = None;
            self.collapse_selection();
            return true;
        };

        if !continuing_stroke
            && !from_was_existing_line
            && let Some(from_base) = from_base
        {
            self.apply_line_ending(
                from,
                self.toolbar.line_start(),
                direction,
                line_style,
                &from_base,
            );
        }
        if !to_was_existing_line {
            self.apply_line_ending(
                to,
                moving_ending,
                direction.opposite(),
                line_style,
                &end_base_glyph,
            );
        }
        self.active_stroke = Some(ActiveStroke {
            end: to,
            end_base_glyph,
            moving_ending,
        });
        self.collapse_selection();
        true
    }

    fn remove_connection(&mut self, coord: Coord, direction: Direction) {
        if let Some(marker) = self.take_line_marker(coord) {
            self.set_cell_contents(coord, marker.base_glyph);
        }
        let Some(line) = self.grid.lines.get_mut(coord.line) else {
            return;
        };
        let (index, column) = index_and_column_for_coord(line, coord.column);
        if column != coord.column {
            return;
        }
        if let Some(atom) = line.get_mut(index)
            && atom_width(atom) == 1
            && let Some(glyph) = glyph_without_connection(&atom.contents, direction)
        {
            atom.contents = glyph.to_string();
        }
    }

    fn cell_contents(&self, coord: Coord) -> Option<&str> {
        let line = self.grid.lines.get(coord.line)?;
        let (index, column) = index_and_column_for_coord(line, coord.column);
        (column == coord.column)
            .then(|| line.get(index))
            .flatten()
            .map(|atom| atom.contents.as_str())
    }

    fn take_line_marker(&mut self, coord: Coord) -> Option<PlacedLineMarker> {
        let index = self
            .line_markers
            .iter()
            .position(|marker| marker.coord == coord)?;
        Some(self.line_markers.remove(index))
    }

    fn remove_line_marker(&mut self, coord: Coord) {
        self.line_markers.retain(|marker| marker.coord != coord);
    }

    fn add_connection(
        &mut self,
        coord: Coord,
        direction: Direction,
        line_style: LineStyle,
        corner_style: CornerStyle,
    ) -> Option<String> {
        self.remove_line_marker(coord);
        let line = &mut self.grid.lines[coord.line];
        let (index, column) = index_and_column_for_coord(line, coord.column);

        if column < coord.column {
            line.extend((column..coord.column).map(|_| blank_atom()));
        }

        if let Some(atom) = line.get_mut(index) {
            if atom_width(atom) == 1
                && let Some(glyph) = glyph_with_connection_and_corner(
                    &atom.contents,
                    direction,
                    line_style,
                    corner_style,
                )
            {
                atom.contents = glyph.to_string();
                return Some(atom.contents.clone());
            }
            None
        } else {
            let contents =
                glyph_with_connection_and_corner(" ", direction, line_style, corner_style)
                    .expect("blank cells accept line connections")
                    .to_string();
            line.push(Atom {
                face: Face::default(),
                contents: contents.clone(),
            });
            Some(contents)
        }
    }

    fn apply_line_ending(
        &mut self,
        coord: Coord,
        ending: LineEnding,
        connected_direction: Direction,
        line_style: LineStyle,
        base_glyph: &str,
    ) {
        self.remove_line_marker(coord);
        self.set_cell_contents(
            coord,
            line_ending_glyph(ending, connected_direction, line_style).to_string(),
        );
        if ending != LineEnding::None {
            self.line_markers.push(PlacedLineMarker {
                coord,
                ending,
                base_glyph: base_glyph.to_string(),
            });
        }
    }

    fn set_cell_contents(&mut self, coord: Coord, contents: String) {
        let line = &mut self.grid.lines[coord.line];
        let (index, column) = index_and_column_for_coord(line, coord.column);
        if column == coord.column
            && let Some(atom) = line.get_mut(index)
            && atom_width(atom) == 1
        {
            atom.contents = contents;
        }
    }

    pub fn end_stroke(&mut self) {
        self.active_stroke = None;
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
        self.replace_selection(Some(&stamp));
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

    pub fn toggle_shape_preview(&mut self) {
        if self.cursor_mode != CursorMode::Shapes {
            return;
        }
        self.end_stroke();
        self.shape_preview = if self.shape_preview.is_some() {
            None
        } else {
            Some(ShapePreview {
                anchor: self.grid.cursor_pos,
                end: self.grid.cursor_pos,
            })
        };
    }

    pub fn start_shape_or_confirm(&mut self) {
        let preview = self.shape_preview.take();
        let had_preview = preview.is_some();
        let had_selection = !self.selection.is_collapsed();
        self.end_stroke();
        self.toolbar.cancel_shortcut();
        self.collapse_selection();

        if self.cursor_mode != CursorMode::Shapes {
            return;
        }

        if had_preview {
            self.shape_preview = preview;
            self.confirm_shape();
            return;
        }
        if !had_preview && !had_selection {
            self.toggle_shape_preview();
        }
    }

    pub fn selection_bounds(&self) -> SelectionBounds {
        self.selection.bounds()
    }

    #[allow(dead_code)] // Public extraction hook for the queued export implementation.
    pub fn selected_text(&self) -> String {
        selected_text(&self.grid.lines, self.selection.bounds())
    }

    pub fn paste_text_rectangle(&mut self, text: &str) -> bool {
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
        self.grid.lines = if lines.is_empty() {
            vec![Vec::new()]
        } else {
            lines
        };
        self.grid.cursor_pos = Coord::default();
        self.cursor_index = 0;
        self.active_stroke = None;
        self.line_markers.clear();
        self.shape_preview = None;
        self.single_replace_pending = false;
        self.pending_prepend = (0, 0);
        self.toolbar.cancel_shortcut();
        self.selection.collapse(Coord::default());
        self.sync_cursor_mode_with_toolbar();
    }

    pub fn clear_canvas(&mut self) {
        let cursor = self.grid.cursor_pos;
        let already_blank = self.content_cells().is_empty()
            && self.line_markers.is_empty()
            && self
                .grid
                .lines
                .iter()
                .flatten()
                .all(|atom| atom.face == Face::default());

        if !already_blank {
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
        self.shape_preview = None;
        self.single_replace_pending = false;
        self.pending_prepend = (0, 0);
        self.toolbar.cancel_shortcut();
        self.selection.collapse(cursor);
        self.sync_cursor_mode_with_toolbar();
    }

    pub fn confirm_shape(&mut self) {
        let Some(preview) = self.shape_preview.take() else {
            dbg!("No preview");
            return;
        };
        for (coord, contents) in self.shape_cells(preview) {
            self.remove_line_marker(coord);
            replace_cell(&mut self.grid.lines, coord, contents);
        }
        self.cursor_index = index_for_column(
            &self.grid.lines[self.grid.cursor_pos.line],
            self.grid.cursor_pos.column,
        );
        self.sync_cursor_column();
    }

    pub fn lines_with_shape_preview(&self) -> Option<Vec<Vec<Atom>>> {
        let preview = self.shape_preview?;
        let mut lines = self.grid.lines.clone();
        for (coord, contents) in self.shape_cells(preview) {
            replace_cell(&mut lines, coord, contents);
        }
        Some(lines)
    }

    fn shape_cells(&self, preview: ShapePreview) -> Vec<(Coord, String)> {
        let left = preview.anchor.column.min(preview.end.column);
        let right = preview.anchor.column.max(preview.end.column);
        let top = preview.anchor.line.min(preview.end.line);
        let bottom = preview.anchor.line.max(preview.end.line);
        let style = self.toolbar.shape_line_style();
        let fill = self.toolbar.shape_fill();
        match self.toolbar.shape_kind() {
            ShapeKind::Rect => rectangle_cells(left, right, top, bottom, style, false, fill),
            ShapeKind::RoundedRect => rectangle_cells(left, right, top, bottom, style, true, fill),
        }
    }

    fn sync_cursor_column(&mut self) {
        self.grid.cursor_pos.column =
            display_width(&self.grid.lines[self.grid.cursor_pos.line][..self.cursor_index]);
    }

    fn collapse_selection(&mut self) {
        self.selection.collapse(self.grid.cursor_pos);
    }

    fn restore_active_cursor_index(&mut self) {
        let active = self.selection.active();
        self.grid.cursor_pos = active;
        self.cursor_index = index_for_column(&self.grid.lines[active.line], active.column);
    }

    fn replace_selection(&mut self, replacement: Option<&str>) {
        let bounds = self.selection.bounds();
        self.cleanup_selection_connections(bounds);
        self.line_markers
            .retain(|marker| !bounds.contains(marker.coord));
        replace_range(&mut self.grid.lines, bounds, replacement);
        self.restore_active_cursor_index();
    }

    fn cleanup_selection_connections(&mut self, bounds: SelectionBounds) {
        if bounds.top > 0 {
            for column in bounds.left..=bounds.right {
                self.remove_connection(
                    Coord {
                        line: bounds.top - 1,
                        column,
                    },
                    Direction::Down,
                );
            }
        }
        if bounds.left > 0 {
            for line in bounds.top..=bounds.bottom {
                self.remove_connection(
                    Coord {
                        line,
                        column: bounds.left - 1,
                    },
                    Direction::Right,
                );
            }
        }
        for column in bounds.left..=bounds.right {
            self.remove_connection(
                Coord {
                    line: bounds.bottom.saturating_add(1),
                    column,
                },
                Direction::Up,
            );
        }
        for line in bounds.top..=bounds.bottom {
            self.remove_connection(
                Coord {
                    line,
                    column: bounds.right.saturating_add(1),
                },
                Direction::Left,
            );
        }
    }

    pub fn take_pending_prepend(&mut self) -> (usize, usize) {
        std::mem::take(&mut self.pending_prepend)
    }

    pub fn content_cells(&self) -> Vec<Coord> {
        let mut cells = Vec::new();
        for (line_index, line) in self.grid.lines.iter().enumerate() {
            let mut column: usize = 0;
            for atom in line {
                let width = atom_width(atom);
                if !atom.contents.chars().all(char::is_whitespace) {
                    cells.extend((column..column.saturating_add(width)).map(|column| Coord {
                        line: line_index,
                        column,
                    }));
                }
                column = column.saturating_add(width);
            }
        }
        cells
    }

    fn prepare_adjacent(&mut self, direction: Direction) -> bool {
        match direction {
            Direction::Up if self.grid.cursor_pos.line == 0 => {
                self.prepend_line();
                true
            }
            Direction::Left if self.grid.cursor_pos.column == 0 => {
                self.prepend_column();
                true
            }
            _ => false,
        }
    }

    fn prepend_line(&mut self) {
        self.grid.lines.insert(0, Vec::new());
        self.grid.cursor_pos.line = self.grid.cursor_pos.line.saturating_add(1);
        self.selection.shift(0, 1);
        if let Some(stroke) = self.active_stroke.as_mut() {
            stroke.end.line = stroke.end.line.saturating_add(1);
        }
        for marker in &mut self.line_markers {
            marker.coord.line = marker.coord.line.saturating_add(1);
        }
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.anchor.line = preview.anchor.line.saturating_add(1);
            preview.end.line = preview.end.line.saturating_add(1);
        }
        self.pending_prepend.1 = self.pending_prepend.1.saturating_add(1);
    }

    fn prepend_column(&mut self) {
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
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.anchor.column = preview.anchor.column.saturating_add(1);
            preview.end.column = preview.end.column.saturating_add(1);
        }
        self.pending_prepend.0 = self.pending_prepend.0.saturating_add(1);
    }
}

fn adjacent_coord(coord: Coord, direction: Direction) -> Option<Coord> {
    match direction {
        Direction::Up => Some(Coord {
            line: coord.line.checked_sub(1)?,
            column: coord.column,
        }),
        Direction::Right => Some(Coord {
            line: coord.line,
            column: coord.column.checked_add(1)?,
        }),
        Direction::Down => Some(Coord {
            line: coord.line.checked_add(1)?,
            column: coord.column,
        }),
        Direction::Left => Some(Coord {
            line: coord.line,
            column: coord.column.checked_sub(1)?,
        }),
    }
}

fn blank_atom() -> Atom {
    Atom {
        face: Face::default(),
        contents: " ".to_string(),
    }
}

fn replace_cell(lines: &mut Vec<Vec<Atom>>, coord: Coord, contents: String) {
    while lines.len() <= coord.line {
        lines.push(Vec::new());
    }
    let line = &mut lines[coord.line];
    let boundary = coord.column.saturating_add(1);
    let mut prefix = Vec::new();
    let mut suffix = Vec::new();
    let mut column = 0usize;
    for atom in line.iter() {
        let width = atom_width(atom);
        let end = column.saturating_add(width);
        if end <= coord.column {
            prefix.push(atom.clone());
        } else if column < coord.column {
            prefix.extend((column..coord.column).map(|_| blank_atom()));
        }
        if column >= boundary {
            suffix.push(atom.clone());
        } else if end > boundary {
            suffix.extend((boundary..end).map(|_| blank_atom()));
        }
        column = end;
    }
    let prefix_width = display_width(&prefix);
    prefix.extend((prefix_width..coord.column).map(|_| blank_atom()));
    prefix.push(Atom {
        face: Face::default(),
        contents,
    });
    prefix.extend(suffix);
    *line = prefix;
}

fn rectangle_cells(
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
    style: LineStyle,
    rounded: bool,
    fill: Option<&str>,
) -> Vec<(Coord, String)> {
    let mut cells = Vec::new();
    if left == right && top == bottom {
        cells.push((
            Coord {
                line: top,
                column: left,
            },
            fill.unwrap_or("□").to_string(),
        ));
        return cells;
    }
    for line in top..=bottom {
        for column in left..=right {
            let on_top = line == top;
            let on_bottom = line == bottom;
            let on_left = column == left;
            let on_right = column == right;
            let glyph = match (on_top, on_right, on_bottom, on_left) {
                (true, true, _, true) | (true, false, true, true) => {
                    line_glyph(&[Direction::Right, Direction::Down], style, rounded)
                }
                (true, true, true, false) => {
                    line_glyph(&[Direction::Down, Direction::Left], style, rounded)
                }
                (false, true, true, true) => {
                    line_glyph(&[Direction::Up, Direction::Right], style, rounded)
                }
                (true, true, false, false) => {
                    line_glyph(&[Direction::Down, Direction::Left], style, rounded)
                }
                (false, true, true, false) => {
                    line_glyph(&[Direction::Up, Direction::Left], style, rounded)
                }
                (false, false, true, true) => {
                    line_glyph(&[Direction::Up, Direction::Right], style, rounded)
                }
                (true, false, false, true) => {
                    line_glyph(&[Direction::Right, Direction::Down], style, rounded)
                }
                (true, false, _, false) | (false, false, true, false) => {
                    line_glyph(&[Direction::Left, Direction::Right], style, rounded)
                }
                (false, true, false, _) | (false, false, false, true) => {
                    line_glyph(&[Direction::Up, Direction::Down], style, rounded)
                }
                _ => match fill {
                    Some(fill) => fill.chars().next().unwrap_or(' '),
                    None => continue,
                },
            };
            cells.push((Coord { line, column }, glyph.to_string()));
        }
    }
    cells
}

fn line_glyph(directions: &[Direction], style: LineStyle, rounded: bool) -> char {
    if !rounded && style == LineStyle::Thin && directions.len() == 2 {
        return match (directions[0], directions[1]) {
            (Direction::Right, Direction::Down) | (Direction::Down, Direction::Right) => '┌',
            (Direction::Down, Direction::Left) | (Direction::Left, Direction::Down) => '┐',
            (Direction::Up, Direction::Right) | (Direction::Right, Direction::Up) => '└',
            (Direction::Up, Direction::Left) | (Direction::Left, Direction::Up) => '┘',
            _ => connected_glyph(directions, style),
        };
    }
    connected_glyph(directions, style)
}

fn connected_glyph(directions: &[Direction], style: LineStyle) -> char {
    let mut glyph = ' ';
    for direction in directions {
        glyph = glyph_with_connection(&glyph.to_string(), *direction, style)
            .expect("generated line glyph accepts another connection");
    }
    glyph
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
    use crate::toolbar::UtilityKind;

    fn state() -> EditorState {
        EditorState::new(&ThemeConfig::default(), "ascdraw")
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
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
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
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
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
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
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
    fn replacement_retains_its_existing_smart_connection_cleanup() {
        let mut state = state();
        state.insert("│\n│\n│");
        state.move_to(Coord { line: 1, column: 0 });

        assert!(state.begin_single_replace());
        state.write_text("x");

        assert_eq!(contents(&state.grid.lines[0]), "╵");
        assert_eq!(contents(&state.grid.lines[1]), "x");
        assert_eq!(contents(&state.grid.lines[2]), "╷");
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
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
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
        assert_eq!(state.toolbar.main_mode(), MainMode::Line);
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

        assert_eq!(state.toolbar.main_mode(), MainMode::Line);
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
        assert_eq!(state.grid.lines[2].len(), 4);
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
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);

        for key in ["1", "2"] {
            assert!(state.handle_toolbar_shortcut(
                &winit::keyboard::Key::Character(key.into()),
                winit::keyboard::ModifiersState::empty(),
            ));
        }
        assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
        assert_eq!(state.cursor_mode, CursorMode::Stamp);
    }

    #[test]
    fn tooltip_tracks_editor_mode_and_export_override() {
        let mut state = EditorState::new(&ThemeConfig::default(), "test");
        assert_eq!(state.tooltip(), Tooltip::Line);

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
    fn toolbar_shortcuts_are_bypassed_in_every_text_accepting_mode() {
        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            let mut state = state();
            state.cursor_mode = mode;

            assert!(
                !state
                    .handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty(),)
            );
            assert_eq!(state.toolbar.main_mode(), MainMode::Line);
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

        assert_eq!(state.toolbar.main_mode(), MainMode::Line);
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
                .all(|atom| atom.contents == " ")
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
                UtilityKind::Select => 0,
                UtilityKind::Push => 1,
                UtilityKind::Pull => 2,
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
