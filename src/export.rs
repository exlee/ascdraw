use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::MacosColorSpace;
use crate::editor::{Editor, PersistedLayer};
use crate::layout::{ViewportOffset, VisibleCanvasCells};
use crate::model::{Atom, Face, LayerId};
use crate::render::{CanvasImage, Renderer, render_canvas_image, render_canvas_layers_image};
use crate::selection::{CanvasRegion, CanvasSelection, region_atoms};
use crate::toolbar::DurableMenuSelections;

const PROJECT_FORMAT: &str = "ascdraw";
const PROJECT_VERSION: u32 = 2;
const LEGACY_SELECTION_VERSION: u32 = 1;
const MAX_PROJECT_COORDINATE: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportAction {
    ClipboardTxt,
    ClipboardPng,
    SaveTxt,
    SaveJson,
    SavePng,
    LoadTxt,
    LoadJson,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportOutcome {
    Unchanged,
    Cancelled,
    DocumentLoaded,
    ProjectLoaded,
    CanvasCleared,
}

pub trait ExportPlatform {
    fn set_clipboard_text(&mut self, text: &str) -> Result<()>;
    fn clipboard_text(&mut self) -> Result<String>;
    fn choose_save_path(&mut self, kind: FileKind) -> Option<PathBuf>;
    fn choose_open_path(&mut self, kind: FileKind) -> Option<PathBuf>;
    fn render_canvas_image(
        &mut self,
        _lines: &[Vec<Atom>],
        _default_face: &Face,
    ) -> Result<CanvasImage> {
        bail!("PNG rendering is unavailable")
    }
    fn render_canvas_layers_image(
        &mut self,
        layers: &[Vec<Vec<Atom>>],
        default_face: &Face,
    ) -> Result<CanvasImage> {
        if let [lines] = layers {
            self.render_canvas_image(lines, default_face)
        } else {
            bail!("layered PNG rendering is unavailable")
        }
    }
    fn set_clipboard_image(&mut self, _image: &CanvasImage) -> Result<()> {
        bail!("PNG clipboard support is unavailable")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Txt,
    Json,
    Png,
}

pub struct NativeExportPlatform<'a> {
    png: Option<NativePngContext<'a>>,
}

struct NativePngContext<'a> {
    renderer: &'a Renderer,
    scale_factor: f64,
    color_space: MacosColorSpace,
}

impl NativeExportPlatform<'static> {
    pub fn text_only() -> Self {
        Self { png: None }
    }
}

impl<'a> NativeExportPlatform<'a> {
    pub fn with_png(
        renderer: &'a Renderer,
        scale_factor: f64,
        color_space: MacosColorSpace,
    ) -> Self {
        Self {
            png: Some(NativePngContext {
                renderer,
                scale_factor,
                color_space,
            }),
        }
    }
}

impl ExportPlatform for NativeExportPlatform<'_> {
    fn set_clipboard_text(&mut self, text: &str) -> Result<()> {
        arboard::Clipboard::new()
            .context("failed to open the system clipboard")?
            .set_text(text)
            .context("failed to copy text to the system clipboard")
    }

    fn clipboard_text(&mut self) -> Result<String> {
        arboard::Clipboard::new()
            .context("failed to open the system clipboard")?
            .get_text()
            .context("failed to read text from the system clipboard")
    }

    fn choose_save_path(&mut self, kind: FileKind) -> Option<PathBuf> {
        let (name, extension) = file_kind_details(kind);
        rfd::FileDialog::new()
            .add_filter(name, &[extension])
            .set_file_name(default_file_name(kind))
            .save_file()
    }

    fn choose_open_path(&mut self, kind: FileKind) -> Option<PathBuf> {
        let (name, extension) = file_kind_details(kind);
        rfd::FileDialog::new()
            .add_filter(name, &[extension])
            .pick_file()
    }

    fn render_canvas_image(
        &mut self,
        lines: &[Vec<Atom>],
        default_face: &Face,
    ) -> Result<CanvasImage> {
        let context = self.png.as_ref().context("PNG renderer is unavailable")?;
        render_canvas_image(
            context.renderer,
            lines,
            default_face,
            context.scale_factor,
            context.color_space,
        )
    }

    fn render_canvas_layers_image(
        &mut self,
        layers: &[Vec<Vec<Atom>>],
        default_face: &Face,
    ) -> Result<CanvasImage> {
        let context = self.png.as_ref().context("PNG renderer is unavailable")?;
        render_canvas_layers_image(
            context.renderer,
            layers,
            default_face,
            context.scale_factor,
            context.color_space,
        )
    }

    fn set_clipboard_image(&mut self, image: &CanvasImage) -> Result<()> {
        arboard::Clipboard::new()
            .context("failed to open the system clipboard")?
            .set_image(arboard::ImageData {
                width: image.width,
                height: image.height,
                bytes: Cow::Borrowed(&image.rgba),
            })
            .context("failed to copy PNG image to the system clipboard")
    }
}

pub fn copy_selection(state: &mut Editor, platform: &mut impl ExportPlatform) -> Result<()> {
    let text = selected_visible_text(state);
    platform.set_clipboard_text(&text)?;
    state.select_custom_stamp(&text);
    Ok(())
}

/// Copies the normalized selection before clearing it. Keeping the clipboard
/// write first makes a failed external operation an editor-state no-op.
pub fn cut_selection(state: &mut Editor, platform: &mut impl ExportPlatform) -> Result<bool> {
    platform.set_clipboard_text(&selected_visible_text(state))?;
    let before = state.edit_snapshot();
    state.clear_selection();
    Ok(!before.same_document(&state.edit_snapshot()))
}

pub fn paste_selection(state: &mut Editor, platform: &mut impl ExportPlatform) -> Result<bool> {
    let text = platform.clipboard_text()?;
    Ok(state.paste_text(&text))
}

fn file_kind_details(kind: FileKind) -> (&'static str, &'static str) {
    match kind {
        FileKind::Txt => ("Text", "txt"),
        FileKind::Json => ("JSON", "json"),
        FileKind::Png => ("PNG", "png"),
    }
}

fn default_file_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Txt => "selection.txt",
        FileKind::Json => "ascdraw.json",
        FileKind::Png => "ascdraw.png",
    }
}

