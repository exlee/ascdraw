use skia_safe::{Canvas, Font, Paint, Rect};

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

fn stretched_glyph_target(character: char, cell: Rect) -> Option<Rect> {
    if matches!(character, '░' | '▒' | '▓') {
        return Some(cell);
    }
    if !matches!(character, '╱' | '╲' | '╳') {
        return None;
    }

    let overhang = (cell.width().min(cell.height()) / 12.0).max(1.0);
    Some(Rect::new(
        cell.left - overhang,
        cell.top - overhang,
        cell.right + overhang,
        cell.bottom + overhang,
    ))
}

fn draw_stretched_glyph(
    canvas: &Canvas,
    text: &str,
    font: &Font,
    paint: &Paint,
    target: Rect,
) -> bool {
    let bounds = font.measure_str(text, Some(paint)).1;
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return false;
    }

    canvas.save();
    canvas.translate((target.left, target.top));
    canvas.scale((
        target.width() / bounds.width(),
        target.height() / bounds.height(),
    ));
    canvas.translate((-bounds.left, -bounds.top));
    canvas.draw_str(text, (0.0, 0.0), font, paint);
    canvas.restore();
    true
}

pub(super) fn draw(
    canvas: &Canvas,
    column: usize,
    top: f32,
    text: &str,
    font: &Font,
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

    let left = (PADDING as f32 + column as f32 * metrics.cell_width).round();
    let right = (PADDING as f32 + (column + 1) as f32 * metrics.cell_width).round();
    let bottom = (top + metrics.cell_height).round();
    let top = top.round();
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

    let cell = Rect::new(left, top, right, bottom);
    let Some(target) = stretched_glyph_target(character, cell) else {
        return false;
    };
    draw_stretched_glyph(canvas, text, font, paint, target)
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
    fn uses_grid_geometry_without_row_gaps() {
        assert_eq!(
            block_element_rects('█'),
            Some([Some((0.0, 0.0, 1.0, 1.0)), None, None, None])
        );
        let cell = Rect::new(10.0, 20.0, 18.0, 36.0);
        for codepoint in 0x2580..=0x259f {
            let character = char::from_u32(codepoint).unwrap();
            assert!(
                block_element_rects(character).is_some()
                    || stretched_glyph_target(character, cell).is_some(),
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
                &metrics.font,
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
    fn fits_shades_to_the_cell_and_stretches_diagonals_past_it() {
        let cell = Rect::new(10.0, 20.0, 18.0, 36.0);
        for shade in ['░', '▒', '▓'] {
            assert_eq!(stretched_glyph_target(shade, cell), Some(cell));
        }
        for diagonal in ['╱', '╲', '╳'] {
            let target = stretched_glyph_target(diagonal, cell).unwrap();
            assert!(target.left < cell.left);
            assert!(target.top < cell.top);
            assert!(target.right > cell.right);
            assert!(target.bottom > cell.bottom);
        }
        assert_eq!(stretched_glyph_target('█', cell), None);
        assert_eq!(stretched_glyph_target('a', cell), None);
    }
}
