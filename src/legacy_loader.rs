use anyhow::{Context, Result, bail};
use serde::Deserialize;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::document::{CanvasPosition, Document};
use crate::editor::PersistedLayer;
use crate::model::{Atom, LayerId};
use crate::toolbar::DurableMenuSelections;

#[derive(Deserialize)]
struct LegacyDocument {
    version: u32,
    #[serde(default)]
    lines: Vec<Vec<Atom>>,
    #[serde(default)]
    layers: Vec<PersistedLayer>,
    #[serde(default, rename = "active-layer")]
    active_layer: Option<LayerId>,
    #[serde(default, rename = "menu-selections")]
    menu_selections: Option<DurableMenuSelections>,
    #[serde(default)]
    position: Option<CanvasPosition>,
}

pub fn load_document(contents: &str) -> Result<Document> {
    let legacy: LegacyDocument = serde_json::from_str(contents)
        .or_else(|_| toml::from_str(contents))
        .context("document is neither sparse JSON nor supported legacy TOML/JSON")?;
    if !matches!(legacy.version, 1 | 2) {
        bail!("unsupported legacy document version {}", legacy.version);
    }
    let mut layers = if legacy.version == 1 {
        vec![PersistedLayer {
            id: LayerId(0),
            visible: true,
            lines: legacy.lines,
        }]
    } else {
        legacy.layers
    };
    for layer in &mut layers {
        layer.lines = normalize_lines(std::mem::take(&mut layer.lines))?;
    }
    Document::new(
        layers,
        legacy.active_layer.unwrap_or(LayerId(0)),
        legacy.menu_selections,
        legacy.position,
    )
}

fn normalize_lines(lines: Vec<Vec<Atom>>) -> Result<Vec<Vec<Atom>>> {
    lines
        .into_iter()
        .map(|row| {
            row.into_iter()
                .flat_map(|atom| {
                    UnicodeSegmentation::graphemes(atom.contents.as_str(), true)
                        .map(move |grapheme| (atom.face.clone(), grapheme.to_owned()))
                        .collect::<Vec<_>>()
                })
                .map(|(face, contents)| {
                    if UnicodeWidthStr::width(contents.as_str()) != 1 {
                        bail!("legacy atom {contents:?} has display width other than 1");
                    }
                    Ok(Atom { face, contents })
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Face;

    #[test]
    fn converts_multigrapheme_legacy_atoms_and_rejects_wide_cells() {
        let document = serde_json::json!({
            "version": 1,
            "lines": [[{"face": Face::default(), "contents": "abc"}]]
        });
        let loaded = load_document(&document.to_string()).unwrap();
        assert_eq!(loaded.canvas.layers()[0].to_dense()[0].len(), 3);

        let wide = serde_json::json!({
            "version": 1,
            "lines": [[{"face": Face::default(), "contents": "界"}]]
        });
        assert!(load_document(&wide.to_string()).is_err());
    }
}
