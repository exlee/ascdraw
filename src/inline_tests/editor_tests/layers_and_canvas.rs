use super::*;

#[test]
fn editor_state_enum_tracks_modes_and_transient_interactions() {
    let mut editor = state();
    assert_eq!(editor.state(), EditorState::StampMode);

    assert!(editor.handle_toolbar_shortcut(&Key::Character("1".into()), ModifiersState::empty()));
    assert_eq!(editor.state(), EditorState::ToolbarMode);
    assert!(editor.cancel_current_state());
    assert_eq!(editor.state(), EditorState::StampMode);

    assert!(editor.apply_toolbar_action(ToolbarAction::ToggleExportMenu));
    assert_eq!(editor.state(), EditorState::ExportMode);
    assert!(editor.cancel_current_state());
    assert_eq!(editor.state(), EditorState::StampMode);

    assert!(editor.apply_toolbar_action(ToolbarAction::ToggleExportMenu));
    assert!(
        editor.apply_toolbar_action(ToolbarAction::SelectExportCategory(
            crate::toolbar::FILES_TOGGLE_CATEGORY,
        ))
    );
    assert_eq!(editor.state(), EditorState::ExportMode);
    assert!(editor.cancel_current_state());

    assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
    assert_eq!(editor.state(), EditorState::LineMode);
    assert!(!editor.start_or_advance_line_preview());
    assert_eq!(editor.state(), EditorState::LinePreviewMode);
    assert!(editor.cancel_current_state());

    editor.extend_selection(Direction::Right);
    assert_eq!(
        editor.state(),
        EditorState::SelectionMode(CursorMode::MoveDraw)
    );
    assert!(editor.begin_selected_move_lift());
    assert_eq!(editor.state(), EditorState::MoveMode);
    assert!(editor.cancel_current_state());
    assert!(editor.cancel_current_state());
    assert_eq!(editor.state(), EditorState::LineMode);

    assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
    assert_eq!(editor.state(), EditorState::ShapeMode);
    editor.toggle_shape_preview();
    assert_eq!(editor.state(), EditorState::ShapePreviewMode);
    assert!(editor.cancel_current_state());

    editor.toggle_text_entry();
    assert_eq!(editor.state(), EditorState::TextMode);
    assert!(editor.cancel_current_state());
    editor.cursor_mode = CursorMode::Insert;
    assert_eq!(editor.state(), EditorState::InsertMode);
    assert!(editor.cancel_current_state());
    editor.toggle_replace_mode();
    assert_eq!(editor.state(), EditorState::ReplaceMode);
    assert!(editor.cancel_current_state());

    assert!(editor.begin_single_replace());
    assert_eq!(editor.state(), EditorState::ReplaceOneMode);
    assert!(editor.cancel_current_state());

    assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
    assert_eq!(editor.state(), EditorState::UtilityMode);
}

#[test]
fn layer_panel_paths_and_disable_preserve_the_active_editor_mode() {
    let mut editor = state();
    assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
    assert!(editor.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
    assert!(editor.apply_toolbar_action(ToolbarAction::BeginLayersPath));
    assert!(editor.handle_toolbar_shortcut(&Key::Character("1".into()), ModifiersState::empty()));
    assert!(editor.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty()));
    assert_eq!(editor.toolbar.main_mode(), MainMode::Line);
    assert_eq!(editor.cursor_mode, CursorMode::MoveDraw);

    assert!(editor.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
    assert_eq!(editor.toolbar.main_mode(), MainMode::Line);
    assert_eq!(editor.cursor_mode, CursorMode::MoveDraw);
}

#[test]
fn color_panel_paths_and_disable_preserve_the_active_editor_mode() {
    let mut editor = state();
    assert!(editor.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
    assert!(editor.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode)));
    assert!(editor.apply_toolbar_action(ToolbarAction::BeginColorsPath));
    assert!(editor.handle_toolbar_shortcut(&Key::Character("1".into()), ModifiersState::empty()));
    assert!(editor.handle_toolbar_shortcut(&Key::Character("2".into()), ModifiersState::empty()));
    assert_eq!(editor.toolbar.active_color(), ColorId(1));
    assert_eq!(editor.toolbar.main_mode(), MainMode::Shapes);
    assert_eq!(editor.cursor_mode, CursorMode::Shapes);

    assert!(editor.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode)));
    assert_eq!(editor.toolbar.main_mode(), MainMode::Shapes);
    assert_eq!(editor.cursor_mode, CursorMode::Shapes);
}

