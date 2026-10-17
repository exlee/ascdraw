use crate::app::AppConfig;
use crate::editor::Editor;
use crate::layout::{ViewportOffset, layout_metrics};
use crate::model::{Coord, Face};
use crate::toolbar::ToolbarState;

use super::*;

const WIDTH: usize = 200;
const HEIGHT: usize = 800;
const ROWS: i16 = 9;

fn fractional_metrics(renderer: &Renderer) -> CellMetrics {
    let mut metrics = renderer.metrics(1.0);
    metrics.cell_width = 8.37;
    metrics.cell_height = 15.43;
    metrics
}

fn layout_for(renderer: &Renderer, metrics: &CellMetrics, config: &AppConfig) -> LayoutMetrics {
    let toolbar_metrics = renderer.title_metrics(1.0);
    layout_metrics(
        WIDTH,
        HEIGHT,
        metrics,
        (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
        &ToolbarState::default(),
        config.transparent_menubar,
        1.0,
    )
}

fn background_pixels(state: &Editor) -> Vec<u8> {
    let root = resolve_root_face(&state.grid.default_face, FALLBACK_FG, FALLBACK_BG);
    [root.bg.b, root.bg.g, root.bg.r, root.bg.a].repeat(WIDTH * HEIGHT)
}

fn with_canvas(pixels: &mut [u8], draw: impl FnOnce(&Canvas)) {
    let info = ImageInfo::new(
        (WIDTH as i32, HEIGHT as i32),
        ColorType::BGRA8888,
        AlphaType::Premul,
        None,
    );
    let mut surface = surfaces::wrap_pixels(&info, pixels, WIDTH * 4, None).unwrap();
    draw(surface.canvas());
}

fn place_text(state: &mut Editor, text: &str, columns: &[i16], face: Option<&Face>) {
    for row in 0..ROWS {
        for &column in columns {
            state.grid.cursor_pos.line = row;
            state.grid.cursor_pos.column = column;
            state.insert(text);
            if let Some(face) = face {
                for offset in 0..text.chars().count() as i16 {
                    state.set_cell_face_for_test(
                        Coord {
                            line: row,
                            column: column + offset,
                        },
                        face.clone(),
                    );
                }
            }
        }
    }
    state.commit_canvas_mutations().unwrap();
}

fn render_grid(
    renderer: &Renderer,
    state: &Editor,
    metrics: &CellMetrics,
    layout: LayoutMetrics,
) -> Vec<u8> {
    let mut pixels = background_pixels(state);
    with_canvas(&mut pixels, |canvas| {
        render_cached_sparse_grid_atoms(
            canvas,
            state.canvas(),
            state,
            metrics,
            layout,
            ViewportOffset::default(),
            WIDTH,
            (0, ROWS),
            (0, 20),
            &renderer.rendered_atom_cache,
        );
    });
    pixels
}

fn cached_raster(state: &Editor, column: i16, row: i16) -> Rc<crate::canvas::Rasterized> {
    state.canvas().layers()[0]
        .get(row, column)
        .unwrap()
        .raster_cache
        .borrow()
        .clone()
        .unwrap()
}

fn pixel(pixels: &[u8], x: f32, y: f32) -> &[u8] {
    let offset = (y as usize * WIDTH + x as usize) * 4;
    &pixels[offset..offset + 4]
}

#[test]
fn text_rasters_land_on_whole_pixels_without_resampling_at_fractional_zoom() {
    let config = AppConfig::default();
    let renderer = load_renderer(&config);
    let metrics = fractional_metrics(&renderer);
    let layout = layout_for(&renderer, &metrics, &config);
    let mut state = Editor::new(&config.theme, "test");
    let columns = [0, 3, 6, 9];
    place_text(&mut state, "g", &columns, None);

    let actual = render_grid(&renderer, &state, &metrics, layout);

    let mut expected = background_pixels(&state);
    with_canvas(&mut expected, |canvas| {
        for row in 0..ROWS {
            for column in columns {
                let raster = cached_raster(&state, column, row);
                assert!(raster.atlas_safe);
                let cell = snapped_cell_rect(column, row, layout.grid_top, &metrics);
                canvas.draw_image(&raster.image, (cell.left, cell.top), None);
            }
        }
    });
    let mismatches = actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .filter(|(actual, expected)| actual != expected)
        .count();
    assert_eq!(mismatches, 0, "glyph pixels were resampled or clipped");
}

#[test]
fn background_rasters_fill_snapped_cells_with_unscaled_glyphs_at_fractional_zoom() {
    let config = AppConfig::default();
    let renderer = load_renderer(&config);
    let metrics = fractional_metrics(&renderer);
    let layout = layout_for(&renderer, &metrics, &config);
    let mut state = Editor::new(&config.theme, "test");
    let face = Face {
        attributes: vec!["reverse".to_owned()],
        ..Face::default()
    };
    let text = "whatgy";
    place_text(&mut state, text, &[0], Some(&face));
    let root = resolve_root_face(&state.grid.default_face, FALLBACK_FG, FALLBACK_BG);
    let resolved = resolve_derived_face(&state.grid.default_face, &face, FALLBACK_FG, FALLBACK_BG);
    assert_ne!(resolved.bg, root.bg);

    let actual = render_grid(&renderer, &state, &metrics, layout);

    for row in 0..ROWS {
        for column in 0..text.chars().count() as i16 {
            let raster = cached_raster(&state, column, row);
            assert!(!raster.atlas_safe);
            let mut image_pixels =
                vec![0u8; raster.image.width() as usize * raster.image.height() as usize * 4];
            let info = ImageInfo::new(
                raster.image.dimensions(),
                ColorType::BGRA8888,
                AlphaType::Premul,
                None,
            );
            assert!(raster.image.read_pixels(
                &info,
                &mut image_pixels,
                raster.image.width() as usize * 4,
                (0, 0),
                skia_safe::image::CachingHint::Allow,
            ));
            let cell = snapped_cell_rect(column, row, layout.grid_top, &metrics);
            let mut y = cell.top;
            while y < cell.bottom {
                let mut x = cell.left;
                while x < cell.right {
                    let image_x = (x - cell.left) as usize;
                    let image_y = (y - cell.top) as usize;
                    let offset = (image_y * raster.image.width() as usize + image_x) * 4;
                    let source = &image_pixels[offset..offset + 4];
                    let target = pixel(&actual, x, y);
                    assert_eq!(source[3], u8::MAX, "transparent seam at ({x}, {y})");
                    assert_eq!(target, source, "resampled pixel at ({x}, {y})");
                    x += 1.0;
                }
                y += 1.0;
            }
        }
    }
}
