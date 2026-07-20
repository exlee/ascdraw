use super::*;
use crate::model::StyledAtom;
use crate::selection::CanvasRegion;

fn data(contents: &str, face: Face) -> CoordData {
    CoordData {
        atom: Rc::new(Atom::new(contents).expect("test data must be one cell")),
        face: Rc::new(face),
        raster_cache: RefCell::new(None),
        line: None,
    }
}

#[test]
fn validates_atoms_and_canonicalizes_default_whitespace() {
    let mut layer = LayerMap::new(LayerId(0), true);
    assert!(Atom::new("界").is_err());
    assert!(Atom::new("ab").is_err());
    layer
        .set_at(
            0,
            0,
            data(" ", Face::default()).atom.as_ref().clone(),
            &Face::default(),
        )
        .unwrap();
    assert!(layer.rows().is_empty());
}

#[test]
fn insertion_beyond_content_extends_logical_row_to_the_inserted_cell() {
    let mut layer = LayerMap::new(LayerId(0), true);
    let atom = data("x", Face::default()).atom.as_ref().clone();
    layer
        .insert_cells(2, 4, vec![(atom, Face::default())])
        .unwrap();

    assert_eq!(layer.row_width(2), 5);
    assert_eq!(layer.get(2, 4).unwrap().atom.contents(), "x");
}

#[test]
fn composition_ignores_whitespace_and_disabled_stack_uses_base_only() {
    let mut base = LayerMap::new(LayerId(0), true);
    base.set_data(2, -3, data("a", Face::default()));
    let mut top = LayerMap::new(LayerId(1), true);
    let styled = Face {
        bg: "selection".to_owned(),
        ..Face::default()
    };
    top.set_data(2, -3, data(" ", styled));
    let stack = LayerStack::new(vec![base.clone(), top], true).unwrap();
    assert_eq!(
        stack.effective_layers()[1]
            .get(2, -3)
            .unwrap()
            .atom
            .contents(),
        " "
    );
    let region = CanvasRegion {
        left: -3,
        top: 2,
        width: 1,
        height: 1,
    };
    assert_eq!(
        crate::dense_exchange::composite_region(&stack, region).unwrap()[0][0].contents,
        "a"
    );

    let mut overlay = LayerMap::new(LayerId(1), true);
    overlay.set_data(2, -3, data("b", Face::default()));
    let stack = LayerStack::new(vec![base, overlay], false).unwrap();
    assert_eq!(
        crate::dense_exchange::composite_region(&stack, region).unwrap()[0][0].contents,
        "a"
    );
}

#[test]
fn combined_dense_exchange_normalizes_signed_visible_layer_bounds() {
    let mut base = LayerMap::new(LayerId(0), true);
    base.set_data(2, -3, data("a", Face::default()));
    let mut top = LayerMap::new(LayerId(1), true);
    top.set_data(-1, 1, data("b", Face::default()));
    let mut hidden = LayerMap::new(LayerId(2), false);
    hidden.set_data(-8, -8, data("x", Face::default()));
    let stack = LayerStack::new(vec![base, top, hidden], true).unwrap();

    let composite = crate::dense_exchange::composite_visible_bounds(&stack).unwrap();
    assert_eq!(composite.len(), 4);
    assert!(composite.iter().all(|row| row.len() == 5));
    assert_eq!(composite[0][4].contents, "b");
    assert_eq!(composite[3][0].contents, "a");

    let layers = crate::dense_exchange::visible_layers_in_combined_bounds(&stack);
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0][3][0].contents, "a");
    assert_eq!(layers[1][0][4].contents, "b");
}

#[test]
fn bounds_follow_insertions_and_edge_deletions() {
    let mut layer = LayerMap::new(LayerId(0), true);
    layer
        .set_at(
            -4,
            7,
            data("a", Face::default()).atom.as_ref().clone(),
            &Face::default(),
        )
        .unwrap();
    layer
        .set_at(
            9,
            -2,
            data("b", Face::default()).atom.as_ref().clone(),
            &Face::default(),
        )
        .unwrap();
    assert_eq!(
        layer.bounds(),
        Some(LayerBounds {
            min_x: -4,
            min_y: -2,
            max_x: 9,
            max_y: 7,
        })
    );

    assert!(layer.delete_at(9, -2));
    assert_eq!(
        layer.bounds(),
        Some(LayerBounds {
            min_x: -4,
            min_y: 7,
            max_x: -4,
            max_y: 7,
        })
    );
    assert!(layer.delete_at(-4, 7));
    assert_eq!(layer.bounds(), None);
}

