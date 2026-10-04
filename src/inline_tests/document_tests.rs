use super::*;
use crate::legacy_loader::LegacyLayer;
use crate::model::StyledAtom;

fn canvas(layers: &[LegacyLayer]) -> LayerStack {
    let maps = layers
        .iter()
        .map(|layer| {
            crate::dense_exchange::from_dense(layer.id, layer.visible, &layer.lines).unwrap()
        })
        .collect();
    LayerStack::new(maps, true).unwrap()
}

#[test]
fn sparse_json_round_trip_and_canonical_deletion() {
    let selections = crate::toolbar::ToolbarState::default().durable_selections();
    let layers = [LegacyLayer {
        id: LayerId(0),
        visible: true,
        lines: vec![vec![
            StyledAtom {
                face: Face::default(),
                contents: "x".to_owned(),
            },
            StyledAtom {
                face: Face::default(),
                contents: " ".to_owned(),
            },
        ]],
    }];
    let position = CanvasPosition {
        cursor: Coord::default(),
        viewport: ViewportOffset::default(),
        zoom: 0,
    };
    let serialized = contents(
        &canvas(&layers),
        &ObjectStore::default(),
        &selections,
        position,
        (1.0, 1.0),
    )
    .unwrap();
    assert!(serialized.contains("\"version\": 4"));
    assert_eq!(serialized.matches("\"v\"").count(), 1);
    let sparse: SparseDocument = serde_json::from_str(&serialized).unwrap();
    assert_eq!(sparse.faces.len(), 1);
    let loaded = sparse_document(sparse).unwrap();
    assert_eq!(
        crate::test_support::dense_layer(&loaded.canvas.layers()[0])[0][0].contents,
        "x"
    );
}

#[test]
fn sparse_write_rejects_wide_atoms() {
    let layers = [LegacyLayer {
        id: LayerId(0),
        visible: true,
        lines: vec![vec![StyledAtom {
            face: Face::default(),
            contents: "界".to_owned(),
        }]],
    }];
    assert!(crate::dense_exchange::from_dense(LayerId(0), true, &layers[0].lines).is_err());
}

#[test]
fn sparse_json_normalizes_coordinates_and_deduplicates_faces() {
    let face = Face {
        fg: "#123456".to_owned(),
        ..Face::default()
    };
    let mut layer = LayerMap::new(LayerId(0), true);
    layer
        .set_at(-10, -7, Atom::new("x").unwrap(), &face)
        .unwrap();
    layer
        .set_at(-8, -6, Atom::new("y").unwrap(), &face)
        .unwrap();
    let canvas = LayerStack::new(vec![layer], true).unwrap();
    let position = CanvasPosition {
        cursor: Coord {
            line: -6,
            column: -8,
        },
        viewport: ViewportOffset { x: -120, y: -70 },
        zoom: 0,
    };
    let selections = crate::toolbar::ToolbarState::default().durable_selections();

    let serialized = contents(
        &canvas,
        &ObjectStore::default(),
        &selections,
        position,
        (10.0, 10.0),
    )
    .unwrap();
    let sparse: SparseDocument = serde_json::from_str(&serialized).unwrap();

    assert_eq!(sparse.faces, vec![face]);
    assert_eq!(sparse.layers[0].cells[0].p, [0, 0]);
    assert_eq!(sparse.layers[0].cells[1].p, [2, 1]);
    assert_eq!(sparse.layers[0].cells[0].face_id, 0);
    assert_eq!(sparse.layers[0].cells[1].face_id, 0);
    assert_eq!(
        sparse.position,
        Some(CanvasPosition {
            cursor: Coord { line: 1, column: 2 },
            viewport: ViewportOffset { x: -220, y: -140 },
            zoom: 0,
        })
    );
}

#[test]
fn version_three_sparse_json_remains_readable() {
    let sparse: LegacySparseDocument = serde_json::from_str(
        r##"{
            "version": 3,
            "layers": [{
                "id": 0,
                "visible": true,
                "cells": [{
                    "line": 7,
                    "column": 10,
                    "face": {"fg":"#123456"},
                    "atom": "x"
                }]
            }],
            "active-layer": 0
        }"##,
    )
    .unwrap();

    let document = legacy_sparse_document(sparse).unwrap();
    assert!(document.needs_migration());
    let cell = document.canvas.layers()[0].get(7, 10).unwrap();
    assert_eq!(cell.atom.contents(), "x");
    assert_eq!(cell.face.fg, "#123456");
}

