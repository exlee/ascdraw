use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use skia_safe::{AlphaType, ColorType, Font, FontMgr, ImageInfo, surfaces};

use super::*;
use crate::app::ThemeConfig;
use crate::toolbar::{ObjectCommand, ToolbarAction};

fn metrics() -> CellMetrics {
    CellMetrics {
        font: Font::default(),
        cell_width: 8.0,
        cell_height: 16.0,
        baseline_offset: 10.0,
        underline_offset: 0.0,
        font_mgr: FontMgr::new(),
        fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
    }
}

#[test]
fn define_edit_dims_only_outside_the_definition() {
    let mut state = Editor::new(&ThemeConfig::default(), "test");
    state.insert("ab");
    state
        .selection
        .select(Coord { line: 0, column: 0 }, Coord { line: 0, column: 1 });
    assert!(state.apply_toolbar_action(ToolbarAction::Object(ObjectCommand::Define)));

    let metrics = metrics();
    let width = PADDING * 2 + metrics.cell_width as usize * 6;
    let height = metrics.cell_height as usize * 3;
    let underlying = [0x33, 0x22, 0x11, 0xff];
    let mut pixels = underlying.repeat(width * height);
    let image_info = ImageInfo::new(
        (width as i32, height as i32),
        ColorType::BGRA8888,
        AlphaType::Premul,
        None,
    );
    let mut surface = surfaces::wrap_pixels(&image_info, pixels.as_mut_slice(), width * 4, None)
        .expect("test surface");
    render_object_overlay(surface.canvas(), &state, &metrics, 0.0);
    drop(surface);

    let pixel = |column: f32, line: f32| {
        let x = PADDING + ((column + 0.5) * metrics.cell_width) as usize;
        let y = ((line + 0.5) * metrics.cell_height) as usize;
        let offset = (y * width + x) * 4;
        pixels[offset..offset + 4].to_vec()
    };
    assert_eq!(pixel(0.0, 0.0), underlying);
    assert_ne!(pixel(4.0, 2.0), underlying);
}

#[test]
fn anchor_lines_end_at_the_outline_box() {
    let half = (4.0, 8.0);
    assert_eq!(box_edge((0.0, 0.0), (20.0, 0.0), half), (4.0, 0.0));
    assert_eq!(box_edge((0.0, 0.0), (0.0, -40.0), half), (0.0, -8.0));
    assert_eq!(box_edge((0.0, 0.0), (2.0, 1.0), half), (2.0, 1.0));
}

#[test]
fn bounds_anchor_lines_start_one_cell_past_each_side() {
    let boundary = Rect::new(10.0, 20.0, 50.0, 60.0);
    let anchor = (14.0, 28.0);
    let cell = (8.0, 16.0);
    assert_eq!(edge_point(anchor, true, true, boundary, cell), (2.0, 28.0));
    assert_eq!(
        edge_point(anchor, true, false, boundary, cell),
        (58.0, 28.0)
    );
    assert_eq!(edge_point(anchor, false, true, boundary, cell), (14.0, 4.0));
    assert_eq!(
        edge_point(anchor, false, false, boundary, cell),
        (14.0, 76.0)
    );
}
