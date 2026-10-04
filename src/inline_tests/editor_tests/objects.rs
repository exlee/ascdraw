use super::*;
use crate::objects::{AnchorKind, Session};
use crate::toolbar::ObjectCommand;

fn at(line: i16, column: i16) -> Coord {
    Coord { line, column }
}

fn put_cursor(state: &mut Editor, coord: Coord) {
    state.grid.cursor_pos = coord;
    state.selection.collapse(coord);
}

/// Runs one user edit the way the window runtime does: inside a history
/// capture, followed by the object sync.
fn edit(state: &mut Editor, change: impl FnOnce(&mut Editor)) {
    state.begin_history_capture();
    change(state);
    state.sync_objects();
    state.finish_history_capture();
}

fn command(state: &mut Editor, command: ObjectCommand) -> bool {
    let mut applied = false;
    edit(state, |state| {
        applied = state.apply_toolbar_action(ToolbarAction::Object(command));
    });
    applied
}

fn replace_at(state: &mut Editor, coord: Coord, text: &str) {
    put_cursor(state, coord);
    edit(state, |state| {
        state.toggle_replace_mode();
        state.write_text(text);
        state.toggle_replace_mode();
    });
}

/// Draws `rows` at the origin and defines them as an object.
fn defined(rows: &[&str]) -> Editor {
    let mut state = state();
    state.insert(&rows.join("\n"));
    let width = rows.iter().map(|row| row.chars().count()).max().unwrap() as i16;
    state
        .selection
        .select(at(0, 0), at(rows.len() as i16 - 1, width - 1));
    assert!(command(&mut state, ObjectCommand::Define));
    state
}

fn place_copy(state: &mut Editor, coord: Coord) {
    put_cursor(state, coord);
    edit(state, |state| {
        assert!(state.place_object());
    });
}

#[test]
fn define_needs_a_selection_and_opens_define_edit() {
    let mut empty = state();
    empty.insert("ab");
    assert!(!empty.object_menu_state().define_enabled);
    command(&mut empty, ObjectCommand::Define);
    assert!(empty.objects().definitions.is_empty());

    let state = defined(&["ab"]);
    let definition = &state.objects().definitions[0];
    assert_eq!((definition.width, definition.height), (2, 1));
    assert!(matches!(state.object_session(), Some(Session::Define(_))));
    assert!(state.selection.is_collapsed());
}

#[test]
fn define_edit_changes_every_copy() {
    let mut state = defined(&["ab"]);
    place_copy(&mut state, at(4, 0));
    replace_at(&mut state, at(0, 0), "X");
    assert_eq!(sparse_row_contents(&state, 4), "Xb");
    assert_eq!(
        state.objects().definitions[0]
            .cells
            .get(at(0, 0))
            .unwrap()
            .atom,
        "X"
    );
}

#[test]
fn define_edit_outside_the_bounds_grows_the_definition() {
    let mut state = defined(&["ab"]);
    place_copy(&mut state, at(4, 0));
    replace_at(&mut state, at(0, 3), "Z");
    let definition = &state.objects().definitions[0];
    assert_eq!(definition.width, 4);
    assert_eq!(sparse_row_contents(&state, 4), "ab Z");
}

#[test]
fn edit_changes_only_the_local_copy_and_a_space_clears_the_definition() {
    let mut state = defined(&["ab"]);
    place_copy(&mut state, at(4, 0));
    command(&mut state, ObjectCommand::DefineEdit);
    assert_eq!(state.object_session(), None);

    put_cursor(&mut state, at(4, 0));
    assert!(command(&mut state, ObjectCommand::Edit));
    replace_at(&mut state, at(4, 0), " ");
    assert_eq!(sparse_row_contents(&state, 0), "ab");
    assert_eq!(sparse_row_contents(&state, 4), " b");
    command(&mut state, ObjectCommand::Edit);

    put_cursor(&mut state, at(0, 0));
    command(&mut state, ObjectCommand::DefineEdit);
    replace_at(&mut state, at(0, 1), "Y");
    assert_eq!(sparse_row_contents(&state, 4), " Y");
}

#[test]
fn define_edit_shows_the_definition_and_restores_the_local_copy_on_exit() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(4, 0));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(4, 1), "Q");
    command(&mut state, ObjectCommand::Edit);

    put_cursor(&mut state, at(4, 0));
    command(&mut state, ObjectCommand::DefineEdit);
    assert_eq!(sparse_row_contents(&state, 4), "ab");
    assert_eq!(state.object_overlay().ghost.len(), 1);
    command(&mut state, ObjectCommand::DefineEdit);
    assert_eq!(sparse_row_contents(&state, 4), "aQ");
}

