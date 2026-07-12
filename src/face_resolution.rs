use skia_safe::Color;

use crate::model::Face;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 0xff }
    }

    pub fn to_color(self) -> Color {
        Color::from_argb(self.a, self.r, self.g, self.b)
    }

    fn dimmed(self) -> Self {
        Self {
            r: self.r / 2,
            g: self.g / 2,
            b: self.b / 2,
            a: self.a,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaceAttributes {
    pub underline: bool,
    pub curly_underline: bool,
    pub double_underline: bool,
    pub reverse: bool,
    pub blink: bool,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub final_fg: bool,
    pub final_bg: bool,
    pub final_attr: bool,
}

impl FaceAttributes {
    fn from_names(attributes: &[String]) -> Self {
        let mut parsed = Self::default();
        for attribute in attributes {
            match attribute.as_str() {
                "underline" => parsed.underline = true,
                "curly_underline" => parsed.curly_underline = true,
                "double_underline" => parsed.double_underline = true,
                "reverse" => parsed.reverse = true,
                "blink" => parsed.blink = true,
                "bold" => parsed.bold = true,
                "dim" => parsed.dim = true,
                "italic" => parsed.italic = true,
                "strikethrough" => parsed.strikethrough = true,
                "final_fg" => parsed.final_fg = true,
                "final_bg" => parsed.final_bg = true,
                "final_attr" => parsed.final_attr = true,
                _ => {}
            }
        }
        parsed
    }

    fn merge(base: Self, face: Self) -> Self {
        if face.final_attr {
            let mut merged = face;
            merged.final_fg |= base.final_fg;
            merged.final_bg |= base.final_bg;
            merged.final_attr |= base.final_attr;
            return merged;
        }

        if base.final_attr {
            return base;
        }

        Self {
            underline: base.underline || face.underline,
            curly_underline: base.curly_underline || face.curly_underline,
            double_underline: base.double_underline || face.double_underline,
            reverse: base.reverse || face.reverse,
            blink: base.blink || face.blink,
            bold: base.bold || face.bold,
            dim: base.dim || face.dim,
            italic: base.italic || face.italic,
            strikethrough: base.strikethrough || face.strikethrough,
            final_fg: base.final_fg || face.final_fg,
            final_bg: base.final_bg || face.final_bg,
            final_attr: base.final_attr || face.final_attr,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaceColor {
    Default,
    Rgba(Rgba),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FaceSpec {
    fg: FaceColor,
    bg: FaceColor,
    underline: FaceColor,
    attributes: FaceAttributes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnderlineStyle {
    Straight,
    Curly,
    Double,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedFace {
    pub fg: Rgba,
    pub bg: Rgba,
    pub underline: Option<Rgba>,
    pub underline_style: Option<UnderlineStyle>,
    pub reverse: bool,
    pub blink: bool,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub strikethrough: bool,
}

pub fn resolve_root_face(face: &Face, fallback_fg: Rgba, fallback_bg: Rgba) -> ResolvedFace {
    resolve_from_spec(parse_face(face), fallback_fg, fallback_bg)
}

pub fn resolve_derived_face(
    base: &Face,
    face: &Face,
    fallback_fg: Rgba,
    fallback_bg: Rgba,
) -> ResolvedFace {
    let merged = merge_face_specs(parse_face(base), parse_face(face));
    resolve_from_spec(merged, fallback_fg, fallback_bg)
}

fn parse_face(face: &Face) -> FaceSpec {
    FaceSpec {
        fg: parse_color(&face.fg),
        bg: parse_color(&face.bg),
        underline: parse_color(&face.underline),
        attributes: FaceAttributes::from_names(&face.attributes),
    }
}

fn merge_face_specs(base: FaceSpec, face: FaceSpec) -> FaceSpec {
    FaceSpec {
        fg: choose_color(
            base.fg,
            face.fg,
            base.attributes.final_fg,
            face.attributes.final_fg,
        ),
        bg: choose_color(
            base.bg,
            face.bg,
            base.attributes.final_bg,
            face.attributes.final_bg,
        ),
        underline: choose_color(base.underline, face.underline, false, false),
        attributes: FaceAttributes::merge(base.attributes, face.attributes),
    }
}

fn choose_color(base: FaceColor, face: FaceColor, base_final: bool, face_final: bool) -> FaceColor {
    if face_final {
        return face;
    }
    if base_final {
        return base;
    }
    if face == FaceColor::Default {
        return base;
    }

    match (base, face) {
        (FaceColor::Rgba(base), FaceColor::Rgba(face)) if face.a != 0xff => {
            FaceColor::Rgba(alpha_blend(base, face))
        }
        _ => face,
    }
}

fn resolve_from_spec(spec: FaceSpec, fallback_fg: Rgba, fallback_bg: Rgba) -> ResolvedFace {
    let mut fg = concretize(spec.fg, fallback_fg);
    let mut bg = concretize(spec.bg, fallback_bg);

    if spec.attributes.reverse {
        std::mem::swap(&mut fg, &mut bg);
    }

    if spec.attributes.dim {
        fg = fg.dimmed();
    }

    ResolvedFace {
        fg,
        bg,
        underline: match spec.underline {
            FaceColor::Default => None,
            FaceColor::Rgba(color) => Some(color),
        },
        underline_style: if spec.attributes.double_underline {
            Some(UnderlineStyle::Double)
        } else if spec.attributes.curly_underline {
            Some(UnderlineStyle::Curly)
        } else if spec.attributes.underline {
            Some(UnderlineStyle::Straight)
        } else {
            None
        },
        reverse: spec.attributes.reverse,
        blink: spec.attributes.blink,
        bold: spec.attributes.bold,
        dim: spec.attributes.dim,
        italic: spec.attributes.italic,
        strikethrough: spec.attributes.strikethrough,
    }
}

fn concretize(color: FaceColor, fallback: Rgba) -> Rgba {
    match color {
        FaceColor::Default => fallback,
        FaceColor::Rgba(color) => color,
    }
}

fn alpha_blend(base: Rgba, color: Rgba) -> Rgba {
    let blend = |base_channel: u8, face_channel: u8| {
        let blended = (base_channel as u32 * (255 - color.a as u32)
            + face_channel as u32 * color.a as u32)
            / 255;
        blended.min(255) as u8
    };
    let alpha = color.a as u32 + base.a as u32 * (255 - color.a as u32) / 255;
    Rgba {
        r: blend(base.r, color.r),
        g: blend(base.g, color.g),
        b: blend(base.b, color.b),
        a: alpha.min(255) as u8,
    }
}

fn named_color(color: &str) -> Option<Rgba> {
    Some(match color {
        "black" => Rgba::rgb(0x00, 0x00, 0x00),
        "red" => Rgba::rgb(0xcd, 0x00, 0x00),
        "green" => Rgba::rgb(0x00, 0xcd, 0x00),
        "yellow" => Rgba::rgb(0xcd, 0xcd, 0x00),
        "blue" => Rgba::rgb(0x00, 0x00, 0xee),
        "magenta" => Rgba::rgb(0xcd, 0x00, 0xcd),
        "cyan" => Rgba::rgb(0x00, 0xcd, 0xcd),
        "white" => Rgba::rgb(0xff, 0xff, 0xff),
        "orangered" => Rgba::rgb(0xff, 0x45, 0x00),
        "gold" => Rgba::rgb(0xff, 0xd7, 0x00),
        "darkblue" => Rgba::rgb(0x00, 0x00, 0x8b),
        "grey" | "gray" => Rgba::rgb(0x80, 0x80, 0x80),
        "bright-black" => Rgba::rgb(0x7f, 0x7f, 0x7f),
        "bright-red" => Rgba::rgb(0xff, 0x00, 0x00),
        "bright-green" => Rgba::rgb(0x00, 0xff, 0x00),
        "bright-yellow" => Rgba::rgb(0xff, 0xff, 0x00),
        "bright-blue" => Rgba::rgb(0x5c, 0x5c, 0xff),
        "bright-magenta" => Rgba::rgb(0xff, 0x00, 0xff),
        "bright-cyan" => Rgba::rgb(0x00, 0xff, 0xff),
        "bright-white" => Rgba::rgb(0xff, 0xff, 0xff),
        _ => return None,
    })
}

fn parse_color(color: &str) -> FaceColor {
    match color {
        "" | "default" => FaceColor::Default,
        value => named_color(value)
            .or_else(|| parse_prefixed_color(value))
            .map(FaceColor::Rgba)
            .unwrap_or(FaceColor::Default),
    }
}

fn parse_prefixed_color(value: &str) -> Option<Rgba> {
    if let Some(rgb) = value.strip_prefix("rgb:").and_then(parse_hex_color) {
        return Some(rgb);
    }
    if let Some(rgb) = value.strip_prefix("rgba:").and_then(parse_rgba_color) {
        return Some(rgb);
    }
    parse_hex_color(value)
}

fn parse_hex_color(value: &str) -> Option<Rgba> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgba::rgb(r, g, b))
}

fn parse_rgba_color(value: &str) -> Option<Rgba> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 8 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
    Some(Rgba { r, g, b, a })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(fg: &str, bg: &str, underline: &str, attributes: &[&str]) -> Face {
        Face {
            fg: fg.into(),
            bg: bg.into(),
            underline: underline.into(),
            attributes: attributes
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
        }
    }

    #[test]
    fn derived_face_inherits_default_colors_from_base() {
        let resolved = resolve_derived_face(
            &face("white", "blue", "default", &[]),
            &face("default", "default", "default", &[]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert_eq!(resolved.fg, Rgba::rgb(0xff, 0xff, 0xff));
        assert_eq!(resolved.bg, Rgba::rgb(0x00, 0x00, 0xee));
    }

    #[test]
    fn derived_face_overrides_explicit_colors() {
        let resolved = resolve_derived_face(
            &face("white", "blue", "default", &[]),
            &face("black", "yellow", "default", &[]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert_eq!(resolved.fg, Rgba::rgb(0x00, 0x00, 0x00));
        assert_eq!(resolved.bg, Rgba::rgb(0xcd, 0xcd, 0x00));
    }

    #[test]
    fn final_fg_blocks_later_foreground_override() {
        let resolved = resolve_derived_face(
            &face("white", "blue", "default", &["final_fg"]),
            &face("black", "yellow", "default", &[]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert_eq!(resolved.fg, Rgba::rgb(0xff, 0xff, 0xff));
        assert_eq!(resolved.bg, Rgba::rgb(0xcd, 0xcd, 0x00));
    }

    #[test]
    fn final_attr_blocks_later_attribute_override() {
        let resolved = resolve_derived_face(
            &face("white", "blue", "default", &["bold", "final_attr"]),
            &face("black", "yellow", "default", &["italic"]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert!(resolved.bold);
        assert!(!resolved.italic);
    }

    #[test]
    fn alpha_rgba_blends_against_base_rgb() {
        let resolved = resolve_derived_face(
            &face("#000000", "#000000", "default", &[]),
            &face("rgba:ffffff80", "default", "default", &[]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert_eq!(resolved.fg, Rgba::rgb(0x80, 0x80, 0x80));
    }

    #[test]
    fn reverse_and_dim_apply_after_color_resolution() {
        let resolved = resolve_derived_face(
            &face("white", "blue", "default", &[]),
            &face("default", "default", "default", &["reverse", "dim"]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert_eq!(resolved.bg, Rgba::rgb(0xff, 0xff, 0xff));
        assert_eq!(resolved.fg, Rgba::rgb(0x00, 0x00, 0x77));
    }

    #[test]
    fn underline_color_and_style_resolve_independently() {
        let resolved = resolve_derived_face(
            &face("white", "blue", "default", &[]),
            &face("default", "default", "red", &["double_underline"]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert_eq!(resolved.underline, Some(Rgba::rgb(0xcd, 0x00, 0x00)));
        assert_eq!(resolved.underline_style, Some(UnderlineStyle::Double));
    }

    #[test]
    fn bright_named_colors_do_not_fall_back_to_default() {
        let resolved = resolve_root_face(
            &face("bright-black", "bright-white", "default", &[]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert_eq!(resolved.fg, Rgba::rgb(0x7f, 0x7f, 0x7f));
        assert_eq!(resolved.bg, Rgba::rgb(0xff, 0xff, 0xff));
    }

    #[test]
    fn bundled_theme_named_colors_resolve_exactly() {
        for (name, expected) in [
            ("orangered", Rgba::rgb(0xff, 0x45, 0x00)),
            ("gold", Rgba::rgb(0xff, 0xd7, 0x00)),
            ("darkblue", Rgba::rgb(0x00, 0x00, 0x8b)),
            ("grey", Rgba::rgb(0x80, 0x80, 0x80)),
        ] {
            assert_eq!(
                resolve_root_face(
                    &face(name, "default", "default", &[]),
                    Rgba::rgb(1, 2, 3),
                    Rgba::rgb(4, 5, 6),
                )
                .fg,
                expected
            );
        }
    }

    #[test]
    fn unknown_named_colors_still_fall_back_to_default() {
        let resolved = resolve_root_face(
            &face("not-a-color", "also-not-a-color", "default", &[]),
            Rgba::rgb(1, 2, 3),
            Rgba::rgb(4, 5, 6),
        );

        assert_eq!(resolved.fg, Rgba::rgb(1, 2, 3));
        assert_eq!(resolved.bg, Rgba::rgb(4, 5, 6));
    }
}
