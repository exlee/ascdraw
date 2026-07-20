use crate::app::CursorMode;
use crate::canvas::LayerStack;
use crate::drawing::{LineStyle, glyph_with_connection};
use crate::model::{Atom, Coord, Direction};
use crate::toolbar::ShapeKind;

use super::{Editor, ShapePreview};

impl Editor {
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

    pub fn start_shape_or_confirm(&mut self) -> bool {
        let preview = self.shape_preview.take();
        let had_preview = preview.is_some();
        let had_selection = !self.selection.is_collapsed();
        self.end_stroke();
        self.toolbar.cancel_shortcut();
        if self.cursor_mode != CursorMode::Shapes {
            self.collapse_selection();
            return false;
        }
        if had_preview {
            self.collapse_selection();
            self.shape_preview = preview;
            self.confirm_shape();
            return true;
        }
        if had_selection {
            let bounds = self.selection.bounds();
            self.shape_preview = Some(ShapePreview {
                anchor: Coord {
                    line: bounds.top.saturating_sub(1),
                    column: bounds.left.saturating_sub(1),
                },
                end: Coord {
                    line: bounds.bottom.saturating_add(1),
                    column: bounds.right.saturating_add(1),
                },
            });
            self.collapse_selection();
            self.confirm_shape();
            return true;
        }
        self.collapse_selection();
        self.toggle_shape_preview();
        false
    }

    pub fn confirm_shape(&mut self) {
        let Some(preview) = self.shape_preview.take() else {
            return;
        };
        let face = self.write_face();
        for (coord, contents) in self.shape_cells(preview) {
            self.remove_line_marker(coord);
            let atom = Atom::new(contents).expect("shape glyph is one cell");
            self.canvas
                .set_at(coord, atom, &face)
                .expect("shape glyphs occupy one sparse cell");
        }
    }

    pub(crate) fn shape_preview_canvas(&self) -> Option<LayerStack> {
        let preview = self.shape_preview?;
        let face = self.write_face();
        let mut canvas = self.canvas.clone();
        for (coord, contents) in self.shape_cells(preview) {
            let atom = Atom::new(contents).expect("shape glyph is one cell");
            canvas
                .set_at(coord, atom, &face)
                .expect("shape preview glyphs occupy one sparse cell");
        }
        Some(canvas)
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
}

fn rectangle_cells(
    left: i16,
    right: i16,
    top: i16,
    bottom: i16,
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