#[test]
fn sessions_stick_across_modes_and_escape_leaves_them_from_any_mode() {
    let mut state = defined(&["ab"]);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    assert!(matches!(state.object_session(), Some(Session::Define(_))));
    assert_eq!(state.tooltip(), Tooltip::ObjectDefine);
    state.selection.select(at(0, 0), at(0, 1));
    assert!(state.cancel_current_state());
    assert!(state.object_session().is_some());
    assert!(state.cancel_current_state());
    assert_eq!(state.object_session(), None);
}

#[test]
fn backspace_on_an_anchor_removes_the_anchor_before_the_cell() {
    let mut state = defined(&["ab"]);
    put_cursor(&mut state, at(0, 1));
    command(&mut state, ObjectCommand::Anchor(AnchorKind::W));
    let backspace = |state: &mut Editor| {
        edit(state, |state| {
            assert!(crate::apply_edit_command(
                state,
                crate::input::EditCommand::Clear
            ));
        });
    };
    backspace(&mut state);
    assert!(state.objects().definitions[0].anchors.is_empty());
    assert_eq!(sparse_row_contents(&state, 0), "ab");
    backspace(&mut state);
    assert_eq!(sparse_row_contents(&state, 0), "a");
}

#[test]
fn anchor_digits_eight_and_nine_win_over_layer_and_color_panels() {
    let mut state = defined(&["abc"]);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode));
    state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    let digit = |text: &str| Key::Character(text.into());
    put_cursor(&mut state, at(0, 0));
    state.handle_toolbar_shortcut(&digit("4"), ModifiersState::empty());
    state.handle_toolbar_shortcut(&digit("8"), ModifiersState::empty());
    put_cursor(&mut state, at(0, 2));
    state.handle_toolbar_shortcut(&digit("4"), ModifiersState::empty());
    state.handle_toolbar_shortcut(&digit("9"), ModifiersState::empty());
    let kinds = state.objects().definitions[0]
        .anchors
        .iter()
        .map(|anchor| anchor.kind)
        .collect::<Vec<_>>();
    assert_eq!(kinds, [AnchorKind::SW, AnchorKind::Extension]);
    assert_eq!(state.toolbar.pending_shortcut(), None);
}

#[test]
fn anchors_need_define_edit() {
    let mut state = defined(&["ab"]);
    assert!(state.object_menu_state().anchor_enabled);
    put_cursor(&mut state, at(0, 1));
    assert!(command(&mut state, ObjectCommand::Anchor(AnchorKind::W)));
    assert_eq!(state.objects().definitions[0].anchors.len(), 1);
    assert_eq!(state.object_overlay().anchors.len(), 1);

    command(&mut state, ObjectCommand::DefineEdit);
    assert!(!state.object_menu_state().anchor_enabled);
    command(&mut state, ObjectCommand::Anchor(AnchorKind::N));
    assert_eq!(state.objects().definitions[0].anchors.len(), 1);
}

#[test]
fn backspace_in_objects_clears_like_normal_editing() {
    let mut state = defined(&["ab"]);
    place_copy(&mut state, at(2, 0));
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    let backspace = Key::Named(NamedKey::Backspace);
    let command = crate::input::edit_command(
        &backspace,
        false,
        ModifiersState::empty(),
        state.cursor_mode,
    );
    assert_eq!(command, Some(crate::input::EditCommand::Clear));

    put_cursor(&mut state, at(0, 1));
    state.selection.select(at(0, 1), at(0, 1));
    edit(&mut state, |state| {
        assert!(state.clear_selection());
    });
    assert_eq!(state.objects().instances.len(), 2);
    assert_eq!(sparse_row_contents(&state, 0), "a");
    assert_eq!(sparse_row_contents(&state, 2), "a");
}

#[test]
fn space_on_a_copy_places_another_copy() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(0, 1));
    assert_eq!(state.objects().instances.len(), 2);
    assert_eq!(state.transient_tip(), None);
}

#[test]
fn history_state_restores_object_definitions() {
    let mut state = defined(&["ab"]);
    let before = state.history_state();
    replace_at(&mut state, at(0, 0), "X");
    assert!(state.history_state().objects_differ(&before));
    state.restore_history_state(before.clone());
    assert!(!state.history_state().objects_differ(&before));
}

