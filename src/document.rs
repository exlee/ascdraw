use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::canvas::{LayerMap, LayerStack};
use crate::editor::PersistedLayer;
use crate::layout::ViewportOffset;
#[cfg(test)]
use crate::model::StyledAtom;
use crate::model::{Atom, Coord, Face, LayerId};
use crate::toolbar::DurableMenuSelections;

const DOCUMENT_VERSION: u32 = 3;
const RECENT_DOCUMENT_LIMIT: usize = 3;

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RecentDocuments {
    #[serde(default)]
    files: Vec<PathBuf>,
}

impl RecentDocuments {
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    pub fn record(&mut self, path: PathBuf) {
        self.files.retain(|candidate| candidate != &path);
        self.files.insert(0, path);
        self.files.truncate(RECENT_DOCUMENT_LIMIT);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub canvas: LayerStack,
    pub menu_selections: Option<DurableMenuSelections>,
    pub position: Option<CanvasPosition>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct CanvasPosition {
    pub cursor: Coord,
    pub canvas_origin: Coord,
    pub viewport: ViewportOffset,
    #[serde(default)]
    pub zoom: i32,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct SparseDocument {
    version: u32,
    layers: Vec<SparseLayer>,
    active_layer: LayerId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    menu_selections: Option<DurableMenuSelections>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<CanvasPosition>,
}

#[derive(Deserialize, Serialize)]
struct SparseLayer {
    id: LayerId,
    visible: bool,
    cells: Vec<SparseCell>,
}

#[derive(Deserialize, Serialize)]
struct SparseCell {
    line: i16,
    column: i16,
    face: Face,
    atom: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line_data: Option<crate::canvas::LineData>,
}

impl Document {
    pub fn new(
        layers: Vec<PersistedLayer>,
        active_layer: LayerId,
        menu_selections: Option<DurableMenuSelections>,
        position: Option<CanvasPosition>,
    ) -> Result<Self> {
        let maps = layers
            .into_iter()
            .map(|layer| LayerMap::from_dense(layer.id, layer.visible, &layer.lines))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            canvas: LayerStack::with_active(maps, active_layer, true)?,
            menu_selections,
            position,
        })
    }
}

pub fn load(path: &Path) -> Result<Option<Document>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let legacy_path = path.with_extension("toml");
            match fs::read_to_string(&legacy_path) {
                Ok(contents) => contents,
                Err(legacy_error) if legacy_error.kind() == ErrorKind::NotFound => return Ok(None),
                Err(legacy_error) => {
                    return Err(legacy_error)
                        .with_context(|| format!("failed to read {}", legacy_path.display()));
                }
            }
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let value = serde_json::from_str::<serde_json::Value>(&contents).ok();
    if value
        .as_ref()
        .and_then(|value| value.get("version"))
        .and_then(serde_json::Value::as_u64)
        == Some(u64::from(DOCUMENT_VERSION))
    {
        let sparse: SparseDocument = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        return sparse_document(sparse).map(Some);
    }
    super::legacy_loader::load_document(&contents)
        .with_context(|| format!("failed to load legacy document {}", path.display()))
        .map(Some)
}

fn sparse_document(sparse: SparseDocument) -> Result<Document> {
    if sparse.layers.is_empty() || sparse.layers.len() > crate::model::MAX_LAYERS {
        bail!("invalid sparse layer count");
    }
    let mut layers = Vec::with_capacity(sparse.layers.len());
    for layer in sparse.layers {
        let mut map = LayerMap::new(layer.id, layer.visible);
        for cell in layer.cells {
            let atom = Atom::new(cell.atom)?;
            map.set_at(cell.column, cell.line, atom, &cell.face)?;
            map.set_line_data(cell.column, cell.line, cell.line_data);
        }
        layers.push(map);
    }
    Ok(Document {
        canvas: LayerStack::with_active(layers, sparse.active_layer, true)?,
        menu_selections: sparse.menu_selections,
        position: sparse.position,
    })
}

