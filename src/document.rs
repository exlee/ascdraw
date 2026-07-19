use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::editor::{PersistedLayer, compact_blank_runs, compacted_blank_runs};
use crate::layout::ViewportOffset;
use crate::model::{Atom, Coord, LayerId};
use crate::toolbar::DurableMenuSelections;

const DOCUMENT_VERSION: u32 = 2;
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Document {
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<Vec<Atom>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<PersistedLayer>,
    #[serde(
        default,
        rename = "active-layer",
        skip_serializing_if = "Option::is_none"
    )]
    pub active_layer: Option<LayerId>,
    #[serde(
        default,
        rename = "menu-selections",
        skip_serializing_if = "Option::is_none"
    )]
    pub menu_selections: Option<DurableMenuSelections>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

impl Document {
    pub fn new(
        layers: Vec<PersistedLayer>,
        active_layer: LayerId,
        menu_selections: Option<DurableMenuSelections>,
        position: Option<CanvasPosition>,
    ) -> Self {
        Self {
            version: DOCUMENT_VERSION,
            lines: Vec::new(),
            layers,
            active_layer: Some(active_layer),
            menu_selections,
            position,
        }
    }
}

pub fn load(path: &Path) -> Result<Option<Document>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let mut document: Document =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    anyhow::ensure!(
        matches!(document.version, 1 | DOCUMENT_VERSION),
        "unsupported document version {} in {}",
        document.version,
        path.display()
    );
    if document.version == 1 {
        compact_blank_runs(&mut document.lines);
        document.layers = vec![PersistedLayer {
            id: LayerId(0),
            visible: true,
            lines: std::mem::take(&mut document.lines),
        }];
        document.active_layer = Some(LayerId(0));
    } else {
        for layer in &mut document.layers {
            compact_blank_runs(&mut layer.lines);
        }
    }
    if document
        .menu_selections
        .as_ref()
        .is_some_and(|selections| !selections.active_color().is_valid())
    {
        anyhow::bail!("saved document active color is outside the supported palette");
    }
    Ok(Some(document))
}