#[test]
fn objects_shortcuts_define_and_attach_anchors() {
    let mut state = state();
    state.insert("ab");
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    state.selection.select(at(0, 0), at(0, 1));
    let digit = |text: &str| Key::Character(text.into());
    assert!(state.handle_toolbar_shortcut(&digit("2"), ModifiersState::empty()));
    assert!(state.take_toolbar_document_change());
    assert!(matches!(state.object_session(), Some(Session::Define(_))));

    put_cursor(&mut state, at(0, 1));
    assert!(state.handle_toolbar_shortcut(&digit("4"), ModifiersState::empty()));
    assert!(state.handle_toolbar_shortcut(&digit("1"), ModifiersState::empty()));
    assert_eq!(
        state.objects().definitions[0].anchors[0].kind,
        AnchorKind::W
    );
}

#[test]
fn objects_menu_dims_unavailable_commands() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    state.sync_object_menu();
    let dimmed = |state: &Editor, wanted: ObjectCommand| {
        state
            .toolbar_spans(crate::toolbar::MENU_FIRST_ROW)
            .iter()
            .find(|span| span.action == Some(ToolbarAction::Object(wanted)))
            .unwrap()
            .tooltip
    };
    assert!(dimmed(&state, ObjectCommand::Define));
    assert!(dimmed(&state, ObjectCommand::Anchor(AnchorKind::W)));
    state.insert("ab");
    state.selection.select(at(0, 0), at(0, 1));
    state.sync_object_menu();
    assert!(!dimmed(&state, ObjectCommand::Define));
}

#[test]
fn backspace_outside_sessions_removes_only_the_copy_under_the_cursor() {
    let mut state = defined(&["ab"]);
    place_copy(&mut state, at(2, 0));
    command(&mut state, ObjectCommand::DefineEdit);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(2, 1));
    edit(&mut state, |state| {
        assert!(crate::apply_edit_command(
            state,
            crate::input::EditCommand::Clear
        ));
    });
    assert_eq!(state.objects().instances.len(), 1);
    assert_eq!(state.objects().definitions.len(), 1);
    assert_eq!(sparse_row_contents(&state, 2), "");
    assert_eq!(sparse_row_contents(&state, 0), "ab");
}

#[test]
fn copied_object_pastes_as_a_copy_with_its_local_edits() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    put_cursor(&mut state, at(0, 0));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(0, 1), "Q");
    command(&mut state, ObjectCommand::Edit);

    put_cursor(&mut state, at(0, 0));
    let text = state.copy_object().unwrap();
    put_cursor(&mut state, at(3, 2));
    assert!(!state.paste_object("other text"));
    edit(&mut state, |state| {
        assert!(state.paste_object(&text));
    });
    assert_eq!(state.objects().instances.len(), 2);
    assert_eq!(sparse_row_contents(&state, 3), "  aQ");

    put_cursor(&mut state, at(5, 5));
    assert_eq!(state.copy_object(), None);
    assert!(!state.paste_object(&text));
}

#[test]
fn local_edits_outside_the_copy_keep_its_boundary() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(4, 0));
    command(&mut state, ObjectCommand::Edit);
    let before = state.objects().bounds(&state.objects().instances[1]);
    replace_at(&mut state, at(4, 3), "Z");
    let copy = &state.objects().instances[1];
    assert_eq!(state.objects().bounds(copy), before);
    assert!(copy.overlay.0.is_empty());
    assert_eq!(sparse_row_contents(&state, 4), "ab Z");
}

#[test]
fn the_cursor_moves_freely_inside_copies() {
    let mut state = defined(&["abc"]);
    command(&mut state, ObjectCommand::DefineEdit);
    put_cursor(&mut state, at(0, 0));
    assert_eq!(state.navigation_target(Direction::Right, 1), Some(at(0, 1)));
    state.move_cursor(Direction::Right);
    assert_eq!(state.grid.cursor_pos, at(0, 1));
    assert_eq!(
        state.object_overlay().cursor_object,
        Some(state.objects().bounds(&state.objects().instances[0]))
    );
}

#[test]
fn backspace_in_edit_restores_the_definition_and_a_space_covers_it() {
    let mut state = shared(&["ab"]);
    put_cursor(&mut state, at(0, 0));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(0, 0), "X");
    replace_at(&mut state, at(0, 1), " ");
    assert_eq!(sparse_row_contents(&state, 0), "X");

    put_cursor(&mut state, at(0, 0));
    edit(&mut state, |state| {
        assert!(crate::apply_edit_command(
            state,
            crate::input::EditCommand::Clear
        ));
    });
    assert_eq!(sparse_row_contents(&state, 0), "a");
    assert_eq!(state.objects().instances[0].overlay.0.len(), 1);
}

