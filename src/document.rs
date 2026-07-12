use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::Atom;
use crate::toolbar::DurableMenuSelections;

const DOCUMENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Document {
    version: u32,
    pub lines: Vec<Vec<Atom>>,
    #[serde(
        default,
        rename = "menu-selections",
        skip_serializing_if = "Option::is_none"
    )]
    pub menu_selections: Option<DurableMenuSelections>,
}

impl Document {
    pub fn new(lines: Vec<Vec<Atom>>, menu_selections: Option<DurableMenuSelections>) -> Self {
        Self {
            version: DOCUMENT_VERSION,
            lines,
            menu_selections,
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
    let document: Document =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    anyhow::ensure!(
        document.version == DOCUMENT_VERSION,
        "unsupported document version {} in {}",
        document.version,
        path.display()
    );
    Ok(Some(document))
}

pub fn save(
    path: &Path,
    lines: &[Vec<Atom>],
    menu_selections: &DurableMenuSelections,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let contents = toml::to_string_pretty(&Document::new(
        lines.to_vec(),
        Some(menu_selections.clone()),
    ))
    .context("failed to serialize document")?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

pub fn default_path() -> PathBuf {
    default_path_with_env(|name| std::env::var_os(name), std::env::temp_dir())
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
        save(&path, &lines, &menu_selections).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(loaded.lines, lines);
        assert_eq!(loaded.menu_selections, Some(menu_selections));
    }

    #[test]
    fn version_one_document_without_menu_selections_preserves_canvas() {
        let atom = Atom {
            face: Face::default(),
            contents: "x".to_owned(),
        };
        let serialized_atom =
            toml::to_string(&Document::new(vec![vec![atom.clone()]], None)).unwrap();
        assert!(!serialized_atom.contains("menu-selections"));

        let document: Document = toml::from_str(&serialized_atom).unwrap();
        assert_eq!(document.lines, vec![vec![atom]]);
        assert_eq!(document.menu_selections, None);
    }

    #[test]
    fn serialized_menu_payload_contains_no_transient_toolbar_state() {
        let selections = crate::toolbar::ToolbarState::default().durable_selections();
        let contents =
            toml::to_string_pretty(&Document::new(vec![Vec::new()], Some(selections))).unwrap();

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
