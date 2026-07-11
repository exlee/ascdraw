use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::Atom;

const DOCUMENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Document {
    version: u32,
    pub lines: Vec<Vec<Atom>>,
}

impl Document {
    pub fn new(lines: Vec<Vec<Atom>>) -> Self {
        Self {
            version: DOCUMENT_VERSION,
            lines,
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

pub fn save(path: &Path, lines: &[Vec<Atom>]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let contents = toml::to_string_pretty(&Document::new(lines.to_vec()))
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

        save(&path, &lines).unwrap();
        let loaded = load(&path).unwrap().unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(loaded.lines, lines);
    }
}
