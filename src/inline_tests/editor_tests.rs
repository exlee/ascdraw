use super::*;
use crate::editor_event::EditorState;
use crate::export::lines_from_text;
use crate::model::ColorId;
use crate::toolbar::{ToggleKind, UtilityKind};

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
    state.set_lines_for_test(
        rows.iter()
            .map(|row| {
                UnicodeSegmentation::graphemes(*row, true)
                    .map(|contents| StyledAtom {
                        face: Face::default(),
                        contents: contents.to_string(),
                    })
                    .collect()
            })
            .collect(),
    );
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