#[test]
fn dark_mode_reverses_root_and_preserves_explicit_ui_accent_colors() {
    let source = ThemeConfig::default();
    let mut reversed = source.clone();
    reverse_theme_colors(&mut reversed);
    let mut state = Editor::new(&source, "ascdraw");

    assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::DarkMode)));
    assert_eq!(state.theme, reversed);
    assert_eq!(state.grid.default_face, reversed.default);
    assert_eq!(state.grid.cursor_face, reversed.cursor_block);
    assert_eq!(state.theme.selection, source.selection);
    assert_eq!(state.theme.selection_highlight, source.selection_highlight);
    assert_eq!(state.theme.color_selection, source.color_selection);
    assert_eq!(state.theme.jump_grid, source.jump_grid);
    assert_eq!(state.theme.cursor_drawing, source.cursor_drawing);
    assert_eq!(state.theme.tooltip, source.tooltip);

    let selection = crate::face_resolution::resolve_derived_face(
        &state.grid.default_face,
        &state.theme.selection,
        crate::face_resolution::Rgba::rgb(0, 0, 0),
        crate::face_resolution::Rgba::rgb(255, 255, 255),
    );
    let highlight = crate::face_resolution::resolve_derived_face(
        &state.grid.default_face,
        &state.theme.selection_highlight,
        crate::face_resolution::Rgba::rgb(0, 0, 0),
        crate::face_resolution::Rgba::rgb(255, 255, 255),
    );
    let tooltip = crate::face_resolution::resolve_derived_face(
        &state.grid.default_face,
        &state.theme.tooltip,
        crate::face_resolution::Rgba::rgb(0, 0, 0),
        crate::face_resolution::Rgba::rgb(255, 255, 255),
    );
    assert_eq!(selection.fg, crate::face_resolution::Rgba::rgb(0xff, 0, 0));
    assert_eq!(
        highlight.fg,
        crate::face_resolution::Rgba::rgb(0x00, 0x4d, 0xff)
    );
    assert_eq!(
        tooltip.fg,
        crate::face_resolution::Rgba::rgb(0x80, 0x80, 0x80)
    );
    assert_eq!(tooltip.bg, crate::face_resolution::Rgba::rgb(0, 0, 0));

    state.apply_theme(&source);
    assert_eq!(state.theme, reversed);

    assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::DarkMode)));
    assert_eq!(state.theme, source);
}

#[test]
fn cursor_coordinates_use_downward_positive_screen_coordinates() {
    let mut state = state();

    for (direction, expected) in [
        (Direction::Right, (1, 0)),
        (Direction::Down, (1, 1)),
        (Direction::Left, (0, 1)),
        (Direction::Up, (0, 0)),
        (Direction::Left, (-1, 0)),
        (Direction::Up, (-1, -1)),
    ] {
        state.move_cursor(direction);
        assert_eq!(state.cursor_coordinates(), expected);
    }
}

#[test]
fn cursor_coordinates_are_absolute_signed_canvas_coordinates() {
    let mut state = state();
    state.replace_canvas(vec![
        Vec::new(),
        vec![
            blank_atom(),
            StyledAtom {
                face: Face::default(),
                contents: "x".to_owned(),
            },
        ],
    ]);

    assert_eq!(state.cursor_coordinates(), (0, 0));
    state.move_to(Coord { line: 1, column: 1 });
    assert_eq!(state.cursor_coordinates(), (1, 1));

    state.move_cursor(Direction::Left);
    state.place_stamp();
    assert_eq!(state.cursor_coordinates(), (0, 1));
    state.clear_selection();
    assert_eq!(state.cursor_coordinates(), (0, 1));
}

#[test]
fn cursor_coordinates_are_centered_on_the_minimap_border_comma() {
    let mut state = state();
    state.move_to(Coord {
        line: 8,
        column: 10,
    });

    let border = crate::toolbar::toolbar_minimap_border_spans(
        80,
        crate::layout::MINIMAP_COLUMNS,
        state.cursor_coordinates(),
    );
    let contents = border[0].contents.as_str();
    assert!(contents.contains("(10,8)"));
    assert_eq!(contents.chars().nth((59 + 78) / 2), Some(','));
}

