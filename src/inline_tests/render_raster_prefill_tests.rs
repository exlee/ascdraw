use crate::app::AppConfig;
use crate::canvas::LayerMap;
use crate::editor::Editor;
use crate::layout::{ViewportOffset, layout_metrics};
use crate::model::{Atom, Coord, Face, LayerId};
use crate::toolbar::ToolbarState;

use super::*;

#[test]
fn snapped_stale_raster_cells_share_physical_boundaries() {
    let renderer = load_renderer(&AppConfig::default());
    let mut metrics = renderer.metrics(1.0);
    metrics.cell_width = 8.25;
    metrics.cell_height = 15.6;
    let grid_top = 4.3;

    for column in -20..20 {
        let current = snapped_cell_rect(column, 0, grid_top, &metrics);
        let next = snapped_cell_rect(column + 1, 0, grid_top, &metrics);
        assert_eq!(current.right, next.left);
    }
    for row in -20..20 {
        let current = snapped_cell_rect(0, row, grid_top, &metrics);
        let next = snapped_cell_rect(0, row + 1, grid_top, &metrics);
        assert_eq!(current.bottom, next.top);
    }
}

#[test]
fn sparse_prefill_cursor_traverses_signed_coordinates_in_row_major_order() {
    let mut layer = LayerMap::new(LayerId(0), true);
    let face = Face::default();
    for coord in [
        Coord {
            line: -2,
            column: 3,
        },
        Coord {
            line: -2,
            column: 8,
        },
        Coord {
            line: 4,
            column: -5,
        },
    ] {
        layer
            .set_at(coord.column, coord.line, Atom::new("x").unwrap(), &face)
            .unwrap();
    }

    let first = next_sparse_coord(&layer, None).unwrap();
    let second = next_sparse_coord(&layer, Some(first)).unwrap();
    let third = next_sparse_coord(&layer, Some(second)).unwrap();
    assert_eq!(
        first,
        Coord {
            line: -2,
            column: 3
        }
    );
    assert_eq!(
        second,
        Coord {
            line: -2,
            column: 8
        }
    );
    assert_eq!(
        third,
        Coord {
            line: 4,
            column: -5
        }
    );
    assert_eq!(next_sparse_coord(&layer, Some(third)), None);
}

#[test]
fn idle_prefill_rasterizes_atoms_outside_the_rendered_viewport() {
    let config = AppConfig::default();
    let renderer = load_renderer(&config);
    let metrics = renderer.metrics(1.0);
    let toolbar_metrics = renderer.title_metrics(1.0);
    let width = 320;
    let height = 240;
    let layout = layout_metrics(
        width,
        height,
        &metrics,
        (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
        &ToolbarState::default(),
        config.transparent_menubar,
        1.0,
    );
    let mut surface = surfaces::raster_n32_premul((width as i32, height as i32)).unwrap();
    let mut state = Editor::new(&config.theme, "test");
    state.insert("x");
    state.grid.cursor_pos.column = 100;
    state.insert("y");
    state.commit_canvas_mutations().unwrap();

    render_cached_sparse_grid_atoms(
        surface.canvas(),
        state.canvas(),
        &state,
        &metrics,
        layout,
        ViewportOffset::default(),
        width,
        0,
        2,
        0,
        2,
        &renderer.rendered_atom_cache,
    );

    let offscreen = state.canvas().layers()[0].get(0, 100).unwrap();
    assert!(offscreen.raster_cache.borrow().is_none());
    while renderer.prefill_sparse_rasters(&state, 1.0) {}
    let cached = offscreen.raster_cache.borrow();
    let cached = cached.as_ref().unwrap();
    assert_eq!(cached.cell_width, metrics.cell_width);
    assert_eq!(cached.cell_height, metrics.cell_height);
}

#[test]
fn shared_atom_cache_holds_two_thousand_rasters() {
    assert_eq!(RENDERED_ATOM_CACHE_CAPACITY, 2048);
}
