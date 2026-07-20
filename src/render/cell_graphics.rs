use skia_safe::{Canvas, Paint, Rect};

use super::{CellMetrics, PADDING};

type CellFractionRect = (f32, f32, f32, f32);
type BlockElementRects = [Option<CellFractionRect>; 4];

fn one_block_rect(rect: CellFractionRect) -> BlockElementRects {
    [Some(rect), None, None, None]
}

fn block_element_rects(character: char) -> Option<BlockElementRects> {
    let codepoint = character as u32;
    match codepoint {
        0x2580 => Some(one_block_rect((0.0, 0.0, 1.0, 0.5))),
        0x2581..=0x2588 => {
            let height = (codepoint - 0x2580) as f32 / 8.0;
            Some(one_block_rect((0.0, 1.0 - height, 1.0, height)))
        }
        0x2589..=0x258f => {
            let width = (0x2590 - codepoint) as f32 / 8.0;
            Some(one_block_rect((0.0, 0.0, width, 1.0)))
        }
        0x2590 => Some(one_block_rect((0.5, 0.0, 0.5, 1.0))),
        0x2594 => Some(one_block_rect((0.0, 0.0, 1.0, 0.125))),
        0x2595 => Some(one_block_rect((0.875, 0.0, 0.125, 1.0))),
        0x2596..=0x259f => {
            const MASKS: [u8; 10] = [
                0b0100, 0b1000, 0b0001, 0b1101, 0b1001, 0b0111, 0b1011, 0b0010, 0b0110, 0b1110,
            ];
            let mask = MASKS[(codepoint - 0x2596) as usize];
            let quadrants = [
                (0b0001, 0.0, 0.0),
                (0b0010, 0.5, 0.0),
                (0b0100, 0.0, 0.5),
                (0b1000, 0.5, 0.5),
            ];
            let mut rects = [None; 4];
            let mut index = 0;
            for (bit, left, top) in quadrants {
                if mask & bit != 0 {
                    rects[index] = Some((left, top, 0.5, 0.5));
                    index += 1;
                }
            }
            Some(rects)
        }
        _ => None,
    }
}

fn shade_level(character: char) -> Option<u8> {
    match character {
        '░' => Some(1),
        '▒' => Some(2),
        '▓' => Some(3),
        _ => None,
    }
}

pub(super) fn raster_overflow(text: &str, metrics: &CellMetrics) -> f32 {
    let mut characters = text.chars();
    if !matches!(characters.next(), Some('╱' | '╲' | '╳')) || characters.next().is_some() {
        return 0.0;
    }
    diagonal_stroke_width(metrics) * 2.0
}

fn diagonal_stroke_width(metrics: &CellMetrics) -> f32 {
    (metrics.cell_width.min(metrics.cell_height) / 14.0).max(1.0)
}

pub(super) fn foreground_coverage(text: &str) -> f32 {
    const BLOCK_COVERAGE_EIGHTHS: [u8; 32] = [
        4, 1, 2, 3, 4, 5, 6, 7, 8, 7, 6, 5, 4, 3, 2, 1, 4, 2, 4, 6, 1, 1, 2, 2, 2, 6, 4, 6, 6, 2,
        4, 6,
    ];
    let mut characters = text.chars();
    let Some(character) = characters.next() else {
        return 0.0;
    };
    if characters.next().is_some() {
        return 0.0;
    }
    let codepoint = character as u32;
    if (0x2580..=0x259f).contains(&codepoint) {
        f32::from(BLOCK_COVERAGE_EIGHTHS[(codepoint - 0x2580) as usize]) / 8.0
    } else {
        0.0
    }
}

fn draw_shade(canvas: &Canvas, cell: Rect, level: u8, paint: &Paint) {
    let mut paint = paint.clone();
    paint
        .set_anti_alias(false)
        .set_alpha_f(f32::from(level) / 4.0);
    canvas.draw_rect(cell, &paint);
}

type DiagonalSegment = ((f32, f32), (f32, f32));