#[test]
fn layer_state_swaps_active_content_and_round_trips_in_edit_snapshots() {
    let mut state = state();
    state.insert("a");
    let base = state.active_layer_id();

    assert!(state.add_layer_above(base));
    let upper = state.active_layer_id();
    assert_ne!(upper, base);
    state.insert("b");

    let views = state.layer_views();
    assert_eq!(contents(&views[0].lines[0]), "a");
    assert_eq!(contents(&views[1].lines[0]), "b");

    let snapshot = state.edit_snapshot();
    assert!(state.select_layer(base));
    state.insert("c");
    assert_eq!(contents(&state.lines_for_test()[0]), "ac");

    state.restore_edit_snapshot(snapshot);
    assert_eq!(state.active_layer_id(), upper);
    assert_eq!(contents(&state.lines_for_test()[0]), "b");
    assert_eq!(contents(&state.layer_views()[0].lines[0]), "a");
}

#[test]
fn layer_limits_base_rules_reordering_deletion_and_symbol_reuse_are_stable() {
    let mut state = state();
    let base = state.active_layer_id();
    let mut created = Vec::new();
    for _ in 1..crate::model::MAX_LAYERS {
        let active = state.active_layer_id();
        assert!(state.add_layer_above(active));
        created.push(state.active_layer_id());
    }
    assert_eq!(state.layer_summaries().len(), crate::model::MAX_LAYERS);
    assert!(!state.add_layer_above(state.active_layer_id()));
    assert!(!state.move_layer_up(base));
    assert!(!state.move_layer_down(base));
    assert!(!state.delete_layer(base));

    assert!(state.toggle_layer_visibility(base));
    assert!(!state.layer_summaries()[0].visible);
    assert!(state.select_layer(base));
    state.insert("base");
    assert_eq!(contents(&state.lines_for_test()[0]), "base");

    let removed = created[2];
    let preserved_active = *created.last().unwrap();
    assert!(state.select_layer(preserved_active));
    assert!(state.delete_layer(removed));
    assert_eq!(state.active_layer_id(), preserved_active);
    assert!(state.add_layer_above(base));
    assert_eq!(state.active_layer_id(), removed);

    let active = state.active_layer_id();
    assert!(state.delete_layer(active));
    assert_eq!(state.active_layer_id(), base);
}

#[test]
fn minimap_projection_content_includes_hidden_layers() {
    let mut state = state();
    state.insert("x");
    let base = state.active_layer_id();
    assert!(state.add_layer_above(base));
    let upper = state.active_layer_id();
    state.insert("   y");
    assert!(state.toggle_layer_visibility(upper));

    assert_eq!(state.content_cells(), vec![Coord::default()]);
    assert_eq!(
        state.content_cells_including_hidden(),
        vec![Coord::default(), Coord { line: 0, column: 3 }]
    );
}

#[test]
fn layer_panel_arrows_move_toward_the_displayed_row_direction() {
    let mut state = state();
    let base = state.active_layer_id();
    assert!(state.add_layer_above(base));
    let middle = state.active_layer_id();
    assert!(state.add_layer_above(middle));
    let top = state.active_layer_id();
    assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));

    for key in ["8", "3", "4"] {
        assert!(
            state.handle_toolbar_shortcut(&Key::Character(key.into()), ModifiersState::empty())
        );
    }
    assert_eq!(
        state
            .layer_summaries()
            .iter()
            .map(|layer| layer.id)
            .collect::<Vec<_>>(),
        [base, top, middle]
    );

    for key in ["8", "2", "5"] {
        assert!(
            state.handle_toolbar_shortcut(&Key::Character(key.into()), ModifiersState::empty())
        );
    }
    assert_eq!(
        state
            .layer_summaries()
            .iter()
            .map(|layer| layer.id)
            .collect::<Vec<_>>(),
        [base, middle, top]
    );
}

