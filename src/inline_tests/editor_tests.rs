use super::*;
use crate::canvas::{CoordData, LayerMap, LineMarker as PlacedLineMarker};
use crate::drawing::LineEnding;
use crate::editor_event::EditorState;
use crate::model::{ColorId, StyledAtom};
use crate::toolbar::{ToggleKind, UtilityKind};

#[derive(Debug, Clone)]
pub(crate) struct LayerView {
    lines: Vec<Vec<StyledAtom>>,
}

impl Editor {
    pub(crate) fn toolbar_spans(&self, row: usize) -> Vec<ToolbarSpan> {
        self.toolbar
            .toolbar_spans_with_layers_for_width(row, usize::MAX, &self.layer_summaries())
    }

    pub(crate) fn content_cells_including_hidden(&self) -> Vec<Coord> {
        self.sparse_content_cells(true)
    }

    /// Builds dense display/exchange fixtures for tests outside the editor behavior suite.
    pub(crate) fn set_lines_for_test(&mut self, lines: Vec<Vec<StyledAtom>>) {
        let active = self.canvas.active_id();
        let visible = self.canvas.layers()[self.canvas.active_index()].visible;
        let markers = self.canvas.active_line_markers();
        let mut replacement = Some(
            crate::dense_exchange::from_dense_with_markers(active, visible, &lines, &markers)
                .expect("test canvas contains valid one-cell atoms"),
        );
        self.canvas.mutate_layers(|id, layer| {
            if id == active {
                *layer = replacement.take().expect("active layer is unique");
            }
        });
    }

    pub(crate) fn lines_for_test(&self) -> Vec<Vec<StyledAtom>> {
        crate::dense_exchange::to_dense(&self.canvas.layers()[self.canvas.active_index()])
    }

    pub(crate) fn active_layer_for_test(&self) -> &LayerMap {
        &self.canvas.layers()[self.canvas.active_index()]
    }

    pub(crate) fn active_cell_for_test(&self, coord: Coord) -> Option<&CoordData> {
        self.canvas.active_cell(coord)
    }

    pub(crate) fn set_cell_face_for_test(&mut self, coord: Coord, face: Face) {
        let atom = self
            .canvas
            .active_cell(coord)
            .expect("test cell exists")
            .atom
            .as_ref()
            .clone();
        self.canvas.set_at(coord, atom, &face).unwrap();
    }

    pub(crate) fn line_markers_for_test(&self) -> Vec<PlacedLineMarker> {
        self.canvas.active_line_markers()
    }

    pub(crate) fn layer_views(&self) -> Vec<LayerView> {
        self.canvas
            .layers()
            .iter()
            .map(|layer| LayerView {
                lines: crate::dense_exchange::to_dense(layer),
            })
            .collect()
    }
}

fn state() -> Editor {
    Editor::new(&ThemeConfig::default(), "ascdraw")
}

fn blank_atom() -> StyledAtom {
    StyledAtom {
        face: Face::default(),
        contents: " ".to_owned(),
    }
}

#[path = "editor_tests/editing_and_lines.rs"]
mod editing_and_lines;
#[path = "editor_tests/layers_and_canvas.rs"]
mod layers_and_canvas;
#[path = "editor_tests/tools_and_moves.rs"]
mod tools_and_moves;

fn utility_state(rows: &[&str], utility: UtilityKind, cursor: Coord) -> Editor {
    let mut state = state();
    state.insert(&rows.join("\n"));
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
    state.selection.collapse(cursor);
    state
}

fn line_contents(state: &Editor) -> Vec<String> {
    state
        .lines_for_test()
        .iter()
        .map(|line| contents(line))
        .collect()
}

fn select_toolbar_option(state: &mut Editor, key: &str, count: usize) {
    let submenu = key.parse::<usize>().expect("numeric toolbar group") - 2;
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu,
        option: count,
    });
}

fn contents(line: &[StyledAtom]) -> String {
    line.iter().map(|atom| atom.contents.as_str()).collect()
}

fn sparse_row_contents(state: &Editor, line: i16) -> String {
    let Some(row) = state.active_layer_for_test().rows().get(&line) else {
        return String::new();
    };
    let Some((&last_column, _)) = row.last_key_value() else {
        return String::new();
    };
    (0..=last_column)
        .map(|column| row.get(&column).map_or(" ", |cell| cell.atom.contents()))
        .collect()
}

fn cell_face(state: &Editor, coord: Coord) -> &Face {
    state
        .active_cell_for_test(coord)
        .map_or(&state.grid.default_face, |cell| cell.face.as_ref())
}
