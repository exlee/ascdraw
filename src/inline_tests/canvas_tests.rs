use super::*;

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
fn top_at_ignores_whitespace_and_disabled_stack_uses_base_only() {
    let mut base = LayerMap::new(LayerId(0), true);
    base.set_data(2, -3, data("a", Face::default()));
    let mut top = LayerMap::new(LayerId(1), true);
    let styled = Face {
        bg: "selection".to_owned(),
        ..Face::default()
    };
    top.set_data(2, -3, data(" ", styled));
    let stack = LayerStack::new(vec![base.clone(), top], true).unwrap();
    assert_eq!(stack.at(2, -3).contents(), " ");
    assert_eq!(stack.top_at(2, -3).contents(), "a");

    let mut overlay = LayerMap::new(LayerId(1), true);
    overlay.set_data(2, -3, data("b", Face::default()));
    let stack = LayerStack::new(vec![base, overlay], false).unwrap();
    assert_eq!(stack.top_at(2, -3).contents(), "a");
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
    let marker = LineMarker {
        coord: Coord::default(),
        ending: LineEnding::Fixed('◆'),
        base_glyph: "╴".to_owned(),
    };

    let map =
        LayerMap::from_dense_with_markers(LayerId(0), true, &lines, std::slice::from_ref(&marker))
            .unwrap();

    assert_eq!(
        map.get(0, 0).and_then(|data| data.line.as_ref()),
        Some(&LineData {
            ending: marker.ending,
            base_glyph: marker.base_glyph.clone(),
        })
    );
    assert_eq!(map.line_markers(), vec![marker]);
}