#[test]
fn layer_merge_consumes_source_and_overlays_nonblank_atoms_and_markers() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 11,
    });
    state.move_to(Coord { line: 0, column: 1 });
    assert!(state.move_or_draw(Direction::Right, true));
    state.end_stroke();
    state.move_to(Coord { line: 0, column: 2 });
    state.clear_selection();
    state.move_to(Coord::default());
    assert!(state.paste_text_rectangle("A"));
    state.move_to(Coord { line: 0, column: 2 });
    assert!(state.paste_text_rectangle("z"));
    let base = state.active_layer_id();
    assert!(state.add_layer_above(base));
    let source = state.active_layer_id();
    state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    state.apply_toolbar_action(ToolbarAction::SelectColor(ColorId(3)));
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 16,
    });
    state.move_to(Coord { line: 0, column: 1 });
    assert!(state.move_or_draw(Direction::Right, true));
    state.end_stroke();
    state.move_to(Coord { line: 0, column: 2 });
    state.clear_selection();
    let source_face = state.lines_for_test()[0][1].face.clone();
    assert!(state.add_layer_above(source));
    let top = state.active_layer_id();
    state.insert("top");

    assert!(state.merge_layer_up(source));

    assert_eq!(state.active_layer_id(), base);
    assert_eq!(
        state
            .layer_summaries()
            .iter()
            .map(|layer| layer.id)
            .collect::<Vec<_>>(),
        [base, top]
    );
    assert_eq!(contents(&state.lines_for_test()[0]), "A●");
    assert_eq!(state.lines_for_test()[0][1].face, source_face);
    assert_eq!(state.line_markers_for_test().len(), 1);
    assert_eq!(
        state.line_markers_for_test()[0].ending,
        LineEnding::Fixed('●')
    );
    assert_eq!(contents(&state.layer_views()[1].lines[0]), "top");
    assert!(!state.merge_layer_up(base));
    assert!(!state.merge_layer_down(base));
    assert!(!state.merge_layer_down(top));

    let mut down = Editor::new(&ThemeConfig::default(), "ascdraw");
    down.insert("base");
    let base = down.active_layer_id();
    assert!(down.add_layer_above(base));
    let source = down.active_layer_id();
    down.insert(" M");
    assert!(down.add_layer_above(source));
    let target = down.active_layer_id();
    down.insert("T");

    assert!(down.merge_layer_down(source));
    assert_eq!(down.active_layer_id(), target);
    assert_eq!(contents(&down.lines_for_test()[0]), "TM");
    assert_eq!(down.layer_summaries().len(), 2);
}

#[test]
fn shifted_layer_shortcut_merges_and_consumes_the_selected_layer() {
    let mut state = Editor::new(&ThemeConfig::default(), "ascdraw");
    state.insert("base");
    let base = state.active_layer_id();
    assert!(state.add_layer_above(base));
    let source = state.active_layer_id();
    state.insert(" top");
    assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));

    for key in ["8", "2"] {
        assert!(
            state.handle_toolbar_shortcut(&Key::Character(key.into()), ModifiersState::empty(),)
        );
    }
    assert!(state.handle_toolbar_shortcut(&Key::Character("4".into()), ModifiersState::SHIFT,));

    assert_eq!(state.layer_summaries().len(), 1);
    assert_eq!(state.active_layer_id(), base);
    assert_ne!(state.active_layer_id(), source);
    assert_eq!(contents(&state.lines_for_test()[0]), "btop");
}

#[test]
fn clear_applies_to_every_layer_while_move_lift_only_applies_to_visible_layers() {
    let mut state = state();
    state.insert("A");
    let base = state.active_layer_id();
    assert!(state.add_layer_above(base));
    let upper = state.active_layer_id();
    state.insert(" B");
    assert!(state.toggle_layer_visibility(base));
    let summaries = state.layer_summaries();

    state.move_to(Coord::default());
    state.extend_selection(Direction::Right);
    state.clear_selection();
    assert_eq!(state.layer_summaries(), summaries);
    assert!(state.layer_views().iter().all(|layer| {
        layer
            .lines
            .iter()
            .flatten()
            .all(|atom| atom.contents.chars().all(char::is_whitespace))
    }));

    assert!(state.select_layer(base));
    state.insert("A");
    assert!(state.select_layer(upper));
    state.insert(" B");
    state.set_cell_face_for_test(Coord { line: 0, column: 1 }, state.theme.tooltip.clone());
    let upper_face = state.lines_for_test()[0][1].face.clone();
    assert_ne!(upper_face, Face::default());
    state.move_to(Coord::default());
    state.extend_selection(Direction::Right);
    assert!(state.begin_selected_move_lift());
    assert!(state.move_lift(Direction::Right));
    let sparse_preview = state
        .move_lift_render_canvas()
        .expect("one-cell move has a sparse preview");
    assert!(
        sparse_preview
            .layers()
            .iter()
            .all(|layer| layer.id != base || !layer.visible)
    );
    let upper_preview = sparse_preview
        .layers()
        .iter()
        .find(|layer| layer.id == upper)
        .expect("upper preview layer");
    assert_eq!(contents(&upper_preview.to_dense()[0]), "  B");
    assert!(state.confirm_move_lift());

    let views = state.layer_views();
    assert_eq!(contents(&views[0].lines[0]), "A");
    assert_eq!(contents(&views[1].lines[0]), "  B");
    assert_eq!(views[1].lines[0][2].face, upper_face);
    assert_eq!(state.layer_summaries(), summaries);
    assert!(state.select_layer(base));
    assert!(state.select_layer(upper));

    state.clear_canvas();
    assert_eq!(state.layer_summaries(), summaries);
    assert!(
        state
            .layer_views()
            .iter()
            .all(|layer| layer.lines.iter().flatten().all(|atom| {
                atom.contents.chars().all(char::is_whitespace) && atom.face == Face::default()
            }))
    );
}

