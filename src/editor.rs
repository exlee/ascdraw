use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::{CursorMode, ThemeConfig};
use crate::drawing::{
    CornerStyle, LineEnding, LineStyle, glyph_with_connection, glyph_with_connection_and_corner,
    glyph_without_connection, is_line_glyph, line_ending_glyph,
};
use crate::model::{Atom, Coord, Direction, Face};
use crate::selection::{CanvasSelection, SelectionBounds, replace_range, selected_text};
use crate::toolbar::{MainMode, ShapeKind, ToolbarAction, ToolbarState, Tooltip};

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

#[derive(Debug, Clone)]
struct ActiveStroke {
    end: Coord,
    end_base_glyph: String,
    moving_ending: LineEnding,
}

#[derive(Debug, Clone)]
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

impl EditorState {
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
            self.cancel_canvas_transients();
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
        if self.cursor_mode == CursorMode::Text {
            self.sync_cursor_mode_with_toolbar();
        } else {
            self.cursor_mode = CursorMode::Text;
        }
    }

    pub fn toggle_replace_mode(&mut self) {
        self.end_stroke();
        self.toolbar.cancel_shortcut();
        self.single_replace_pending = false;
        if self.cursor_mode == CursorMode::Replace {
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
        self.replace_selection(None);
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

    pub fn cancel_canvas_transients(&mut self) {
        let had_preview = self.shape_preview.take().is_some();
        let had_selection = !self.selection.is_collapsed();
        self.end_stroke();
        self.toolbar.cancel_shortcut();
        self.collapse_selection();
        if !had_preview && !had_selection && self.cursor_mode == CursorMode::Shapes {
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

    pub fn confirm_shape(&mut self) {
        let Some(preview) = self.shape_preview.take() else {
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
            ShapeKind::Ellipse => ellipse_cells(left, right, top, bottom, style, fill),
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
    let (index, column) = index_and_column_for_coord(line, coord.column);
    if column < coord.column && index == line.len() {
        line.extend((column..coord.column).map(|_| blank_atom()));
    }
    let width = line.get(index).map(atom_width).unwrap_or(1);
    let replacement = std::iter::once(Atom {
        face: Face::default(),
        contents,
    })
    .chain((1..width).map(|_| blank_atom()));
    if index < line.len() {
        line.splice(index..=index, replacement);
    } else {
        line.extend(replacement);
    }
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

fn ellipse_cells(
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
    style: LineStyle,
    fill: Option<&str>,
) -> Vec<(Coord, String)> {
    if right.saturating_sub(left) < 3 || bottom.saturating_sub(top) < 2 {
        return rectangle_cells(left, right, top, bottom, style, true, fill);
    }
    let cx = (left + right) as f64 / 2.0;
    let cy = (top + bottom) as f64 / 2.0;
    let rx = (right - left) as f64 / 2.0;
    let ry = (bottom - top) as f64 / 2.0;
    let inside = |line: isize, column: isize| {
        let dx = (column as f64 - cx) / rx;
        let dy = (line as f64 - cy) / ry;
        dx * dx + dy * dy <= 1.0
    };
    let mut cells = Vec::new();
    for line in top..=bottom {
        for column in left..=right {
            if !inside(line as isize, column as isize) {
                continue;
            }
            let boundary = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .into_iter()
                .any(|(dy, dx)| !inside(line as isize + dy, column as isize + dx));
            let glyph = if boundary {
                let nx = (column as f64 - cx) / rx;
                let ny = (line as f64 - cy) / ry;
                if (nx.abs() - ny.abs()).abs() < 0.35 {
                    let directions = match (ny.is_sign_negative(), nx.is_sign_negative()) {
                        (true, true) => [Direction::Right, Direction::Down],
                        (true, false) => [Direction::Down, Direction::Left],
                        (false, true) => [Direction::Up, Direction::Right],
                        (false, false) => [Direction::Up, Direction::Left],
                    };
                    line_glyph(&directions, style, true)
                } else if nx.abs() > ny.abs() {
                    line_glyph(&[Direction::Up, Direction::Down], style, true)
                } else {
                    line_glyph(&[Direction::Left, Direction::Right], style, true)
                }
            } else if let Some(fill) = fill {
                fill.chars().next().unwrap_or(' ')
            } else {
                continue;
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

    fn state() -> EditorState {
        EditorState::new(&ThemeConfig::default(), "ascdraw")
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
        state.cancel_canvas_transients();
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
            ending: LineEnding::Arrow,
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
            ending: LineEnding::Arrow,
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
            option: 1,
        });
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 1,
        });

        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "◀─╮");
        assert_eq!(contents(&state.grid.lines[1]), "  ▼");
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
        select_toolbar_option(&mut state, "3", 1);
        state.move_or_draw(Direction::Right, true);
        state.move_to(Coord { line: 0, column: 1 });

        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "╶╮");
        assert_eq!(contents(&state.grid.lines[1]), " ▼");
    }

    #[test]
    fn drawing_from_a_start_marker_moves_it_to_the_new_end() {
        let mut state = state();
        select_toolbar_option(&mut state, "2", 1);
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
    fn ellipse_preview_uses_the_selected_shape() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        });
        state.toggle_shape_preview();
        for _ in 0..6 {
            state.move_cursor(Direction::Right);
        }
        for _ in 0..4 {
            state.move_cursor(Direction::Down);
        }

        let preview = state.lines_with_shape_preview().unwrap();
        let non_blank = preview
            .iter()
            .flatten()
            .filter(|atom| atom.contents != " ")
            .count();
        assert!(non_blank >= 8);
        assert!(non_blank < 7 * 5);
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