fn diagonal_segments(
    character: char,
    cell: Rect,
    overlap: f32,
) -> Option<[Option<DiagonalSegment>; 2]> {
    let diagonals = match character {
        '╱' => (true, false),
        '╲' => (false, true),
        '╳' => (true, true),
        _ => return None,
    };
    let length = cell.width().hypot(cell.height());
    let overlap_x = overlap * cell.width() / length;
    let overlap_y = overlap * cell.height() / length;
    Some([
        diagonals.0.then_some((
            (cell.left - overlap_x, cell.bottom + overlap_y),
            (cell.right + overlap_x, cell.top - overlap_y),
        )),
        diagonals.1.then_some((
            (cell.left - overlap_x, cell.top - overlap_y),
            (cell.right + overlap_x, cell.bottom + overlap_y),
        )),
    ])
}

pub(super) fn draw(
    canvas: &Canvas,
    column: usize,
    top: f32,
    text: &str,
    metrics: &CellMetrics,
    paint: &Paint,
) -> bool {
    let mut characters = text.chars();
    let Some(character) = characters.next() else {
        return false;
    };
    if characters.next().is_some() {
        return false;
    }

    let cell = Rect::from_xywh(
        PADDING as f32 + column as f32 * metrics.cell_width,
        top,
        metrics.cell_width,
        metrics.cell_height,
    );
    let left = cell.left.round();
    let right = cell.right.round();
    let bottom = cell.bottom.round();
    let top = cell.top.round();
    if let Some(rects) = block_element_rects(character) {
        let mut paint = paint.clone();
        paint.set_anti_alias(false);
        for (x, y, width, height) in rects.into_iter().flatten() {
            let rect_left = (left + x * (right - left)).round();
            let rect_top = (top + y * (bottom - top)).round();
            let rect_right = (left + (x + width) * (right - left)).round();
            let rect_bottom = (top + (y + height) * (bottom - top)).round();
            canvas.draw_rect(
                Rect::new(rect_left, rect_top, rect_right, rect_bottom),
                &paint,
            );
        }
        return true;
    }

    if let Some(level) = shade_level(character) {
        draw_shade(canvas, Rect::new(left, top, right, bottom), level, paint);
        return true;
    }

    let stroke_width = diagonal_stroke_width(metrics);
    let Some(segments) = diagonal_segments(character, cell, stroke_width) else {
        return false;
    };
    let mut paint = paint.clone();
    paint.set_anti_alias(true).set_stroke_width(stroke_width);
    for (start, end) in segments.into_iter().flatten() {
        canvas.draw_line(start, end, &paint);
    }
    true
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use skia_safe::{AlphaType, ColorType, Font, FontMgr, ImageInfo, surfaces};

    use super::*;
    use crate::face_resolution::Rgba;

    #[test]
    fn foreground_coverage_matches_custom_block_and_shade_rendering() {
        for codepoint in 0x2580..=0x259f {
            let character = char::from_u32(codepoint).unwrap();
            let rendered_coverage = block_element_rects(character)
                .map(|rects| {
                    rects
                        .into_iter()
                        .flatten()
                        .map(|(_, _, width, height)| width * height)
                        .sum()
                })
                .or_else(|| shade_level(character).map(|level| f32::from(level) / 4.0))
                .unwrap_or(0.0);
            assert_eq!(
                foreground_coverage(&character.to_string()),
                rendered_coverage
            );
        }
        assert_eq!(foreground_coverage("x"), 0.0);
    }

    #[test]
    fn uses_grid_geometry_without_row_gaps() {
        assert_eq!(
            block_element_rects('█'),
            Some([Some((0.0, 0.0, 1.0, 1.0)), None, None, None])
        );
        for codepoint in 0x2580..=0x259f {
            let character = char::from_u32(codepoint).unwrap();
            assert!(
                block_element_rects(character).is_some() || shade_level(character).is_some(),
                "missing Block Element U+{codepoint:04X}"
            );
        }

        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.25,
            cell_height: 16.375,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let width = PADDING + metrics.cell_width.ceil() as usize;
        let rows = 4;
        let height = (metrics.cell_height * rows as f32).ceil() as usize;
        let mut pixels = vec![0xff; width * height * 4];
        let image_info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::BGRA8888,
            AlphaType::Premul,
            None,
        );
        let mut surface =
            surfaces::wrap_pixels(&image_info, pixels.as_mut_slice(), width * 4, None)
                .expect("test surface");
        let mut paint = Paint::default();
        paint.set_color(Rgba::rgb(0, 0, 0).to_color());
        for row in 0..rows {
            assert!(draw(
                surface.canvas(),
                0,
                row as f32 * metrics.cell_height,
                "█",
                &metrics,
                &paint,
            ));
        }
        drop(surface);

        let x = PADDING + 1;
        for y in 0..height {
            let offset = (y * width + x) * 4;
            assert_eq!(&pixels[offset..offset + 4], &[0, 0, 0, 0xff]);
        }
    }

    #[test]
    fn shades_have_terminal_densities_and_diagonals_extend_past_the_cell() {
        assert_eq!(shade_level('░'), Some(1));
        assert_eq!(shade_level('▒'), Some(2));
        assert_eq!(shade_level('▓'), Some(3));
        assert_eq!(shade_level('█'), None);

        let cell = Rect::new(10.0, 20.0, 18.0, 36.0);
        let segments = diagonal_segments('╳', cell, 2.0).unwrap();
        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: cell.width(),
            cell_height: cell.height(),
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        assert_eq!(raster_overflow("╳", &metrics), 2.0);
        for (start, end) in segments.into_iter().flatten() {
            assert!(start.0 < cell.left);
            assert!(end.0 > cell.right);
            assert!(start.1 < cell.top || start.1 > cell.bottom);
            assert!(end.1 < cell.top || end.1 > cell.bottom);
        }
        assert_eq!(diagonal_segments('a', cell, 2.0), None);

        let width = cell.width();
        let height = cell.height();
        let upper_right = Rect::from_xywh(cell.right, cell.top, width, height);
        let lower_left = Rect::from_xywh(cell.left, cell.bottom, width, height);
        let upper = diagonal_segments('╱', upper_right, 2.0).unwrap()[0].unwrap();
        let lower = diagonal_segments('╱', lower_left, 2.0).unwrap()[0].unwrap();
        let upper_direction = (upper.1.0 - upper.0.0, upper.1.1 - upper.0.1);
        let lower_direction = (lower.1.0 - lower.0.0, lower.1.1 - lower.0.1);
        assert!((upper_direction.0 - lower_direction.0).abs() < 0.0001);
        assert!((upper_direction.1 - lower_direction.1).abs() < 0.0001);
        assert!(lower.1.0 > upper.0.0);
        assert!(lower.1.1 < upper.0.1);
    }

    #[test]
    fn shade_cells_are_uniform_and_increase_in_density() {
        let metrics = CellMetrics {
            font: Font::default(),
            cell_width: 8.25,
            cell_height: 16.375,
            baseline_offset: 10.0,
            underline_offset: 0.0,
            font_mgr: FontMgr::new(),
            fallback_fonts: Rc::new(RefCell::new(HashMap::new())),
        };
        let width = PADDING + (metrics.cell_width * 3.0).ceil() as usize;
        let height = metrics.cell_height.ceil() as usize;
        let mut pixels = vec![0xff; width * height * 4];
        let image_info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::BGRA8888,
            AlphaType::Premul,
            None,
        );
        let mut surface =
            surfaces::wrap_pixels(&image_info, pixels.as_mut_slice(), width * 4, None)
                .expect("test surface");
        let mut paint = Paint::default();
        paint.set_color(Rgba::rgb(0, 0, 0).to_color());
        for (column, shade) in ["░", "▒", "▓"].into_iter().enumerate() {
            assert!(draw(surface.canvas(), column, 0.0, shade, &metrics, &paint,));
        }
        drop(surface);

        let mut shades = Vec::new();
        let bottom = metrics.cell_height.round() as usize;
        for column in 0..3 {
            let left = (PADDING as f32 + column as f32 * metrics.cell_width).round() as usize;
            let right =
                (PADDING as f32 + (column + 1) as f32 * metrics.cell_width).round() as usize;
            let sample = pixels[((height / 2) * width + left) * 4];
            for y in 0..bottom {
                for x in left..right {
                    assert_eq!(pixels[(y * width + x) * 4], sample);
                }
            }
            shades.push(sample);
        }
        assert!(shades[0] > shades[1]);
        assert!(shades[1] > shades[2]);
    }
}