#[test]
fn push_and_pull_apply_the_same_structural_change_to_every_layer() {
    let mut state = utility_state(&["ABC"], UtilityKind::Push, Coord::default());
    let base = state.active_layer_id();
    assert!(state.add_layer_above(base));
    let upper = state.active_layer_id();
    state.insert(" xyz");
    state.move_to(Coord::default());

    assert!(state.apply_utility(Direction::Right));
    let views = state.layer_views();
    assert_eq!(contents(&views[0].lines[0]), "A BC");
    assert_eq!(contents(&views[1].lines[0]), "  xyz");

    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 1,
    });
    assert!(state.apply_utility(Direction::Left));
    let views = state.layer_views();
    assert_eq!(contents(&views[0].lines[0]), "ABC");
    assert_eq!(contents(&views[1].lines[0]), " xyz");
    assert_eq!(state.active_layer_id(), upper);
}

#[test]
fn selected_color_applies_only_to_future_nonblank_writes_in_every_editing_path() {
    let color = crate::model::ColorId(9);
    let foreground = color.hex().unwrap();

    let mut text = state();
    text.insert("a");
    text.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    text.apply_toolbar_action(ToolbarAction::SelectColor(color));
    text.insert("b");
    assert_eq!(text.lines_for_test()[0][0].face, Face::default());
    assert_eq!(text.lines_for_test()[0][1].face.fg, foreground);

    text.move_home();
    assert!(text.begin_single_replace());
    text.write_text("r");
    assert_eq!(text.lines_for_test()[0][0].face.fg, foreground);
    text.toggle_replace_mode();
    text.write_text("z");
    assert_eq!(text.lines_for_test()[0][0].face.fg, foreground);

    let mut stamp = state();
    stamp.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    stamp.apply_toolbar_action(ToolbarAction::SelectColor(color));
    stamp.place_stamp();
    assert_eq!(stamp.lines_for_test()[0][0].face.fg, foreground);

    let mut line = state();
    line.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    line.apply_toolbar_action(ToolbarAction::SelectColor(color));
    line.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    assert!(line.move_or_draw(Direction::Right, true));
    assert!(
        line.lines_for_test()
            .iter()
            .flatten()
            .filter(|atom| !atom.contents.chars().all(char::is_whitespace))
            .all(|atom| atom.face.fg == foreground)
    );

    let mut shape = state();
    shape.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    shape.apply_toolbar_action(ToolbarAction::SelectColor(color));
    shape.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    shape.toggle_shape_preview();
    shape.move_cursor(Direction::Right);
    shape.move_cursor(Direction::Down);
    shape.confirm_shape();
    assert!(
        shape
            .lines_for_test()
            .iter()
            .flatten()
            .filter(|atom| !atom.contents.chars().all(char::is_whitespace))
            .all(|atom| atom.face.fg == foreground)
    );

    let mut paste = state();
    paste.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    paste.apply_toolbar_action(ToolbarAction::SelectColor(color));
    assert!(paste.paste_text_rectangle("p q"));
    assert_eq!(paste.lines_for_test()[0][0].face.fg, foreground);
    assert_eq!(paste.lines_for_test()[0][1].face, Face::default());
    assert_eq!(paste.lines_for_test()[0][2].face.fg, foreground);
}