#[test]
fn objects_round_trip_with_normalized_instance_origins() {
    let mut map = LayerMap::new(LayerId(0), true);
    map.set_at_untracked(5, 3, Atom::new("x").unwrap(), &Face::default())
        .unwrap();
    let canvas = LayerStack::new(vec![map], true).unwrap();
    let mut objects = ObjectStore::default();
    let mut cells = crate::objects::Cells::default();
    cells.insert(
        Coord::default(),
        crate::objects::ObjectCell {
            atom: "x".to_owned(),
            face: Face::default(),
            line: None,
        },
    );
    objects.define(LayerId(0), Coord { line: 3, column: 5 }, 1, 1, cells);
    let position = CanvasPosition {
        cursor: Coord::default(),
        viewport: ViewportOffset::default(),
        zoom: 0,
    };
    let selections = crate::toolbar::ToolbarState::default().durable_selections();
    let serialized = contents(&canvas, &objects, &selections, position, (1.0, 1.0)).unwrap();
    let restored = parse_contents(&serialized).unwrap();
    assert_eq!(restored.objects.definitions, objects.definitions);
    assert_eq!(restored.objects.instances[0].origin, Coord::default());
}

#[test]
fn documents_without_objects_omit_the_field() {
    let selections = crate::toolbar::ToolbarState::default().durable_selections();
    let position = CanvasPosition {
        cursor: Coord::default(),
        viewport: ViewportOffset::default(),
        zoom: 0,
    };
    let canvas = LayerStack::new(vec![LayerMap::new(LayerId(0), true)], true).unwrap();
    let serialized = contents(
        &canvas,
        &ObjectStore::default(),
        &selections,
        position,
        (1.0, 1.0),
    )
    .unwrap();
    assert!(!serialized.contains("objects"));
    assert!(parse_contents(&serialized).unwrap().objects.is_empty());
}

