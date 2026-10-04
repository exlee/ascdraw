use super::*;

fn cells(rows: &[&str]) -> Cells {
    let mut cells = Cells::default();
    for (line, row) in rows.iter().enumerate() {
        for (column, character) in row.chars().enumerate() {
            if character != ' ' {
                cells.insert(
                    Coord {
                        line: line as i16,
                        column: column as i16,
                    },
                    ObjectCell {
                        atom: character.to_string(),
                        face: Face::default(),
                        line: None,
                    },
                );
            }
        }
    }
    cells
}

fn text(cells: &Cells, width: i16, height: i16) -> Vec<String> {
    (0..height)
        .map(|line| {
            (0..width)
                .map(|column| {
                    cells
                        .get(Coord { line, column })
                        .map_or(" ".to_owned(), |cell| cell.atom.clone())
                })
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn definition(rows: &[&str]) -> ObjectDefinition {
    ObjectDefinition {
        id: ObjectId(0),
        width: rows
            .iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(1) as i16,
        height: rows.len() as i16,
        cells: cells(rows),
        anchors: Vec::new(),
        next_anchor: 0,
    }
}

fn at(line: i16, column: i16) -> Coord {
    Coord { line, column }
}

#[test]
fn stretching_lengthens_connected_lines_in_both_directions() {
    let box_definition = definition(&["╭──╮", "│  │", "╰──╯"]);
    let stretched = render_definition(&box_definition, 7, 5);
    assert_eq!(
        text(&stretched, 7, 5),
        ["╭─────╮", "│     │", "│     │", "│     │", "╰─────╯"]
    );
}

#[test]
fn stretching_keeps_double_line_style() {
    let rule = definition(&["╔═╗"]);
    assert_eq!(text(&render_definition(&rule, 6, 1), 6, 1), ["╔════╗"]);
}

#[test]
fn unanchored_group_keeps_its_shape_and_stays_centered() {
    let label = definition(&["│ ab │"]);
    assert_eq!(
        text(&render_definition(&label, 10, 1), 10, 1),
        ["│   ab   │"]
    );
}

#[test]
fn implicit_groups_join_touching_cells_and_letters_across_one_space() {
    let groups = implicit_groups(&cells(&["ab %%", "c", "", "x y", "", "a  b"]));
    let mut groups = groups
        .into_iter()
        .map(|mut group| {
            group.sort_by_key(|coord| (coord.line, coord.column));
            group
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|group| (group[0].line, group[0].column));
    assert_eq!(
        groups,
        [
            vec![at(0, 0), at(0, 1), at(1, 0)],
            vec![at(0, 3), at(0, 4)],
            vec![at(3, 0), at(3, 2)],
            vec![at(5, 0)],
            vec![at(5, 3)],
        ]
    );
}

#[test]
fn line_glyphs_never_join_implicit_groups() {
    let groups = implicit_groups(&cells(&["│ab│"]));
    assert_eq!(groups, [vec![at(0, 1), at(0, 2)]]);
}

#[test]
fn west_anchor_keeps_its_column_while_its_line_scales() {
    let mut anchored = definition(&["│    │", "│X   │", "│    │"]);
    assert_eq!(anchored.toggle_anchor(at(1, 1), AnchorKind::W), Ok(true));
    let stretched = render_definition(&anchored, 12, 5);
    assert_eq!(stretched.get(at(2, 1)).unwrap().atom, "X");
}

#[test]
fn east_anchor_keeps_its_distance_from_the_right_edge() {
    let mut anchored = definition(&["│   X│"]);
    anchored.toggle_anchor(at(0, 4), AnchorKind::E).unwrap();
    let stretched = render_definition(&anchored, 10, 1);
    assert_eq!(text(&stretched, 10, 1), [format!("│{}X│", " ".repeat(7))]);
}

#[test]
fn north_west_anchor_does_not_move_when_stretched() {
    let mut anchored = definition(&["      ", "  X   ", "      "]);
    anchored.toggle_anchor(at(1, 2), AnchorKind::NW).unwrap();
    let resolved = resolve_anchors(&anchored, 20, 9);
    let anchor = anchored.anchors[0].id;
    assert_eq!(resolved[&anchor], at(1, 2));
    assert_eq!(
        render_definition(&anchored, 20, 9)
            .get(at(1, 2))
            .unwrap()
            .atom,
        "X"
    );
}

#[test]
fn west_anchor_attached_to_a_north_anchor_follows_it_horizontally() {
    let mut anchored = definition(&["          ", "    N  W  ", "          "]);
    anchored.toggle_anchor(at(1, 4), AnchorKind::N).unwrap();
    anchored.toggle_anchor(at(1, 7), AnchorKind::W).unwrap();
    let north = anchored.anchors[0].id;
    let west = anchored.anchors[1].id;
    assert_eq!(
        anchored.anchors[1].horizontal,
        Some(AnchorTarget::Anchor(north))
    );
    let resolved = resolve_anchors(&anchored, 19, 3);
    assert_eq!(resolved[&north].line, 1);
    assert_ne!(resolved[&north].column, 4);
    assert_eq!(resolved[&west].column - resolved[&north].column, 3);
}

#[test]
fn extension_keeps_a_diagonal_offset_from_the_anchor_it_extends() {
    let mut anchored = definition(&["          ", "          ", "          ", "    X     "]);
    anchored.toggle_anchor(at(1, 2), AnchorKind::W).unwrap();
    assert_eq!(
        anchored.toggle_anchor(at(3, 4), AnchorKind::Extension),
        Ok(true)
    );
    let base = anchored.anchors[0].id;
    let extension = anchored.anchors[1].id;
    let resolved = resolve_anchors(&anchored, 30, 12);
    assert_eq!(resolved[&base].column, 2);
    assert_eq!(resolved[&extension].line - resolved[&base].line, 2);
    assert_eq!(resolved[&extension].column - resolved[&base].column, 2);
}

#[test]
fn extension_needs_an_existing_anchor() {
    let mut empty = definition(&["   "]);
    assert_eq!(
        empty.toggle_anchor(at(0, 1), AnchorKind::Extension),
        Err(AnchorError::NothingToExtend)
    );
    assert_eq!(
        empty.toggle_anchor(at(0, 5), AnchorKind::W),
        Err(AnchorError::OutsideDefinition)
    );
}

#[test]
fn toggling_the_same_anchor_removes_it_and_its_extensions() {
    let mut anchored = definition(&["     ", "     "]);
    anchored.toggle_anchor(at(0, 1), AnchorKind::N).unwrap();
    anchored
        .toggle_anchor(at(1, 3), AnchorKind::Extension)
        .unwrap();
    assert_eq!(anchored.toggle_anchor(at(0, 1), AnchorKind::N), Ok(false));
    assert!(anchored.anchors.is_empty());
}

#[test]
fn growing_left_shifts_cells_anchors_and_instance_origins() {
    let mut store = ObjectStore::default();
    let first = store.define(LayerId(0), at(5, 5), 2, 1, cells(&["ab"]));
    let object = store.instance(first).unwrap().object;
    let second = store.place(object, LayerId(0), at(10, 10));
    store
        .definition_mut(object)
        .unwrap()
        .toggle_anchor(at(0, 0), AnchorKind::W)
        .unwrap();
    let shift = store.grow_definition(
        object,
        SelectionBounds {
            left: -2,
            right: 1,
            top: 0,
            bottom: 1,
        },
    );
    assert_eq!(shift, (0, 2));
    let grown = store.definition(object).unwrap();
    assert_eq!((grown.width, grown.height), (4, 2));
    assert_eq!(grown.cells.get(at(0, 2)).unwrap().atom, "a");
    assert_eq!(grown.anchors[0].at, at(0, 2));
    assert_eq!(store.instance(first).unwrap().origin, at(5, 3));
    assert_eq!(store.instance(second).unwrap().origin, at(10, 8));
}

#[test]
fn instance_lookup_prefers_the_latest_instance_and_respects_stretch() {
    let mut store = ObjectStore::default();
    let first = store.define(LayerId(0), at(0, 0), 3, 3, Cells::default());
    let object = store.instance(first).unwrap().object;
    let second = store.place(object, LayerId(0), at(1, 1));
    assert_eq!(store.instance_at(LayerId(0), at(1, 1)), Some(second));
    assert_eq!(store.instance_at(LayerId(0), at(0, 0)), Some(first));
    assert_eq!(store.instance_at(LayerId(1), at(0, 0)), None);
    store.instance_mut(first).unwrap().stretch = (4, 0);
    assert_eq!(store.instance_at(LayerId(0), at(0, 6)), Some(first));
}

#[test]
fn store_round_trips_through_json_without_the_session() {
    let mut store = ObjectStore::default();
    let instance = store.define(LayerId(0), at(2, 3), 2, 1, cells(&["ab"]));
    store.session = Some(Session::Define(instance));
    let restored: ObjectStore =
        serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
    assert_eq!(restored.session, None);
    assert_eq!(restored.definitions, store.definitions);
    assert_eq!(restored.instances, store.instances);
    let mut restored = restored;
    let next = restored.place(ObjectId(0), LayerId(0), at(0, 0));
    assert_ne!(next, instance);
}

#[test]
fn corner_anchors_pin_lines_and_the_box_lengthens_between_them() {
    let mut framed = definition(&[
        "         ",
        " ╭─────╮ ",
        " │x    │ ",
        " ╰─────╯ ",
        "         ",
    ]);
    framed.toggle_anchor(at(1, 1), AnchorKind::NW).unwrap();
    framed.toggle_anchor(at(3, 7), AnchorKind::SE).unwrap();
    framed
        .toggle_anchor(at(2, 2), AnchorKind::Extension)
        .unwrap();
    let stretched = render_definition(&framed, 12, 7);
    assert_eq!(
        text(&stretched, 12, 7),
        [
            "",
            " ╭────────╮",
            " │x       │",
            " │        │",
            " │        │",
            " ╰────────╯",
            "",
        ]
    );
}

#[test]
fn growing_one_side_keeps_the_pinned_corner_in_place() {
    let mut framed = definition(&["        ", " ╭────╮ ", " ╰────╯ "]);
    framed.toggle_anchor(at(1, 1), AnchorKind::NW).unwrap();
    framed.toggle_anchor(at(2, 6), AnchorKind::SE).unwrap();
    let stretched = render_definition(&framed, 8, 5);
    assert_eq!(
        text(&stretched, 8, 5),
        ["", " ╭────╮", " │    │", " │    │", " ╰────╯"]
    );
}

#[test]
fn a_symbol_on_a_line_stays_joined_when_stretched() {
    for (row, expected) in [
        ("──=──", "────=────"),
        ("──╪──", "────╪────"),
        ("──═──", "────═────"),
        ("─═══─", "─═══════─"),
    ] {
        let rule = definition(&[row]);
        assert_eq!(
            text(&render_definition(&rule, 9, 1), 9, 1),
            [expected],
            "{row}"
        );
    }
}

#[test]
fn a_symbol_on_a_vertical_line_stays_joined() {
    let column = definition(&["│", "x", "│"]);
    assert_eq!(
        text(&render_definition(&column, 1, 7), 1, 7),
        ["│", "│", "│", "x", "│", "│", "│"]
    );
}

#[test]
fn shrinking_keeps_corners_where_line_cells_collapse() {
    let path = definition(&["◀──╮ ", "   │ ", "   ╰─▶"]);
    let shrunk = render_definition(&path, 4, 3);
    assert_eq!(text(&shrunk, 4, 3)[2].chars().nth(2), Some('╰'));
    let narrow = definition(&["╭──╮", "╰──╯"]);
    assert_eq!(text(&render_definition(&narrow, 2, 2), 2, 2), ["╭╮", "╰╯"]);
}

#[test]
fn cells_with_line_data_round_trip_and_old_duplicate_keys_still_load() {
    let mut saved = Cells::default();
    saved.insert(
        at(0, 0),
        ObjectCell {
            atom: "◀".to_owned(),
            face: Face::default(),
            line: Some(LineData {
                ending: crate::drawing::LineEnding::None,
                base_glyph: "╶".to_owned(),
            }),
        },
    );
    let json = serde_json::to_string(&saved).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value[0]["p"], serde_json::json!([0, 0]));
    assert_eq!(value[0]["v"], "◀");
    assert_eq!(serde_json::from_str::<Cells>(&json).unwrap(), saved);

    let old =
        r#"[{"line": 0, "column": 0, "atom": "◀", "line": {"ending": "None", "base_glyph": "╶"}}]"#;
    assert_eq!(serde_json::from_str::<Cells>(old).unwrap(), saved);
}
