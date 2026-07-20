use std::io::Cursor;

use anyhow::{Context, Result, anyhow, bail};
use png::{
    BitDepth, ColorType as PngColorType, Encoder, Info, ScaledFloat, SourceChromaticities,
    SrgbRenderingIntent,
};
use skia_safe::{
    AlphaType, ColorSpace, ColorType, ImageInfo, Paint, PixelGeometry, Rect, SurfaceProps,
    SurfacePropsFlags, named_primaries, named_transfer_fn, surfaces,
};
use unicode_width::UnicodeWidthStr;

use super::{DrawOrigin, FALLBACK_BG, Renderer, resolve_root_face};
use crate::app::MacosColorSpace;
use crate::model::{Face, StyledAtom};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    pub png: Vec<u8>,
    pub color_space: MacosColorSpace,
}

pub fn render_canvas_image(
    renderer: &Renderer,
    lines: &[Vec<StyledAtom>],
    default_face: &Face,
    scale_factor: f64,
    color_space: MacosColorSpace,
) -> Result<CanvasImage> {
    render_canvas_layers_image(
        renderer,
        &[lines.to_vec()],
        default_face,
        scale_factor,
        color_space,
    )
}

pub fn render_canvas_layers_image(
    renderer: &Renderer,
    layers: &[Vec<Vec<StyledAtom>>],
    default_face: &Face,
    scale_factor: f64,
    color_space: MacosColorSpace,
) -> Result<CanvasImage> {
    let metrics = renderer.metrics(scale_factor);
    let columns = layers
        .iter()
        .flat_map(|lines| lines.iter())
        .map(|line| display_width(line))
        .max()
        .unwrap_or(0);
    let rows = layers.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 || rows == 0 {
        bail!("cannot render an empty canvas region as PNG");
    }
    let width = (columns as f32 * metrics.cell_width).ceil() as usize;
    let height = (rows as f32 * metrics.cell_height).ceil() as usize;
    let dimensions = (
        i32::try_from(width).context("PNG canvas is too wide")?,
        i32::try_from(height).context("PNG canvas is too tall")?,
    );
    let skia_color_space = skia_color_space(color_space)?;
    let image_info = ImageInfo::new(
        dimensions,
        ColorType::RGBA8888,
        AlphaType::Premul,
        Some(skia_color_space.clone()),
    );
    let props = SurfaceProps::new(
        SurfacePropsFlags::USE_DEVICE_INDEPENDENT_FONTS,
        PixelGeometry::Unknown,
    );
    let mut surface = surfaces::raster(&image_info, None, Some(&props))
        .context("failed to allocate offscreen PNG surface")?;
    let canvas = surface.canvas();

    // PNG exports are deliberately opaque, matching the canvas window. Start
    // with the renderer fallback and composite a translucent configured root
    // background over it before drawing individual cells.
    canvas.clear(FALLBACK_BG.to_color());
    let root = resolve_root_face(default_face, super::FALLBACK_FG, FALLBACK_BG);
    let mut background = Paint::default();
    background
        .set_anti_alias(false)
        .set_color(root.bg.to_color());
    canvas.draw_rect(
        Rect::from_xywh(0.0, 0.0, width as f32, height as f32),
        &background,
    );

    canvas.save();
    canvas.translate((-(super::PADDING as f32), 0.0));
    for lines in layers {
        for (row, line) in lines.iter().enumerate() {
            super::render_overlay_line(
                canvas,
                row,
                line,
                default_face,
                0..columns,
                &metrics,
                DrawOrigin::Grid { top_padding: 0.0 },
            );
        }
    }
    canvas.restore();

    let mut rgba = vec![
        0;
        width
            .checked_mul(height)
            .and_then(|n| n.checked_mul(4))
            .context("PNG pixel buffer size overflowed",)?
    ];
    let read_info = ImageInfo::new(
        dimensions,
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        Some(skia_color_space),
    );
    if !surface.read_pixels(&read_info, &mut rgba, width * 4, (0, 0)) {
        return Err(anyhow!("failed to read offscreen PNG pixels"));
    }
    let png = encode_png(width, height, &rgba, color_space)?;
    Ok(CanvasImage {
        width,
        height,
        rgba,
        png,
        color_space,
    })
}

fn display_width(line: &[StyledAtom]) -> usize {
    line.iter()
        .map(|atom| UnicodeWidthStr::width(atom.contents.as_str()).max(1))
        .sum()
}

pub(super) fn skia_color_space(color_space: MacosColorSpace) -> Result<ColorSpace> {
    match color_space {
        MacosColorSpace::Srgb => Ok(ColorSpace::new_srgb()),
        MacosColorSpace::P3 => ColorSpace::new_cicp(
            named_primaries::CicpId::SMPTE_EG_432_1,
            named_transfer_fn::CicpId::IEC61966_2_1,
        )
        .context("Skia does not support the configured Display P3 color space"),
    }
}

