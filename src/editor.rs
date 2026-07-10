use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{CursorMode, ThemeConfig};
use crate::drawing::glyph_with_connection;
use crate::model::{Atom, Coord, Direction, Face};
use crate::toolbar::ToolbarState;

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
        }
    }

    pub fn apply_theme(&mut self, theme: &ThemeConfig) {
        self.grid.default_face = theme.default.clone();
        self.grid.cursor_face = theme.cursor.clone();
    }

    pub fn insert(&mut self, text: &str) {
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

    pub fn newline(&mut self) {
        let remainder = self.grid.lines[self.grid.cursor_pos.line].split_off(self.cursor_index);
        self.grid.cursor_pos.line += 1;
        self.grid.lines.insert(self.grid.cursor_pos.line, remainder);
        self.cursor_index = 0;
        self.sync_cursor_column();
    }

    pub fn backspace(&mut self) {
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
        let line = self.grid.cursor_pos.line;
        if self.cursor_index < self.grid.lines[line].len() {
            self.grid.lines[line].remove(self.cursor_index);
        } else if line + 1 < self.grid.lines.len() {
            let next = self.grid.lines.remove(line + 1);
            self.grid.lines[line].extend(next);
        }
        self.sync_cursor_column();
    }

    pub fn move_left(&mut self) {
        if self.cursor_index > 0 {
            self.cursor_index -= 1;
        } else if self.grid.cursor_pos.line > 0 {
            self.grid.cursor_pos.line -= 1;
            self.cursor_index = self.grid.lines[self.grid.cursor_pos.line].len();
        }
        self.sync_cursor_column();
    }

    pub fn move_right(&mut self) {
        let line = self.grid.cursor_pos.line;
        if self.cursor_index < self.grid.lines[line].len() {
            self.cursor_index += 1;
        } else if line + 1 < self.grid.lines.len() {
            self.grid.cursor_pos.line += 1;
            self.cursor_index = 0;
        }
        self.sync_cursor_column();
    }

    pub fn move_up(&mut self) {
        if self.grid.cursor_pos.line > 0 {
            let column = self.grid.cursor_pos.column;
            self.grid.cursor_pos.line -= 1;
            self.cursor_index =
                index_for_column(&self.grid.lines[self.grid.cursor_pos.line], column);
            self.sync_cursor_column();
        }
    }

    pub fn move_down(&mut self) {
        if self.grid.cursor_pos.line + 1 < self.grid.lines.len() {
            let column = self.grid.cursor_pos.column;
            self.grid.cursor_pos.line += 1;
            self.cursor_index =
                index_for_column(&self.grid.lines[self.grid.cursor_pos.line], column);
            self.sync_cursor_column();
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_index = 0;
        self.sync_cursor_column();
    }

    pub fn move_end(&mut self) {
        self.cursor_index = self.grid.lines[self.grid.cursor_pos.line].len();
        self.sync_cursor_column();
    }

    pub fn move_to(&mut self, coord: Coord) {
        while self.grid.lines.len() <= coord.line {
            self.grid.lines.push(Vec::new());
        }
        self.grid.cursor_pos.line = coord.line;
        self.cursor_index = index_for_column(&self.grid.lines[coord.line], coord.column);
        let current_width = display_width(&self.grid.lines[coord.line][..self.cursor_index]);
        if current_width < coord.column {
            self.grid.lines[coord.line].extend((current_width..coord.column).map(|_| Atom {
                face: Face::default(),
                contents: " ".to_string(),
            }));
            self.cursor_index = self.grid.lines[coord.line].len();
        }
        self.sync_cursor_column();
    }

    pub fn move_cursor(&mut self, direction: Direction) {
        if self.cursor_mode == CursorMode::MoveDraw {
            self.move_or_draw(direction, false);
            return;
        }

        match direction {
            Direction::Up => self.move_up(),
            Direction::Right => self.move_right(),
            Direction::Down => self.move_down(),
            Direction::Left => self.move_left(),
        }
    }

    pub fn move_or_draw(&mut self, direction: Direction, draw: bool) {
        let from = self.grid.cursor_pos;
        let Some(to) = adjacent_coord(from, direction) else {
            return;
        };
        let line_style = self.toolbar.line_style();

        if draw {
            self.add_connection(from, direction, line_style);
        }
        self.move_to(to);
        if draw {
            self.add_connection(to, direction.opposite(), line_style);
        }
    }

    fn add_connection(
        &mut self,
        coord: Coord,
        direction: Direction,
        line_style: crate::drawing::LineStyle,
    ) {
        let line = &mut self.grid.lines[coord.line];
        let (index, column) = index_and_column_for_coord(line, coord.column);

        if column < coord.column {
            line.extend((column..coord.column).map(|_| blank_atom()));
        }

        if let Some(atom) = line.get_mut(index) {
            if atom_width(atom) == 1
                && let Some(glyph) = glyph_with_connection(&atom.contents, direction, line_style)
            {
                atom.contents = glyph.to_string();
            }
        } else {
            line.push(Atom {
                face: Face::default(),
                contents: glyph_with_connection(" ", direction, line_style)
                    .expect("blank cells accept line connections")
                    .to_string(),
            });
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
    fn cursor_column_tracks_wide_graphemes() {
        let mut state = state();
        state.insert("😀x");
        assert_eq!(state.grid.cursor_pos.column, 3);
        state.move_left();
        assert_eq!(state.grid.cursor_pos.column, 2);
        state.move_left();
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
    fn draw_preserves_non_line_text() {
        let mut state = state();
        state.insert("x");
        state.move_to(Coord { line: 0, column: 0 });
        state.move_or_draw(Direction::Right, true);

        assert_eq!(contents(&state.grid.lines[0]), "x╴");
    }

    fn contents(line: &[Atom]) -> String {
        line.iter().map(|atom| atom.contents.as_str()).collect()
    }
}