#[test]
fn anchor_hotkeys_are_evenly_spaced() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    for row in [
        crate::toolbar::MENU_FIRST_ROW,
        crate::toolbar::MENU_FIRST_ROW + 1,
    ] {
        let widths = state
            .toolbar_spans(row)
            .iter()
            .filter(|span| {
                matches!(
                    span.action,
                    Some(ToolbarAction::Object(ObjectCommand::Anchor(_)))
                )
            })
            .map(|span| unicode_width::UnicodeWidthStr::width(span.contents.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(widths.len(), AnchorKind::ALL.len());
        assert!(widths.iter().all(|width| *width == widths[0]));
    }
}

/// An object with a second copy far away, so the first copy keeps a
/// definition separate from its local copy.
fn shared(rows: &[&str]) -> Editor {
    let mut state = defined(rows);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(20, 0));
    put_cursor(&mut state, at(0, 0));
    state
}

fn boxed() -> Editor {
    let mut state = shared(&["╭───╮", "│   │", "╰───╯"]);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    state
}

fn stretch(state: &mut Editor, cursor: Coord, direction: Direction) {
    put_cursor(state, cursor);
    edit(state, |state| {
        assert!(crate::apply_edit_command(
            state,
            crate::input::EditCommand::StretchObject(direction)
        ));
    });
}

fn copy_bounds(state: &Editor) -> SelectionBounds {
    state.objects().bounds(&state.objects().instances[0])
}

#[test]
fn ctrl_toward_the_far_side_expands_that_edge() {
    let mut state = boxed();
    stretch(&mut state, at(1, 4), Direction::Right);
    assert_eq!(copy_bounds(&state).right, 5);
    assert_eq!(sparse_row_contents(&state, 0), "╭────╮");
    stretch(&mut state, at(1, 0), Direction::Left);
    assert_eq!(copy_bounds(&state).left, -1);
    stretch(&mut state, at(2, 2), Direction::Down);
    assert_eq!(copy_bounds(&state).bottom, 3);
    stretch(&mut state, at(0, 2), Direction::Up);
    assert_eq!(copy_bounds(&state).top, -1);
    assert_eq!(state.objects().definitions[0].width, 5);
    assert_eq!(state.objects().definitions[0].height, 3);
}

#[test]
fn ctrl_toward_the_center_contracts_the_opposite_edge() {
    let mut state = boxed();
    stretch(&mut state, at(1, 4), Direction::Left);
    assert_eq!(
        (copy_bounds(&state).left, copy_bounds(&state).right),
        (0, 3)
    );
    stretch(&mut state, at(1, 0), Direction::Right);
    assert_eq!(
        (copy_bounds(&state).left, copy_bounds(&state).right),
        (1, 3)
    );
    assert_eq!(sparse_row_contents(&state, 0), " ╭─╮");
    stretch(&mut state, at(2, 2), Direction::Up);
    assert_eq!(
        (copy_bounds(&state).top, copy_bounds(&state).bottom),
        (0, 1)
    );
}

#[test]
fn copies_do_not_stretch_in_define_edit_or_edit() {
    let mut state = defined(&["ab"]);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(0, 1));
    assert!(!state.stretch_object(Direction::Right));
    command(&mut state, ObjectCommand::DefineEdit);
    command(&mut state, ObjectCommand::Edit);
    assert!(!state.stretch_object(Direction::Right));
    command(&mut state, ObjectCommand::Edit);
    assert!(state.stretch_object(Direction::Right));
}

#[test]
fn alt_direction_on_a_copy_moves_it_with_the_cursor() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    put_cursor(&mut state, at(0, 1));
    edit(&mut state, |state| {
        assert!(crate::apply_edit_command(
            state,
            crate::input::EditCommand::Erase(Direction::Right)
        ));
    });
    assert_eq!(sparse_row_contents(&state, 0), " ab");
    assert_eq!(state.objects().instances[0].origin, at(0, 1));
    assert_eq!(state.grid.cursor_pos, at(0, 2));
    assert_eq!(
        state.objects().definitions[0]
            .cells
            .get(at(0, 0))
            .unwrap()
            .atom,
        "a"
    );
}

