use super::*;

#[test]
fn single_replace_consumes_one_grapheme_without_moving_the_cursor() {
    let mut state = state();
    state.insert("abc");
    state.move_to(Coord { line: 0, column: 1 });

    assert!(state.begin_single_replace());
    assert_eq!(state.cursor_mode, CursorMode::Replace);
    state.write_text("XY");

    assert_eq!(contents(&state.lines_for_test()[0]), "aXc");
    assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
    assert_eq!(state.cursor_mode, CursorMode::Stamp);
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
fn selection_extends_into_negative_coordinates_without_moving_anchor() {
    let mut state = state();
    state.move_to(Coord { line: 0, column: 0 });

    assert!(!state.extend_selection(Direction::Up));
    assert_eq!(state.selection.anchor(), Coord { line: 0, column: 0 });
    assert_eq!(
        state.selection.active(),
        Coord {
            line: -1,
            column: 0
        }
    );
    assert_eq!(
        state.grid.cursor_pos,
        Coord {
            line: -1,
            column: 0
        }
    );

    assert!(!state.extend_selection(Direction::Left));
    assert_eq!(state.selection.anchor(), Coord { line: 0, column: 0 });
    assert_eq!(
        state.selection.active(),
        Coord {
            line: -1,
            column: -1
        }
    );
}

#[test]
fn moving_to_a_drag_start_collapses_a_previous_selection() {
    let mut state = state();
    state.insert("abcdef");
    state
        .selection
        .select(Coord { line: 0, column: 5 }, Coord { line: 0, column: 1 });

    state.move_to(Coord { line: 0, column: 3 });

    assert!(state.selection.is_collapsed());
    assert_eq!(state.selection.anchor(), Coord { line: 0, column: 3 });
    assert_eq!(state.selection.active(), Coord { line: 0, column: 3 });
}

#[test]
fn clear_is_literal_and_does_not_cap_neighboring_line_cells() {
    let mut state = state();
    state.insert("│\n│\n│");
    state.move_to(Coord { line: 1, column: 0 });

    state.clear_selection();

    assert_eq!(contents(&state.lines_for_test()[0]), "│");
    assert_eq!(contents(&state.lines_for_test()[1]), "");
    assert_eq!(contents(&state.lines_for_test()[2]), "│");
}

#[test]
fn rectangular_clear_leaves_every_perimeter_atom_and_face_unchanged() {
    let mut state = state();
    let perimeter_face = state.theme.selection.clone();
    let center_face = state.theme.cursor_drawing.clone();
    assert!(state.paste_styled_rectangle_at_cursor(&TextRectangle::new(
        ["┌┬┐", "├┼┤", "└┴┘"]
            .into_iter()
            .map(|row| {
                row.chars()
                    .map(|contents| StyledAtom {
                        face: perimeter_face.clone(),
                        contents: contents.to_string(),
                    })
                    .collect()
            })
            .collect(),
    )));
    state.set_cell_face_for_test(Coord { line: 1, column: 1 }, center_face);
    let before = state.lines_for_test().clone();
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
        let line = usize::try_from(coord.line).unwrap();
        let column = usize::try_from(coord.column).unwrap();
        assert_eq!(state.lines_for_test()[line][column], before[line][column]);
    }
    assert_eq!(contents(&state.lines_for_test()[1]), "├ ┤");
}

#[test]
fn clear_removes_only_markers_whose_cells_are_selected() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 11,
    });
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 1,
        option: 11,
    });
    assert!(state.move_or_draw(Direction::Right, true));
    assert!(state.move_or_draw(Direction::Right, true));
    state.end_stroke();
    let outside = state.line_markers_for_test()[1].clone();
    state.move_to(Coord::default());

    state.clear_selection();

    assert_eq!(contents(&state.lines_for_test()[0]), " ─◆");
    assert_eq!(state.line_markers_for_test(), vec![outside]);
}

