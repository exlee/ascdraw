use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use winit::keyboard::ModifiersState;

use crate::editor::Editor;
use crate::face_resolution::{Rgba, UnderlineStyle, resolve_derived_face};
use crate::layout::PADDING;
use crate::model::{Face, StyledAtom};
use crate::render::{FALLBACK_BG, FALLBACK_FG};
use crate::selection::TextRectangle;
use crate::toolbar::{ToolbarSpan, toolbar_border_spans, toolbar_bottom_border_spans};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotspotClick {
    Ignore,
    Consume,
    Paste { box_width: usize },
}

pub(crate) fn toolbar_box_width(viewport_width: usize, cell_width: f32) -> usize {
    (viewport_width.saturating_sub(PADDING * 2) as f32 / cell_width.max(1.0)) as usize
}

pub(crate) fn toolbar_hotspot_at(
    x: f64,
    y: f64,
    viewport_width: usize,
    cell_width: f32,
    cell_height: f32,
    top_padding: f32,
) -> Option<usize> {
    let box_width = toolbar_box_width(viewport_width, cell_width);
    if box_width < 2 {
        return None;
    }
    let left = PADDING as f32 + (box_width - 1) as f32 * cell_width.max(1.0);
    let right = left + cell_width.max(1.0);
    let bottom = top_padding + cell_height;
    (x >= left as f64 && x < right as f64 && y >= top_padding as f64 && y < bottom as f64)
        .then_some(box_width)
}

fn command_click_for_platform(modifiers: ModifiersState, macos: bool) -> bool {
    if macos {
        modifiers.super_key()
    } else {
        modifiers.control_key()
    }
}

pub(crate) fn hotspot_click(hotspot: Option<usize>, modifiers: ModifiersState) -> HotspotClick {
    let Some(box_width) = hotspot else {
        return HotspotClick::Ignore;
    };
    if command_click_for_platform(modifiers, cfg!(target_os = "macos")) {
        HotspotClick::Paste { box_width }
    } else {
        HotspotClick::Consume
    }
}

pub(crate) fn toolbar_atoms(spans: &[ToolbarSpan], state: &Editor) -> Vec<StyledAtom> {
    let mut atoms = Vec::new();
    for span in spans {
        let split = display_width_byte_index(&span.contents, span.bold_prefix);
        let (bold, normal) = span.contents.split_at(split);
        if !bold.is_empty() {
            let mut face = toolbar_span_face(span, state);
            face.attributes.push("bold".to_string());
            atoms.push(StyledAtom {
                face,
                contents: bold.to_string(),
            });
        }
        if !normal.is_empty() {
            atoms.push(StyledAtom {
                face: toolbar_span_face(span, state),
                contents: normal.to_string(),
            });
        }
    }
    atoms
}