#[test]
fn edit_menu_path_opens_definition_and_local_edits() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(0, 0));
    let digit = |text: &str| Key::Character(text.into());
    state.handle_toolbar_shortcut(&digit("3"), ModifiersState::empty());
    state.handle_toolbar_shortcut(&digit("2"), ModifiersState::empty());
    assert!(matches!(state.object_session(), Some(Session::Edit(_))));
    state.handle_toolbar_shortcut(&digit("3"), ModifiersState::empty());
    state.handle_toolbar_shortcut(&digit("1"), ModifiersState::empty());
    assert!(matches!(state.object_session(), Some(Session::Define(_))));
}

#[test]
fn reset_returns_a_copy_to_its_definition() {
    let mut state = boxed();
    stretch(&mut state, at(1, 4), Direction::Right);
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(1, 1), "Q");
    command(&mut state, ObjectCommand::Edit);
    put_cursor(&mut state, at(1, 1));
    assert!(command(&mut state, ObjectCommand::Reset));
    let copy = &state.objects().instances[0];
    assert!(copy.overlay.0.is_empty());
    assert_eq!(copy.stretch, (0, 0));
    assert_eq!(sparse_row_contents(&state, 0), "╭───╮");
    assert_eq!(sparse_row_contents(&state, 1), "│   │");
}

#[test]
fn edit_dims_cells_that_still_show_the_definition() {
    let mut state = shared(&["ab"]);
    put_cursor(&mut state, at(0, 0));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(0, 0), "X");
    assert_eq!(state.object_overlay().definition_cells, [at(0, 1)]);
}

#[test]
fn repeated_stretch_keeps_contracting_past_the_center() {
    let mut state = boxed();
    put_cursor(&mut state, at(1, 3));
    for _ in 0..3 {
        edit(&mut state, |state| {
            assert!(state.stretch_object(Direction::Left));
        });
    }
    assert_eq!(
        (copy_bounds(&state).left, copy_bounds(&state).right),
        (0, 1)
    );

    // A press from another cell decides afresh.
    put_cursor(&mut state, at(1, 0));
    edit(&mut state, |state| {
        assert!(state.stretch_object(Direction::Left));
    });
    assert_eq!(copy_bounds(&state).left, -1);
}

#[test]
fn overlapping_copies_compose_instead_of_erasing_each_other() {
    let mut state = defined(&["a b"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(0, 1));
    assert_eq!(sparse_row_contents(&state, 0), "aabb");

    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(0, 1));
    edit(&mut state, |state| {
        assert!(state.remove_object());
    });
    assert_eq!(sparse_row_contents(&state, 0), "a b");
}

#[test]
fn side_handles_resize_the_copy_with_the_mouse() {
    let mut state = boxed();
    let handle = |state: &Editor, coord: Coord| state.object_handle_at(coord).unwrap();
    let (id, right) = handle(&state, at(1, 4));
    assert_eq!(right, crate::editor::HandleSide::Right);
    edit(&mut state, |state| {
        assert!(state.drag_object_handle(id, right, at(1, 7)));
    });
    assert_eq!(sparse_row_contents(&state, 0), "╭──────╮");

    let (id, left) = handle(&state, at(1, 0));
    edit(&mut state, |state| {
        assert!(state.drag_object_handle(id, left, at(1, 2)));
    });
    assert_eq!(copy_bounds(&state).left, 2);
    assert_eq!(state.objects().definitions[0].width, 5);

    let (id, bottom) = handle(&state, at(2, 4));
    edit(&mut state, |state| {
        assert!(state.drag_object_handle(id, bottom, at(-5, 4)));
    });
    assert_eq!(copy_bounds(&state).bottom, copy_bounds(&state).top);
}

#[test]
fn edges_and_corners_are_handles_in_every_mode_outside_sessions() {
    use crate::editor::HandleSide;
    let mut state = defined(&["abc", "def", "ghi"]);
    assert_eq!(state.object_handle_at(at(1, 0)), None);
    command(&mut state, ObjectCommand::DefineEdit);
    let side = |state: &Editor, coord| state.object_handle_at(coord).map(|(_, side)| side);
    assert_eq!(side(&state, at(0, 0)), Some(HandleSide::TopLeft));
    assert_eq!(side(&state, at(2, 2)), Some(HandleSide::BottomRight));
    assert_eq!(side(&state, at(1, 0)), Some(HandleSide::Left));
    assert_eq!(side(&state, at(0, 1)), Some(HandleSide::Top));
    assert_eq!(side(&state, at(1, 1)), None);
    put_cursor(&mut state, at(1, 1));
    command(&mut state, ObjectCommand::Edit);
    assert_eq!(side(&state, at(0, 0)), None);
}