#[test]
fn single_replacement_preserves_neighboring_line_segments() {
    let mut state = state();
    state.insert("╷\n│\n╵");
    state.move_to(Coord { line: 1, column: 0 });

    assert!(state.begin_single_replace());
    state.write_text("x");

    assert_eq!(contents(&state.lines_for_test()[0]), "╷");
    assert_eq!(contents(&state.lines_for_test()[1]), "x");
    assert_eq!(contents(&state.lines_for_test()[2]), "╵");
}

#[test]
fn paste_rectangular_overwrite_uses_selection_origin_and_selects_result() {
    let mut state = state();
    let outside = Face {
        fg: "#123456".to_string(),
        ..Face::default()
    };
    assert!(state.paste_styled_rectangle_at_cursor(&TextRectangle::new(vec![
        vec![
            StyledAtom {
                face: outside.clone(),
                contents: "L".into(),
            },
            StyledAtom {
                face: outside.clone(),
                contents: "a".into(),
            },
            StyledAtom {
                face: outside.clone(),
                contents: "b".into(),
            },
            StyledAtom {
                face: outside.clone(),
                contents: "R".into(),
            },
        ],
        vec![
            StyledAtom {
                face: outside.clone(),
                contents: "l".into(),
            },
            StyledAtom {
                face: outside.clone(),
                contents: "c".into(),
            },
            StyledAtom {
                face: outside.clone(),
                contents: "d".into(),
            },
            StyledAtom {
                face: outside.clone(),
                contents: "r".into(),
            },
        ],
    ])));
    state.move_to(Coord { line: 1, column: 2 });
    state.extend_selection(Direction::Left);
    state.extend_selection(Direction::Up);

    assert!(state.paste_text_rectangle("x\nYZ"));

    assert_eq!(contents(&state.lines_for_test()[0]), "Lx R");
    assert_eq!(contents(&state.lines_for_test()[1]), "lYZr");
    assert_eq!(state.lines_for_test()[0][0].face, outside);
    assert_eq!(state.lines_for_test()[0][1].face, Face::default());
    assert_eq!(state.selection.anchor(), Coord { line: 0, column: 1 });
    assert_eq!(state.selection.active(), Coord { line: 1, column: 2 });
    assert_eq!(state.grid.cursor_pos, Coord { line: 1, column: 2 });
    assert_eq!(state.selected_text(), "x \nYZ");
}

#[test]
fn single_cell_paste_fills_the_current_selection() {
    let mut state = state();
    state.insert("abcd\nefgh");
    state.move_to(Coord { line: 1, column: 2 });
    state.extend_selection(Direction::Left);
    state.extend_selection(Direction::Up);

    assert!(state.paste_text("x"));

    assert_eq!(contents(&state.lines_for_test()[0]), "axxd");
    assert_eq!(contents(&state.lines_for_test()[1]), "exxh");
    assert_eq!(state.selection_bounds().width(), 2);
    assert_eq!(state.selection_bounds().height(), 2);
    assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
}

#[test]
fn paste_rejects_wide_source_graphemes_transactionally() {
    let mut state = state();
    state.move_to(Coord { line: 2, column: 3 });
    let before = state.edit_snapshot();

    assert!(!state.paste_text_rectangle("😀\r\nq"));

    assert_eq!(state.edit_snapshot(), before);
    assert!(state.transient_tip().is_some());
}

