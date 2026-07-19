use super::*;

#[test]
fn tooltip_tracks_editor_mode_and_export_override() {
    let mut state = Editor::new(&ThemeConfig::default(), "test");
    assert_eq!(state.tooltip(), Tooltip::Stamp);

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
fn tooltip_reacts_to_selection_and_transient_editor_states() {
    let mut state = Editor::new(&ThemeConfig::default(), "test");
    state.insert("abcd");
    state.move_home();
    assert_eq!(state.tooltip(), Tooltip::Stamp);
    assert!(state.tooltip().text().starts_with("Stamp:"));

    state.extend_selection(Direction::Right);
    assert_eq!(state.tooltip(), Tooltip::Selection);
    assert!(
        state
            .tooltip()
            .text()
            .contains("Alt-direction lifts and moves")
    );

    assert!(state.begin_selected_move_lift());
    assert_eq!(state.tooltip(), Tooltip::SelectionMoveLift);
    assert!(
        state
            .tooltip()
            .text()
            .contains("direction confirms and moves")
    );
    assert!(state.cancel_move_lift());

    state.move_to(Coord::default());
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    state.toggle_shape_preview();
    assert_eq!(state.tooltip(), Tooltip::ShapePreview);
    assert!(state.tooltip().text().contains("Space confirms"));

    state.toggle_shape_preview();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
    assert!(state.begin_single_replace());
    assert_eq!(state.tooltip(), Tooltip::SingleReplace);
    assert!(
        state
            .tooltip()
            .text()
            .contains("type or paste one character")
    );

    state.cancel_text_entry();
    state.clear_canvas();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.move_or_draw(Direction::Right, true);
    assert_eq!(state.tooltip(), Tooltip::LineStroke);
    assert!(state.tooltip().text().contains("release Ctrl to finish"));
}

#[test]
fn export_activation_is_transient_and_does_not_mutate_editor_state() {
    let mut state = state();
    assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
    assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 2,
    }));
    state.insert("canvas");
    let edit = state.edit_snapshot();
    let cursor_mode = state.cursor_mode;
    let durable = state.toolbar.durable_selections();

    assert!(state.handle_toolbar_shortcut(&Key::Character("0".into()), ModifiersState::empty(),));
    assert!(state.toolbar.export_menu_open());
    assert_eq!(state.edit_snapshot(), edit);
    assert_eq!(state.cursor_mode, cursor_mode);
    assert_eq!(state.toolbar.durable_selections(), durable);

    assert!(state.handle_toolbar_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty(),));
    assert!(!state.toolbar.export_menu_open());
    assert_eq!(state.edit_snapshot(), edit);
    assert_eq!(state.cursor_mode, cursor_mode);
    assert_eq!(state.toolbar.durable_selections(), durable);

    assert!(state.apply_toolbar_action(ToolbarAction::ToggleExportMenu));
    assert!(state.toolbar.export_menu_open());
    assert_eq!(state.edit_snapshot(), edit);
    assert_eq!(state.cursor_mode, cursor_mode);
    assert_eq!(state.toolbar.durable_selections(), durable);
}

#[test]
fn toolbar_shortcuts_are_bypassed_in_every_text_accepting_mode() {
    for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
        let mut state = state();
        state.cursor_mode = mode;

        assert!(
            !state.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty(),)
        );
        assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
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

    assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
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

    assert_eq!(contents(&state.lines_for_test()[0]), "█");
    assert_eq!(state.grid.cursor_pos, Coord::default());
}

#[test]
fn custom_stamp_fills_selection_until_a_bundled_stamp_is_selected() {
    let mut state = state();
    state.insert("abcd");
    state.move_to(Coord { line: 0, column: 1 });
    state.extend_selection(Direction::Right);

    assert!(state.select_custom_stamp("◇"));
    state.place_stamp();
    assert_eq!(contents(&state.lines_for_test()[0]), "a◇◇d");
    assert_eq!(state.toolbar.custom_stamp(), Some("◇"));

    assert!(state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 0,
    }));
    assert_eq!(state.toolbar.custom_stamp(), None);
    assert_eq!(state.toolbar.stamp(), "□");
    assert!(!state.select_custom_stamp("😀"));
    assert!(!state.select_custom_stamp("xy"));
}

