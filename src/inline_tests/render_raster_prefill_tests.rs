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
fn direct_diagonals_connect_at_the_same_snapped_corner() {
    let config = AppConfig::default();
    let renderer = load_renderer(&config);
    let mut metrics = renderer.metrics(1.0);
    metrics.cell_width = 8.25;
    metrics.cell_height = 16.375;
    let root = resolve_root_face(&Face::default(), FALLBACK_FG, FALLBACK_BG);
    let background = [root.bg.b, root.bg.g, root.bg.r, root.bg.a];
    let mut pixels = background.repeat(64 * 64);
    let image_info = ImageInfo::new((64, 64), ColorType::BGRA8888, AlphaType::Premul, None);
    let mut surface = surfaces::wrap_pixels(&image_info, &mut pixels, 64 * 4, None).unwrap();
    let mut paint = Paint::default();
    paint.set_color(root.fg.to_color());
    let upper_right = snapped_cell_rect(1, 0, 0.0, &metrics);
    let lower_left = snapped_cell_rect(0, 1, 0.0, &metrics);
    assert!(cell_graphics::draw_in_cell(
        surface.canvas(),
        upper_right,
        "╱",
        &metrics,
        &paint,
    ));
    assert!(cell_graphics::draw_in_cell(
        surface.canvas(),
        lower_left,
        "╱",
        &metrics,
        &paint,
    ));
    drop(surface);

    let corner_x = upper_right.left as usize;
    let corner_y = lower_left.top as usize;
    let corner_offset = (corner_y * 64 + corner_x) * 4;
    assert_ne!(pixels[corner_offset..corner_offset + 4], background);
}

#[test]
fn direct_full_cells_have_no_pixel_seam_during_zoom() {
    let config = AppConfig::default();
    let renderer = load_renderer(&config);
    let mut old_metrics = renderer.metrics(1.0);
    old_metrics.cell_width = 8.25;
    old_metrics.cell_height = 15.6;
    let toolbar_metrics = renderer.title_metrics(1.0);
    let width = 800;
    let height = 600;
    let old_layout = layout_metrics(
        width,
        height,
        &old_metrics,
        (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
        &ToolbarState::default(),
        config.transparent_menubar,
        1.0,
    );
    let mut state = Editor::new(&config.theme, "test");
    state.insert("██");
    state.commit_canvas_mutations().unwrap();
    let mut old_surface = surfaces::raster_n32_premul((width as i32, height as i32)).unwrap();
    render_cached_sparse_grid_atoms(
        old_surface.canvas(),
        state.canvas(),
        &state,
        &old_metrics,
        old_layout,
        ViewportOffset::default(),
        width,
        (0, 2),
        (0, 3),
        &renderer.rendered_atom_cache,
    );
    let mut new_metrics = old_metrics.clone();
    new_metrics.cell_width = 10.3;
    new_metrics.cell_height = 18.2;
    let new_layout = layout_metrics(
        width,
        height,
        &new_metrics,
        (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
        &ToolbarState::default(),
        config.transparent_menubar,
        1.0,
    );
    let root_face = resolve_root_face(&state.grid.default_face, FALLBACK_FG, FALLBACK_BG);
    let background = [
        root_face.bg.b,
        root_face.bg.g,
        root_face.bg.r,
        root_face.bg.a,
    ];
    let foreground = [
        root_face.fg.b,
        root_face.fg.g,
        root_face.fg.r,
        root_face.fg.a,
    ];
    let mut pixels = background.repeat(width * height);
    let image_info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::BGRA8888,
        AlphaType::Premul,
        None,
    );
    let mut surface = surfaces::wrap_pixels(&image_info, &mut pixels, width * 4, None).unwrap();
    render_cached_sparse_grid_atoms(
        surface.canvas(),
        state.canvas(),
        &state,
        &new_metrics,
        new_layout,
        ViewportOffset::default(),
        width,
        (0, 2),
        (0, 3),
        &renderer.rendered_atom_cache,
    );

    let first = snapped_cell_rect(0, 0, new_layout.grid_top, &new_metrics);
    let second = snapped_cell_rect(1, 0, new_layout.grid_top, &new_metrics);
    let y = ((first.top + first.bottom) / 2.0) as usize;
    for x in first.left as usize..second.right as usize {
        let offset = (y * width + x) * 4;
        assert_eq!(&pixels[offset..offset + 4], &foreground, "seam at x={x}");
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
        (0, 2),
        (0, 2),
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
fn decorated_diagonal_raster_fallback_preserves_intentional_overflow() {
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
    state.insert("╱");
    let mut decorated = state.theme.tooltip.clone();
    decorated.attributes.push("underline".to_owned());
    state.set_cell_face_for_test(Coord::default(), decorated);
    state.commit_canvas_mutations().unwrap();

    render_cached_sparse_grid_atoms(
        surface.canvas(),
        state.canvas(),
        &state,
        &metrics,
        layout,
        ViewportOffset::default(),
        width,
        (0, 2),
        (0, 2),
        &renderer.rendered_atom_cache,
    );

    let cached = state.canvas().layers()[0]
        .get(0, 0)
        .unwrap()
        .raster_cache
        .borrow()
        .clone()
        .unwrap();
    assert!(cached.overflow > 0.0);
    assert!(cached.image.width() as f32 >= metrics.cell_width + cached.overflow * 2.0);
    assert!(cached.image.height() as f32 >= metrics.cell_height + cached.overflow * 2.0);
}