#[test]
fn styled_toolbar_paste_uses_cursor_origin_active_layer_dimensions_and_one_undo() {
    use crate::selection::{CanvasRegion, region_atoms};
    use crate::toolbar_stamp::styled_toolbar_snapshot;

    let mut editor = state();
    editor.insert("base");
    let base = editor.active_layer_id();
    assert!(editor.add_layer_above(base));
    let upper = editor.active_layer_id();
    let base_before = editor.layer_views()[0].lines.to_vec();
    editor.move_to(Coord { line: 2, column: 3 });
    let origin = editor.grid.cursor_pos;
    let signed_origin = editor.cursor_coordinates();
    let rectangle = styled_toolbar_snapshot(&editor, 52).unwrap();
    let previous = editor.edit_snapshot();

    assert!(editor.paste_styled_rectangle_at_cursor(&rectangle));
    assert_eq!(
        editor.navigation_target(Direction::Right, 1),
        Some(Coord {
            line: origin.line,
            column: origin.column + 1,
        })
    );
    assert_eq!(editor.grid.cursor_pos, origin);
    assert_eq!(editor.cursor_coordinates(), signed_origin);
    assert!(editor.selection.is_collapsed());
    assert_eq!(editor.active_layer_id(), upper);
    assert_eq!(editor.layer_views()[0].lines, base_before);
    let pasted = region_atoms(
        &editor.lines_for_test(),
        CanvasRegion {
            left: origin.column as i64,
            top: origin.line as i64,
            width: rectangle.width,
            height: rectangle.rows.len(),
        },
    );
    assert_eq!(pasted, rectangle.rows);
    assert!(pasted.iter().any(|row| {
        row.iter()
            .any(|atom| atom.contents.bytes().all(|byte| byte == b' ') && atom.face.bg != "default")
    }));

    assert_ne!(editor.edit_snapshot(), previous);
}

#[test]
fn navigation_targets_cross_row_and_column_zero_without_canvas_mutation() {
    let editor = state();

    assert_eq!(
        editor.navigation_target(Direction::Left, 1),
        Some(Coord {
            line: 0,
            column: -1,
        })
    );
    assert_eq!(
        editor.navigation_target(Direction::Up, 2),
        Some(Coord {
            line: -2,
            column: 0,
        })
    );
    assert!(editor.content_cells().is_empty());
}

#[test]
fn plain_toolbar_hotspot_click_is_an_exact_editor_no_op() {
    use crate::toolbar_stamp::{HotspotClick, hotspot_click, styled_toolbar_snapshot};

    let mut editor = state();
    editor.insert("unchanged");
    let before = editor.edit_snapshot();
    let click = hotspot_click(Some(48), ModifiersState::empty());
    if let HotspotClick::Paste { box_width } = click {
        let rectangle = styled_toolbar_snapshot(&editor, box_width).unwrap();
        editor.paste_styled_rectangle_at_cursor(&rectangle);
    }

    assert_eq!(click, HotspotClick::Consume);
    assert_eq!(editor.edit_snapshot(), before);
}

#[test]
fn single_replace_fills_the_range_and_restores_mode_without_moving_active_corner() {
    let mut state = state();
    state.move_to(Coord { line: 0, column: 0 });
    state.extend_selection(Direction::Right);
    state.extend_selection(Direction::Right);
    state.extend_selection(Direction::Down);
    let active = state.grid.cursor_pos;

    assert!(state.begin_single_replace());
    state.write_text("zignored");

    assert_eq!(state.selected_text(), "zzz\nzzz");
    assert_eq!(state.grid.cursor_pos, active);
    assert_eq!(state.selection.active(), active);
    assert_eq!(state.cursor_mode, CursorMode::Stamp);
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
    assert!(state.handle_toolbar_shortcut(&Key::Character("1".into()), ModifiersState::empty(),));

    assert!(state.handle_toolbar_shortcut(&Key::Named(NamedKey::Escape), ModifiersState::empty(),));

    assert!(state.selection.is_collapsed());
    assert_eq!(state.toolbar.pending_shortcut(), None);
    assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
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
    assert!(state.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty(),));

    assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
}

#[test]
fn invalid_wide_text_is_transactional_and_sets_a_tip() {
    let mut state = state();
    state.insert("x");
    let before = state.edit_snapshot();
    state.insert("😀x");
    assert_eq!(state.edit_snapshot(), before);
    assert!(state.transient_tip().is_some());
}

#[test]
fn clicking_beyond_content_does_not_allocate_blank_cells() {
    let mut state = state();
    state.move_to(Coord { line: 2, column: 4 });
    assert_eq!(state.lines_for_test(), vec![Vec::new()]);
    assert!(
        state.canvas.layers()[state.canvas.active_index()]
            .rows()
            .is_empty()
    );
    assert_eq!(state.grid.cursor_pos, Coord { line: 2, column: 4 });
}