#[test]
fn stamp_in_middle_of_line_preserves_the_other_segments() {
    let mut state = state();
    state.insert("╷\n│\n╵");
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 1,
        option: 0,
    });
    state.move_to(Coord { line: 1, column: 0 });

    state.place_stamp();

    assert_eq!(contents(&state.lines_for_test()[0]), "╷");
    assert_eq!(contents(&state.lines_for_test()[1]), "△");
    assert_eq!(contents(&state.lines_for_test()[2]), "╵");
}

#[test]
fn shift_drawing_in_stamp_mode_stamps_both_ends_of_the_move() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));

    state.draw_stamp(Direction::Right);

    assert_eq!(contents(&state.lines_for_test()[0]), "□□");
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
            .lines_for_test()
            .iter()
            .flatten()
            .all(|atom| atom.contents.chars().all(char::is_whitespace))
    );

    state.confirm_shape();
    assert!(state.lines_with_shape_preview().is_none());
    assert_eq!(contents(&state.lines_for_test()[0]), "┌──┐");
    assert_eq!(contents(&state.lines_for_test()[2]), "└──┘");
}

#[test]
fn shape_space_draws_one_cell_outside_a_selected_region() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    state
        .selection
        .select(Coord { line: 3, column: 5 }, Coord { line: 2, column: 3 });

    assert!(state.start_shape_or_confirm());

    assert!(state.selection.is_collapsed());
    assert!(state.shape_preview.is_none());
    assert_eq!(state.take_pending_prepend(), (0, 0));
    assert_eq!(
        state
            .lines_for_test()
            .iter()
            .map(|line| contents(line))
            .collect::<Vec<_>>(),
        ["", "  ┌───┐", "  │   │", "  │   │", "  └───┘"]
    );
}

#[test]
fn shape_space_prepends_to_surround_a_selection_at_the_origin() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    state
        .selection
        .select(Coord { line: 1, column: 1 }, Coord { line: 0, column: 0 });

    assert!(state.start_shape_or_confirm());

    assert_eq!(state.take_pending_prepend(), (1, 1));
    assert_eq!(
        state
            .lines_for_test()
            .iter()
            .map(|line| contents(line))
            .collect::<Vec<_>>(),
        ["┌──┐", "│  │", "│  │", "└──┘"]
    );
}

#[test]
fn shape_preview_and_commit_keep_right_edge_aligned_on_ragged_rows() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    state.set_lines_for_test(
        [11, 7, 0, 7, 11]
            .into_iter()
            .map(|width| (0..width).map(|_| blank_atom()).collect())
            .collect(),
    );
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
            .lines_for_test()
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
    state.set_lines_for_test(vec![(0..4).map(|_| blank_atom()).collect()]);
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
            .lines_for_test()
            .iter()
            .map(|line| contents(line))
            .collect::<Vec<_>>(),
        expected
    );
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
    assert!(
        state.lines_for_test()[0]
            .iter()
            .all(|atom| atom.contents == " ")
    );
}