#[test]
fn disabling_colors_stops_future_coloring_and_moves_preserve_existing_colors() {
    let color = crate::model::ColorId(10);
    let foreground = color.hex().unwrap();
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    state.apply_toolbar_action(ToolbarAction::SelectColor(color));
    state.insert("x");
    state.move_home();
    state.extend_selection(Direction::Right);
    assert!(state.begin_selected_move_lift());
    assert!(state.move_lift(Direction::Right));
    assert!(state.confirm_move_lift());
    assert_eq!(state.lines_for_test()[0][1].face.fg, foreground);

    state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    state.move_to(Coord { line: 0, column: 2 });
    state.insert("y");
    assert_eq!(state.lines_for_test()[0][2].face, Face::default());

    state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::DarkMode));
    assert_eq!(state.lines_for_test()[0][1].face.fg, foreground);
}

#[test]
fn line_connection_regeneration_uses_the_current_color() {
    let first = crate::model::ColorId(1);
    let second = crate::model::ColorId(6);
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.apply_toolbar_action(ToolbarAction::SelectColor(first));
    assert!(state.move_or_draw(Direction::Right, true));
    state.end_stroke();

    state.apply_toolbar_action(ToolbarAction::SelectColor(second));
    assert!(state.move_or_draw(Direction::Down, true));

    assert_eq!(state.lines_for_test()[0][0].face.fg, first.hex().unwrap());
    assert_eq!(state.lines_for_test()[0][1].face.fg, second.hex().unwrap());
    assert_eq!(
        state.lines_for_test()[1]
            .iter()
            .find(|atom| !atom.contents.chars().all(char::is_whitespace))
            .unwrap()
            .face
            .fg,
        second.hex().unwrap()
    );
    assert!(state.erase(Direction::Up));
    assert!(
        state
            .lines_for_test()
            .iter()
            .flatten()
            .filter(|atom| atom.contents.chars().all(char::is_whitespace))
            .all(|atom| atom.face == Face::default())
    );
}

#[test]
fn edit_snapshot_restores_document_cursor_and_line_continuation_only() {
    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.move_or_draw(Direction::Right, true);
    let snapshot = state.edit_snapshot();

    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
    state.toggle_text_entry();
    state.window_title = "current title".into();
    state.theme.selection.fg = "#123456".into();
    state.insert("changed");

    state.restore_edit_snapshot(snapshot.clone());

    assert_eq!(state.edit_snapshot(), snapshot);
    assert_eq!(state.cursor_mode, CursorMode::Text);
    assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
    assert_eq!(state.window_title, "current title");
    assert_eq!(state.theme.selection.fg, "#123456");
}

#[test]
fn restoring_durable_menu_state_syncs_mode_and_clears_transient_editor_state() {
    let mut selected = ToolbarState::default();
    selected.apply_action(ToolbarAction::SelectMain(MainMode::Utilities));
    selected.apply_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 2,
    });
    let selections = selected.durable_selections();

    let mut state = state();
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    state.toggle_shape_preview();
    state.restore_menu_selections(&selections);

    assert_eq!(state.toolbar.durable_selections(), selections);
    assert_eq!(state.toolbar.main_mode(), MainMode::Utilities);
    assert_eq!(state.toolbar.utility_kind(), UtilityKind::View);
    assert_eq!(state.cursor_mode, CursorMode::Utilities);
    assert!(state.shape_preview.is_none());
    assert!(!state.toolbar.export_menu_open());
    assert_eq!(state.toolbar.pending_shortcut(), None);

    let mut selection = Editor::new(&ThemeConfig::default(), "ascdraw");
    selection.extend_selection(Direction::Right);
    selection.restore_menu_selections(&selections);
    assert!(selection.selection.is_collapsed());

    let mut replace = Editor::new(&ThemeConfig::default(), "ascdraw");
    assert!(replace.begin_single_replace());
    replace.restore_menu_selections(&selections);
    assert!(!replace.single_replace_pending);
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
fn clear_canvas_resets_cells_faces_cursor_selection_and_line_markers() {
    let mut state = state();
    assert!(
        state.paste_styled_rectangle_at_cursor(
            &TextRectangle::from_rows(vec![vec![StyledAtom {
                face: Face {
                    fg: "#123456".into(),
                    bg: "#abcdef".into(),
                    underline: "#fedcba".into(),
                    attributes: vec!["reverse".into()],
                },
                contents: "x".into(),
            }]])
            .unwrap()
        )
    );
    state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
        submenu: 0,
        option: 2,
    });
    state.move_to(Coord { line: 1, column: 0 });
    assert!(state.move_or_draw(Direction::Right, true));
    state.end_stroke();
    state.grid.cursor_pos = Coord { line: 3, column: 4 };
    state
        .selection
        .select(Coord { line: 1, column: 2 }, Coord { line: 3, column: 4 });
    state.clear_canvas();

    assert_eq!(state.lines_for_test(), vec![Vec::new()]);
    assert!(state.content_cells().is_empty());
    assert_eq!(state.grid.cursor_pos, Coord { line: 3, column: 4 });
    assert!(state.selection.is_collapsed());
    assert_eq!(state.selection.active(), Coord { line: 3, column: 4 });
    assert!(state.line_markers_for_test().is_empty());
    assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
}