#[test]
fn a_corner_drag_resizes_both_axes() {
    let mut state = boxed();
    let (id, corner) = state.object_handle_at(at(2, 4)).unwrap();
    edit(&mut state, |state| {
        assert!(state.drag_object_handle(id, corner, at(4, 6)));
    });
    let bounds = copy_bounds(&state);
    assert_eq!((bounds.bottom, bounds.right), (4, 6));
}

#[test]
fn a_local_space_paints_over_what_lies_below() {
    let mut state = defined(&["a b"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(0, 1));
    assert_eq!(sparse_row_contents(&state, 0), "aabb");

    put_cursor(&mut state, at(0, 2));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(0, 2), " ");
    assert_eq!(state.object_overlay().blank_cells, [at(0, 2)]);
    command(&mut state, ObjectCommand::Edit);
    assert_eq!(sparse_row_contents(&state, 0), "aa b");

    put_cursor(&mut state, at(0, 1));
    edit(&mut state, |state| {
        assert!(state.move_object(Direction::Down));
    });
    assert_eq!(sparse_row_contents(&state, 0), "a b");
    assert_eq!(sparse_row_contents(&state, 1), " a b");
}

#[test]
fn copy_inside_sessions_copies_cells_not_the_object() {
    let mut state = defined(&["ab"]);
    put_cursor(&mut state, at(0, 0));
    assert_eq!(state.copy_object(), None);
    command(&mut state, ObjectCommand::DefineEdit);
    command(&mut state, ObjectCommand::Edit);
    assert_eq!(state.copy_object(), None);
    command(&mut state, ObjectCommand::Edit);
    assert!(state.copy_object().is_some());
}

#[test]
fn a_local_space_on_an_empty_cell_is_opaque_too() {
    let mut state = shared(&["a b"]);
    put_cursor(&mut state, at(0, 0));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(0, 1), " ");
    assert_eq!(state.object_overlay().blank_cells, [at(0, 1)]);
}

#[test]
fn the_copy_under_the_cursor_has_one_outline() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(3, 0));
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(0, 0));
    let overlay = state.object_overlay();
    let cursor = overlay.cursor_object.unwrap();
    assert!(!overlay.outlines.contains(&cursor));
    assert_eq!(overlay.outlines.len(), 1);
}

#[test]
fn text_written_over_a_copy_lies_above_it_and_leaves_the_copy_alone() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    replace_at(&mut state, at(0, 0), "X");
    assert_eq!(sparse_row_contents(&state, 0), "Xb");
    assert!(state.objects().instances[0].overlay.0.is_empty());
    assert_eq!(
        state.objects().definitions[0]
            .cells
            .get(at(0, 0))
            .unwrap()
            .atom,
        "a"
    );

    put_cursor(&mut state, at(0, 0));
    edit(&mut state, |state| {
        assert!(state.clear_selection());
    });
    assert_eq!(sparse_row_contents(&state, 0), "ab");
}

#[test]
fn text_already_there_stays_above_a_placed_copy() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    put_cursor(&mut state, at(3, 1));
    edit(&mut state, |state| state.insert("Q"));
    place_copy(&mut state, at(3, 0));
    assert_eq!(sparse_row_contents(&state, 3), "aQ");
}

#[test]
fn moving_and_resizing_a_copy_clear_nothing() {
    let mut state = boxed();
    put_cursor(&mut state, at(1, 7));
    edit(&mut state, |state| state.insert("Q"));
    replace_at(&mut state, at(1, 2), "T");
    stretch(&mut state, at(1, 4), Direction::Right);
    stretch(&mut state, at(1, 5), Direction::Right);
    stretch(&mut state, at(1, 6), Direction::Right);
    assert_eq!(sparse_row_contents(&state, 1), "│ T    Q");
    for _ in 0..3 {
        let cursor = state.grid.cursor_pos;
        stretch(&mut state, cursor, Direction::Left);
    }
    assert_eq!(sparse_row_contents(&state, 1), "│ T │  Q");

    put_cursor(&mut state, at(0, 0));
    edit(&mut state, |state| {
        assert!(state.move_object(Direction::Down));
    });
    assert_eq!(sparse_row_contents(&state, 1), "╭─T─╮  Q");
}

