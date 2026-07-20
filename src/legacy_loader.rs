use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::canvas::LayerStack;
use crate::dense_exchange;
use crate::document::{CanvasPosition, Document};
use crate::model::{LayerId, StyledAtom};
use crate::toolbar::DurableMenuSelections;

#[derive(Deserialize)]
struct LegacyDocument {
    version: u32,
    #[serde(default)]
    lines: Vec<Vec<StyledAtom>>,
    #[serde(default)]
    layers: Vec<LegacyLayer>,
    #[serde(default, rename = "active-layer")]
    active_layer: Option<LayerId>,
    #[serde(default, rename = "menu-selections")]
    menu_selections: Option<DurableMenuSelections>,
    #[serde(default)]
    position: Option<CanvasPosition>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct LegacyLayer {
    pub id: LayerId,
    pub visible: bool,
    pub lines: Vec<Vec<StyledAtom>>,
}

pub fn load_document(contents: &str) -> Result<Document> {
    let legacy: LegacyDocument = serde_json::from_str(contents)
        .or_else(|_| toml::from_str(contents))
        .context("document is neither sparse JSON nor supported legacy TOML/JSON")?;
    if !matches!(legacy.version, 1 | 2) {
        bail!("unsupported legacy document version {}", legacy.version);
    }
    let mut layers = if legacy.version == 1 {
        vec![LegacyLayer {
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
    let canvas = into_canvas(layers, legacy.active_layer.unwrap_or(LayerId(0)))?;
    Ok(Document::from_legacy(
        canvas,
        legacy.menu_selections,
        legacy.position,
    ))
}

pub(crate) fn into_canvas(layers: Vec<LegacyLayer>, active_layer: LayerId) -> Result<LayerStack> {
    let maps = layers
        .into_iter()
        .map(|layer| dense_exchange::from_dense(layer.id, layer.visible, &layer.lines))
        .collect::<Result<Vec<_>>>()?;
    LayerStack::with_active(maps, active_layer, true)
}

fn normalize_lines(lines: Vec<Vec<StyledAtom>>) -> Result<Vec<Vec<StyledAtom>>> {
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
                    Ok(StyledAtom { face, contents })
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
        assert_eq!(
            crate::test_support::dense_layer(&loaded.canvas.layers()[0])[0].len(),
            3
        );

        let wide = serde_json::json!({
            "version": 1,
            "lines": [[{"face": Face::default(), "contents": "界"}]]
        });
        assert!(load_document(&wide.to_string()).is_err());
    }
}