pub fn perform(
    action: ExportAction,
    state: &mut Editor,
    viewport: &mut ViewportOffset,
    visible_canvas: VisibleCanvasCells,
    platform: &mut impl ExportPlatform,
) -> Result<ExportOutcome> {
    match action {
        ExportAction::ClipboardTxt => {
            platform.set_clipboard_text(&text_export(state, visible_canvas))?;
            Ok(ExportOutcome::Unchanged)
        }
        ExportAction::SaveTxt => {
            let Some(path) = platform.choose_save_path(FileKind::Txt) else {
                return Ok(ExportOutcome::Cancelled);
            };
            fs::write(&path, text_export(state, visible_canvas))
                .with_context(|| format!("failed to write {}", path.display()))?;
            Ok(ExportOutcome::Unchanged)
        }
        ExportAction::SaveJson => {
            let Some(path) = platform.choose_save_path(FileKind::Json) else {
                return Ok(ExportOutcome::Cancelled);
            };
            save_project_json(&path, state, *viewport)?;
            Ok(ExportOutcome::Unchanged)
        }
        ExportAction::ClipboardPng => {
            let layers = canvas_layers_for_export(state, visible_canvas);
            let image = platform.render_canvas_layers_image(&layers, &state.grid.default_face)?;
            platform.set_clipboard_image(&image)?;
            Ok(ExportOutcome::Unchanged)
        }
        ExportAction::SavePng => {
            let Some(path) = platform.choose_save_path(FileKind::Png) else {
                return Ok(ExportOutcome::Cancelled);
            };
            let layers = canvas_layers_for_export(state, visible_canvas);
            let image = platform.render_canvas_layers_image(&layers, &state.grid.default_face)?;
            fs::write(&path, &image.png)
                .with_context(|| format!("failed to write {}", path.display()))?;
            Ok(ExportOutcome::Unchanged)
        }
        ExportAction::LoadTxt => {
            let Some(path) = platform.choose_open_path(FileKind::Txt) else {
                return Ok(ExportOutcome::Cancelled);
            };
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            state.replace_canvas(lines_from_text(&text));
            Ok(ExportOutcome::DocumentLoaded)
        }
        ExportAction::LoadJson => {
            let Some(path) = platform.choose_open_path(FileKind::Json) else {
                return Ok(ExportOutcome::Cancelled);
            };
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            match project_from_json(&contents)? {
                LoadedJson::Project(project) => {
                    let mut staged = state.clone();
                    staged.restore_project(
                        project.layers,
                        project.active_layer,
                        project.cursor,
                        project.selection,
                        &project.menu_selections,
                    )?;
                    *state = staged;
                    *viewport = project.viewport;
                    return Ok(ExportOutcome::ProjectLoaded);
                }
                LoadedJson::Legacy(lines) => state.replace_canvas(lines),
            }
            Ok(ExportOutcome::DocumentLoaded)
        }
        ExportAction::Clear => {
            state.clear_canvas();
            Ok(ExportOutcome::CanvasCleared)
        }
    }
}

fn text_export(state: &Editor, visible_canvas: VisibleCanvasCells) -> String {
    atoms_text(&canvas_atoms_for_export(state, visible_canvas))
}

fn selected_visible_text(state: &Editor) -> String {
    let region = CanvasRegion::from_selection(state.selection_bounds());
    atoms_text(&flatten_visible_layers(&visible_layer_atoms(state, region)))
}