pub(crate) fn styled_toolbar_snapshot(state: &Editor, box_width: usize) -> Option<TextRectangle> {
    if box_width == 0 {
        return None;
    }
    let mut span_rows = Vec::with_capacity(state.toolbar.rows_for_width(box_width));
    span_rows.push(toolbar_border_spans(box_width, true));
    for row in 0..state.toolbar.content_rows_for_width(box_width) {
        span_rows.push(state.boxed_toolbar_spans_for_width(row, box_width));
    }
    span_rows.push(toolbar_bottom_border_spans(box_width, 0, (0, 0)));

    let rows = span_rows
        .iter()
        .map(|spans| {
            toolbar_atoms(spans, state)
                .into_iter()
                .map(|mut atom| {
                    atom.face = concrete_toolbar_face(&state.grid.default_face, &atom.face);
                    atom
                })
                .flat_map(split_atom_graphemes)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    debug_assert!(rows.iter().all(|row| {
        row.iter()
            .map(|atom| UnicodeWidthStr::width(atom.contents.as_str()))
            .sum::<usize>()
            == box_width
    }));
    Some(TextRectangle {
        rows,
        width: box_width,
    })
}

fn split_atom_graphemes(atom: StyledAtom) -> Vec<StyledAtom> {
    atom.contents
        .graphemes(true)
        .map(|contents| StyledAtom {
            contents: contents.to_owned(),
            face: atom.face.clone(),
        })
        .collect()
}

fn toolbar_span_face(span: &ToolbarSpan, state: &Editor) -> Face {
    let mut face = if span.tooltip {
        state.theme.tooltip.clone()
    } else {
        Face::default()
    };
    if let Some(foreground) = &span.foreground {
        face.fg = foreground.clone();
    }
    face
}

fn concrete_toolbar_face(base: &Face, face: &Face) -> Face {
    let resolved = resolve_derived_face(base, face, FALLBACK_FG, FALLBACK_BG);
    let mut attributes = Vec::new();
    match resolved.underline_style {
        Some(UnderlineStyle::Straight) => attributes.push("underline".to_owned()),
        Some(UnderlineStyle::Curly) => attributes.push("curly_underline".to_owned()),
        Some(UnderlineStyle::Double) => attributes.push("double_underline".to_owned()),
        None => {}
    }
    for (enabled, name) in [
        (resolved.blink, "blink"),
        (resolved.bold, "bold"),
        (resolved.italic, "italic"),
        (resolved.strikethrough, "strikethrough"),
    ] {
        if enabled {
            attributes.push(name.to_owned());
        }
    }
    Face {
        fg: rgb_hex(resolved.fg),
        bg: rgb_hex(resolved.bg),
        underline: resolved
            .underline
            .map(rgb_hex)
            .unwrap_or_else(|| "default".to_owned()),
        attributes,
    }
}

fn rgb_hex(color: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn display_width_byte_index(text: &str, width: usize) -> usize {
    if width == 0 {
        return 0;
    }
    let mut used = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let character_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
        if used + character_width > width {
            break;
        }
        used += character_width;
        end = index + character.len_utf8();
        if used == width {
            break;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppConfig, CursorMode};
    use crate::model::ColorId;
    use crate::toolbar::{MainMode, ToggleKind, ToolbarAction};

    fn row_text(row: &[StyledAtom]) -> String {
        row.iter().map(|atom| atom.contents.as_str()).collect()
    }

    #[test]
    fn hotspot_is_exactly_the_rendered_top_right_corner_cell() {
        let cell_width = 8;
        let cell_height = 16;
        let top = 7;
        let viewport_width = PADDING * 2 + 20 * cell_width;
        let left = PADDING + 19 * cell_width;

        assert_eq!(
            toolbar_hotspot_at(
                left as f64,
                top as f64,
                viewport_width,
                cell_width as f32,
                cell_height as f32,
                top as f32,
            ),
            Some(20)
        );
        assert_eq!(
            toolbar_hotspot_at(
                (left + cell_width - 1) as f64,
                (top + cell_height - 1) as f64,
                viewport_width,
                cell_width as f32,
                cell_height as f32,
                top as f32,
            ),
            Some(20)
        );
        for (x, y) in [
            (left - 1, top),
            (left + cell_width, top),
            (left, top - 1),
            (left, top + cell_height),
        ] {
            assert_eq!(
                toolbar_hotspot_at(
                    x as f64,
                    y as f64,
                    viewport_width,
                    cell_width as f32,
                    cell_height as f32,
                    top as f32,
                ),
                None
            );
        }
    }

    #[test]
    fn hotspot_click_consumes_plain_clicks_and_gates_each_platform_command_modifier() {
        for macos in [false, true] {
            let command = if macos {
                ModifiersState::SUPER
            } else {
                ModifiersState::CONTROL
            };
            let other = if macos {
                ModifiersState::CONTROL
            } else {
                ModifiersState::SUPER
            };
            assert!(command_click_for_platform(command, macos));
            for modifiers in [
                command | ModifiersState::SHIFT,
                command | ModifiersState::ALT,
                command | other,
            ] {
                assert!(command_click_for_platform(modifiers, macos));
            }
            for modifiers in [ModifiersState::empty(), other] {
                assert!(!command_click_for_platform(modifiers, macos));
            }
        }
        assert_eq!(
            hotspot_click(Some(40), ModifiersState::empty()),
            HotspotClick::Consume
        );
        let platform_command = if cfg!(target_os = "macos") {
            ModifiersState::SUPER
        } else {
            ModifiersState::CONTROL
        };
        for modifiers in [
            platform_command,
            platform_command | ModifiersState::SHIFT,
            platform_command | ModifiersState::ALT,
        ] {
            assert_eq!(
                hotspot_click(Some(40), modifiers),
                HotspotClick::Paste { box_width: 40 }
            );
        }
        let other_only = if cfg!(target_os = "macos") {
            ModifiersState::CONTROL
        } else {
            ModifiersState::SUPER
        };
        assert_eq!(hotspot_click(Some(40), other_only), HotspotClick::Consume);
        assert_eq!(
            hotspot_click(None, ModifiersState::CONTROL),
            HotspotClick::Ignore
        );
    }

    #[test]
    fn styled_snapshot_keeps_box_dimensions_theme_bold_and_palette_faces() {
        let config = AppConfig::default();
        let mut theme = config.theme.clone();
        theme.default.underline = config.theme.selection.fg.clone();
        let mut state = Editor::new(&theme, "test");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode,)));
        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode,)));
        let snapshot = styled_toolbar_snapshot(&state, 64).unwrap();

        assert_eq!(snapshot.width, 64);
        assert_eq!(snapshot.rows.len(), state.toolbar.rows_for_width(64));
        assert_eq!(row_text(&snapshot.rows[0]), format!("┌{}┐", "─".repeat(62)));
        assert_eq!(
            row_text(snapshot.rows.last().unwrap()),
            format!("└{}┘", "─".repeat(62))
        );
        assert!(snapshot.rows.iter().all(|row| {
            row.iter()
                .map(|atom| UnicodeWidthStr::width(atom.contents.as_str()))
                .sum::<usize>()
                == 64
        }));
        let text = snapshot
            .rows
            .iter()
            .map(|row| row_text(row))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Lyrs"));
        assert!(text.contains("Clrs"));
        assert!(text.contains("8.1. α × ▪"), "{text}");
        assert!(text.contains("9.1. ■"));
        let structural = snapshot
            .rows
            .iter()
            .flatten()
            .find(|atom| {
                atom.contents == "L" && atom.face.attributes.iter().any(|name| name == "bold")
            })
            .unwrap();
        let expected = resolve_derived_face(
            &theme.default,
            &Face {
                attributes: vec!["bold".to_owned()],
                ..Face::default()
            },
            FALLBACK_FG,
            FALLBACK_BG,
        );
        assert_eq!(structural.face.fg, rgb_hex(expected.fg));
        assert_eq!(structural.face.bg, rgb_hex(expected.bg));
        assert_eq!(
            structural.face.underline,
            expected
                .underline
                .map(rgb_hex)
                .unwrap_or_else(|| Face::default().underline)
        );
        assert!(expected.underline.is_some());
        assert_eq!(
            structural.face.attributes.iter().any(|name| name == "bold"),
            expected.bold
        );
        assert_eq!(
            structural
                .face
                .attributes
                .iter()
                .any(|name| name == "italic"),
            expected.italic
        );
        assert_eq!(
            structural
                .face
                .attributes
                .iter()
                .any(|name| name == "strikethrough"),
            expected.strikethrough
        );

        let palette = snapshot
            .rows
            .iter()
            .filter(|row| {
                let text = row_text(row);
                text.contains("9.1.") || text.contains("9.2.")
            })
            .flatten()
            .filter(|atom| atom.contents.contains('■'))
            .collect::<Vec<_>>();
        assert_eq!(palette.len(), ColorId::COUNT);
        for (index, atom) in palette.into_iter().enumerate() {
            assert_eq!(atom.face.fg, ColorId(index as u8).hex().unwrap());
        }
        assert_eq!(state.cursor_mode, CursorMode::MoveDraw);
    }
}