pub fn save(
    path: &Path,
    canvas: &LayerStack,
    menu_selections: &DurableMenuSelections,
    position: CanvasPosition,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let contents = contents(canvas, menu_selections, position)?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

pub fn contents(
    canvas: &LayerStack,
    menu_selections: &DurableMenuSelections,
    position: CanvasPosition,
) -> Result<String> {
    let layers = canvas
        .layers()
        .iter()
        .map(|layer| {
            let cells = layer
                .rows()
                .iter()
                .flat_map(|(&line, row)| {
                    row.iter().map(move |(&column, data)| SparseCell {
                        line,
                        column,
                        face: data.face.as_ref().clone(),
                        atom: data.atom.contents().to_owned(),
                        line_data: data.line.clone(),
                    })
                })
                .collect();
            SparseLayer {
                id: layer.id,
                visible: layer.visible,
                cells,
            }
        })
        .collect();
    serde_json::to_string_pretty(&SparseDocument {
        version: DOCUMENT_VERSION,
        layers,
        active_layer: canvas.active_id(),
        menu_selections: Some(menu_selections.clone()),
        position: Some(position),
    })
    .context("failed to serialize sparse document")
}

pub fn default_path() -> PathBuf {
    default_path_with_env(|name| std::env::var_os(name), std::env::temp_dir())
}

pub fn recent_path() -> PathBuf {
    default_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("recent-documents.json")
}

pub fn load_recent(path: &Path) -> Result<RecentDocuments> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let legacy_path = path.with_extension("toml");
            match fs::read_to_string(&legacy_path) {
                Ok(contents) => {
                    return toml::from_str(&contents)
                        .with_context(|| format!("failed to parse {}", legacy_path.display()));
                }
                Err(legacy_error) if legacy_error.kind() == ErrorKind::NotFound => {
                    return Ok(RecentDocuments::default());
                }
                Err(legacy_error) => {
                    return Err(legacy_error)
                        .with_context(|| format!("failed to read {}", legacy_path.display()));
                }
            }
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    serde_json::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn save_recent(path: &Path, recent: &RecentDocuments) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let contents =
        serde_json::to_string_pretty(recent).context("failed to serialize recent documents")?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

fn default_path_with_env(env_var: impl Fn(&str) -> Option<OsString>, temp_dir: PathBuf) -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(home) = env_var("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("ascdraw")
            .join("document.json");
    }

    #[cfg(target_os = "windows")]
    if let Some(app_data) = env_var("APPDATA") {
        return PathBuf::from(app_data)
            .join("ascdraw")
            .join("document.json");
    }

    if let Some(data_home) = env_var("XDG_DATA_HOME") {
        return PathBuf::from(data_home)
            .join("ascdraw")
            .join("document.json");
    }
    if let Some(home) = env_var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("ascdraw")
            .join("document.json");
    }
    temp_dir.join("ascdraw").join("document.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(layers: &[PersistedLayer]) -> LayerStack {
        let maps = layers
            .iter()
            .map(|layer| LayerMap::from_dense(layer.id, layer.visible, &layer.lines).unwrap())
            .collect();
        LayerStack::new(maps, true).unwrap()
    }

    #[test]
    fn sparse_json_round_trip_and_canonical_deletion() {
        let selections = crate::toolbar::ToolbarState::default().durable_selections();
        let layers = [PersistedLayer {
            id: LayerId(0),
            visible: true,
            lines: vec![vec![
                StyledAtom {
                    face: Face::default(),
                    contents: "x".to_owned(),
                },
                StyledAtom {
                    face: Face::default(),
                    contents: " ".to_owned(),
                },
            ]],
        }];
        let position = CanvasPosition {
            cursor: Coord::default(),
            canvas_origin: Coord::default(),
            viewport: ViewportOffset::default(),
            zoom: 0,
        };
        let serialized = contents(&canvas(&layers), &selections, position).unwrap();
        assert!(serialized.contains("\"version\": 3"));
        assert_eq!(serialized.matches("\"atom\"").count(), 1);
        let sparse: SparseDocument = serde_json::from_str(&serialized).unwrap();
        let loaded = sparse_document(sparse).unwrap();
        assert_eq!(loaded.canvas.layers()[0].to_dense()[0][0].contents, "x");
    }

    #[test]
    fn sparse_write_rejects_wide_atoms() {
        let layers = [PersistedLayer {
            id: LayerId(0),
            visible: true,
            lines: vec![vec![StyledAtom {
                face: Face::default(),
                contents: "界".to_owned(),
            }]],
        }];
        assert!(LayerMap::from_dense(LayerId(0), true, &layers[0].lines).is_err());
    }
}