#[test]
fn move_draw_uses_grid_movement_without_wrapping() {
    let mut state = state();
    state.move_or_draw(Direction::Right, false);
    state.move_or_draw(Direction::Down, false);
    assert_eq!(state.grid.cursor_pos, Coord { line: 1, column: 1 });
    assert_eq!(state.lines_for_test(), vec![Vec::new()]);
}

#[test]
fn stored_content_respects_maximum_dimensions_without_bounding_navigation() {
    let mut state = state();
    state.move_to(Coord {
        line: i16::try_from(MAX_CANVAS_HEIGHT - 1).unwrap(),
        column: i16::try_from(MAX_CANVAS_WIDTH - 1).unwrap(),
    });
    state.insert("xy");

    assert_eq!(state.lines_for_test().len(), MAX_CANVAS_HEIGHT);
    assert_eq!(
        display_width(state.lines_for_test().last().unwrap()),
        MAX_CANVAS_WIDTH
    );
    assert_eq!(
        state.grid.cursor_pos,
        Coord {
            line: i16::try_from(MAX_CANVAS_HEIGHT - 1).unwrap(),
            column: i16::try_from(MAX_CANVAS_WIDTH).unwrap(),
        }
    );
    assert!(!state.move_cursor(Direction::Right));
    assert!(!state.move_cursor(Direction::Down));
    assert_eq!(
        state.grid.cursor_pos,
        Coord {
            line: 20_000,
            column: 20_001
        }
    );

    state.move_to(Coord::default());
    assert!(!state.move_cursor(Direction::Left));
    assert!(!state.move_cursor(Direction::Up));
    assert_eq!(
        state.grid.cursor_pos,
        Coord {
            line: -1,
            column: -1
        }
    );
    assert_eq!(state.lines_for_test().len(), MAX_CANVAS_HEIGHT);
    assert_eq!(
        display_width(state.lines_for_test().last().unwrap()),
        MAX_CANVAS_WIDTH
    );
}

#[test]
fn replacing_canvas_truncates_oversized_rows_and_columns() {
    let mut lines = vec![Vec::new(); MAX_CANVAS_HEIGHT + 1];
    lines[0].push(StyledAtom {
        face: Face::default(),
        contents: "x".repeat(MAX_CANVAS_WIDTH + 1),
    });
    let mut state = state();

    state.replace_canvas(lines);

    assert_eq!(state.lines_for_test(), vec![Vec::new()]);
    assert!(state.content_cells().is_empty());
}

#[test]
fn moving_up_at_zero_enters_implicit_space_without_shifting_content() {
    let mut state = state();
    state.insert("ab");
    state.move_to(Coord { line: 0, column: 1 });
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    state.toggle_shape_preview();

    assert!(!state.move_cursor(Direction::Up));

    assert_eq!(
        state.grid.cursor_pos,
        Coord {
            line: -1,
            column: 1
        }
    );
    assert_eq!(contents(&state.lines_for_test()[0]), "ab");
    let preview = state.shape_preview.unwrap();
    assert_eq!(preview.anchor.line, 0);
    assert_eq!(preview.end, state.grid.cursor_pos);
}

#[test]
fn moving_left_at_zero_enters_implicit_space_without_shifting_content() {
    let mut state = state();
    state.insert("a\nb");
    state.move_to(Coord::default());
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    state.toggle_shape_preview();

    assert!(!state.move_cursor(Direction::Left));

    assert_eq!(
        state.grid.cursor_pos,
        Coord {
            line: 0,
            column: -1
        }
    );
    assert_eq!(contents(&state.lines_for_test()[0]), "a");
    assert_eq!(contents(&state.lines_for_test()[1]), "b");
    let preview = state.shape_preview.unwrap();
    assert_eq!(preview.anchor.column, 0);
    assert_eq!(preview.end, state.grid.cursor_pos);
}