#[test]
fn a_selection_move_carries_copies_inside_it_with_the_text() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    put_cursor(&mut state, at(0, 3));
    edit(&mut state, |state| state.insert("Z"));
    state.selection.select(at(0, 0), at(0, 3));
    edit(&mut state, |state| {
        assert!(state.begin_selected_move_lift());
        assert!(state.move_lift(Direction::Down));
        assert!(state.move_lift(Direction::Down));
        assert!(state.confirm_move_lift());
    });
    assert_eq!(sparse_row_contents(&state, 0), "");
    assert_eq!(sparse_row_contents(&state, 2), "ab Z");
    assert_eq!(state.objects().instances[0].origin, at(2, 0));
}

#[test]
fn a_drawn_rectangle_becomes_an_object_anchored_in_its_corners() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    put_cursor(&mut state, at(1, 1));
    edit(&mut state, |state| {
        state.start_shape_or_confirm();
        state.move_cursor(Direction::Right);
        state.move_cursor(Direction::Right);
        state.move_cursor(Direction::Right);
        state.move_cursor(Direction::Down);
        state.move_cursor(Direction::Down);
        assert!(state.start_shape_or_confirm());
    });
    let definition = &state.objects().definitions[0];
    assert_eq!((definition.width, definition.height), (4, 3));
    let mut kinds = definition
        .anchors
        .iter()
        .map(|anchor| (anchor.at, anchor.kind))
        .collect::<Vec<_>>();
    kinds.sort_by_key(|(at, _)| (at.line, at.column));
    assert_eq!(
        kinds,
        [
            (at(0, 0), AnchorKind::NW),
            (at(0, 3), AnchorKind::NE),
            (at(2, 0), AnchorKind::SW),
            (at(2, 3), AnchorKind::SE),
        ]
    );
    assert_eq!(state.objects().instances[0].origin, at(1, 1));
    assert_eq!(state.object_session(), None);

    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(2, 4));
    edit(&mut state, |state| {
        assert!(state.stretch_object(Direction::Right));
    });
    assert_eq!(
        sparse_row_contents(&state, 1).trim_start().chars().count(),
        5
    );
}

#[test]
fn a_lone_copy_folds_its_local_edits_but_keeps_its_size_as_stretch() {
    let mut state = defined(&["╭───╮", "│   │", "╰───╯"]);
    command(&mut state, ObjectCommand::DefineEdit);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(1, 1));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(1, 1), "Q");
    command(&mut state, ObjectCommand::Edit);
    let definition = &state.objects().definitions[0];
    assert_eq!(definition.cells.get(at(1, 1)).unwrap().atom, "Q");
    assert!(state.objects().instances[0].overlay.0.is_empty());

    stretch(&mut state, at(1, 4), Direction::Right);
    assert_eq!(state.objects().definitions[0].width, 5);
    assert_eq!(state.objects().instances[0].stretch, (1, 0));
    assert_eq!(sparse_row_contents(&state, 0), "╭────╮");
}

#[test]
fn shrinking_and_growing_a_lone_copy_restores_its_structure() {
    let rows = [
        "╭──┬─────╮",
        "│  │     │",
        "├──┴─────┤",
        "│        │",
        "╰────────╯",
    ];
    let mut state = defined(&rows);
    command(&mut state, ObjectCommand::DefineEdit);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(2, 9));
    for _ in 0..7 {
        let cursor = state.grid.cursor_pos;
        stretch(&mut state, cursor, Direction::Left);
    }
    assert_eq!(copy_bounds(&state).right, 2);
    for _ in 0..7 {
        let cursor = state.grid.cursor_pos;
        stretch(&mut state, cursor, Direction::Right);
    }
    for (line, row) in rows.iter().enumerate() {
        assert_eq!(sparse_row_contents(&state, line as i16), *row);
    }
}

#[test]
fn copies_of_a_shared_definition_keep_their_local_edits() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(3, 0));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(3, 0), "Q");
    command(&mut state, ObjectCommand::Edit);
    assert_eq!(
        state.objects().definitions[0]
            .cells
            .get(at(0, 0))
            .unwrap()
            .atom,
        "a"
    );
    assert_eq!(state.objects().instances[1].overlay.0.len(), 1);
}