#[test]
fn cloning_coordinate_data_drops_raster_cache() {
    let source = data("a", Face::default());
    let image = skia_safe::surfaces::raster_n32_premul((1, 1))
        .unwrap()
        .image_snapshot();
    *source.raster_cache.borrow_mut() = Some(Rc::new(Rasterized {
        generation: 3,
        image,
        cell_width: 1.0,
        cell_height: 1.0,
        overflow: 0.0,
        atlas_safe: false,
    }));

    let cloned = source.clone();

    assert!(cloned.raster_cache.borrow().is_none());
    assert_eq!(cloned, source);
}

#[test]
fn line_markers_are_stored_with_their_coordinate_data() {
    let lines = vec![vec![StyledAtom {
        face: Face::default(),
        contents: "◆".to_owned(),
    }]];
    let line_data = LineData {
        ending: LineEnding::Fixed('◆'),
        base_glyph: "╴".to_owned(),
    };

    let mut map = crate::dense_exchange::from_dense(LayerId(0), true, &lines).unwrap();
    assert!(map.set_line_data(0, 0, Some(line_data.clone())));

    assert_eq!(
        map.get(0, 0).and_then(|data| data.line.as_ref()),
        Some(&line_data)
    );
}

#[test]
fn cell_and_row_edits_remap_embedded_line_metadata() {
    let line_data = LineData {
        ending: LineEnding::Fixed('◆'),
        base_glyph: "╴".to_owned(),
    };
    let styled = |contents: &str| StyledAtom {
        face: Face::default(),
        contents: contents.to_owned(),
    };
    let lines = vec![
        vec![styled("a"), styled("◆")],
        vec![styled("b"), styled("◆")],
    ];
    let mut map = crate::dense_exchange::from_dense(LayerId(0), true, &lines).unwrap();
    assert!(map.set_line_data(1, 0, Some(line_data.clone())));
    assert!(map.set_line_data(1, 1, Some(line_data.clone())));

    map.insert_cells(0, 0, vec![(Atom::new("z").unwrap(), Face::default())])
        .unwrap();
    assert_eq!(map.line_at(Coord { line: 0, column: 2 }), Some(&line_data));

    map.remove_cells(0, 2, 1).unwrap();
    assert!(map.line_at(Coord { line: 0, column: 2 }).is_none());
    assert_eq!(map.line_at(Coord { line: 1, column: 1 }), Some(&line_data));

    map.split_row(0, 1).unwrap();
    assert_eq!(map.line_at(Coord { line: 2, column: 1 }), Some(&line_data));
    assert!(map.join_row_with_next(1).unwrap());
    assert_eq!(map.line_at(Coord { line: 1, column: 2 }), Some(&line_data));
}

#[test]
fn structural_row_and_column_edits_remap_embedded_line_metadata() {
    let styled = |contents: &str| StyledAtom {
        face: Face::default(),
        contents: contents.to_owned(),
    };
    let marker = |contents: &str| LineData {
        ending: LineEnding::Fixed('◆'),
        base_glyph: contents.to_owned(),
    };
    let mut columns = crate::dense_exchange::from_dense(
        LayerId(0),
        true,
        &[vec![styled("A"), styled("B"), styled("C"), styled("D")]],
    )
    .unwrap();
    assert!(columns.set_line_data(1, 0, Some(marker("B"))));
    assert!(columns.set_line_data(3, 0, Some(marker("D"))));

    columns.pull_column_left(1, &BTreeSet::from([0])).unwrap();
    assert_eq!(
        columns.line_at(Coord { line: 0, column: 2 }),
        Some(&marker("D"))
    );
    columns.insert_column(2).unwrap();
    assert_eq!(
        columns.line_at(Coord { line: 0, column: 3 }),
        Some(&marker("D"))
    );

    let mut rows = crate::dense_exchange::from_dense(
        LayerId(0),
        true,
        &[
            vec![styled("A")],
            vec![styled("B")],
            vec![styled("C")],
            vec![styled("D")],
        ],
    )
    .unwrap();
    assert!(rows.set_line_data(0, 2, Some(marker("C"))));
    assert!(rows.set_line_data(0, 3, Some(marker("D"))));

    rows.remove_row(2).unwrap();
    assert_eq!(
        rows.line_at(Coord { line: 2, column: 0 }),
        Some(&marker("D"))
    );
}