#[test]
fn first_face_id_is_omitted_and_restored() {
    let other = Face {
        fg: "#123456".to_owned(),
        ..Face::default()
    };
    let mut layer = LayerMap::new(LayerId(0), true);
    layer
        .set_at(0, 0, Atom::new("x").unwrap(), &Face::default())
        .unwrap();
    layer.set_at(1, 0, Atom::new("y").unwrap(), &other).unwrap();
    let canvas = LayerStack::new(vec![layer], true).unwrap();
    let position = CanvasPosition {
        cursor: Coord::default(),
        viewport: ViewportOffset::default(),
        zoom: 0,
    };
    let selections = crate::toolbar::ToolbarState::default().durable_selections();
    let serialized = contents(
        &canvas,
        &ObjectStore::default(),
        &selections,
        position,
        (1.0, 1.0),
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    let cells = json["layers"][0]["cells"].as_array().unwrap();
    assert!(cells[0].get("face_id").is_none());
    assert_eq!(cells[1]["face_id"], 1);

    let restored = parse_contents(&serialized).unwrap();
    let layer = &restored.canvas.layers()[0];
    assert_eq!(layer.get(0, 0).unwrap().face.as_ref(), &Face::default());
    assert_eq!(layer.get(0, 1).unwrap().face.as_ref(), &other);
}

#[test]
fn cells_save_position_as_p_and_atom_as_v() {
    let mut layer = LayerMap::new(LayerId(0), true);
    layer
        .set_at(3, 2, Atom::new("x").unwrap(), &Face::default())
        .unwrap();
    layer
        .set_at(5, 4, Atom::new("y").unwrap(), &Face::default())
        .unwrap();
    let canvas = LayerStack::new(vec![layer], true).unwrap();
    let position = CanvasPosition {
        cursor: Coord::default(),
        viewport: ViewportOffset::default(),
        zoom: 0,
    };
    let selections = crate::toolbar::ToolbarState::default().durable_selections();
    let serialized = contents(
        &canvas,
        &ObjectStore::default(),
        &selections,
        position,
        (1.0, 1.0),
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    let cell = &json["layers"][0]["cells"][1];
    assert_eq!(cell["p"], serde_json::json!([2, 2]));
    assert_eq!(cell["v"], "y");
    for old_key in ["line", "column", "atom"] {
        assert!(cell.get(old_key).is_none());
    }
}

#[test]
fn version_four_cells_with_line_column_and_atom_remain_readable() {
    let document = parse_contents(
        r##"{
            "version": 4,
            "faces": [{}, {"fg":"#123456"}],
            "layers": [{
                "id": 0,
                "visible": true,
                "cells": [
                    {"line": 7, "column": 10, "face_id": 0, "atom": "x"},
                    {"line": 8, "column": 11, "face_id": 1, "atom": "y"}
                ]
            }],
            "active-layer": 0
        }"##,
    )
    .unwrap();

    assert!(!document.needs_migration());
    let layer = &document.canvas.layers()[0];
    assert_eq!(layer.get(7, 10).unwrap().atom.contents(), "x");
    let second = layer.get(8, 11).unwrap();
    assert_eq!(second.atom.contents(), "y");
    assert_eq!(second.face.fg, "#123456");
}

#[test]
fn faces_omit_default_colors_and_empty_attributes() {
    let styled = Face {
        fg: "#123456".to_owned(),
        ..Face::default()
    };
    let mut layer = LayerMap::new(LayerId(0), true);
    layer
        .set_at(0, 0, Atom::new("x").unwrap(), &Face::default())
        .unwrap();
    layer
        .set_at(1, 0, Atom::new("y").unwrap(), &styled)
        .unwrap();
    let canvas = LayerStack::new(vec![layer], true).unwrap();
    let position = CanvasPosition {
        cursor: Coord::default(),
        viewport: ViewportOffset::default(),
        zoom: 0,
    };
    let selections = crate::toolbar::ToolbarState::default().durable_selections();
    let serialized = contents(
        &canvas,
        &ObjectStore::default(),
        &selections,
        position,
        (1.0, 1.0),
    )
    .unwrap();

    let json: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(json["faces"][0], serde_json::json!({}));
    assert_eq!(json["faces"][1], serde_json::json!({"fg": "#123456"}));

    let restored = parse_contents(&serialized).unwrap();
    let layer = &restored.canvas.layers()[0];
    assert_eq!(layer.get(0, 0).unwrap().face.as_ref(), &Face::default());
    assert_eq!(layer.get(0, 1).unwrap().face.as_ref(), &styled);
}

#[test]
fn faces_with_explicit_default_fields_remain_readable() {
    let face: Face = serde_json::from_str(
        r#"{"fg":"default","bg":"default","underline":"default","attributes":[]}"#,
    )
    .unwrap();
    assert_eq!(face, Face::default());
}

fn object_document(face: &Face) -> (LayerStack, ObjectStore) {
    let mut map = LayerMap::new(LayerId(0), true);
    map.set_at_untracked(0, 0, Atom::new("x").unwrap(), &Face::default())
        .unwrap();
    let canvas = LayerStack::new(vec![map], true).unwrap();
    let mut objects = ObjectStore::default();
    let mut cells = crate::objects::Cells::default();
    cells.insert(
        Coord::default(),
        crate::objects::ObjectCell {
            atom: "x".to_owned(),
            face: face.clone(),
            line: None,
        },
    );
    objects.define(LayerId(0), Coord::default(), 1, 1, cells);
    (canvas, objects)
}

#[test]
fn object_cells_name_faces_by_id() {
    let styled = Face {
        fg: "#123456".to_owned(),
        ..Face::default()
    };
    let (canvas, objects) = object_document(&styled);
    let position = CanvasPosition {
        cursor: Coord::default(),
        viewport: ViewportOffset::default(),
        zoom: 0,
    };
    let selections = crate::toolbar::ToolbarState::default().durable_selections();
    let serialized = contents(&canvas, &objects, &selections, position, (1.0, 1.0)).unwrap();

    let json: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    let cell = &json["objects"]["definitions"][0]["cells"][0];
    assert!(cell.get("face").is_none());
    assert_eq!(
        json["faces"][cell["face_id"].as_u64().unwrap() as usize]["fg"],
        "#123456"
    );

    let restored = parse_contents(&serialized).unwrap();
    assert_eq!(restored.objects.definitions, objects.definitions);
}

#[test]
fn object_cells_with_inline_faces_remain_readable() {
    let document = parse_contents(
        r##"{
            "version": 4,
            "faces": [{}],
            "layers": [{"id": 0, "visible": true, "cells": [{"p": [0, 0], "v": "x"}]}],
            "active-layer": 0,
            "objects": {
                "definitions": [{
                    "id": 0, "width": 1, "height": 1,
                    "cells": [{"line": 0, "column": 0, "atom": "x", "face": {"fg": "#123456"}}]
                }]
            }
        }"##,
    )
    .unwrap();

    let cell = document.objects.definitions[0]
        .cells
        .get(Coord::default())
        .unwrap();
    assert_eq!(cell.face.fg, "#123456");
}

#[test]
fn short_containers_are_written_on_one_line() {
    let mut out = String::new();
    write_json(
        &mut out,
        &serde_json::json!({"cells": [{"p": [1, 2], "v": "x"}], "long": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}),
        0,
    );
    assert!(out.contains(r#"{"p":[1,2],"v":"x"}"#));
    assert!(out.starts_with("{\n  \"cells\": "));
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["cells"][0]["p"], serde_json::json!([1, 2]));
}