pub fn save(
    path: &Path,
    layers: &[PersistedLayer],
    active_layer: LayerId,
    menu_selections: &DurableMenuSelections,
    position: CanvasPosition,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let contents = contents(layers, active_layer, menu_selections, position)?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

pub fn contents(
    layers: &[PersistedLayer],
    active_layer: LayerId,
    menu_selections: &DurableMenuSelections,
    position: CanvasPosition,
) -> Result<String> {
    let layers = layers
        .iter()
        .map(|layer| PersistedLayer {
            id: layer.id,
            visible: layer.visible,
            lines: compacted_blank_runs(&layer.lines),
        })
        .collect::<Vec<_>>();
    toml::to_string_pretty(&Document::new(
        layers,
        active_layer,
        Some(menu_selections.clone()),
        Some(position),
    ))
    .context("failed to serialize document")
}

pub fn default_path() -> PathBuf {
    default_path_with_env(|name| std::env::var_os(name), std::env::temp_dir())
}

pub fn recent_path() -> PathBuf {
    default_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("recent-documents.toml")
}

pub fn load_recent(path: &Path) -> Result<RecentDocuments> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(RecentDocuments::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn save_recent(path: &Path, recent: &RecentDocuments) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let contents =
        toml::to_string_pretty(recent).context("failed to serialize recent documents")?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

fn default_path_with_env(env_var: impl Fn(&str) -> Option<OsString>, temp_dir: PathBuf) -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(home) = env_var("HOME") {
        return PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("ascdraw")
            .join("document.toml");
    }

    #[cfg(target_os = "windows")]
    if let Some(app_data) = env_var("APPDATA") {
        return PathBuf::from(app_data)
            .join("ascdraw")
            .join("document.toml");
    }

    if let Some(data_home) = env_var("XDG_DATA_HOME") {
        return PathBuf::from(data_home)
            .join("ascdraw")
            .join("document.toml");
    }
    if let Some(home) = env_var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("ascdraw")
            .join("document.toml");
    }
    temp_dir.join("ascdraw").join("document.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Face;

    #[test]
    fn recent_documents_are_most_recent_first_deduplicated_capped_and_persistent() {
        let mut recent = RecentDocuments::default();
        for path in [
            "one.toml",
            "two.toml",
            "three.toml",
            "four.toml",
            "two.toml",
        ] {
            recent.record(PathBuf::from(path));
        }
        assert_eq!(
            recent.files(),
            [
                PathBuf::from("two.toml"),
                PathBuf::from("four.toml"),
                PathBuf::from("three.toml"),
            ]
        );

        let path = std::env::temp_dir().join(format!(
            "ascdraw-recent-{}-{}.toml",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        save_recent(&path, &recent).unwrap();
        assert_eq!(load_recent(&path).unwrap(), recent);
        let serialized = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_file(path);
        assert!(serialized.contains("files = ["));
    }

    #[test]
    fn document_round_trips_atoms() {
        let path = std::env::temp_dir().join(format!(
            "ascdraw-document-{}-{}.toml",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let lines = vec![vec![Atom {
            face: Face::default(),
            contents: "┌".to_string(),
        }]];

        let menu_selections = crate::toolbar::ToolbarState::default().durable_selections();
        let layers = [PersistedLayer {
            id: LayerId(0),
            visible: true,
            lines: lines.clone(),
        }];
        let position = CanvasPosition {
            cursor: Coord { line: 3, column: 5 },
            canvas_origin: Coord { line: 2, column: 1 },
            viewport: ViewportOffset { x: -17, y: 23 },
            zoom: 4,
        };
        save(&path, &layers, LayerId(0), &menu_selections, position).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(loaded.layers[0].lines, lines);
        assert_eq!(loaded.active_layer, Some(LayerId(0)));
        assert_eq!(loaded.menu_selections, Some(menu_selections));
        assert_eq!(loaded.position, Some(position));
    }

    #[test]
    fn version_two_round_trips_layer_order_visibility_active_layer_and_faces() {
        let path = std::env::temp_dir().join(format!(
            "ascdraw-layer-document-{}-{}.toml",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let configured_face = crate::app::ThemeConfig::default().selection;
        let layers = vec![
            PersistedLayer {
                id: LayerId(0),
                visible: false,
                lines: vec![vec![Atom {
                    face: Face::default(),
                    contents: "a".to_owned(),
                }]],
            },
            PersistedLayer {
                id: LayerId(1),
                visible: true,
                lines: vec![vec![Atom {
                    face: configured_face.clone(),
                    contents: "b".to_owned(),
                }]],
            },
        ];
        let mut toolbar = crate::toolbar::ToolbarState::default();
        toolbar.apply_action(crate::toolbar::ToolbarAction::Toggle(
            crate::toolbar::ToggleKind::MultiLayerMode,
        ));
        toolbar.apply_action(crate::toolbar::ToolbarAction::Toggle(
            crate::toolbar::ToggleKind::MultiColorMode,
        ));
        toolbar.apply_action(crate::toolbar::ToolbarAction::SelectColor(
            crate::model::ColorId(12),
        ));
        let selections = toolbar.durable_selections();

        save(
            &path,
            &layers,
            LayerId(1),
            &selections,
            CanvasPosition {
                cursor: Coord::default(),
                canvas_origin: Coord::default(),
                viewport: ViewportOffset::default(),
                zoom: 0,
            },
        )
        .unwrap();
        let loaded = load(&path).unwrap().unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(loaded.layers, layers);
        assert_eq!(loaded.active_layer, Some(LayerId(1)));
        assert_eq!(loaded.menu_selections, Some(selections));
        assert_eq!(loaded.layers[1].lines[0][0].face, configured_face);
    }

    #[test]
    fn version_two_save_compacts_styled_trailing_spaces_without_changing_text() {
        let path = std::env::temp_dir().join(format!(
            "ascdraw-compact-document-{}-{}.toml",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let face = Face {
            bg: "selection".to_owned(),
            ..Face::default()
        };
        let lines = vec![vec![
            Atom {
                face: Face::default(),
                contents: "x".to_owned(),
            },
            Atom {
                face: face.clone(),
                contents: " ".to_owned(),
            },
            Atom {
                face: face.clone(),
                contents: " ".to_owned(),
            },
        ]];
        let selections = crate::toolbar::ToolbarState::default().durable_selections();

        save(
            &path,
            &[PersistedLayer {
                id: LayerId(0),
                visible: true,
                lines,
            }],
            LayerId(0),
            &selections,
            CanvasPosition {
                cursor: Coord::default(),
                canvas_origin: Coord::default(),
                viewport: ViewportOffset::default(),
                zoom: 0,
            },
        )
        .unwrap();
        let loaded = load(&path).unwrap().unwrap();
        let serialized = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_file(path);

        assert!(serialized.contains("version = 2"));
        assert_eq!(loaded.layers[0].lines[0].len(), 2);
        assert_eq!(loaded.layers[0].lines[0][1].contents, "  ");
        assert_eq!(loaded.layers[0].lines[0][1].face, face);
        assert_eq!(
            loaded.layers[0].lines[0]
                .iter()
                .map(|atom| atom.contents.as_str())
                .collect::<String>(),
            "x  "
        );
    }

    #[test]
    fn version_one_document_without_menu_selections_migrates_to_the_base_layer() {
        #[derive(Serialize)]
        struct VersionOneDocument<'a> {
            version: u32,
            lines: &'a [Vec<Atom>],
        }
        let atom = Atom {
            face: Face::default(),
            contents: "x".to_owned(),
        };
        let lines = vec![vec![atom.clone()]];
        let serialized_atom = toml::to_string(&VersionOneDocument {
            version: 1,
            lines: &lines,
        })
        .unwrap();
        assert!(!serialized_atom.contains("menu-selections"));

        let mut document: Document = toml::from_str(&serialized_atom).unwrap();
        assert_eq!(document.lines, vec![vec![atom]]);
        compact_blank_runs(&mut document.lines);
        document.layers = vec![PersistedLayer {
            id: LayerId(0),
            visible: true,
            lines: std::mem::take(&mut document.lines),
        }];
        assert_eq!(document.layers[0].id, LayerId(0));
    }

    #[test]
    fn serialized_menu_payload_contains_no_transient_toolbar_state() {
        let selections = crate::toolbar::ToolbarState::default().durable_selections();
        let contents = toml::to_string_pretty(&Document::new(
            vec![PersistedLayer {
                id: LayerId(0),
                visible: true,
                lines: vec![Vec::new()],
            }],
            LayerId(0),
            Some(selections),
            None,
        ))
        .unwrap();

        assert!(contents.contains("[menu-selections]"));
        for transient in [
            "shortcut-prefix",
            "export-open",
            "active-export-category",
            "pending-export-action",
            "cursor-mode",
            "\nselection =",
            "tooltip",
        ] {
            assert!(!contents.contains(transient));
        }
    }
}