#[test]
fn drawing_connects_across_negative_top_and_left_cells() {
    let mut top = state();
    top.move_or_draw(Direction::Right, true);
    top.move_or_draw(Direction::Up, true);
    assert_eq!(
        top.cell_contents(Coord {
            line: -1,
            column: 1
        }),
        Some("╷")
    );
    assert_eq!(top.cell_contents(Coord { line: 0, column: 0 }), Some("╶"));
    assert_eq!(top.cell_contents(Coord { line: 0, column: 1 }), Some("╯"));

    let mut left = state();
    left.move_or_draw(Direction::Down, true);
    left.move_or_draw(Direction::Left, true);
    assert_eq!(left.cell_contents(Coord { line: 0, column: 0 }), Some("╷"));
    assert_eq!(
        left.cell_contents(Coord {
            line: 1,
            column: -1
        }),
        Some("╶")
    );
    assert_eq!(left.cell_contents(Coord { line: 1, column: 0 }), Some("╯"));
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
    let lines = state.lines_for_test().clone();

    state.clamp_cursor_to_content(Coord { line: 5, column: 5 });

    assert_eq!(state.grid.cursor_pos, Coord { line: 5, column: 5 });
    assert!(state.selection.is_collapsed());
    assert_eq!(state.selection.active(), state.grid.cursor_pos);
    assert_eq!(state.lines_for_test(), lines);
}

#[test]
fn draw_connects_straights_and_rounded_corners() {
    let mut state = state();
    state.move_or_draw(Direction::Right, true);
    state.move_or_draw(Direction::Right, true);
    state.move_or_draw(Direction::Down, true);

    assert_eq!(contents(&state.lines_for_test()[0]), "╶─╮");
    assert_eq!(contents(&state.lines_for_test()[1]), "  ╵");
}

#[test]
fn dashed_style_draws_repeated_half_segments() {
    let mut horizontal = state();
    horizontal.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    horizontal.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 2,
        option: 3,
    });
    for _ in 0..4 {
        horizontal.move_or_draw(Direction::Right, true);
    }
    assert_eq!(contents(&horizontal.lines_for_test()[0]), "╴╴╴╴╴");

    let mut vertical = state();
    vertical.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    vertical.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 2,
        option: 3,
    });
    for _ in 0..4 {
        vertical.move_or_draw(Direction::Down, true);
    }
    assert_eq!(contents(&vertical.lines_for_test()[0]), "╵");
    assert_eq!(contents(&vertical.lines_for_test()[1]), "╵");
    assert_eq!(contents(&vertical.lines_for_test()[2]), "╵");
    assert_eq!(contents(&vertical.lines_for_test()[3]), "╵");
    assert_eq!(contents(&vertical.lines_for_test()[4]), "╵");
}

#[test]
fn dashed_style_honors_sharp_corner_selection() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 2,
        option: 3,
    });
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 4,
        option: 1,
    });

    state.move_or_draw(Direction::Right, true);
    state.move_or_draw(Direction::Right, true);
    state.move_or_draw(Direction::Down, true);

    assert_eq!(contents(&state.lines_for_test()[0]), "╴╴┐");
    assert_eq!(contents(&state.lines_for_test()[1]), "  ╵");
}

#[test]
fn dashed_stroke_keeps_the_incoming_direction_when_turning_left_then_up() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 1,
    });
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 2,
        option: 3,
    });
    state.move_to(Coord { line: 0, column: 3 });

    for direction in [
        Direction::Down,
        Direction::Down,
        Direction::Left,
        Direction::Left,
        Direction::Left,
        Direction::Up,
        Direction::Up,
    ] {
        state.move_or_draw(direction, true);
    }

    assert_eq!(contents(&state.lines_for_test()[0]), "╵  △");
    assert_eq!(contents(&state.lines_for_test()[1]), "╵  ╵");
    assert_eq!(contents(&state.lines_for_test()[2]), "╰╴╴╯");
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

    assert_eq!(contents(&state.lines_for_test()[1]), "╶┼╴");
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

    assert_eq!(contents(&state.lines_for_test()[2]), " ╶┴╯");
}