#[test]
fn history_preparation_cancels_transients_without_closing_export_or_durable_tools() {
    let mut shape = state();
    shape.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    shape.toggle_shape_preview();
    assert!(shape.prepare_history_command());
    assert!(shape.shape_preview.is_none());

    let mut line = state();
    line.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    line.move_or_draw(Direction::Right, true);
    assert!(line.active_stroke.is_some());
    assert!(line.prepare_history_command());
    assert!(line.active_stroke.is_none());

    let mut lift = utility_state(&["abc"], UtilityKind::Push, Coord::default());
    lift.selection
        .select(Coord::default(), Coord { line: 0, column: 1 });
    let before_lift = lift.edit_snapshot();
    assert!(lift.begin_selected_move_lift());
    assert!(lift.move_lift(Direction::Right));
    assert!(lift.prepare_history_command());
    assert!(!lift.move_lift_active());
    assert_eq!(lift.edit_snapshot(), before_lift);

    let mut export = state();
    let durable = export.toolbar.durable_selections();
    export.apply_toolbar_action(ToolbarAction::ToggleExportMenu);
    assert!(export.toolbar.export_menu_open());
    assert!(export.toolbar.pending_shortcut().is_some());
    assert!(export.prepare_history_command());
    assert!(export.toolbar.export_menu_open());
    assert!(export.toolbar.pending_shortcut().is_none());
    assert_eq!(export.toolbar.durable_selections(), durable);
    assert!(!export.prepare_history_command());
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
    assert_eq!(line_contents(&right), vec!["ab", "cd"]);

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
    assert_eq!(line_contents(&left), vec!["abd", "xy"]);

    let mut right = utility_state(
        &["abcd", "xy", ""],
        UtilityKind::Pull,
        Coord { line: 0, column: 1 },
    );
    assert!(right.apply_utility(Direction::Right));
    assert_eq!(line_contents(&right), vec![" abd", " xy"]);
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
    let mut lines = state.lines_for_test();
    for (index, atom) in lines[0].iter_mut().enumerate() {
        atom.face.fg = format!("#{index}{index}{index}{index}{index}{index}");
    }
    state.set_lines_for_test(lines);
    state
        .selection
        .select(Coord { line: 0, column: 0 }, Coord { line: 0, column: 3 });
    state.active_stroke = Some(ActiveStroke {
        end: Coord { line: 0, column: 1 },
        end_base_glyph: "─".into(),
        moving_ending: LineEnding::None,
        incoming_connection: Direction::Left,
        end_was_existing_line: false,
    });
    state.extend_line_markers_for_test([
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
    assert_eq!(state.lines_for_test()[0][0].face.fg, "#000000");
    assert_eq!(state.lines_for_test()[0][1].face.fg, "#222222");
    assert_eq!(state.lines_for_test()[0][2].face.fg, "#333333");
    assert_eq!(state.selection.active().column, 2);
    assert!(state.active_stroke.is_none());
    assert_eq!(state.line_markers_for_test().len(), 1);
    assert_eq!(state.line_markers_for_test()[0].coord.column, 2);
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
        incoming_connection: Direction::Up,
        end_was_existing_line: false,
    });
    state.extend_line_markers_for_test([
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
    assert_eq!(state.line_markers_for_test().len(), 1);
    assert_eq!(state.line_markers_for_test()[0].coord.line, 2);
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

    let mut unchanged = utility_state(&["", "x"], UtilityKind::Pull, Coord { line: 1, column: 0 });
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
        incoming_connection: Direction::Left,
        end_was_existing_line: false,
    });
    state.push_line_marker_for_test(PlacedLineMarker {
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
    assert_eq!(state.line_markers_for_test()[0].coord.column, 3);
    let preview = state.shape_preview.unwrap();
    assert_eq!((preview.anchor.column, preview.end.column), (2, 3));
}

#[test]
fn move_lift_previews_without_mutation_then_composes_edited_cells() {
    let mut state = utility_state(&["abXX", "cdYY"], UtilityKind::Push, Coord::default());
    let configured_face = state.theme.tooltip.clone();
    state.set_cell_face_for_test(Coord::default(), configured_face.clone());
    state
        .selection
        .select(Coord::default(), Coord { line: 1, column: 1 });
    let before = state.edit_snapshot();

    assert!(state.begin_selected_move_lift());
    assert!(state.move_lift(Direction::Right));
    assert!(state.move_lift(Direction::Right));
    assert_eq!(state.edit_snapshot(), before);
    let preview = state
        .lines_with_shape_preview()
        .expect("lifted selection has a composited preview");
    assert_eq!(contents(&preview[0]), "  ab");
    assert_eq!(contents(&preview[1]), "  cd");
    assert_eq!(preview[0][2].face, configured_face);

    assert!(state.confirm_move_lift());
    assert!(!state.move_lift_active());
    assert_eq!(line_contents(&state), vec!["  ab", "  cd"]);
    assert_eq!(state.lines_for_test()[0][2].face, state.theme.tooltip);
    assert_eq!(
        state.selection_bounds(),
        SelectionBounds {
            left: 2,
            right: 3,
            top: 0,
            bottom: 1,
        }
    );
}

#[test]
fn clone_move_lift_clones_once_per_shift_press_and_can_clone_after_moving() {
    let mut initial = utility_state(&["A"], UtilityKind::Push, Coord::default());
    initial
        .selection
        .select(Coord::default(), Coord { line: 0, column: 1 });
    let before = initial.edit_snapshot();

    assert!(initial.begin_selected_move_lift());
    assert!(initial.clone_move_lift(Direction::Right, 1));
    assert_eq!(initial.edit_snapshot(), before);
    assert_eq!(
        contents(&initial.lines_with_shape_preview().unwrap()[0]),
        "AA"
    );

    assert!(initial.clone_move_lift(Direction::Right, 1));
    assert_eq!(
        contents(&initial.lines_with_shape_preview().unwrap()[0]),
        "A A"
    );

    assert!(initial.clone_move_lift(Direction::Left, 2));
    assert_eq!(
        contents(&initial.lines_with_shape_preview().unwrap()[0]),
        "AAA"
    );
    assert!(initial.confirm_move_lift());
    assert_eq!(line_contents(&initial), vec!["AAA"]);

    let mut delayed = utility_state(&["A"], UtilityKind::Push, Coord::default());
    delayed
        .selection
        .select(Coord::default(), Coord { line: 0, column: 1 });
    assert!(delayed.begin_selected_move_lift());
    assert!(delayed.move_lift(Direction::Right));
    assert!(delayed.clone_move_lift(Direction::Right, 1));
    assert!(delayed.confirm_move_lift());
    assert_eq!(line_contents(&delayed), vec![" AA"]);
}

#[test]
fn clone_move_lift_preserves_faces_and_line_markers_for_every_copy() {
    let mut state = utility_state(&["A"], UtilityKind::Push, Coord::default());
    let configured_face = state.theme.tooltip.clone();
    state.set_cell_face_for_test(Coord::default(), configured_face.clone());
    state.push_line_marker_for_test(PlacedLineMarker {
        coord: Coord::default(),
        ending: LineEnding::Fixed('◆'),
        base_glyph: "A".into(),
    });
    state
        .selection
        .select(Coord::default(), Coord { line: 0, column: 1 });

    assert!(state.begin_selected_move_lift());
    assert!(state.clone_move_lift(Direction::Right, 1));
    assert!(state.clone_move_lift(Direction::Right, 2));
    assert!(state.confirm_move_lift());

    assert_eq!(line_contents(&state), vec!["AAA"]);
    assert!(
        state.lines_for_test()[0]
            .iter()
            .all(|atom| atom.face == configured_face)
    );
    assert_eq!(
        state
            .line_markers_for_test()
            .iter()
            .map(|marker| marker.coord.column)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );

    let mut overlap = utility_state(&["AB"], UtilityKind::Push, Coord::default());
    overlap.extend_line_markers_for_test([
        PlacedLineMarker {
            coord: Coord::default(),
            ending: LineEnding::Fixed('◆'),
            base_glyph: "A".into(),
        },
        PlacedLineMarker {
            coord: Coord { line: 0, column: 1 },
            ending: LineEnding::Fixed('◆'),
            base_glyph: "B".into(),
        },
    ]);
    overlap
        .selection
        .select(Coord::default(), Coord { line: 0, column: 1 });
    assert!(overlap.begin_selected_move_lift());
    assert!(overlap.clone_move_lift(Direction::Right, 1));
    assert!(overlap.confirm_move_lift());
    assert_eq!(line_contents(&overlap), vec!["AAB"]);
    assert_eq!(overlap.line_markers_for_test().len(), 3);
    assert_eq!(
        overlap
            .line_markers_for_test()
            .iter()
            .map(|marker| marker.coord.column)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn move_lift_treats_unedited_cells_as_transparent() {
    let mut state = utility_state(&["A C", "x─z"], UtilityKind::Push, Coord::default());
    state
        .selection
        .select(Coord::default(), Coord { line: 0, column: 2 });
    state.push_line_marker_for_test(PlacedLineMarker {
        coord: Coord { line: 1, column: 1 },
        ending: LineEnding::Fixed('◆'),
        base_glyph: "─".into(),
    });

    assert!(state.begin_selected_move_lift());
    assert!(state.move_lift(Direction::Down));
    let preview = state
        .lines_with_shape_preview()
        .expect("lifted selection has a composited preview");
    assert_eq!(contents(&preview[0]), "");
    assert_eq!(contents(&preview[1]), "A─C");

    assert!(state.confirm_move_lift());
    assert_eq!(line_contents(&state), vec!["", "A─C"]);
    assert_eq!(state.line_markers_for_test().len(), 1);
    assert_eq!(
        state.line_markers_for_test()[0].coord,
        Coord { line: 1, column: 1 }
    );
}

#[test]
fn move_lift_cancel_restores_exact_cursor_selection_and_document() {
    let mut state = utility_state(&["abc"], UtilityKind::Push, Coord { line: 0, column: 2 });
    state
        .selection
        .select(Coord { line: 0, column: 2 }, Coord { line: 0, column: 1 });
    let before = state.edit_snapshot();

    assert!(state.begin_selected_move_lift());
    assert!(state.move_lift(Direction::Down));
    assert!(state.move_lift(Direction::Right));
    assert!(state.cancel_move_lift());

    assert_eq!(state.edit_snapshot(), before);
    assert!(state.lines_with_shape_preview().is_none());
}

#[test]
fn move_lift_extends_past_the_top_left_canvas_origin() {
    let mut left = utility_state(&["  AB"], UtilityKind::Push, Coord { line: 0, column: 3 });
    left.selection
        .select(Coord { line: 0, column: 2 }, Coord { line: 0, column: 3 });
    let before = left.edit_snapshot();
    assert!(left.begin_selected_move_lift());
    for _ in 0..5 {
        assert!(left.move_lift(Direction::Left));
    }
    assert_eq!(left.move_lift_bounds().unwrap().left, 0);
    assert_eq!(left.edit_snapshot(), before);
    assert!(left.cancel_move_lift());
    assert_eq!(left.edit_snapshot(), before);
    assert_eq!(left.take_pending_prepend(), (-3, 0));

    assert!(left.begin_selected_move_lift());
    for _ in 0..5 {
        assert!(left.move_lift(Direction::Left));
    }
    assert!(left.confirm_move_lift());
    assert_eq!(left.canvas_origin.column, 3);
    assert_eq!(left.selection_bounds().left, 0);
    assert_eq!(left.selected_text(), "AB");

    let mut up = utility_state(
        &["", "", "AB"],
        UtilityKind::Push,
        Coord { line: 2, column: 1 },
    );
    up.selection
        .select(Coord { line: 2, column: 0 }, Coord { line: 2, column: 1 });
    assert!(up.begin_selected_move_lift());
    for _ in 0..4 {
        assert!(up.move_lift(Direction::Up));
    }
    assert!(up.confirm_move_lift());
    assert_eq!(up.canvas_origin.line, 2);
    assert_eq!(up.selection_bounds().top, 0);
    assert_eq!(up.selected_text(), "AB");

    let mut stationary = utility_state(&["AB"], UtilityKind::Push, Coord { line: 0, column: 1 });
    stationary
        .selection
        .select(Coord::default(), Coord { line: 0, column: 1 });
    let before = stationary.edit_snapshot();
    assert!(stationary.begin_selected_move_lift());
    assert!(stationary.move_lift(Direction::Left));
    assert!(stationary.move_lift(Direction::Right));
    assert!(!stationary.confirm_move_lift());
    assert_eq!(stationary.edit_snapshot(), before);
    assert_eq!(stationary.take_pending_prepend(), (-1, 0));
}

#[test]
fn move_lift_handles_overlapping_destinations() {
    let mut overlap = utility_state(&["abcd"], UtilityKind::Push, Coord { line: 0, column: 2 });
    overlap
        .selection
        .select(Coord { line: 0, column: 1 }, Coord { line: 0, column: 2 });
    assert!(overlap.begin_selected_move_lift());
    assert!(overlap.move_lift(Direction::Right));
    assert!(overlap.confirm_move_lift());
    assert_eq!(line_contents(&overlap), vec!["a bc"]);
}

#[test]
fn confirming_a_stationary_move_lift_is_an_exact_document_no_op() {
    let mut state = utility_state(&["abc"], UtilityKind::Push, Coord { line: 0, column: 1 });
    state
        .selection
        .select(Coord { line: 0, column: 1 }, Coord { line: 0, column: 2 });
    let before = state.edit_snapshot();

    assert!(state.begin_selected_move_lift());
    assert!(!state.confirm_move_lift());

    assert_eq!(state.edit_snapshot(), before);
}