fn encode_png(
    width: usize,
    height: usize,
    rgba: &[u8],
    color_space: MacosColorSpace,
) -> Result<Vec<u8>> {
    let mut info = Info::with_size(
        u32::try_from(width).context("PNG canvas is too wide")?,
        u32::try_from(height).context("PNG canvas is too tall")?,
    );
    info.color_type = PngColorType::Rgba;
    info.bit_depth = BitDepth::Eight;
    match color_space {
        MacosColorSpace::Srgb => {
            info.srgb = Some(SrgbRenderingIntent::RelativeColorimetric);
        }
        MacosColorSpace::P3 => {
            info.source_gamma = Some(ScaledFloat::from_scaled(45_455));
            info.source_chromaticities = Some(SourceChromaticities::new(
                (0.3127, 0.3290),
                (0.6800, 0.3200),
                (0.2650, 0.6900),
                (0.1500, 0.0600),
            ));
        }
    }
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = Encoder::with_info(&mut bytes, info)
        .context("failed to configure PNG encoder")?
        .write_header()
        .context("failed to write PNG header")?;
    writer
        .write_image_data(rgba)
        .context("failed to encode PNG pixels")?;
    drop(writer);
    Ok(bytes.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppConfig;
    use crate::face_resolution::resolve_derived_face;
    use crate::render::load_renderer;

    fn atom(contents: &str, face: Face) -> StyledAtom {
        StyledAtom {
            face,
            contents: contents.to_string(),
        }
    }

    #[test]
    fn offscreen_png_dimensions_follow_cells_scale_and_wide_graphemes() {
        let config = AppConfig::default();
        let renderer = load_renderer(&config);
        let scale_factor = 2.0;
        let metrics = renderer.metrics(scale_factor);
        let image = render_canvas_image(
            &renderer,
            &[
                vec![atom("😀", Face::default())],
                vec![atom("  ", Face::default())],
            ],
            &config.theme.default,
            scale_factor,
            config.macos.color_space,
        )
        .unwrap();

        assert_eq!(image.width, (metrics.cell_width * 2.0).ceil() as usize);
        assert_eq!(image.height, (metrics.cell_height * 2.0).ceil() as usize);
        assert_eq!(image.rgba.len(), image.width * image.height * 4);
    }

    #[test]
    fn offscreen_png_is_opaque_and_preserves_theme_resolved_cell_backgrounds() {
        let config = AppConfig::default();
        let renderer = load_renderer(&config);
        let face = config.theme.cursor_block.clone();
        let resolved = resolve_derived_face(
            &config.theme.default,
            &face,
            super::super::FALLBACK_FG,
            super::super::FALLBACK_BG,
        );
        let image = render_canvas_image(
            &renderer,
            &[vec![atom(" ", face)]],
            &config.theme.default,
            1.0,
            config.macos.color_space,
        )
        .unwrap();

        assert!(image.rgba.chunks_exact(4).all(|pixel| pixel[3] == u8::MAX));
        assert_eq!(
            &image.rgba[..3],
            &[resolved.bg.r, resolved.bg.g, resolved.bg.b]
        );
    }

    #[test]
    fn foreground_only_overlays_are_transparent_but_explicit_backgrounds_occlude() {
        let config = AppConfig::default();
        let renderer = load_renderer(&config);
        let lower = Face {
            bg: "#ff0000".to_owned(),
            ..Face::default()
        };
        let foreground_only = Face {
            fg: "#0000ff".to_owned(),
            ..Face::default()
        };
        let explicit_background = Face {
            bg: "#00ff00".to_owned(),
            ..Face::default()
        };
        let lower_only = render_canvas_layers_image(
            &renderer,
            &[vec![vec![atom(" ", lower.clone())]]],
            &config.theme.default,
            1.0,
            config.macos.color_space,
        )
        .unwrap();

        let transparent = render_canvas_layers_image(
            &renderer,
            &[
                vec![vec![atom(" ", lower.clone())]],
                vec![vec![atom(".", foreground_only)]],
            ],
            &config.theme.default,
            1.0,
            config.macos.color_space,
        )
        .unwrap();
        assert_eq!(&transparent.rgba[..3], &lower_only.rgba[..3]);

        let upper_only = render_canvas_layers_image(
            &renderer,
            &[vec![vec![atom(" ", explicit_background.clone())]]],
            &config.theme.default,
            1.0,
            config.macos.color_space,
        )
        .unwrap();

        let occluded = render_canvas_layers_image(
            &renderer,
            &[
                vec![vec![atom(" ", lower)]],
                vec![vec![atom(".", explicit_background)]],
            ],
            &config.theme.default,
            1.0,
            config.macos.color_space,
        )
        .unwrap();
        assert_eq!(&occluded.rgba[..3], &upper_only.rgba[..3]);
    }

    #[test]
    fn encoded_png_declares_the_configured_color_space_and_matches_rgba_pixels() {
        let config = AppConfig::default();
        let renderer = load_renderer(&config);
        for color_space in [MacosColorSpace::Srgb, MacosColorSpace::P3] {
            let image = render_canvas_image(
                &renderer,
                &[vec![atom("x", Face::default())]],
                &config.theme.default,
                1.0,
                color_space,
            )
            .unwrap();
            let mut decoder = png::Decoder::new(Cursor::new(&image.png));
            decoder.set_transformations(png::Transformations::IDENTITY);
            let mut reader = decoder.read_info().unwrap();
            let info = reader.info();
            assert_eq!(info.width as usize, image.width);
            assert_eq!(info.height as usize, image.height);
            match color_space {
                MacosColorSpace::Srgb => assert!(info.srgb.is_some()),
                MacosColorSpace::P3 => {
                    assert!(info.srgb.is_none());
                    assert!(info.gama_chunk.is_some());
                    assert!(info.chrm_chunk.is_some());
                }
            }
            let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
            let output = reader.next_frame(&mut decoded).unwrap();
            assert_eq!(&decoded[..output.buffer_size()], image.rgba);
        }
    }
}
