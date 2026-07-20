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
    let serialized = contents(&canvas(&layers), &selections, position, (1.0, 1.0)).unwrap();
    assert!(serialized.contains("\"version\": 4"));
    assert_eq!(serialized.matches("\"atom\"").count(), 1);
    assert_eq!(serialized.matches("\"fg\"").count(), 1);
    let sparse: SparseDocument = serde_json::from_str(&serialized).unwrap();
    let loaded = sparse_document(sparse).unwrap();
    assert_eq!(
        crate::dense_exchange::to_dense(&loaded.canvas.layers()[0])[0][0].contents,
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

    let serialized = contents(&canvas, &selections, position, (10.0, 10.0)).unwrap();
    let sparse: SparseDocument = serde_json::from_str(&serialized).unwrap();

    assert_eq!(sparse.faces, vec![face]);
    assert_eq!(sparse.layers[0].cells[0].line, 0);
    assert_eq!(sparse.layers[0].cells[0].column, 0);
    assert_eq!(sparse.layers[0].cells[1].line, 1);
    assert_eq!(sparse.layers[0].cells[1].column, 2);
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
