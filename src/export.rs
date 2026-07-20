use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::MacosColorSpace;
use crate::canvas::LayerStack;
use crate::dense_exchange;
use crate::document::{self, CanvasPosition, Document};
use crate::editor::Editor;
use crate::layout::{ViewportOffset, VisibleCanvasCells};
use crate::legacy_loader::{LegacyLayer, into_canvas};
use crate::model::{Face, LayerId, MAX_CANVAS_HEIGHT, MAX_CANVAS_WIDTH, StyledAtom};
use crate::render::{CanvasImage, Renderer, render_canvas_image, render_canvas_layers_image};
use crate::selection::{CanvasRegion, CanvasSelection, TextRectangle};
use crate::toolbar::DurableMenuSelections;

const PROJECT_FORMAT: &str = "ascdraw";
const PROJECT_VERSION: u32 = 2;
const LEGACY_SELECTION_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportAction {
    ClipboardTxt,
    ClipboardPng,
    SaveTxt,
    SaveJson,
    SavePng,
    LoadTxt,
    LoadJson,
    ImportTxt,
    ImportJson,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportOutcome {
    Unchanged,
    Cancelled,
    Saved { path: PathBuf, format: FileKind },
    DocumentLoaded { path: PathBuf, format: FileKind },
    ProjectLoaded { path: PathBuf, zoom: i32 },
    DocumentImported,
    CanvasCleared,
}

pub trait ExportPlatform {
    fn set_clipboard_text(&mut self, text: &str) -> Result<()>;
    fn clipboard_text(&mut self) -> Result<String>;
    fn choose_save_path(&mut self, kind: FileKind) -> Option<PathBuf>;
    fn choose_open_path(&mut self, kind: FileKind) -> Option<PathBuf>;
    fn document_metrics(&self) -> ((f32, f32), i32) {
        ((1.0, 1.0), 0)
    }
    fn render_canvas_image(
        &mut self,
        _lines: &[Vec<StyledAtom>],
        _default_face: &Face,
    ) -> Result<CanvasImage> {
        bail!("PNG rendering is unavailable")
    }
    fn render_canvas_layers_image(
        &mut self,
        layers: &[Vec<Vec<StyledAtom>>],
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

    fn document_metrics(&self) -> ((f32, f32), i32) {
        self.png.as_ref().map_or(((1.0, 1.0), 0), |context| {
            let metrics = context.renderer.metrics(context.scale_factor);
            (
                (metrics.cell_width, metrics.cell_height),
                context.renderer.zoom(),
            )
        })
    }

    fn render_canvas_image(
        &mut self,
        lines: &[Vec<StyledAtom>],
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
        layers: &[Vec<Vec<StyledAtom>>],
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

pub fn copy_selection(state: &Editor, platform: &mut impl ExportPlatform) -> Result<()> {
    let text = selected_visible_text(state);
    platform.set_clipboard_text(&text)
}

/// Copies the normalized selection before clearing it. Keeping the clipboard
/// write first makes a failed external operation an editor-state no-op.
pub fn cut_selection(state: &mut Editor, platform: &mut impl ExportPlatform) -> Result<bool> {
    platform.set_clipboard_text(&selected_visible_text(state))?;
    Ok(state.clear_selection())
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
            let contents = if state.selection.is_collapsed() {
                plain_text(state)
            } else {
                text_export(state, visible_canvas)
            };
            fs::write(&path, contents)
                .with_context(|| format!("failed to write {}", path.display()))?;
            Ok(if state.selection.is_collapsed() {
                ExportOutcome::Saved {
                    path,
                    format: FileKind::Txt,
                }
            } else {
                ExportOutcome::Unchanged
            })
        }
        ExportAction::SaveJson => {
            let Some(path) = platform.choose_save_path(FileKind::Json) else {
                return Ok(ExportOutcome::Cancelled);
            };
            let (cell_size, zoom) = platform.document_metrics();
            document::save(
                &path,
                state.canvas(),
                &state.toolbar.durable_selections(),
                CanvasPosition {
                    cursor: state.grid.cursor_pos,
                    viewport: *viewport,
                    zoom,
                },
                cell_size,
            )?;
            Ok(if state.selection.is_collapsed() {
                ExportOutcome::Saved {
                    path,
                    format: FileKind::Json,
                }
            } else {
                ExportOutcome::Unchanged
            })
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
            state.restore_canvas(canvas_from_text(&text)?);
            Ok(ExportOutcome::DocumentLoaded {
                path,
                format: FileKind::Txt,
            })
        }
        ExportAction::LoadJson => {
            let Some(path) = platform.choose_open_path(FileKind::Json) else {
                return Ok(ExportOutcome::Cancelled);
            };
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            match project_from_json(&contents)? {
                LoadedJson::Native(document) => {
                    let zoom = restore_native_document(state, viewport, *document);
                    return Ok(ExportOutcome::ProjectLoaded { path, zoom });
                }
                LoadedJson::Project(project) => {
                    let mut staged = state.clone();
                    staged.restore_project(
                        project.canvas,
                        project.cursor,
                        project.selection,
                        &project.menu_selections,
                    )?;
                    *state = staged;
                    *viewport = project.viewport;
                    return Ok(ExportOutcome::ProjectLoaded { path, zoom: 0 });
                }
                LoadedJson::Legacy(lines) => state.restore_canvas(canvas_from_dense_lines(lines)?),
            }
            Ok(ExportOutcome::DocumentLoaded {
                path,
                format: FileKind::Json,
            })
        }
        ExportAction::ImportTxt => {
            let Some(path) = platform.choose_open_path(FileKind::Txt) else {
                return Ok(ExportOutcome::Cancelled);
            };
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let changed = TextRectangle::from_text(&text)
                .is_some_and(|rectangle| state.paste_styled_rectangle_at_cursor(&rectangle));
            Ok(if changed {
                ExportOutcome::DocumentImported
            } else {
                ExportOutcome::Unchanged
            })
        }
        ExportAction::ImportJson => {
            let Some(path) = platform.choose_open_path(FileKind::Json) else {
                return Ok(ExportOutcome::Cancelled);
            };
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let rectangle = imported_json_rectangle(project_from_json(&contents)?);
            let changed = rectangle
                .is_some_and(|rectangle| state.paste_styled_rectangle_at_cursor(&rectangle));
            Ok(if changed {
                ExportOutcome::DocumentImported
            } else {
                ExportOutcome::Unchanged
            })
        }
        ExportAction::Clear => {
            state.clear_canvas();
            Ok(ExportOutcome::CanvasCleared)
        }
    }
}

fn text_export(state: &Editor, visible_canvas: VisibleCanvasCells) -> String {
    let region = canvas_region_for_export(state, visible_canvas);
    atoms_text(
        &sparse_composite_region(state, region)
            .unwrap_or_else(|| canvas_atoms_for_export(state, visible_canvas)),
    )
}

fn selected_visible_text(state: &Editor) -> String {
    let region = CanvasRegion::from_selection(state.selection_bounds());
    atoms_text(
        &sparse_composite_region(state, region)
            .unwrap_or_else(|| flatten_visible_layers(&visible_layer_atoms(state, region))),
    )
}

fn atoms_text(lines: &[Vec<StyledAtom>]) -> String {
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
    let content = state.content_cells();
    let Some(left) = content.iter().map(|coord| coord.column).min() else {
        return String::new();
    };
    let right = content
        .iter()
        .map(|coord| coord.column)
        .max()
        .expect("nonempty content has a maximum column");
    let top = content
        .iter()
        .map(|coord| coord.line)
        .min()
        .expect("nonempty content has a minimum line");
    let bottom = content
        .iter()
        .map(|coord| coord.line)
        .max()
        .expect("nonempty content has a maximum line");
    let region = CanvasRegion {
        left: i64::try_from(left).unwrap_or(i64::MAX),
        top: i64::try_from(top).unwrap_or(i64::MAX),
        width: usize::try_from(i32::from(right) - i32::from(left) + 1).unwrap_or(usize::MAX),
        height: usize::try_from(i32::from(bottom) - i32::from(top) + 1).unwrap_or(usize::MAX),
    };
    let rows = sparse_composite_region(state, region)
        .unwrap_or_else(|| flatten_visible_layers(&visible_layer_atoms(state, region)));
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

fn sparse_composite_region(state: &Editor, region: CanvasRegion) -> Option<Vec<Vec<StyledAtom>>> {
    dense_exchange::composite_region(state.canvas(), region)
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
) -> Vec<Vec<StyledAtom>> {
    flatten_visible_layers(&canvas_layers_for_export(state, visible_canvas))
}

pub fn canvas_layers_for_export(
    state: &Editor,
    visible_canvas: VisibleCanvasCells,
) -> Vec<Vec<Vec<StyledAtom>>> {
    let region = canvas_region_for_export(state, visible_canvas);
    visible_layer_atoms(state, region)
}

fn visible_layer_atoms(state: &Editor, region: CanvasRegion) -> Vec<Vec<Vec<StyledAtom>>> {
    state
        .canvas()
        .layers()
        .iter()
        .filter(|layer| layer.visible)
        .map(|layer| dense_exchange::atoms_in_region(layer, region))
        .collect()
}

fn flatten_visible_layers(layers: &[Vec<Vec<StyledAtom>>]) -> Vec<Vec<StyledAtom>> {
    let height = layers.iter().map(Vec::len).max().unwrap_or(0);
    let width = layers
        .iter()
        .flat_map(|layer| layer.iter())
        .map(|line| display_width(line))
        .max()
        .unwrap_or(0);
    let mut flattened = Vec::with_capacity(height);
    for row in 0..height {
        let mut glyphs: Vec<Option<(StyledAtom, usize)>> = vec![None; width];
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
                            StyledAtom {
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
    lines: Vec<Vec<StyledAtom>>,
}

#[derive(Deserialize)]
struct NativeJsonDocument {
    lines: Vec<Vec<StyledAtom>>,
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
    layers: Vec<LegacyLayer>,
    #[serde(
        default,
        rename = "active-layer",
        skip_serializing_if = "Option::is_none"
    )]
    active_layer: Option<LayerId>,
}

#[derive(Debug)]
struct RestoredProject {
    canvas: crate::canvas::LayerStack,
    cursor: crate::model::Coord,
    selection: CanvasSelection,
    viewport: ViewportOffset,
    menu_selections: DurableMenuSelections,
}

#[derive(Debug)]
enum LoadedJson {
    Native(Box<Document>),
    Project(Box<RestoredProject>),
    Legacy(Vec<Vec<StyledAtom>>),
}

pub(crate) fn load_project_json(
    path: &Path,
    state: &mut Editor,
    viewport: &mut ViewportOffset,
) -> Result<i32> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    match project_from_json(&contents)? {
        LoadedJson::Native(document) => Ok(restore_native_document(state, viewport, *document)),
        LoadedJson::Project(project) => {
            state.restore_project(
                project.canvas,
                project.cursor,
                project.selection,
                &project.menu_selections,
            )?;
            *viewport = project.viewport;
            Ok(0)
        }
        LoadedJson::Legacy(lines) => {
            state.restore_canvas(canvas_from_dense_lines(lines)?);
            Ok(0)
        }
    }
}

fn imported_json_rectangle(loaded: LoadedJson) -> Option<TextRectangle> {
    let rows = match loaded {
        LoadedJson::Native(document) => flatten_visible_layers(
            &document
                .canvas
                .effective_layers()
                .iter()
                .filter(|layer| layer.visible)
                .map(dense_exchange::to_dense)
                .collect::<Vec<_>>(),
        ),
        LoadedJson::Project(project) => flatten_visible_layers(
            &project
                .canvas
                .effective_layers()
                .iter()
                .filter(|layer| layer.visible)
                .map(|layer| dense_exchange::to_dense(layer))
                .collect::<Vec<_>>(),
        ),
        LoadedJson::Legacy(lines) => lines,
    };
    TextRectangle::from_rows(rows)
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
    if value.get("version").is_some() && value.get("width").is_none() {
        return document::parse_contents(contents)
            .map(Box::new)
            .map(LoadedJson::Native);
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

fn restore_native_document(
    state: &mut Editor,
    viewport: &mut ViewportOffset,
    document: Document,
) -> i32 {
    if let Some(selections) = document.menu_selections {
        state.restore_menu_selections(&selections);
    }
    state.restore_canvas(document.canvas);
    let position = document.position.unwrap_or(CanvasPosition {
        cursor: crate::model::Coord::default(),
        viewport: ViewportOffset::default(),
        zoom: 0,
    });
    state.restore_canvas_position(position.cursor);
    *viewport = position.viewport;
    position.zoom
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
            vec![LegacyLayer {
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
    let canvas = into_canvas(layers, active_layer)?;
    Ok(RestoredProject {
        canvas,
        cursor: document.cursor,
        selection: document.selection,
        viewport: document.viewport,
        menu_selections: document.menu_selections,
    })
}

fn validate_persisted_layers(layers: &[LegacyLayer], active_layer: LayerId) -> Result<()> {
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
    if coord.line < 0
        || coord.column < 0
        || usize::try_from(coord.line).unwrap_or(usize::MAX) >= MAX_CANVAS_HEIGHT
        || usize::try_from(coord.column).unwrap_or(usize::MAX) >= MAX_CANVAS_WIDTH
    {
        bail!(
            "{name} ({}, {}) exceeds the {MAX_CANVAS_WIDTH}x{MAX_CANVAS_HEIGHT} canvas",
            coord.line,
            coord.column
        );
    }
    Ok(())
}

fn pad_to_coordinate(
    lines: &mut Vec<Vec<StyledAtom>>,
    name: &str,
    coord: crate::model::Coord,
) -> Result<()> {
    let line_index = usize::try_from(coord.line).context("legacy row cannot be negative")?;
    let target_column =
        usize::try_from(coord.column).context("legacy column cannot be negative")?;
    while lines.len() <= line_index {
        lines.push(Vec::new());
    }
    let line = &mut lines[line_index];
    let mut column = 0;
    for atom in line.iter() {
        let width = atom_display_width(atom);
        if column < target_column && target_column < column.saturating_add(width) {
            bail!(
                "{name} column {} falls inside a wide grapheme on row {}",
                coord.column,
                coord.line
            );
        }
        column = column.saturating_add(width);
    }
    if column < target_column {
        line.extend((column..target_column).map(|_| blank_atom()));
    }
    Ok(())
}

fn normalize_legacy_lines(
    lines: Vec<Vec<StyledAtom>>,
    width: usize,
) -> Result<Vec<Vec<StyledAtom>>> {
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

fn default_faced_lines(lines: Vec<Vec<StyledAtom>>) -> Vec<Vec<StyledAtom>> {
    lines
        .into_iter()
        .map(|line| {
            line.into_iter()
                .flat_map(|atom| row_atoms(&atom.contents))
                .collect()
        })
        .collect()
}

fn row_atoms(row: &str) -> Vec<StyledAtom> {
    UnicodeSegmentation::graphemes(row, true)
        .map(|contents| StyledAtom {
            face: Face::default(),
            contents: contents.to_owned(),
        })
        .collect()
}

pub fn lines_from_text(text: &str) -> Vec<Vec<StyledAtom>> {
    let mut lines: Vec<Vec<StyledAtom>> = text
        .split('\n')
        .take(MAX_CANVAS_HEIGHT)
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let mut width: usize = 0;
            UnicodeSegmentation::graphemes(line, true)
                .take_while(|contents| {
                    let next = width.saturating_add(UnicodeWidthStr::width(*contents).max(1));
                    if next > MAX_CANVAS_WIDTH {
                        return false;
                    }
                    width = next;
                    true
                })
                .map(|contents| StyledAtom {
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

pub(crate) fn canvas_from_text(text: &str) -> Result<LayerStack> {
    canvas_from_dense_lines(lines_from_text(text))
}

pub(crate) fn canvas_from_dense_lines(mut lines: Vec<Vec<StyledAtom>>) -> Result<LayerStack> {
    truncate_dense_lines(&mut lines);
    let map = dense_exchange::from_dense_with_markers(LayerId(0), true, &lines, &[])?;
    LayerStack::new(vec![map], false)
}

fn truncate_dense_lines(lines: &mut Vec<Vec<StyledAtom>>) {
    lines.truncate(MAX_CANVAS_HEIGHT);
    for line in lines {
        let mut width = 0usize;
        let keep = line
            .iter()
            .take_while(|atom| {
                let next = width.saturating_add(atom_display_width(atom));
                if next > MAX_CANVAS_WIDTH {
                    return false;
                }
                width = next;
                true
            })
            .count();
        line.truncate(keep);
    }
}

fn nonempty_lines(lines: Vec<Vec<StyledAtom>>) -> Vec<Vec<StyledAtom>> {
    if lines.is_empty() {
        vec![Vec::new()]
    } else {
        lines
    }
}

fn display_width(line: &[StyledAtom]) -> usize {
    line.iter().map(atom_display_width).sum()
}

fn atom_display_width(atom: &StyledAtom) -> usize {
    UnicodeWidthStr::width(atom.contents.as_str()).max(usize::from(!atom.contents.is_empty()))
}

fn blank_atom() -> StyledAtom {
    StyledAtom {
        face: Face::default(),
        contents: " ".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{EditHistory, HistorySnapshot};

    fn contents(line: &[StyledAtom]) -> String {
        line.iter().map(|atom| atom.contents.as_str()).collect()
    }
    use crate::app::{CursorMode, ThemeConfig};
    use crate::model::Coord;
    use crate::toolbar::{MainMode, ToggleKind, ToolbarAction};

    fn project_document(state: &Editor, viewport: ViewportOffset) -> ProjectDocument {
        ProjectDocument {
            format: PROJECT_FORMAT.to_owned(),
            version: PROJECT_VERSION,
            canvas: ProjectCanvas {
                rows: Vec::new(),
                layers: state
                    .canvas()
                    .layers()
                    .iter()
                    .map(|layer| LegacyLayer {
                        id: layer.id,
                        visible: layer.visible,
                        lines: dense_exchange::to_dense(layer),
                    })
                    .collect(),
                active_layer: Some(state.active_layer_id()),
            },
            cursor: state.grid.cursor_pos,
            selection: state.selection,
            viewport,
            menu_selections: state.toolbar.durable_selections(),
        }
    }

    fn save_native_json(path: &Path, state: &Editor, viewport: ViewportOffset) -> Result<()> {
        document::save(
            path,
            state.canvas(),
            &state.toolbar.durable_selections(),
            CanvasPosition {
                cursor: state.grid.cursor_pos,
                viewport,
                zoom: 0,
            },
            (1.0, 1.0),
        )
    }

    #[test]
    fn plain_text_preserves_ragged_rows_without_blank_canvas_padding() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.restore_canvas(canvas_from_text("one\nlonger\n").unwrap());

        assert_eq!(plain_text(&state), "one\nlonger");
    }

    #[test]
    fn plain_text_rebases_visible_content_to_its_minimum_coordinates() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.restore_canvas(canvas_from_text("\n  ab\n\n   c").unwrap());

        assert_eq!(plain_text(&state), "ab\n\n c");
    }

    #[derive(Default)]
    struct MockPlatform {
        clipboard: Option<String>,
        clipboard_image: Option<CanvasImage>,
        rendered_lines: Option<Vec<Vec<StyledAtom>>>,
        rendered_layers: Option<Vec<Vec<Vec<StyledAtom>>>>,
        image: Option<CanvasImage>,
        fail_clipboard_read: bool,
        fail_clipboard_write: bool,
        fail_image_render: bool,
        fail_image_write: bool,
        save: Option<PathBuf>,
        open: Option<PathBuf>,
        document_cell_size: Option<(f32, f32)>,
        document_zoom: i32,
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
        fn document_metrics(&self) -> ((f32, f32), i32) {
            (
                self.document_cell_size.unwrap_or((1.0, 1.0)),
                self.document_zoom,
            )
        }
        fn render_canvas_image(
            &mut self,
            lines: &[Vec<StyledAtom>],
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
            layers: &[Vec<Vec<StyledAtom>>],
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
        state.set_lines_for_test(lines_from_text("outside\n  ab  \n  cd  "));
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
        state.set_lines_for_test(lines_from_text("abc\nde"));
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
    fn clipboard_txt_combines_visible_layers_with_the_topmost_nonblank_winning() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.set_lines_for_test(lines_from_text("AAA"));
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
        state.set_lines_for_test(lines_from_text("  BBB"));
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
        state.set_lines_for_test(lines_from_text("AAA"));
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        assert!(state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiLayerMode)));
        state.set_lines_for_test(lines_from_text("  BBB"));
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
    fn copying_does_not_change_the_active_tool_or_toolbar_height() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.set_lines_for_test(lines_from_text("◇x"));
        assert!(state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Line)));
        let toolbar_rows = state.toolbar.rows();
        let mut platform = MockPlatform::default();

        copy_selection(&mut state, &mut platform).unwrap();

        assert_eq!(platform.clipboard.as_deref(), Some("◇"));
        assert_eq!(state.toolbar.main_mode(), MainMode::Line);
        assert_eq!(state.toolbar.rows(), toolbar_rows);

        state.extend_selection(crate::model::Direction::Right);
        copy_selection(&mut state, &mut platform).unwrap();
        assert_eq!(platform.clipboard.as_deref(), Some("◇x"));
        assert_eq!(state.toolbar.main_mode(), MainMode::Line);
        assert_eq!(state.toolbar.rows(), toolbar_rows);
    }

    #[test]
    fn png_export_passes_every_visible_layer_in_bottom_to_top_order() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.set_lines_for_test(lines_from_text("◯"));
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        state.set_lines_for_test(lines_from_text("○"));
        let middle = state.active_layer_id();
        assert!(state.add_layer_above(middle));
        state.set_lines_for_test(lines_from_text("•"));
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
        let mut lines = state.lines_for_test();
        lines.push(row_atoms("outside"));
        state.set_lines_for_test(lines);
        let mut viewport = ViewportOffset::default();
        let mut platform = MockPlatform {
            save: Some(path.clone()),
            document_cell_size: Some((8.0, 16.0)),
            document_zoom: 3,
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
        let expected = document::contents(
            state.canvas(),
            &state.toolbar.durable_selections(),
            CanvasPosition {
                cursor: state.grid.cursor_pos,
                viewport,
                zoom: 3,
            },
            (8.0, 16.0),
        )
        .unwrap();
        assert_eq!(saved, expected);
        let LoadedJson::Native(document) = project_from_json(&saved).unwrap() else {
            panic!("expected native sparse document");
        };
        assert_eq!(
            contents(
                dense_exchange::to_dense(&document.canvas.layers()[0])
                    .last()
                    .unwrap(),
            ),
            "outside"
        );
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
        state.set_lines_for_test(lines_from_text("ab"));
        state.apply_toolbar_action(ToolbarAction::SelectMain(MainMode::Shapes));
        state.toggle_shape_preview();
        state.move_cursor(crate::model::Direction::Right);
        let expected = dense_exchange::atoms_in_region(
            &state.canvas().layers()[state.canvas().active_index()],
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
        lifted.set_lines_for_test(lines_from_text("ab"));
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
        state.set_lines_for_test(lines_from_text("ab\n\nz"));
        state.move_to(Coord::default());
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Down);
        state.extend_selection(crate::model::Direction::Down);
        let before = state.clone();
        let mut platform = MockPlatform::default();
        copy_selection(&mut state, &mut platform).unwrap();
        assert_eq!(platform.clipboard.as_deref(), Some("ab \n   \nz  "));
        assert_eq!(state.lines_for_test(), before.lines_for_test());
        assert_eq!(state.selection, before.selection);
        assert_eq!(state.grid.cursor_pos, before.grid.cursor_pos);
    }

    #[test]
    fn cut_copies_exact_ragged_rectangle_then_clears_only_that_rectangle() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.set_lines_for_test(lines_from_text("A│Z\nB\nCXYQ"));
        state.move_to(Coord { line: 0, column: 1 });
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Down);
        state.extend_selection(crate::model::Direction::Down);
        let mut platform = MockPlatform::default();

        assert!(cut_selection(&mut state, &mut platform).unwrap());

        assert_eq!(platform.clipboard.as_deref(), Some("│Z\n  \nXY"));
        assert_eq!(state.selected_text(), "  \n  \n  ");
        let lines = state.lines_for_test();
        assert_eq!(contents(&lines[0]), "A");
        assert_eq!(contents(&lines[1]), "B");
        assert_eq!(contents(&lines[2]), "C  Q");
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
        assert_eq!(state.lines_for_test(), before.lines_for_test());
        assert_eq!(state.selection, before.selection);
        assert_eq!(state.grid.cursor_pos, before.grid.cursor_pos);

        let mut blank = Editor::new(&ThemeConfig::default(), "test");
        blank.extend_selection(crate::model::Direction::Right);
        blank.extend_selection(crate::model::Direction::Down);
        let blank_before = blank.clone();
        let mut platform = MockPlatform::default();
        assert!(!cut_selection(&mut blank, &mut platform).unwrap());
        assert_eq!(platform.clipboard.as_deref(), Some("  \n  "));
        assert_eq!(blank.lines_for_test(), blank_before.lines_for_test());
        assert_eq!(blank.selection, blank_before.selection);
    }

    #[test]
    fn cut_does_not_smart_break_neighboring_lines() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.set_lines_for_test(lines_from_text("│\n│\n│"));
        state.move_to(Coord { line: 1, column: 0 });
        let outside_faces = [
            state.lines_for_test()[0][0].face.clone(),
            state.lines_for_test()[2][0].face.clone(),
        ];
        let mut platform = MockPlatform::default();

        assert!(cut_selection(&mut state, &mut platform).unwrap());

        assert_eq!(platform.clipboard.as_deref(), Some("│"));
        let lines = state.lines_for_test();
        assert_eq!(contents(&lines[0]), "│");
        assert_eq!(contents(&lines[1]), "");
        assert_eq!(contents(&lines[2]), "│");
        assert_eq!(lines[0][0].face, outside_faces[0]);
        assert_eq!(lines[2][0].face, outside_faces[1]);
    }

    #[test]
    fn real_cut_is_one_undoable_edit_and_blank_cut_preserves_redo() {
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.set_lines_for_test(lines_from_text("wide\ntext"));
        state.move_to(Coord::default());
        state.extend_selection(crate::model::Direction::Right);
        let before = HistorySnapshot {
            edit: state.history_state(),
            viewport: ViewportOffset { x: 3, y: -2 },
        };
        state.begin_history_capture();
        let mut platform = MockPlatform::default();
        assert!(cut_selection(&mut state, &mut platform).unwrap());
        let cut = HistorySnapshot {
            edit: state.history_state(),
            viewport: before.viewport,
        };
        let delta = state.finish_history_capture();
        let mut history = EditHistory::default();
        assert!(history.record_change(before, cut, delta));
        let restored = history.undo().expect("real cut undo entry");
        state.apply_history_delta(&restored.canvas, restored.forward);
        state.restore_history_state(restored.edit);
        let redone = history.redo().expect("real cut redo entry");
        state.apply_history_delta(&redone.canvas, redone.forward);
        state.restore_history_state(redone.edit);

        let restored = history.undo().expect("real cut undo entry");
        state.apply_history_delta(&restored.canvas, restored.forward);
        state.restore_history_state(restored.edit);
        state.move_to(Coord { line: 1, column: 4 });
        let before_blank = HistorySnapshot {
            edit: state.history_state(),
            viewport: restored.viewport,
        };
        state.begin_history_capture();
        let mut blank_platform = MockPlatform::default();
        assert!(!cut_selection(&mut state, &mut blank_platform).unwrap());
        assert_eq!(blank_platform.clipboard.as_deref(), Some(" "));
        let after_blank = HistorySnapshot {
            edit: state.history_state(),
            viewport: before_blank.viewport,
        };
        let delta = state.finish_history_capture();
        assert!(!history.record_change(before_blank, after_blank, delta));
        assert!(history.redo().is_some());
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
        assert_eq!(state.lines_for_test(), before.lines_for_test());
        assert_eq!(state.selection, before.selection);

        let mut empty = MockPlatform {
            clipboard: Some("\n\r\n".to_string()),
            ..MockPlatform::default()
        };
        assert!(!paste_selection(&mut state, &mut empty).unwrap());
        assert_eq!(state.lines_for_test(), before.lines_for_test());
        assert_eq!(state.selection, before.selection);
    }

    #[test]
    fn canceled_dialog_is_a_clean_no_op() {
        let mut state = state_with_selection();
        let before = state.lines_for_test();
        let mut platform = MockPlatform::default();
        assert_eq!(
            perform_action(ExportAction::LoadTxt, &mut state, &mut platform).unwrap(),
            ExportOutcome::Cancelled
        );
        assert_eq!(state.lines_for_test(), before);
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
        state.set_cell_face_for_test(Coord { line: 1, column: 2 }, state.theme.selection.clone());
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
        state.set_lines_for_test(vec![vec![StyledAtom {
            face: state.theme.selection.clone(),
            contents: "a".to_owned(),
        }]]);
        let base = state.active_layer_id();
        assert!(state.add_layer_above(base));
        let upper = state.active_layer_id();
        state.set_lines_for_test(vec![vec![StyledAtom {
            face: state.theme.cursor_block.clone(),
            contents: "b".to_owned(),
        }]]);
        assert!(state.toggle_layer_visibility(base));
        state.apply_toolbar_action(ToolbarAction::Toggle(ToggleKind::MultiColorMode));
        state.apply_toolbar_action(ToolbarAction::SelectColor(crate::model::ColorId(14)));

        let json =
            serde_json::to_string(&project_document(&state, ViewportOffset::default())).unwrap();
        let LoadedJson::Project(restored) = project_from_json(&json).unwrap() else {
            panic!("version two project must use project loading")
        };

        let layers = restored.canvas.layers();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].id, base);
        assert!(!layers[0].visible);
        assert_eq!(
            dense_exchange::to_dense(&layers[0])[0][0].face,
            state.theme.selection
        );
        assert_eq!(layers[1].id, upper);
        assert_eq!(
            dense_exchange::to_dense(&layers[1])[0][0].face,
            state.theme.cursor_block
        );
        assert_eq!(restored.canvas.active_id(), upper);
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
            "canvas": {"rows": ["a", "b"]},
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

        let layers = restored.canvas.layers();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].id, LayerId(0));
        assert!(layers[0].visible);
        let lines = dense_exchange::to_dense(&layers[0]);
        assert_eq!(contents(&lines[0]), "a");
        assert!(
            lines
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
    fn collapsed_txt_save_exports_the_whole_canvas_and_can_become_active() {
        let path = temp_path("txt");
        let mut state = Editor::new(&ThemeConfig::default(), "test");
        state.set_lines_for_test(lines_from_text("one\ntwo"));
        let mut platform = MockPlatform {
            save: Some(path.clone()),
            ..MockPlatform::default()
        };

        assert_eq!(
            perform_action(ExportAction::SaveTxt, &mut state, &mut platform).unwrap(),
            ExportOutcome::Saved {
                path: path.clone(),
                format: FileKind::Txt,
            }
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn imports_paste_at_the_cursor_without_replacing_surrounding_content() {
        let text_path = temp_path("txt");
        fs::write(&text_path, "XY\nZ ").unwrap();
        let mut target = Editor::new(&ThemeConfig::default(), "test");
        target.set_lines_for_test(lines_from_text("aaaa\nbbbb\ncccc"));
        target.move_to(Coord { line: 1, column: 1 });
        let mut platform = MockPlatform {
            open: Some(text_path.clone()),
            ..MockPlatform::default()
        };

        assert_eq!(
            perform_action(ExportAction::ImportTxt, &mut target, &mut platform).unwrap(),
            ExportOutcome::DocumentImported
        );
        let lines = target.lines_for_test();
        assert_eq!(contents(&lines[0]), "aaaa");
        assert_eq!(contents(&lines[1]), "bXYb");
        assert_eq!(contents(&lines[2]), "cZ c");
        assert_eq!(target.grid.cursor_pos, Coord { line: 1, column: 1 });
        assert!(target.selection.is_collapsed());
        let _ = fs::remove_file(text_path);
    }

    #[test]
    fn json_import_uses_canvas_data_without_restoring_project_state() {
        let path = temp_path("json");
        let mut source = Editor::new(&ThemeConfig::default(), "source");
        source.set_lines_for_test(lines_from_text("XY"));
        source.move_to(Coord { line: 0, column: 1 });
        save_native_json(&path, &source, ViewportOffset { x: 20, y: 30 }).unwrap();

        let mut target = Editor::new(&ThemeConfig::default(), "target");
        target.set_lines_for_test(lines_from_text("aaaa\nbbbb"));
        target.move_to(Coord { line: 1, column: 1 });
        let mut platform = MockPlatform {
            open: Some(path.clone()),
            ..MockPlatform::default()
        };

        assert_eq!(
            perform_action(ExportAction::ImportJson, &mut target, &mut platform).unwrap(),
            ExportOutcome::DocumentImported
        );
        let lines = target.lines_for_test();
        assert_eq!(contents(&lines[0]), "aaaa");
        assert_eq!(contents(&lines[1]), "bXYb");
        assert_eq!(target.grid.cursor_pos, Coord { line: 1, column: 1 });
        let _ = fs::remove_file(path);
    }

    #[test]
    fn txt_load_replaces_canvas_and_resets_cursor_selection_and_transients() {
        let path = temp_path("txt");
        fs::write(&path, "new\nok").unwrap();
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
            ExportOutcome::DocumentLoaded {
                path: path.clone(),
                format: FileKind::Txt,
            }
        );
        assert_eq!(state.grid.cursor_pos, Coord::default());
        assert!(state.selection.is_collapsed());
        assert!(!state.has_shape_preview());
        assert_eq!(state.cursor_mode, CursorMode::Shapes);
        assert_eq!(state.selected_text(), "n");
        assert_eq!(state.toolbar.durable_selections(), menu_selections);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn native_json_round_trip_matches_session_state_and_collapses_selection() {
        let path = temp_path("json");
        let mut source = state_with_selection();
        source.set_lines_for_test(vec![row_atoms("Qx  "), Vec::new(), row_atoms(" z")]);
        source.set_cell_face_for_test(Coord::default(), source.theme.selection.clone());
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
        save_native_json(&path, &source, source_viewport).unwrap();

        let mut target = Editor::new(&ThemeConfig::default(), "target");
        target.set_lines_for_test(lines_from_text("unrelated outside content"));
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
            ExportOutcome::ProjectLoaded {
                path: path.clone(),
                zoom: 0,
            }
        );
        assert_eq!(target.grid.cursor_pos, Coord { line: 2, column: 1 });
        assert_eq!(target.selection.anchor(), Coord { line: 2, column: 1 });
        assert_eq!(target.selection.active(), Coord { line: 2, column: 1 });
        assert_eq!(target_viewport, source_viewport);
        assert_eq!(target.toolbar.durable_selections(), source_menu);
        assert_eq!(target.cursor_mode, CursorMode::Utilities);
        assert_eq!(
            target
                .lines_for_test()
                .iter()
                .map(|line| line
                    .iter()
                    .map(|atom| atom.contents.as_str())
                    .collect::<String>())
                .collect::<Vec<_>>(),
            ["Qx", "", " z"]
        );
        assert_eq!(
            target.lines_for_test()[0][0].face,
            source.lines_for_test()[0][0].face
        );
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
        value["canvas"]["layers"][0]["lines"][0] = serde_json::to_value(vec![StyledAtom {
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
        let legacy_atom = StyledAtom {
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