#[test]
fn backspace_clears_text_over_a_copy_before_the_copy() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    replace_at(&mut state, at(0, 0), "X");
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    put_cursor(&mut state, at(0, 0));
    let backspace = |state: &mut Editor| {
        edit(state, |state| {
            assert!(crate::apply_edit_command(
                state,
                crate::input::EditCommand::Clear
            ));
        });
    };
    backspace(&mut state);
    assert_eq!(sparse_row_contents(&state, 0), "ab");
    assert_eq!(state.objects().instances.len(), 1);
    backspace(&mut state);
    assert!(state.objects().instances.is_empty());
    assert_eq!(sparse_row_contents(&state, 0), "");
}

#[test]
fn dissolving_a_copy_leaves_its_glyphs_as_plain_text() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(2, 0));
    assert!(state.dissolve_object_at(at(2, 1)));
    assert_eq!(state.objects().instances.len(), 1);
    assert_eq!(state.objects().definitions.len(), 1);
    assert_eq!(sparse_row_contents(&state, 2), "ab");

    assert!(state.dissolve_object_at(at(0, 0)));
    assert!(state.objects().definitions.is_empty());
    assert_eq!(sparse_row_contents(&state, 0), "ab");
    assert!(!state.dissolve_object_at(at(0, 0)));
}

#[test]
fn a_corner_anchor_is_attached_to_both_sides_of_the_box() {
    use crate::editor::AnchorSegment;
    let mut state = defined(&["ab", "cd"]);
    put_cursor(&mut state, at(0, 0));
    command(&mut state, ObjectCommand::Anchor(AnchorKind::NW));
    let marks = state.object_overlay().anchors;
    assert_eq!(
        marks[0].segments,
        [
            AnchorSegment::Edge {
                horizontal: true,
                min: true
            },
            AnchorSegment::Edge {
                horizontal: false,
                min: true
            },
        ]
    );
}

#[test]
fn local_edits_of_a_lone_copy_are_not_dimmed() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    put_cursor(&mut state, at(0, 0));
    command(&mut state, ObjectCommand::Edit);
    replace_at(&mut state, at(0, 0), "X");
    assert!(state.object_overlay().definition_cells.is_empty());
}

#[test]
fn opening_define_edit_on_a_stretched_lone_copy_shows_the_local_version() {
    let mut state = defined(&["╭─╮", "╰─╯"]);
    command(&mut state, ObjectCommand::DefineEdit);
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Objects));
    stretch(&mut state, at(0, 2), Direction::Right);
    stretch(&mut state, at(0, 3), Direction::Right);
    assert_eq!(sparse_row_contents(&state, 0), "╭───╮");
    put_cursor(&mut state, at(0, 1));
    command(&mut state, ObjectCommand::DefineEdit);
    assert_eq!(sparse_row_contents(&state, 0), "╭───╮");
    assert_eq!(state.objects().definitions[0].width, 5);
    assert_eq!(state.objects().instances[0].stretch, (0, 0));
}

#[test]
fn opening_local_edit_from_a_double_click_moves_the_cursor_there() {
    let mut state = defined(&["ab"]);
    command(&mut state, ObjectCommand::DefineEdit);
    place_copy(&mut state, at(3, 0));
    put_cursor(&mut state, at(9, 9));
    assert!(state.open_local_edit_at(at(3, 1)));
    assert_eq!(state.grid.cursor_pos, at(3, 1));
    let id = state.objects().instances[1].id;
    assert_eq!(state.object_session(), Some(Session::Edit(id)));
    assert!(!state.open_local_edit_at(at(0, 0)));
}

#[test]
fn an_alt_gesture_started_on_text_only_erases() {
    let mut state = defined(&["abc"]);
    command(&mut state, ObjectCommand::DefineEdit);
    replace_at(&mut state, at(0, 0), "X");
    put_cursor(&mut state, at(0, 0));
    for _ in 0..3 {
        edit(&mut state, |state| {
            state.alt_step(Direction::Right);
        });
    }
    assert_eq!(state.objects().instances[0].origin, at(0, 0));
    assert_eq!(sparse_row_contents(&state, 0), "abc");
}

#[test]
fn an_alt_gesture_started_on_a_copy_only_moves_it() {
    let mut state = defined(&["abc"]);
    command(&mut state, ObjectCommand::DefineEdit);
    put_cursor(&mut state, at(0, 4));
    edit(&mut state, |state| state.insert("Z"));
    put_cursor(&mut state, at(0, 2));
    for _ in 0..3 {
        edit(&mut state, |state| {
            state.alt_step(Direction::Right);
        });
    }
    assert_eq!(state.objects().instances[0].origin, at(0, 3));
    assert_eq!(sparse_row_contents(&state, 0), "   aZc");
}
