use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::canvas::{LayerMap, LayerStack};
use crate::layout::ViewportOffset;
use crate::model::{Atom, Coord, Face, LayerId};
use crate::objects::ObjectStore;
use crate::toolbar::DurableMenuSelections;

const DOCUMENT_VERSION: u32 = 4;
const LEGACY_SPARSE_DOCUMENT_VERSION: u32 = 3;
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
    pub objects: ObjectStore,
    needs_migration: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct CanvasPosition {
    pub cursor: Coord,
    pub viewport: ViewportOffset,
    #[serde(default)]
    pub zoom: i32,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct SparseDocument {
    version: u32,
    faces: Vec<Face>,
    layers: Vec<SparseLayer>,
    active_layer: LayerId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    menu_selections: Option<DurableMenuSelections>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<CanvasPosition>,
    /// Object store whose cells name faces by `face_id`, like layer cells.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    objects: serde_json::Value,
}

#[derive(Deserialize, Serialize)]
struct SparseLayer {
    id: LayerId,
    visible: bool,
    cells: Vec<SparseCell>,
}

/// One saved cell. `p` is `[column, line]` and `v` the atom; version 4
/// documents saved before that wrote `line`, `column` and `atom`, which are
/// still read.
#[derive(Deserialize, Serialize)]
#[serde(try_from = "RawSparseCell")]
struct SparseCell {
    p: [i16; 2],
    v: String,
    #[serde(skip_serializing_if = "is_zero")]
    face_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    line_data: Option<crate::canvas::LineData>,
}

#[derive(Deserialize)]
struct RawSparseCell {
    #[serde(default)]
    p: Option<[i16; 2]>,
    #[serde(default)]
    line: Option<i16>,
    #[serde(default)]
    column: Option<i16>,
    #[serde(alias = "atom")]
    v: String,
    #[serde(default)]
    face_id: u32,
    #[serde(default)]
    line_data: Option<crate::canvas::LineData>,
}

impl TryFrom<RawSparseCell> for SparseCell {
    type Error = String;

    fn try_from(raw: RawSparseCell) -> std::result::Result<Self, String> {
        let p = match (raw.p, raw.column, raw.line) {
            (Some(p), _, _) => p,
            (None, Some(column), Some(line)) => [column, line],
            _ => return Err("cell has no position".to_owned()),
        };
        Ok(Self {
            p,
            v: raw.v,
            face_id: raw.face_id,
            line_data: raw.line_data,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct LegacySparseDocument {
    version: u32,
    layers: Vec<LegacySparseLayer>,
    active_layer: LayerId,
    #[serde(default)]
    menu_selections: Option<DurableMenuSelections>,
    #[serde(default)]
    position: Option<CanvasPosition>,
}

#[derive(Deserialize)]
struct LegacySparseLayer {
    id: LayerId,
    visible: bool,
    cells: Vec<LegacySparseCell>,
}

#[derive(Deserialize)]
struct LegacySparseCell {
    line: i16,
    column: i16,
    face: Face,
    atom: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line_data: Option<crate::canvas::LineData>,
}

impl Document {
    pub(crate) fn from_legacy(
        canvas: LayerStack,
        menu_selections: Option<DurableMenuSelections>,
        position: Option<CanvasPosition>,
    ) -> Self {
        Self {
            canvas,
            menu_selections,
            position,
            objects: ObjectStore::default(),
            needs_migration: true,
        }
    }

    pub fn needs_migration(&self) -> bool {
        self.needs_migration
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
    parse_contents(&contents)
        .with_context(|| format!("failed to load document {}", path.display()))
        .map(Some)
}

pub(crate) fn parse_contents(contents: &str) -> Result<Document> {
    let value = serde_json::from_str::<serde_json::Value>(contents).ok();
    let version = value
        .as_ref()
        .and_then(|value| value.get("version"))
        .and_then(serde_json::Value::as_u64);
    match version {
        Some(version) if version == u64::from(DOCUMENT_VERSION) => {
            let sparse: SparseDocument =
                serde_json::from_str(contents).context("failed to parse sparse document")?;
            sparse_document(sparse)
        }
        Some(version) if version == u64::from(LEGACY_SPARSE_DOCUMENT_VERSION) => {
            let sparse: LegacySparseDocument =
                serde_json::from_str(contents).context("failed to parse sparse v3 document")?;
            legacy_sparse_document(sparse)
        }
        _ => super::legacy_loader::load_document(contents),
    }
}

fn sparse_document(sparse: SparseDocument) -> Result<Document> {
    if sparse.version != DOCUMENT_VERSION {
        bail!("invalid sparse document version");
    }
    if sparse.layers.is_empty() || sparse.layers.len() > crate::model::MAX_LAYERS {
        bail!("invalid sparse layer count");
    }
    let mut layers = Vec::with_capacity(sparse.layers.len());
    for layer in sparse.layers {
        let mut map = LayerMap::new(layer.id, layer.visible);
        for cell in layer.cells {
            let face = face_by_id(&sparse.faces, cell.face_id)?;
            let [column, line] = cell.p;
            let atom = Atom::new(cell.v)?;
            map.set_at_untracked(column, line, atom, face)?;
            map.set_line_data(column, line, cell.line_data);
        }
        layers.push(map);
    }
    let mut objects = sparse.objects;
    for_each_object_cell(&mut objects, |cell| {
        if cell.contains_key("face") {
            return Ok(());
        }
        let face_id = match cell.remove("face_id") {
            Some(value) => serde_json::from_value(value).context("invalid object face ID")?,
            None if sparse.faces.is_empty() => return Ok(()),
            None => 0,
        };
        let face = face_by_id(&sparse.faces, face_id)?;
        cell.insert("face".to_owned(), serde_json::to_value(face)?);
        Ok(())
    })?;
    let objects = if objects.is_null() {
        ObjectStore::default()
    } else {
        serde_json::from_value(objects).context("failed to parse objects")?
    };
    Ok(Document {
        canvas: LayerStack::with_active(layers, sparse.active_layer, true)?,
        menu_selections: sparse.menu_selections,
        position: sparse.position,
        objects,
        needs_migration: false,
    })
}

fn face_by_id(faces: &[Face], face_id: u32) -> Result<&Face> {
    faces
        .get(usize::try_from(face_id).context("face ID exceeds platform range")?)
        .with_context(|| format!("invalid face ID {face_id}"))
}

/// Calls `visit` with every saved object cell: definition cells and instance
/// overlays.
fn for_each_object_cell(
    objects: &mut serde_json::Value,
    mut visit: impl FnMut(&mut serde_json::Map<String, serde_json::Value>) -> Result<()>,
) -> Result<()> {
    for (list, key) in [("definitions", "cells"), ("instances", "overlay")] {
        let Some(entries) = objects
            .get_mut(list)
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        for entry in entries {
            let Some(cells) = entry.get_mut(key).and_then(serde_json::Value::as_array_mut) else {
                continue;
            };
            for cell in cells {
                if let Some(cell) = cell.as_object_mut() {
                    visit(cell)?;
                }
            }
        }
    }
    Ok(())
}

/// Faces in first-use order, so each one is saved once.
#[derive(Default)]
struct FaceTable {
    faces: Vec<Face>,
    ids: HashMap<Face, u32>,
}

impl FaceTable {
    fn id(&mut self, face: &Face) -> Result<u32> {
        if let Some(&face_id) = self.ids.get(face) {
            return Ok(face_id);
        }
        let face_id = u32::try_from(self.faces.len()).context("too many document faces")?;
        self.faces.push(face.clone());
        self.ids.insert(face.clone(), face_id);
        Ok(face_id)
    }
}

fn legacy_sparse_document(sparse: LegacySparseDocument) -> Result<Document> {
    if sparse.version != LEGACY_SPARSE_DOCUMENT_VERSION {
        bail!("invalid legacy sparse document version");
    }
    if sparse.layers.is_empty() || sparse.layers.len() > crate::model::MAX_LAYERS {
        bail!("invalid sparse layer count");
    }
    let mut layers = Vec::with_capacity(sparse.layers.len());
    for layer in sparse.layers {
        let mut map = LayerMap::new(layer.id, layer.visible);
        for cell in layer.cells {
            let atom = Atom::new(cell.atom)?;
            map.set_at_untracked(cell.column, cell.line, atom, &cell.face)?;
            map.set_line_data(cell.column, cell.line, cell.line_data);
        }
        layers.push(map);
    }
    Ok(Document {
        canvas: LayerStack::with_active(layers, sparse.active_layer, true)?,
        menu_selections: sparse.menu_selections,
        position: sparse.position,
        objects: ObjectStore::default(),
        needs_migration: true,
    })
}

pub fn save(
    path: &Path,
    canvas: &LayerStack,
    objects: &ObjectStore,
    menu_selections: &DurableMenuSelections,
    position: CanvasPosition,
    cell_size: (f32, f32),
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let contents = contents(canvas, objects, menu_selections, position, cell_size)?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

pub fn contents(
    canvas: &LayerStack,
    objects: &ObjectStore,
    menu_selections: &DurableMenuSelections,
    mut position: CanvasPosition,
    cell_size: (f32, f32),
) -> Result<String> {
    let (origin_x, origin_y) = canvas
        .bounds()
        .map_or((0, 0), |bounds| (bounds.min_x, bounds.min_y));
    position.cursor = shifted_coord(position.cursor, origin_x, origin_y);
    let mut objects = objects.clone();
    objects.session = None;
    for instance in &mut objects.instances {
        instance.origin = shifted_coord(instance.origin, origin_x, origin_y);
        let mut painted = crate::objects::Cells::default();
        for (coord, cell) in instance.painted.iter() {
            painted.insert(shifted_coord(coord, origin_x, origin_y), cell.clone());
        }
        instance.painted = painted;
    }
    position
        .viewport
        .translate_canvas(-i64::from(origin_x), -i64::from(origin_y), cell_size);

    let mut faces = FaceTable::default();
    let mut layers = Vec::with_capacity(canvas.layers().len());
    for layer in canvas.layers() {
        let mut cells = Vec::new();
        for (&line, row) in layer.rows() {
            for (&column, data) in row {
                let face_id = faces.id(data.face.as_ref())?;
                cells.push(SparseCell {
                    p: [
                        normalized_key(column, origin_x)?,
                        normalized_key(line, origin_y)?,
                    ],
                    v: data.atom.contents().to_owned(),
                    face_id,
                    line_data: data.line.clone(),
                });
            }
        }
        layers.push(SparseLayer {
            id: layer.id,
            visible: layer.visible,
            cells,
        });
    }
    let objects = if objects.is_empty() {
        serde_json::Value::Null
    } else {
        let mut value = serde_json::to_value(&objects).context("failed to serialize objects")?;
        for_each_object_cell(&mut value, |cell| {
            let face = match cell.remove("face") {
                Some(face) => serde_json::from_value(face).context("invalid object face")?,
                None => Face::default(),
            };
            let face_id = faces.id(&face)?;
            if face_id != 0 {
                cell.insert("face_id".to_owned(), face_id.into());
            }
            Ok(())
        })?;
        value
    };
    let document = serde_json::to_value(SparseDocument {
        version: DOCUMENT_VERSION,
        faces: faces.faces,
        layers,
        active_layer: canvas.active_id(),
        menu_selections: Some(menu_selections.clone()),
        position: Some(position),
        objects,
    })
    .context("failed to serialize sparse document")?;
    let mut out = String::new();
    write_json(&mut out, &document, 0);
    Ok(out)
}

/// Widest container, indentation included, written on one line.
const INLINE_JSON_WIDTH: usize = 100;

/// Pretty JSON that keeps short containers, such as cells, on one line.
fn write_json(out: &mut String, value: &serde_json::Value, indent: usize) {
    let compact = value.to_string();
    if indent + compact.len() <= INLINE_JSON_WIDTH {
        out.push_str(&compact);
        return;
    }
    let newline = |out: &mut String, indent: usize| {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', indent));
    };
    match value {
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                newline(out, indent + 2);
                write_json(out, item, indent + 2);
            }
            newline(out, indent);
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            out.push('{');
            for (index, (key, item)) in map.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                newline(out, indent + 2);
                out.push_str(&serde_json::Value::from(key.as_str()).to_string());
                out.push_str(": ");
                write_json(out, item, indent + 2);
            }
            newline(out, indent);
            out.push('}');
        }
        _ => out.push_str(&compact),
    }
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn normalized_key(value: i16, origin: i16) -> Result<i16> {
    i16::try_from(i32::from(value) - i32::from(origin)).context("normalized coordinate exceeds i16")
}

fn shifted_coord(coord: Coord, origin_x: i16, origin_y: i16) -> Coord {
    fn shift(value: i16, origin: i16) -> i16 {
        value.saturating_sub(origin)
    }
    Coord {
        line: shift(coord.line, origin_y),
        column: shift(coord.column, origin_x),
    }
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
#[path = "inline_tests/document_tests.rs"]
mod tests;