fn atoms_text(lines: &[Vec<Atom>]) -> String {
    lines
        .iter()
        .map(|row| {
            row.iter()
                .map(|atom| atom.contents.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn plain_text(state: &Editor) -> String {
    let views = state.layer_views();
    let height = views
        .iter()
        .map(|layer| layer.lines.len())
        .max()
        .unwrap_or(1);
    let width = views
        .iter()
        .flat_map(|layer| layer.lines.iter())
        .map(|line| display_width(line))
        .max()
        .unwrap_or(0);
    let rows = flatten_visible_layers(&visible_layer_atoms(
        state,
        CanvasRegion {
            left: 0,
            top: 0,
            width,
            height,
        },
    ));
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|atom| atom.contents.as_str())
                .collect::<String>()
                .trim_end_matches(' ')
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn canvas_region_for_export(
    state: &Editor,
    visible_canvas: VisibleCanvasCells,
) -> CanvasRegion {
    if state.selection.is_collapsed() {
        CanvasRegion {
            left: visible_canvas.origin.0,
            top: visible_canvas.origin.1,
            width: visible_canvas.columns,
            height: visible_canvas.rows,
        }
    } else {
        CanvasRegion::from_selection(state.selection_bounds())
    }
}

pub fn canvas_atoms_for_export(
    state: &Editor,
    visible_canvas: VisibleCanvasCells,
) -> Vec<Vec<Atom>> {
    flatten_visible_layers(&canvas_layers_for_export(state, visible_canvas))
}

pub fn canvas_layers_for_export(
    state: &Editor,
    visible_canvas: VisibleCanvasCells,
) -> Vec<Vec<Vec<Atom>>> {
    let region = canvas_region_for_export(state, visible_canvas);
    visible_layer_atoms(state, region)
}

fn visible_layer_atoms(state: &Editor, region: CanvasRegion) -> Vec<Vec<Vec<Atom>>> {
    state
        .layer_views()
        .into_iter()
        .filter(|layer| layer.visible)
        .map(|layer| region_atoms(layer.lines, region))
        .collect()
}

fn flatten_visible_layers(layers: &[Vec<Vec<Atom>>]) -> Vec<Vec<Atom>> {
    let height = layers.iter().map(Vec::len).max().unwrap_or(0);
    let width = layers
        .iter()
        .flat_map(|layer| layer.iter())
        .map(|line| display_width(line))
        .max()
        .unwrap_or(0);
    let mut flattened = Vec::with_capacity(height);
    for row in 0..height {
        let mut glyphs: Vec<Option<(Atom, usize)>> = vec![None; width];
        let mut owners = vec![None; width];
        for layer in layers {
            let Some(line) = layer.get(row) else {
                continue;
            };
            let mut column: usize = 0;
            for atom in line {
                for cluster in UnicodeSegmentation::graphemes(atom.contents.as_str(), true) {
                    let cluster_width = UnicodeWidthStr::width(cluster).max(1);
                    let end = column.saturating_add(cluster_width);
                    if end <= width && !cluster.chars().all(char::is_whitespace) {
                        let covered: std::collections::HashSet<usize> = owners[column..end]
                            .iter()
                            .flatten()
                            .copied()
                            .collect::<std::collections::HashSet<_>>();
                        for start in covered {
                            if let Some((_, old_width)) = glyphs[start].take() {
                                for owner in &mut owners[start..start + old_width] {
                                    *owner = None;
                                }
                            }
                        }
                        glyphs[column] = Some((
                            Atom {
                                face: atom.face.clone(),
                                contents: cluster.to_owned(),
                            },
                            cluster_width,
                        ));
                        for owner in &mut owners[column..end] {
                            *owner = Some(column);
                        }
                    }
                    column = end;
                }
            }
        }
        let mut line = Vec::new();
        let mut column = 0;
        while column < width {
            if let Some((atom, glyph_width)) = glyphs[column].take() {
                line.push(atom);
                column += glyph_width;
            } else {
                line.push(blank_atom());
                column += 1;
            }
        }
        flattened.push(line);
    }
    flattened
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct LegacySelectionDocument {
    version: u32,
    width: usize,
    height: usize,
    lines: Vec<Vec<Atom>>,
}

#[derive(Deserialize)]
struct NativeJsonDocument {
    lines: Vec<Vec<Atom>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
struct ProjectDocument {
    format: String,
    version: u32,
    canvas: ProjectCanvas,
    cursor: crate::model::Coord,
    selection: CanvasSelection,
    /// Signed renderer-pixel translation, round-tripped exactly when valid.
    viewport: ViewportOffset,
    menu_selections: DurableMenuSelections,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
struct ProjectCanvas {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rows: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    layers: Vec<PersistedLayer>,
    #[serde(
        default,
        rename = "active-layer",
        skip_serializing_if = "Option::is_none"
    )]
    active_layer: Option<LayerId>,
}

#[derive(Debug)]
struct RestoredProject {
    layers: Vec<PersistedLayer>,
    active_layer: LayerId,
    cursor: crate::model::Coord,
    selection: CanvasSelection,
    viewport: ViewportOffset,
    menu_selections: DurableMenuSelections,
}

#[derive(Debug)]
enum LoadedJson {
    Project(Box<RestoredProject>),
    Legacy(Vec<Vec<Atom>>),
}

fn project_document(state: &Editor, viewport: ViewportOffset) -> ProjectDocument {
    ProjectDocument {
        format: PROJECT_FORMAT.to_owned(),
        version: PROJECT_VERSION,
        canvas: ProjectCanvas {
            rows: Vec::new(),
            layers: state.persisted_layers(),
            active_layer: Some(state.active_layer_id()),
        },
        cursor: state.grid.cursor_pos,
        selection: state.selection,
        viewport,
        menu_selections: state.toolbar.durable_selections(),
    }
}

fn save_project_json(path: &Path, state: &Editor, viewport: ViewportOffset) -> Result<()> {
    let contents = serde_json::to_string_pretty(&project_document(state, viewport))
        .context("failed to serialize ascdraw project")?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

fn project_from_json(contents: &str) -> Result<LoadedJson> {
    let value: serde_json::Value =
        serde_json::from_str(contents).context("failed to parse canvas JSON")?;
    if value.get("format").is_some() {
        let document: ProjectDocument =
            serde_json::from_value(value).context("failed to parse ascdraw project JSON")?;
        return restore_project_document(document)
            .map(Box::new)
            .map(LoadedJson::Project);
    }
    if value.get("width").is_some() || value.get("height").is_some() {
        let document: LegacySelectionDocument =
            serde_json::from_value(value).context("failed to parse legacy selection JSON")?;
        if document.version != LEGACY_SELECTION_VERSION {
            bail!(
                "unsupported legacy selection JSON version {}",
                document.version
            );
        }
        if document.height != document.lines.len() {
            bail!(
                "legacy selection JSON height {} does not match {} rows",
                document.height,
                document.lines.len()
            );
        }
        return normalize_legacy_lines(document.lines, document.width).map(LoadedJson::Legacy);
    }
    let document: NativeJsonDocument =
        serde_json::from_value(value).context("failed to parse legacy canvas JSON")?;
    Ok(LoadedJson::Legacy(nonempty_lines(default_faced_lines(
        document.lines,
    ))))
}

fn restore_project_document(document: ProjectDocument) -> Result<RestoredProject> {
    if document.format != PROJECT_FORMAT {
        bail!("unsupported project format {:?}", document.format);
    }
    if !matches!(document.version, 1 | PROJECT_VERSION) {
        bail!("unsupported ascdraw project version {}", document.version);
    }
    validate_coordinate("cursor", document.cursor)?;
    validate_coordinate("selection anchor", document.selection.anchor())?;
    validate_coordinate("selection active", document.selection.active())?;
    if !document.menu_selections.active_color().is_valid() {
        bail!("project active color is outside the supported palette");
    }
    let (mut layers, active_layer) = if document.version == 1 {
        (
            vec![PersistedLayer {
                id: LayerId(0),
                visible: true,
                lines: nonempty_lines(
                    document
                        .canvas
                        .rows
                        .iter()
                        .map(|row| row_atoms(row))
                        .collect(),
                ),
            }],
            LayerId(0),
        )
    } else {
        let active_layer = document
            .canvas
            .active_layer
            .context("project has no active layer")?;
        validate_persisted_layers(&document.canvas.layers, active_layer)?;
        (document.canvas.layers, active_layer)
    };
    let active_lines = &mut layers
        .iter_mut()
        .find(|layer| layer.id == active_layer)
        .context("active layer is not present in the project")?
        .lines;
    for (name, coord) in [
        ("cursor", document.cursor),
        ("selection anchor", document.selection.anchor()),
        ("selection active", document.selection.active()),
    ] {
        pad_to_coordinate(active_lines, name, coord)?;
    }
    Ok(RestoredProject {
        layers,
        active_layer,
        cursor: document.cursor,
        selection: document.selection,
        viewport: document.viewport,
        menu_selections: document.menu_selections,
    })
}

fn validate_persisted_layers(layers: &[PersistedLayer], active_layer: LayerId) -> Result<()> {
    if layers.is_empty() || layers.len() > crate::model::MAX_LAYERS {
        bail!(
            "project must contain between 1 and {} layers",
            crate::model::MAX_LAYERS
        );
    }
    if layers[0].id != LayerId(0) {
        bail!("the base layer must be first");
    }
    let mut ids = std::collections::HashSet::new();
    for layer in layers {
        if !layer.id.is_valid() {
            bail!(
                "layer symbol id {} is outside the supported pool",
                layer.id.0
            );
        }
        if !ids.insert(layer.id) {
            bail!("layer symbol {} is duplicated", layer.id.symbol());
        }
    }
    if !ids.contains(&active_layer) {
        bail!("active layer is not present in the project");
    }
    Ok(())
}

fn validate_coordinate(name: &str, coord: crate::model::Coord) -> Result<()> {
    if coord.line > MAX_PROJECT_COORDINATE || coord.column > MAX_PROJECT_COORDINATE {
        bail!(
            "{name} ({}, {}) exceeds the safe project coordinate limit {MAX_PROJECT_COORDINATE}",
            coord.line,
            coord.column
        );
    }
    Ok(())
}

fn pad_to_coordinate(
    lines: &mut Vec<Vec<Atom>>,
    name: &str,
    coord: crate::model::Coord,
) -> Result<()> {
    while lines.len() <= coord.line {
        lines.push(Vec::new());
    }
    let line = &mut lines[coord.line];
    let mut column = 0;
    for atom in line.iter() {
        let width = atom_display_width(atom);
        if column < coord.column && coord.column < column.saturating_add(width) {
            bail!(
                "{name} column {} falls inside a wide grapheme on row {}",
                coord.column,
                coord.line
            );
        }
        column = column.saturating_add(width);
    }
    if column < coord.column {
        line.extend((column..coord.column).map(|_| blank_atom()));
    }
    Ok(())
}

fn normalize_legacy_lines(lines: Vec<Vec<Atom>>, width: usize) -> Result<Vec<Vec<Atom>>> {
    let lines = default_faced_lines(lines);
    let mut normalized = Vec::with_capacity(lines.len());
    for (row, line) in lines.into_iter().enumerate() {
        let actual_width = display_width(&line);
        if actual_width > width {
            bail!("selection JSON row {row} exceeds declared width {width}");
        }
        let mut line = line;
        line.extend((actual_width..width).map(|_| blank_atom()));
        normalized.push(line);
    }
    Ok(nonempty_lines(normalized))
}

fn default_faced_lines(lines: Vec<Vec<Atom>>) -> Vec<Vec<Atom>> {
    lines
        .into_iter()
        .map(|line| {
            line.into_iter()
                .flat_map(|atom| row_atoms(&atom.contents))
                .collect()
        })
        .collect()
}

fn row_atoms(row: &str) -> Vec<Atom> {
    UnicodeSegmentation::graphemes(row, true)
        .map(|contents| Atom {
            face: Face::default(),
            contents: contents.to_owned(),
        })
        .collect()
}

pub fn lines_from_text(text: &str) -> Vec<Vec<Atom>> {
    let mut lines: Vec<Vec<Atom>> = text
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            UnicodeSegmentation::graphemes(line, true)
                .map(|contents| Atom {
                    face: Face::default(),
                    contents: contents.to_string(),
                })
                .collect()
        })
        .collect();
    let width = lines
        .iter()
        .map(|line| display_width(line))
        .max()
        .unwrap_or(0);
    for line in &mut lines {
        let line_width = display_width(line);
        line.extend((line_width..width).map(|_| blank_atom()));
    }
    nonempty_lines(lines)
}

fn nonempty_lines(lines: Vec<Vec<Atom>>) -> Vec<Vec<Atom>> {
    if lines.is_empty() {
        vec![Vec::new()]
    } else {
        lines
    }
}

fn display_width(line: &[Atom]) -> usize {
    line.iter().map(atom_display_width).sum()
}

fn atom_display_width(atom: &Atom) -> usize {
    UnicodeWidthStr::width(atom.contents.as_str()).max(usize::from(!atom.contents.is_empty()))
}

fn blank_atom() -> Atom {
    Atom {
        face: Face::default(),
        contents: " ".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{EditHistory, HistorySnapshot};

    fn contents(line: &[Atom]) -> String {
        line.iter().map(|atom| atom.contents.as_str()).collect()
    }
    use crate::app::{CursorMode, ThemeConfig};
    use crate::model::Coord;
    use crate::toolbar::{MainMode, ToggleKind, ToolbarAction};

    #[test]
    fn plain_text_preserves_ragged_rows_and_a_trailing_newline_without_padding() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.replace_canvas(lines_from_text("one\nlonger\n"));

        assert_eq!(plain_text(&state), "one\nlonger\n");
    }

    #[derive(Default)]
    struct MockPlatform {
        clipboard: Option<String>,
        clipboard_image: Option<CanvasImage>,
        rendered_lines: Option<Vec<Vec<Atom>>>,
        rendered_layers: Option<Vec<Vec<Vec<Atom>>>>,
        image: Option<CanvasImage>,
        fail_clipboard_read: bool,
        fail_clipboard_write: bool,
        fail_image_render: bool,
        fail_image_write: bool,
        save: Option<PathBuf>,
        open: Option<PathBuf>,
    }

    impl ExportPlatform for MockPlatform {
        fn set_clipboard_text(&mut self, text: &str) -> Result<()> {
            if self.fail_clipboard_write {
                bail!("mock clipboard write failed");
            }
            self.clipboard = Some(text.to_string());
            Ok(())
        }
        fn clipboard_text(&mut self) -> Result<String> {
            if self.fail_clipboard_read {
                bail!("mock clipboard read failed");
            }
            Ok(self.clipboard.clone().unwrap_or_default())
        }
        fn choose_save_path(&mut self, _kind: FileKind) -> Option<PathBuf> {
            self.save.take()
        }
        fn choose_open_path(&mut self, _kind: FileKind) -> Option<PathBuf> {
            self.open.take()
        }
        fn render_canvas_image(
            &mut self,
            lines: &[Vec<Atom>],
            _default_face: &Face,
        ) -> Result<CanvasImage> {
            self.rendered_lines = Some(lines.to_vec());
            if self.fail_image_render {
                bail!("mock PNG render failed");
            }
            self.image.clone().context("mock PNG image is missing")
        }
        fn render_canvas_layers_image(
            &mut self,
            layers: &[Vec<Vec<Atom>>],
            _default_face: &Face,
        ) -> Result<CanvasImage> {
            self.rendered_layers = Some(layers.to_vec());
            if let [lines] = layers {
                self.rendered_lines = Some(lines.clone());
            }
            if self.fail_image_render {
                bail!("mock PNG render failed");
            }
            self.image.clone().context("mock PNG image is missing")
        }
        fn set_clipboard_image(&mut self, image: &CanvasImage) -> Result<()> {
            if self.fail_image_write {
                bail!("mock PNG clipboard write failed");
            }
            self.clipboard_image = Some(image.clone());
            Ok(())
        }
    }

    fn sample_image() -> CanvasImage {
        let rgba = vec![1, 2, 3, 255];
        CanvasImage {
            width: 1,
            height: 1,
            rgba,
            png: b"deterministic PNG bytes".to_vec(),
            color_space: MacosColorSpace::Srgb,
        }
    }

    fn state_with_selection() -> Editor {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("outside\n  ab  \n  cd  ");
        state.move_to(Coord { line: 1, column: 2 });
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Down);
        state
    }

    fn temp_path(extension: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "ascdraw-export-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            extension
        ))
    }

    fn perform_action(
        action: ExportAction,
        state: &mut Editor,
        platform: &mut MockPlatform,
    ) -> Result<ExportOutcome> {
        perform(
            action,
            state,
            &mut ViewportOffset::default(),
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 1,
                rows: 1,
            },
            platform,
        )
    }

    #[test]
    fn clipboard_receives_exact_selected_text_only() {
        let mut state = state_with_selection();
        let mut platform = MockPlatform::default();
        assert_eq!(
            perform_action(ExportAction::ClipboardTxt, &mut state, &mut platform).unwrap(),
            ExportOutcome::Unchanged
        );
        assert_eq!(platform.clipboard.as_deref(), Some("ab\ncd"));
    }

    #[test]
    fn collapsed_selection_exports_signed_viewport_with_blank_padding_and_trailing_cells() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("abc\nde");
        let before = state.edit_snapshot();
        let mut viewport = ViewportOffset { x: 13, y: -7 };
        let mut platform = MockPlatform::default();

        assert_eq!(
            perform(
                ExportAction::ClipboardTxt,
                &mut state,
                &mut viewport,
                VisibleCanvasCells {
                    origin: (-1, 0),
                    columns: 5,
                    rows: 3,
                },
                &mut platform,
            )
            .unwrap(),
            ExportOutcome::Unchanged
        );

        assert_eq!(platform.clipboard.as_deref(), Some(" abc \n de  \n     "));
        assert_eq!(state.edit_snapshot(), before);
        assert_eq!(viewport, ViewportOffset { x: 13, y: -7 });
    }

    #[test]
    fn expanded_selection_overrides_viewport_and_clipboard_matches_saved_txt() {
        let path = temp_path("txt");
        let mut state = state_with_selection();
        let visible = VisibleCanvasCells {
            origin: (-20, -10),
            columns: 2,
            rows: 1,
        };
        let mut viewport = ViewportOffset::default();
        let before = state.edit_snapshot();
        let mut clipboard = MockPlatform::default();
        perform(
            ExportAction::ClipboardTxt,
            &mut state,
            &mut viewport,
            visible,
            &mut clipboard,
        )
        .unwrap();
        let mut save = MockPlatform {
            save: Some(path.clone()),
            ..MockPlatform::default()
        };
        perform(
            ExportAction::SaveTxt,
            &mut state,
            &mut viewport,
            visible,
            &mut save,
        )
        .unwrap();

        assert_eq!(clipboard.clipboard.as_deref(), Some("ab\ncd"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "ab\ncd");
        assert_eq!(state.edit_snapshot(), before);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn txt_flattens_the_highest_visible_glyph_without_splitting_wide_graphemes() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("😀");
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let upper = state.active_layer_id();
        state.grid.lines = lines_from_text(" x");
        let visible = VisibleCanvasCells {
            origin: (0, 0),
            columns: 2,
            rows: 1,
        };

        assert_eq!(text_export(&state, visible), " x");
        assert!(state.toggle_layer_visibility(upper));
        assert_eq!(text_export(&state, visible), "😀");
    }

    #[test]
    fn clipboard_txt_combines_visible_layers_with_the_topmost_nonblank_winning() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("AAA");
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        state.grid.lines = lines_from_text("  BBB");
        let visible = VisibleCanvasCells {
            origin: (0, 0),
            columns: 5,
            rows: 1,
        };
        let mut platform = MockPlatform::default();
        let mut viewport = ViewportOffset::default();

        assert_eq!(
            perform(
                ExportAction::ClipboardTxt,
                &mut state,
                &mut viewport,
                visible,
                &mut platform,
            )
            .unwrap(),
            ExportOutcome::Unchanged
        );
        assert_eq!(platform.clipboard.as_deref(), Some("AABBB"));
    }

    #[test]
    fn edit_copy_and_cut_flatten_visible_layers_and_cut_clears_every_layer() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("AAA");
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        state.grid.lines = lines_from_text("  BBB");
        state.move_to(Coord::default());
        for _ in 0..4 {
            state.extend_selection(crate::model::Direction::Right);
        }
        let mut platform = MockPlatform::default();

        copy_selection(&mut state, &mut platform).unwrap();
        assert_eq!(platform.clipboard.as_deref(), Some("AABBB"));

        assert!(cut_selection(&mut state, &mut platform).unwrap());
        assert_eq!(platform.clipboard.as_deref(), Some("AABBB"));
        assert_eq!(state.selected_text(), "     ");
        assert_eq!(
            text_export(
                &state,
                VisibleCanvasCells {
                    origin: (0, 0),
                    columns: 5,
                    rows: 1,
                },
            ),
            "     "
        );
    }

    #[test]
    fn copying_one_display_cell_selects_it_as_a_custom_stamp() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("◇x");
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        let mut platform = MockPlatform::default();

        copy_selection(&mut state, &mut platform).unwrap();

        assert_eq!(platform.clipboard.as_deref(), Some("◇"));
        assert_eq!(state.toolbar.main_mode(), MainMode::Stamp);
        assert_eq!(state.toolbar.custom_stamp(), Some("◇"));
        assert_eq!(state.toolbar.stamp(), "◇");

        state.extend_selection(crate::model::Direction::Right);
        copy_selection(&mut state, &mut platform).unwrap();
        assert_eq!(platform.clipboard.as_deref(), Some("◇x"));
        assert_eq!(state.toolbar.custom_stamp(), Some("◇"));

        let mut wide = Editor::new(&ThemeConfig::default(), "test");
        wide.grid.lines = lines_from_text("😀");
        wide.extend_selection(crate::model::Direction::Right);
        copy_selection(&mut wide, &mut platform).unwrap();
        assert_eq!(wide.toolbar.custom_stamp(), None);
    }

    #[test]
    fn png_export_passes_every_visible_layer_in_bottom_to_top_order() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("◯");
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        state.grid.lines = lines_from_text("○");
        let middle = state.active_layer_id();
        assert!(state.add_layer_above(middle));
        state.grid.lines = lines_from_text("•");
        let top = state.active_layer_id();
        assert!(state.select_layer(middle));
        let mut platform = MockPlatform {
            image: Some(sample_image()),
            ..MockPlatform::default()
        };

        perform(
            ExportAction::ClipboardPng,
            &mut state,
            &mut ViewportOffset::default(),
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 1,
                rows: 1,
            },
            &mut platform,
        )
        .unwrap();

        let rendered = platform.rendered_layers.unwrap();
        assert_eq!(rendered.len(), 3);
        assert_eq!(contents(&rendered[0][0]), "◯");
        assert_eq!(contents(&rendered[1][0]), "○");
        assert_eq!(contents(&rendered[2][0]), "•");
        assert_eq!(state.active_layer_id(), middle);
        assert_ne!(top, middle);
    }

    #[test]
    fn png_export_preserves_canvas_palette_faces() {
        let color = crate::model::ColorId(13);
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        state.apply_toolbar_action(ToolbarAction::SelectColor(color));
        state.insert("x");
        let mut platform = MockPlatform {
            image: Some(sample_image()),
            ..MockPlatform::default()
        };

        perform(
            ExportAction::ClipboardPng,
            &mut state,
            &mut ViewportOffset::default(),
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 1,
                rows: 1,
            },
            &mut platform,
        )
        .unwrap();

        assert_eq!(
            platform.rendered_layers.unwrap()[0][0][0].face.fg,
            color.hex().unwrap()
        );
    }

    #[test]
    fn json_save_remains_whole_project_when_selection_and_viewport_are_smaller() {
        let path = temp_path("json");
        let mut state = state_with_selection();
        state.grid.lines.push(row_atoms("outside"));
        let mut viewport = ViewportOffset::default();
        let mut platform = MockPlatform {
            save: Some(path.clone()),
            ..MockPlatform::default()
        };
        perform(
            ExportAction::SaveJson,
            &mut state,
            &mut viewport,
            VisibleCanvasCells {
                origin: (1, 1),
                columns: 1,
                rows: 1,
            },
            &mut platform,
        )
        .unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        let LoadedJson::Project(project) = project_from_json(&saved).unwrap() else {
            panic!("expected project document");
        };
        assert_eq!(contents(project.layers[0].lines.last().unwrap()), "outside");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn png_clipboard_and_save_use_identical_selected_canvas_pixels() {
        let path = temp_path("png");
        let image = sample_image();
        let mut state = state_with_selection();
        let before = state.edit_snapshot();
        let visible = VisibleCanvasCells {
            origin: (-10, -10),
            columns: 1,
            rows: 1,
        };
        let mut viewport = ViewportOffset { x: 7, y: -9 };
        let mut clipboard = MockPlatform {
            image: Some(image.clone()),
            ..MockPlatform::default()
        };
        perform(
            ExportAction::ClipboardPng,
            &mut state,
            &mut viewport,
            visible,
            &mut clipboard,
        )
        .unwrap();
        let mut save = MockPlatform {
            image: Some(image.clone()),
            save: Some(path.clone()),
            ..MockPlatform::default()
        };
        perform(
            ExportAction::SavePng,
            &mut state,
            &mut viewport,
            visible,
            &mut save,
        )
        .unwrap();

        assert_eq!(
            clipboard.rendered_lines.as_deref(),
            save.rendered_lines.as_deref()
        );
        assert_eq!(clipboard.clipboard_image.as_ref(), Some(&image));
        assert_eq!(fs::read(&path).unwrap(), image.png);
        assert_eq!(state.edit_snapshot(), before);
        assert_eq!(viewport, ViewportOffset { x: 7, y: -9 });
        let _ = fs::remove_file(path);
    }

    #[test]
    fn png_viewport_source_is_raw_canvas_only_even_with_active_overlays_and_preview() {
        let config = ThemeConfig::default();
        let mut state = Editor::new(&config, "title excluded");
        state.grid.lines = lines_from_text("ab");
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.toggle_shape_preview();
        state.move_cursor(crate::model::Direction::Right);
        let expected = region_atoms(
            &state.grid.lines,
            CanvasRegion {
                left: 0,
                top: 0,
                width: 2,
                height: 1,
            },
        );
        let mut platform = MockPlatform {
            image: Some(sample_image()),
            ..MockPlatform::default()
        };

        perform(
            ExportAction::ClipboardPng,
            &mut state,
            &mut ViewportOffset::default(),
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 2,
                rows: 1,
            },
            &mut platform,
        )
        .unwrap();

        assert_eq!(platform.rendered_lines.as_ref(), Some(&expected));

        let mut lifted = Editor::new(&config, "title excluded");
        lifted.grid.lines = lines_from_text("ab");
        assert!(lifted.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities)));
        assert!(lifted.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 0,
        }));
        lifted.extend_selection(crate::model::Direction::Right);
        assert!(lifted.begin_selected_move_lift());
        assert!(lifted.move_lift(crate::model::Direction::Right));
        lifted.selection.collapse(crate::model::Coord::default());
        perform(
            ExportAction::ClipboardPng,
            &mut lifted,
            &mut ViewportOffset::default(),
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 2,
                rows: 1,
            },
            &mut platform,
        )
        .unwrap();
        assert_eq!(platform.rendered_lines.as_ref(), Some(&expected));
    }

    #[test]
    fn png_save_cancellation_skips_rendering_and_is_an_exact_no_op() {
        let mut state = state_with_selection();
        let before = state.edit_snapshot();
        let mut platform = MockPlatform {
            fail_image_render: true,
            ..MockPlatform::default()
        };

        assert_eq!(
            perform_action(ExportAction::SavePng, &mut state, &mut platform).unwrap(),
            ExportOutcome::Cancelled
        );
        assert!(platform.rendered_lines.is_none());
        assert_eq!(state.edit_snapshot(), before);
    }

    #[test]
    fn png_render_and_clipboard_failures_are_atomic() {
        for mut platform in [
            MockPlatform {
                fail_image_render: true,
                ..MockPlatform::default()
            },
            MockPlatform {
                image: Some(sample_image()),
                fail_image_write: true,
                ..MockPlatform::default()
            },
        ] {
            let mut state = state_with_selection();
            let before = state.edit_snapshot();
            assert!(perform_action(ExportAction::ClipboardPng, &mut state, &mut platform).is_err());
            assert_eq!(state.edit_snapshot(), before);
        }
    }

    #[test]
    fn png_file_write_failure_is_atomic() {
        let mut state = state_with_selection();
        let before = state.edit_snapshot();
        let mut platform = MockPlatform {
            image: Some(sample_image()),
            save: Some(std::env::temp_dir()),
            ..MockPlatform::default()
        };

        assert!(perform_action(ExportAction::SavePng, &mut state, &mut platform).is_err());
        assert_eq!(state.edit_snapshot(), before);
    }

    #[test]
    fn clipboard_copy_preserves_blank_rows_and_trailing_spaces_without_mutating_state() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("ab\n\nz");
        state.move_to(Coord::default());
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Down);
        state.extend_selection(crate::model::Direction::Down);
        let before = state.clone();
        let mut platform = MockPlatform::default();
        copy_selection(&mut state, &mut platform).unwrap();
        assert_eq!(platform.clipboard.as_deref(), Some("ab \n   \nz  "));
        assert_eq!(state.grid.lines, before.grid.lines);
        assert_eq!(state.selection, before.selection);
        assert_eq!(state.grid.cursor_pos, before.grid.cursor_pos);
    }

    #[test]
    fn cut_copies_exact_ragged_rectangle_then_clears_only_that_rectangle() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("A│Z\nB\nC界Q");
        state.move_to(Coord { line: 0, column: 1 });
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Down);
        state.extend_selection(crate::model::Direction::Down);
        let mut platform = MockPlatform::default();

        assert!(cut_selection(&mut state, &mut platform).unwrap());

        assert_eq!(platform.clipboard.as_deref(), Some("│Z\n  \n界"));
        assert_eq!(state.selected_text(), "  \n  \n  ");
        assert_eq!(contents(&state.grid.lines[0]), "A   ");
        assert_eq!(contents(&state.grid.lines[1]), "B   ");
        assert_eq!(contents(&state.grid.lines[2]), "C  Q");
        assert_eq!(state.selection.bounds().left, 1);
        assert_eq!(state.grid.cursor_pos, Coord { line: 2, column: 2 });
    }

    #[test]
    fn cut_failure_and_blank_cut_are_document_no_ops() {
        let mut state = state_with_selection();
        let before = state.clone();
        let mut failure = MockPlatform {
            fail_clipboard_write: true,
            ..MockPlatform::default()
        };
        assert!(cut_selection(&mut state, &mut failure).is_err());
        assert_eq!(state.grid.lines, before.grid.lines);
        assert_eq!(state.selection, before.selection);
        assert_eq!(state.grid.cursor_pos, before.grid.cursor_pos);

        let mut blank = Editor::new(&ThemeConfig::default(), "test");
        blank.extend_selection(crate::model::Direction::Right);
        blank.extend_selection(crate::model::Direction::Down);
        let blank_before = blank.clone();
        let mut platform = MockPlatform::default();
        assert!(!cut_selection(&mut blank, &mut platform).unwrap());
        assert_eq!(platform.clipboard.as_deref(), Some("  \n  "));
        assert_eq!(blank.grid.lines, blank_before.grid.lines);
        assert_eq!(blank.selection, blank_before.selection);
    }

    #[test]
    fn cut_does_not_smart_break_neighboring_lines() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("│\n│\n│");
        state.move_to(Coord { line: 1, column: 0 });
        let outside_faces = [
            state.grid.lines[0][0].face.clone(),
            state.grid.lines[2][0].face.clone(),
        ];
        let mut platform = MockPlatform::default();

        assert!(cut_selection(&mut state, &mut platform).unwrap());

        assert_eq!(platform.clipboard.as_deref(), Some("│"));
        assert_eq!(contents(&state.grid.lines[0]), "│");
        assert_eq!(contents(&state.grid.lines[1]), " ");
        assert_eq!(contents(&state.grid.lines[2]), "│");
        assert_eq!(state.grid.lines[0][0].face, outside_faces[0]);
        assert_eq!(state.grid.lines[2][0].face, outside_faces[1]);
    }

    #[test]
    fn real_cut_is_one_undoable_edit_and_blank_cut_preserves_redo() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("wide\ntext");
        state.move_to(Coord::default());
        state.extend_selection(crate::model::Direction::Right);
        let before = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: ViewportOffset { x: 3, y: -2 },
        };
        let mut platform = MockPlatform::default();
        assert!(cut_selection(&mut state, &mut platform).unwrap());
        let cut = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: before.viewport,
        };
        let mut history = EditHistory::default();
        assert!(history.record_change(before.clone(), &cut));
        assert_eq!(history.undo(cut.clone()), Some(before.clone()));
        assert_eq!(history.redo(before.clone()), Some(cut.clone()));

        let restored = history.undo(cut.clone()).expect("real cut undo entry");
        state.restore_edit_snapshot(restored.edit.clone());
        state.move_to(Coord { line: 1, column: 4 });
        let before_blank = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: restored.viewport,
        };
        let mut blank_platform = MockPlatform::default();
        assert!(!cut_selection(&mut state, &mut blank_platform).unwrap());
        assert_eq!(blank_platform.clipboard.as_deref(), Some(" "));
        let after_blank = HistorySnapshot {
            edit: state.edit_snapshot(),
            viewport: before_blank.viewport,
        };
        assert!(!history.record_change(before_blank, &after_blank));
        assert_eq!(history.redo(after_blank), Some(cut));
    }

    #[test]
    fn clipboard_errors_and_zero_width_text_do_not_mutate_state() {
        let mut state = state_with_selection();
        let before = state.clone();
        let mut read_error = MockPlatform {
            fail_clipboard_read: true,
            ..MockPlatform::default()
        };
        assert!(paste_selection(&mut state, &mut read_error).is_err());
        assert_eq!(state.grid.lines, before.grid.lines);
        assert_eq!(state.selection, before.selection);

        let mut empty = MockPlatform {
            clipboard: Some("\n\r\n".to_string()),
            ..MockPlatform::default()
        };
        assert!(!paste_selection(&mut state, &mut empty).unwrap());
        assert_eq!(state.grid.lines, before.grid.lines);
        assert_eq!(state.selection, before.selection);
    }

    #[test]
    fn canceled_dialog_is_a_clean_no_op() {
        let mut state = state_with_selection();
        let before = state.grid.lines.clone();
        let mut platform = MockPlatform::default();
        assert_eq!(
            perform_action(ExportAction::LoadTxt, &mut state, &mut platform).unwrap(),
            ExportOutcome::Cancelled
        );
        assert_eq!(state.grid.lines, before);
    }

    #[test]
    fn txt_save_writes_exact_selection_and_nothing_outside_it() {
        let path = temp_path("txt");
        let mut state = state_with_selection();
        let mut platform = MockPlatform {
            save: Some(path.clone()),
            ..MockPlatform::default()
        };
        assert_eq!(
            perform_action(ExportAction::SaveTxt, &mut state, &mut platform).unwrap(),
            ExportOutcome::Unchanged
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "ab\ncd");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn project_json_is_human_readable_and_preserves_cell_styles() {
        let mut state = state_with_selection();
        state.grid.lines[1][2].face = state.theme.selection.clone();
        let json = serde_json::to_string_pretty(&project_document(
            &state,
            ViewportOffset { x: -12, y: 34 },
        ))
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["format"], PROJECT_FORMAT);
        assert_eq!(value["version"], PROJECT_VERSION);
        assert_eq!(value["canvas"]["layers"][0]["id"], 0);
        assert_eq!(value["canvas"]["layers"][0]["lines"][1][2]["contents"], "a");
        assert_eq!(
            value["canvas"]["layers"][0]["lines"][1][2]["face"],
            serde_json::to_value(&state.theme.selection).unwrap()
        );
    }

    #[test]
    fn project_v2_round_trips_layer_order_visibility_active_layer_and_faces() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.grid.lines = vec![vec![Atom {
            face: state.theme.selection.clone(),
            contents: "a".to_owned(),
        }]];
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let upper = state.active_layer_id();
        state.grid.lines = vec![vec![Atom {
            face: state.theme.cursor_block.clone(),
            contents: "b".to_owned(),
        }]];
        assert!(state.toggle_layer_visibility(base));
        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        state.apply_toolbar_action(ToolbarAction::SelectColor(crate::model::ColorId(14)));

        let json =
            serde_json::to_string(&project_document(&state, ViewportOffset::default())).unwrap();
        let LoadedJson::Project(restored) = project_from_json(&json).unwrap() else {
            panic!("version two project must use project loading")
        };

        assert_eq!(restored.layers.len(), 2);
        assert_eq!(restored.layers[0].id, base);
        assert!(!restored.layers[0].visible);
        assert_eq!(restored.layers[0].lines[0][0].face, state.theme.selection);
        assert_eq!(restored.layers[1].id, upper);
        assert_eq!(
            restored.layers[1].lines[0][0].face,
            state.theme.cursor_block
        );
        assert_eq!(restored.active_layer, upper);
        assert_eq!(
            restored.menu_selections.active_color(),
            crate::model::ColorId(14)
        );
    }

    #[test]
    fn version_one_project_migrates_to_a_default_faced_base_layer() {
        let json = serde_json::json!({
            "format": PROJECT_FORMAT,
            "version": 1,
            "canvas": {"rows": ["a", "😀"]},
            "cursor": {"line": 0, "column": 0},
            "selection": {
                "anchor": {"line": 0, "column": 0},
                "active": {"line": 0, "column": 0}
            },
            "viewport": {"x": 0, "y": 0},
            "menu-selections": {}
        });
        let LoadedJson::Project(restored) = project_from_json(&json.to_string()).unwrap() else {
            panic!("version one project must migrate through project loading")
        };

        assert_eq!(restored.layers.len(), 1);
        assert_eq!(restored.layers[0].id, LayerId(0));
        assert!(restored.layers[0].visible);
        assert_eq!(contents(&restored.layers[0].lines[0]), "a");
        assert!(
            restored.layers[0]
                .lines
                .iter()
                .flatten()
                .all(|atom| atom.face == Face::default())
        );
    }

    #[test]
    fn text_import_is_grapheme_aware_rectangular_and_preserves_blank_rows() {
        let lines = lines_from_text("😀x\n\ny");
        assert_eq!(lines.len(), 3);
        assert_eq!(display_width(&lines[0]), 3);
        assert!(lines.iter().all(|line| display_width(line) == 3));
        assert_eq!(lines[0][0].contents, "😀");
    }

    #[test]
    fn txt_load_replaces_canvas_and_resets_cursor_selection_and_transients() {
        let path = temp_path("txt");
        fs::write(&path, "new\n😀").unwrap();
        let mut state = state_with_selection();
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes)));
        let menu_selections = state.toolbar.durable_selections();
        state.toggle_shape_preview();
        let mut platform = MockPlatform {
            open: Some(path.clone()),
            ..MockPlatform::default()
        };

        assert_eq!(
            perform_action(ExportAction::LoadTxt, &mut state, &mut platform).unwrap(),
            ExportOutcome::DocumentLoaded
        );
        assert_eq!(state.grid.cursor_pos, Coord::default());
        assert!(state.selection.is_collapsed());
        assert!(state.lines_with_shape_preview().is_none());
        assert_eq!(state.cursor_mode, CursorMode::Shapes);
        assert_eq!(state.selected_text(), "n");
        assert_eq!(state.toolbar.durable_selections(), menu_selections);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn json_project_round_trip_restores_full_canvas_state_with_default_faces() {
        let path = temp_path("json");
        let mut source = state_with_selection();
        source.grid.lines = vec![row_atoms("😀x  "), Vec::new(), row_atoms(" z")];
        source.grid.lines[0][0].face = source.theme.selection.clone();
        source.move_to(Coord { line: 2, column: 1 });
        source
            .selection
            .select(Coord { line: 0, column: 2 }, Coord { line: 2, column: 1 });
        source.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line));
        source.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 7,
        });
        source.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities));
        source.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        });
        let source_menu = source.toolbar.durable_selections();
        let source_viewport = ViewportOffset { x: -12, y: 34 };
        save_project_json(&path, &source, source_viewport).unwrap();

        let mut target = Editor::new(&ThemeConfig::default(), "target");
        target.grid.lines = lines_from_text("unrelated outside content");
        target.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Stamp));
        let mut target_viewport = ViewportOffset::default();
        let mut platform = MockPlatform {
            open: Some(path.clone()),
            ..MockPlatform::default()
        };
        assert_eq!(
            perform(
                ExportAction::LoadJson,
                &mut target,
                &mut target_viewport,
                VisibleCanvasCells {
                    origin: (0, 0),
                    columns: 80,
                    rows: 24,
                },
                &mut platform,
            )
            .unwrap(),
            ExportOutcome::ProjectLoaded
        );
        assert_eq!(target.grid.cursor_pos, Coord { line: 2, column: 1 });
        assert_eq!(target.selection.anchor(), Coord { line: 0, column: 2 });
        assert_eq!(target.selection.active(), Coord { line: 2, column: 1 });
        assert_eq!(target_viewport, source_viewport);
        assert_eq!(target.toolbar.durable_selections(), source_menu);
        assert_eq!(target.cursor_mode, CursorMode::Utilities);
        assert_eq!(
            target
                .grid
                .lines
                .iter()
                .map(|line| line
                    .iter()
                    .map(|atom| atom.contents.as_str())
                    .collect::<String>())
                .collect::<Vec<_>>(),
            ["😀x  ", "", " z"]
        );
        assert_eq!(target.grid.lines[0][0].face, source.grid.lines[0][0].face);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn project_save_uses_project_filename_and_does_not_mutate_editor_state() {
        assert_eq!(default_file_name(FileKind::Txt), "selection.txt");
        assert_eq!(default_file_name(FileKind::Json), "ascdraw.json");
        assert_eq!(default_file_name(FileKind::Png), "ascdraw.png");

        let path = temp_path("json");
        let mut state = state_with_selection();
        let viewport = ViewportOffset { x: -7, y: 19 };
        let before = state.edit_snapshot();
        let menus = state.toolbar.durable_selections();
        let mut actual_viewport = viewport;
        let mut platform = MockPlatform {
            save: Some(path.clone()),
            ..MockPlatform::default()
        };
        assert_eq!(
            perform(
                ExportAction::SaveJson,
                &mut state,
                &mut actual_viewport,
                VisibleCanvasCells {
                    origin: (0, 0),
                    columns: 80,
                    rows: 24,
                },
                &mut platform,
            )
            .unwrap(),
            ExportOutcome::Unchanged
        );
        assert_eq!(state.edit_snapshot(), before);
        assert_eq!(state.toolbar.durable_selections(), menus);
        assert_eq!(actual_viewport, viewport);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_project_load_is_atomic() {
        let path = temp_path("json");
        let source = Editor::new(&ThemeConfig::default(), "source");
        let mut value =
            serde_json::to_value(project_document(&source, ViewportOffset { x: 4, y: -8 }))
                .unwrap();
        value["canvas"]["layers"][0]["lines"][0] = serde_json::to_value(vec![Atom {
            face: Face::default(),
            contents: "😀".to_owned(),
        }])
        .unwrap();
        value["cursor"] = serde_json::json!({"line": 0, "column": 1});
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let mut target = state_with_selection();
        let before = target.edit_snapshot();
        let menus = target.toolbar.durable_selections();
        let mut viewport = ViewportOffset { x: 22, y: -31 };
        let mut platform = MockPlatform {
            open: Some(path.clone()),
            ..MockPlatform::default()
        };
        let error = perform(
            ExportAction::LoadJson,
            &mut target,
            &mut viewport,
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 80,
                rows: 24,
            },
            &mut platform,
        )
        .unwrap_err();
        assert!(error.to_string().contains("inside a wide grapheme"));
        assert_eq!(target.edit_snapshot(), before);
        assert_eq!(target.toolbar.durable_selections(), menus);
        assert_eq!(viewport, ViewportOffset { x: 22, y: -31 });
        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_project_color_is_rejected_atomically() {
        let path = temp_path("json");
        let source = Editor::new(&ThemeConfig::default(), "source");
        let mut value =
            serde_json::to_value(project_document(&source, ViewportOffset::default())).unwrap();
        value["menu-selections"]["active-color"] = serde_json::json!(99);
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let mut target = state_with_selection();
        let before = target.edit_snapshot();
        let menus = target.toolbar.durable_selections();
        let mut viewport = ViewportOffset { x: 4, y: -5 };
        let mut platform = MockPlatform {
            open: Some(path.clone()),
            ..MockPlatform::default()
        };
        let error = perform(
            ExportAction::LoadJson,
            &mut target,
            &mut viewport,
            VisibleCanvasCells {
                origin: (0, 0),
                columns: 80,
                rows: 24,
            },
            &mut platform,
        )
        .unwrap_err();

        assert!(error.to_string().contains("active color"));
        assert_eq!(target.edit_snapshot(), before);
        assert_eq!(target.toolbar.durable_selections(), menus);
        assert_eq!(viewport, ViewportOffset { x: 4, y: -5 });
        let _ = fs::remove_file(path);
    }

    #[test]
    fn unsupported_project_format_and_version_are_rejected() {
        let state = Editor::new(&ThemeConfig::default(), "source");
        let document = project_document(&state, ViewportOffset::default());
        let mut wrong_format = serde_json::to_value(&document).unwrap();
        wrong_format["format"] = serde_json::json!("some-other-app");
        assert!(
            project_from_json(&wrong_format.to_string())
                .unwrap_err()
                .to_string()
                .contains("unsupported project format")
        );
        let mut wrong_version = serde_json::to_value(document).unwrap();
        wrong_version["version"] = serde_json::json!(PROJECT_VERSION + 1);
        assert!(
            project_from_json(&wrong_version.to_string())
                .unwrap_err()
                .to_string()
                .contains("unsupported ascdraw project version")
        );
    }

    #[test]
    fn legacy_selection_and_native_json_keep_glyphs_but_drop_faces() {
        let configured_face = ThemeConfig::default().selection;
        let legacy_atom = Atom {
            face: configured_face,
            contents: "┌".to_owned(),
        };
        let selection = serde_json::json!({
            "version": LEGACY_SELECTION_VERSION,
            "width": 3,
            "height": 2,
            "lines": [[legacy_atom.clone()], []]
        });
        let LoadedJson::Legacy(selection_lines) =
            project_from_json(&selection.to_string()).unwrap()
        else {
            panic!("legacy selection must use legacy import")
        };
        assert_eq!(selection_lines.len(), 2);
        assert!(selection_lines.iter().all(|line| display_width(line) == 3));
        assert_eq!(selection_lines[0][0].contents, "┌");
        assert!(
            selection_lines
                .iter()
                .flatten()
                .all(|atom| atom.face == Face::default())
        );

        let native = serde_json::json!({"lines": [[legacy_atom], []]});
        let LoadedJson::Legacy(native_lines) = project_from_json(&native.to_string()).unwrap()
        else {
            panic!("native canvas must use legacy import")
        };
        assert_eq!(native_lines.len(), 2);
        assert_eq!(native_lines[0][0].contents, "┌");
        assert_eq!(native_lines[0][0].face, Face::default());
    }

    #[test]
    fn clear_replaces_the_canvas_without_using_the_platform() {
        let mut state = state_with_selection();
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Utilities));
        state.apply_toolbar_action(ToolbarAction::SelectSubmenu {
            submenu: 0,
            option: 2,
        });
        let menu_selections = state.toolbar.durable_selections();
        let mut platform = MockPlatform {
            fail_clipboard_read: true,
            fail_clipboard_write: true,
            ..MockPlatform::default()
        };

        assert_eq!(
            perform_action(ExportAction::Clear, &mut state, &mut platform).unwrap(),
            ExportOutcome::CanvasCleared
        );
        assert!(state.content_cells().is_empty());
        assert_eq!(state.grid.cursor_pos, Coord { line: 2, column: 3 });
        assert!(state.selection.is_collapsed());
        assert!(platform.save.is_none());
        assert!(platform.open.is_none());
        assert!(platform.clipboard.is_none());
        assert_eq!(state.toolbar.durable_selections(), menu_selections);
    }
}