#[test]
fn draw_preserves_non_line_text() {
    let mut state = state();
    state.insert("x");
    state.move_to(Coord { line: 0, column: 0 });
    state.move_or_draw(Direction::Right, true);

    assert_eq!(contents(&state.lines_for_test()[0]), "x╴");
}

#[test]
fn selected_line_endings_stay_at_the_stroke_endpoints() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
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

    assert_eq!(contents(&state.lines_for_test()[0]), "◀─╮");
    assert_eq!(contents(&state.lines_for_test()[1]), "  ▼");
}

#[test]
fn fixed_start_and_directional_end_survive_turning_and_marker_history_state() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
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

    assert_eq!(contents(&state.lines_for_test()[0]), "◆─╮");
    assert_eq!(contents(&state.lines_for_test()[1]), "  ↓");
    assert_eq!(state.line_markers_for_test().len(), 2);
    assert_eq!(
        state.line_markers_for_test()[0].ending,
        LineEnding::Fixed('◆')
    );
    assert_eq!(
        state.line_markers_for_test()[1].ending,
        LineEnding::Directional(crate::drawing::DirectionalEnding::Arrow)
    );

    let snapshot = state.edit_snapshot();
    state.clear_selection();
    state.restore_edit_snapshot(snapshot);
    assert_eq!(contents(&state.lines_for_test()[0]), "◆─╮");
    assert_eq!(contents(&state.lines_for_test()[1]), "  ↓");
    assert_eq!(
        state.line_markers_for_test()[1].ending,
        LineEnding::Directional(crate::drawing::DirectionalEnding::Arrow)
    );
}

#[test]
fn unadorned_endings_use_the_selected_double_line_style() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 2,
        option: 2,
    });

    state.move_or_draw(Direction::Right, true);
    state.move_or_draw(Direction::Right, true);

    assert_eq!(contents(&state.lines_for_test()[0]), "═══");
}

#[test]
fn drawing_from_an_existing_line_keeps_the_full_tee() {
    let mut state = state();
    state.insert("│");
    state.move_to(Coord { line: 0, column: 0 });
    select_toolbar_option(&mut state, "2", 1);

    state.move_or_draw(Direction::Right, true);

    assert_eq!(contents(&state.lines_for_test()[0]), "├╴");
}

#[test]
fn drawing_from_an_end_marker_moves_it_to_the_new_end() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    select_toolbar_option(&mut state, "3", 2);
    state.move_or_draw(Direction::Right, true);
    state.move_to(Coord { line: 0, column: 1 });

    state.move_or_draw(Direction::Down, true);

    assert_eq!(contents(&state.lines_for_test()[0]), "╶╮");
    assert_eq!(contents(&state.lines_for_test()[1]), " ▼");
}

#[test]
fn drawing_from_a_start_marker_moves_it_to_the_new_end() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    select_toolbar_option(&mut state, "2", 2);
    state.move_or_draw(Direction::Right, true);
    state.move_to(Coord { line: 0, column: 0 });

    state.move_or_draw(Direction::Down, true);

    assert_eq!(contents(&state.lines_for_test()[0]), "╭╴");
    assert_eq!(contents(&state.lines_for_test()[1]), "▼");
}

#[test]
fn clearing_a_cell_preserves_its_canvas_width() {
    let mut state = state();
    state.insert("abx");
    state.move_to(Coord { line: 0, column: 0 });

    state.clear_selection();

    assert_eq!(contents(&state.lines_for_test()[0]), " bx");
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
    assert_eq!(state.cursor_mode, CursorMode::Stamp);

    for key in ["1", "2"] {
        assert!(state.handle_toolbar_shortcut(
            &winit::keyboard::Key::Character(key.into()),
            winit::keyboard::ModifiersState::empty(),
        ));
    }
    assert_eq!(state.toolbar.main_mode(), MainMode::Line);
    assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
}
