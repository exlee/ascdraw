use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use winit::keyboard::{Key, ModifiersState};

use crate::app::{CursorMode, ThemeConfig};
use crate::drawing::{
    CornerStyle, LineEnding, LineStyle, glyph_with_connection, glyph_with_connection_and_corner,
    glyph_without_connection, is_line_glyph, line_ending_glyph,
};
use crate::model::{Atom, Coord, Direction, Face};
use crate::toolbar::{MainMode, ShapeKind, ToolbarAction, ToolbarState};

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
    pub window_title: String,
    pub cursor_mode: CursorMode,
    pub toolbar: ToolbarState,
    cursor_index: usize,
    active_stroke: Option<ActiveStroke>,
    line_markers: Vec<PlacedLineMarker>,
    shape_preview: Option<ShapePreview>,
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
                cursor_face: theme.cursor.clone(),
            },
            window_title: window_title.into(),
            cursor_mode: CursorMode::MoveDraw,
            toolbar: ToolbarState::default(),
            cursor_index: 0,
            active_stroke: None,
            line_markers: Vec::new(),
            shape_preview: None,
        }
    }

    pub fn apply_theme(&mut self, theme: &ThemeConfig) {
        self.grid.default_face = theme.default.clone();
        self.grid.cursor_face = theme.cursor.clone();
    }

    pub fn cycle_toolbar_shortcut(&mut self, key: &Key, modifiers: ModifiersState) -> bool {
        if self.cursor_mode == CursorMode::Text {
            return false;
        }
        if !self.toolbar.cycle_shortcut(key, modifiers) {
            return false;
        }
        self.end_stroke();
        self.shape_preview = None;
        self.sync_cursor_mode_with_toolbar();
        true
    }

    pub fn apply_toolbar_action(&mut self, action: ToolbarAction) -> bool {
        if !self.toolbar.apply_action(action) {
            return false;
        }
        self.end_stroke();
        self.shape_preview = None;
        self.sync_cursor_mode_with_toolbar();
        true
    }

    pub fn toggle_text_entry(&mut self) {
        self.end_stroke();
        if self.cursor_mode == CursorMode::Text {
            self.sync_cursor_mode_with_toolbar();
        } else {
            self.cursor_mode = CursorMode::Text;
        }
    }

    pub fn toggle_replace_mode(&mut self) {
        self.end_stroke();
        if self.cursor_mode == CursorMode::Replace {
            self.sync_cursor_mode_with_toolbar();
        } else {
            self.cursor_mode = CursorMode::Replace;
        }
    }

    fn sync_cursor_mode_with_toolbar(&mut self) {
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
    }

    pub fn write_text(&mut self, text: &str) {
        if self.cursor_mode == CursorMode::Replace {
            self.replace(text);
        } else {
            self.insert(text);
        }
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
    }

    pub fn newline(&mut self) {
        self.end_stroke();
        let remainder = self.grid.lines[self.grid.cursor_pos.line].split_off(self.cursor_index);
        self.grid.cursor_pos.line += 1;
        self.grid.lines.insert(self.grid.cursor_pos.line, remainder);
        self.cursor_index = 0;
        self.sync_cursor_column();
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
    }

    pub fn move_home(&mut self) {
        self.end_stroke();
        self.cursor_index = 0;
        self.sync_cursor_column();
    }

    pub fn move_end(&mut self) {
        self.end_stroke();
        self.cursor_index = self.grid.lines[self.grid.cursor_pos.line].len();
        self.sync_cursor_column();
    }

    pub fn move_to(&mut self, coord: Coord) {
        self.end_stroke();
        self.move_to_without_ending_stroke(coord);
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.end = self.grid.cursor_pos;
        }
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

    pub fn move_cursor(&mut self, direction: Direction) {
        self.move_or_draw(direction, false);
        if let Some(preview) = self.shape_preview.as_mut() {
            preview.end = self.grid.cursor_pos;
        }
    }

    pub fn move_or_draw(&mut self, direction: Direction, draw: bool) {
        let from = self.grid.cursor_pos;
        let Some(to) = adjacent_coord(from, direction) else {
            return;
        };
        let line_style = self.toolbar.line_style();
        let corner_style = self.toolbar.line_corner();

        if !draw {
            self.end_stroke();
            self.move_to_without_ending_stroke(to);
            return;
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
            return;
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
    }

    pub fn move_or_erase(&mut self, direction: Direction) {
        self.end_stroke();
        let from = self.grid.cursor_pos;
        let Some(to) = adjacent_coord(from, direction) else {
            return;
        };
        self.remove_connection(from, direction);
        self.move_to_without_ending_stroke(to);
        self.remove_connection(to, direction.opposite());
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

    pub fn clear_cell(&mut self) {
        self.end_stroke();
        let coord = self.grid.cursor_pos;
        self.remove_line_marker(coord);
        let line = &mut self.grid.lines[coord.line];
        let (index, column) = index_and_column_for_coord(line, coord.column);
        if column != coord.column {
            return;
        }
        let Some(width) = line.get(index).map(atom_width) else {
            return;
        };
        line.splice(index..=index, (0..width).map(|_| blank_atom()));
        self.cursor_index = index;
        self.sync_cursor_column();
    }

    pub fn place_stamp(&mut self) {
        self.end_stroke();
        let coord = self.grid.cursor_pos;
        let stamp = self.toolbar.stamp().to_string();
        self.remove_line_marker(coord);
        replace_cell(&mut self.grid.lines, coord, stamp);
        self.cursor_index = index_for_column(&self.grid.lines[coord.line], coord.column);
        self.sync_cursor_column();
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
    fn erasing_movement_removes_only_the_traversed_segments() {
        let mut state = state();
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);

        state.move_or_erase(Direction::Left);
        assert_eq!(contents(&state.grid.lines[0]), "╶╴ ");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });

        state.move_or_erase(Direction::Left);
        assert_eq!(contents(&state.grid.lines[0]), "   ");
        assert_eq!(state.grid.cursor_pos, Coord::default());
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
        state.toolbar.cycle_shortcut(
            &winit::keyboard::Key::Character("2".into()),
            winit::keyboard::ModifiersState::empty(),
        );
        state.toolbar.cycle_shortcut(
            &winit::keyboard::Key::Character("3".into()),
            winit::keyboard::ModifiersState::empty(),
        );

        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Right, true);
        state.move_or_draw(Direction::Down, true);

        assert_eq!(contents(&state.grid.lines[0]), "◀─╮");
        assert_eq!(contents(&state.grid.lines[1]), "  ▼");
    }

    #[test]
    fn unadorned_endings_use_the_selected_double_line_style() {
        let mut state = state();
        state.toolbar.cycle_shortcut(
            &winit::keyboard::Key::Character("4".into()),
            winit::keyboard::ModifiersState::empty(),
        );
        state.toolbar.cycle_shortcut(
            &winit::keyboard::Key::Character("4".into()),
            winit::keyboard::ModifiersState::empty(),
        );

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

        state.clear_cell();

        assert_eq!(contents(&state.grid.lines[0]), "  x");
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 0 });
    }

    #[test]
    fn toolbar_main_mode_controls_editor_mode() {
        let mut state = state();
        state.toggle_text_entry();
        assert_eq!(state.cursor_mode, CursorMode::Text);
        assert!(!state.cycle_toolbar_shortcut(
            &winit::keyboard::Key::Character("1".into()),
            winit::keyboard::ModifiersState::empty(),
        ));
        state.move_cursor(Direction::Right);
        assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
        state.toggle_text_entry();
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);

        assert!(state.cycle_toolbar_shortcut(
            &winit::keyboard::Key::Character("1".into()),
            winit::keyboard::ModifiersState::empty(),
        ));
        assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
        assert_eq!(state.cursor_mode, CursorMode::Stamp);
    }

    #[test]
    fn stamp_mode_places_the_exclusively_selected_stamp() {
        let mut state = state();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 1,
            option: 3,
        });

        state.place_stamp();

        assert_eq!(contents(&state.grid.lines[0]), "█");
        assert_eq!(state.grid.cursor_pos, Coord::default());
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
        for _ in 0..count {
            state.toolbar.cycle_shortcut(
                &winit::keyboard::Key::Character(key.into()),
                winit::keyboard::ModifiersState::empty(),
            );
        }
    }

    fn contents(line: &[Atom]) -> String {
        line.iter().map(|atom| atom.contents.as_str()).collect()
    }
}