#[test]
fn clear_canvas_cancels_reachable_drawing_transients() {
    let mut line = state();
    line.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
    assert!(line.move_or_draw(Direction::Right, true));
    assert!(line.active_stroke.is_some());
    line.clear_canvas();
    assert!(line.active_stroke.is_none());

    let mut shape = state();
    shape.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
    shape.toggle_shape_preview();
    assert!(shape.shape_preview.is_some());
    shape.clear_canvas();
    assert!(shape.shape_preview.is_none());
}

#[test]
fn clear_canvas_preserves_a_far_cursor_and_later_inserts_there() {
    let mut state = state();
    state.insert("drawing\n\nx");
    let cursor = Coord {
        line: 5,
        column: 12,
    };
    state.move_to(cursor);

    state.clear_canvas();

    assert_eq!(state.grid.cursor_pos, cursor);
    assert_eq!(state.selection.active(), cursor);
    assert!(state.content_cells().is_empty());
    assert_eq!(state.lines_for_test(), vec![Vec::new()]);

    state.insert("x");
    let line = usize::try_from(cursor.line).unwrap();
    let column = usize::try_from(cursor.column).unwrap();
    assert_eq!(state.lines_for_test()[line][column].contents, "x");
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
    let mut state = Editor::new(&theme, "ascdraw");
    let cursor = Coord { line: 2, column: 3 };
    assert!(
        state.paste_styled_rectangle_at_cursor(
            &TextRectangle::from_rows(vec![
                vec![StyledAtom {
                    face: theme.selection.clone(),
                    contents: " ".into(),
                }],
                Vec::new(),
                (0..3)
                    .map(|_| StyledAtom {
                        face: theme.tooltip.clone(),
                        contents: " ".into(),
                    })
                    .collect(),
            ])
            .unwrap()
        )
    );
    state.grid.cursor_pos = cursor;

    state.clear_canvas();

    assert_eq!(state.grid.cursor_pos, cursor);
    assert!(state.content_cells().is_empty());
    assert!(
        state
            .lines_for_test()
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
    assert_eq!(contents(&state.lines_for_test()[0]), "  ◆");
    assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });

    assert!(state.erase(Direction::Right));
    assert_eq!(contents(&state.lines_for_test()[0]), "");
    assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 2 });
}

#[test]
fn erasing_a_traversed_line_edge_preserves_unrelated_connections() {
    let mut state = state();
    state.move_or_draw(Direction::Right, true);
    state.move_or_draw(Direction::Right, true);

    assert!(state.erase(Direction::Left));

    assert_eq!(contents(&state.lines_for_test()[0]), "╶╴");
    assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
}

#[test]
fn erasing_a_display_cell_removes_trailing_implicit_blanks() {
    let mut state = state();
    state.insert("ABC");
    state.move_to(Coord { line: 0, column: 2 });

    assert!(state.erase(Direction::Left));

    assert_eq!(contents(&state.lines_for_test()[0]), "A");
    assert_eq!(display_width(&state.lines_for_test()[0]), 1);
    assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 1 });
}

#[test]
fn blank_origin_erasing_moves_into_implicit_space_without_document_edit() {
    let mut state = state();

    assert!(!state.erase(Direction::Left));

    assert_eq!(
        state.grid.cursor_pos,
        Coord {
            line: 0,
            column: -1
        }
    );
    assert!(state.selection.is_collapsed());
}

#[test]
fn inserts_and_edits_multiple_lines() {
    let mut state = state();
    state.insert("ab\ncd");
    assert_eq!(state.lines_for_test().len(), 2);
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

    assert_eq!(contents(&state.lines_for_test()[0]), "aXY");
    assert_eq!(state.grid.cursor_pos, Coord { line: 0, column: 3 });
    state.toggle_replace_mode();
    assert_eq!(state.cursor_mode, CursorMode::Stamp);
}
