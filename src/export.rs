use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::editor::EditorState;
use crate::layout::ViewportOffset;
use crate::model::{Atom, Face};
use crate::selection::CanvasSelection;
use crate::toolbar::DurableMenuSelections;

const PROJECT_FORMAT: &str = "ascdraw";
const PROJECT_VERSION: u32 = 1;
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

impl ExportAction {
    pub fn is_png(self) -> bool {
        matches!(self, Self::ClipboardPng | Self::SavePng)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportOutcome {
    Unchanged,
    DocumentLoaded,
    ProjectLoaded,
    CanvasCleared,
}

pub trait ExportPlatform {
    fn set_clipboard_text(&mut self, text: &str) -> Result<()>;
    fn clipboard_text(&mut self) -> Result<String>;
    fn choose_save_path(&mut self, kind: FileKind) -> Option<PathBuf>;
    fn choose_open_path(&mut self, kind: FileKind) -> Option<PathBuf>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Txt,
    Json,
}

pub struct NativeExportPlatform;

impl ExportPlatform for NativeExportPlatform {
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
}

pub fn copy_selection(state: &EditorState, platform: &mut impl ExportPlatform) -> Result<()> {
    platform.set_clipboard_text(&state.selected_text())
}

/// Copies the normalized selection before clearing it. Keeping the clipboard
/// write first makes a failed external operation an editor-state no-op.
pub fn cut_selection(state: &mut EditorState, platform: &mut impl ExportPlatform) -> Result<bool> {
    platform.set_clipboard_text(&state.selected_text())?;
    let before = state.edit_snapshot();
    state.clear_selection();
    Ok(!before.same_document(&state.edit_snapshot()))
}

pub fn paste_selection(
    state: &mut EditorState,
    platform: &mut impl ExportPlatform,
) -> Result<bool> {
    let text = platform.clipboard_text()?;
    Ok(state.paste_text_rectangle(&text))
}

fn file_kind_details(kind: FileKind) -> (&'static str, &'static str) {
    match kind {
        FileKind::Txt => ("Text", "txt"),
        FileKind::Json => ("JSON", "json"),
    }
}

fn default_file_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Txt => "selection.txt",
        FileKind::Json => "ascdraw.json",
    }
}

pub fn perform(
    action: ExportAction,
    state: &mut EditorState,
    viewport: &mut ViewportOffset,
    platform: &mut impl ExportPlatform,
) -> Result<ExportOutcome> {
    match action {
        ExportAction::ClipboardTxt => {
            platform.set_clipboard_text(&state.selected_text())?;
            Ok(ExportOutcome::Unchanged)
        }
        ExportAction::SaveTxt => {
            let Some(path) = platform.choose_save_path(FileKind::Txt) else {
                return Ok(ExportOutcome::Unchanged);
            };
            fs::write(&path, state.selected_text())
                .with_context(|| format!("failed to write {}", path.display()))?;
            Ok(ExportOutcome::Unchanged)
        }
        ExportAction::SaveJson => {
            let Some(path) = platform.choose_save_path(FileKind::Json) else {
                return Ok(ExportOutcome::Unchanged);
            };
            save_project_json(&path, state, *viewport)?;
            Ok(ExportOutcome::Unchanged)
        }
        ExportAction::LoadTxt => {
            let Some(path) = platform.choose_open_path(FileKind::Txt) else {
                return Ok(ExportOutcome::Unchanged);
            };
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            state.replace_canvas(lines_from_text(&text));
            Ok(ExportOutcome::DocumentLoaded)
        }
        ExportAction::LoadJson => {
            let Some(path) = platform.choose_open_path(FileKind::Json) else {
                return Ok(ExportOutcome::Unchanged);
            };
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            match project_from_json(&contents)? {
                LoadedJson::Project(project) => {
                    let mut staged = state.clone();
                    staged.restore_project(
                        project.lines,
                        project.cursor,
                        project.selection,
                        &project.menu_selections,
                    );
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
        ExportAction::ClipboardPng | ExportAction::SavePng => Ok(ExportOutcome::Unchanged),
    }
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
    rows: Vec<String>,
}

#[derive(Debug)]
struct RestoredProject {
    lines: Vec<Vec<Atom>>,
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

fn project_document(state: &EditorState, viewport: ViewportOffset) -> ProjectDocument {
    ProjectDocument {
        format: PROJECT_FORMAT.to_owned(),
        version: PROJECT_VERSION,
        canvas: ProjectCanvas {
            rows: state
                .grid
                .lines
                .iter()
                .map(|line| line.iter().map(|atom| atom.contents.as_str()).collect())
                .collect(),
        },
        cursor: state.grid.cursor_pos,
        selection: state.selection,
        viewport,
        menu_selections: state.toolbar.durable_selections(),
    }
}

fn save_project_json(path: &Path, state: &EditorState, viewport: ViewportOffset) -> Result<()> {
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
    if document.version != PROJECT_VERSION {
        bail!("unsupported ascdraw project version {}", document.version);
    }
    validate_coordinate("cursor", document.cursor)?;
    validate_coordinate("selection anchor", document.selection.anchor())?;
    validate_coordinate("selection active", document.selection.active())?;
    let mut lines: Vec<Vec<Atom>> = document
        .canvas
        .rows
        .iter()
        .map(|row| row_atoms(row))
        .collect();
    lines = nonempty_lines(lines);
    for (name, coord) in [
        ("cursor", document.cursor),
        ("selection anchor", document.selection.anchor()),
        ("selection active", document.selection.active()),
    ] {
        pad_to_coordinate(&mut lines, name, coord)?;
    }
    Ok(RestoredProject {
        lines,
        cursor: document.cursor,
        selection: document.selection,
        viewport: document.viewport,
        menu_selections: document.menu_selections,
    })
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
    use crate::toolbar::{MainMode, ToolbarAction};

    #[derive(Default)]
    struct MockPlatform {
        clipboard: Option<String>,
        fail_clipboard_read: bool,
        fail_clipboard_write: bool,
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
    }

    fn state_with_selection() -> EditorState {
        let mut state = EditorState::new(&ThemeConfig::default(), "test");
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
        state: &mut EditorState,
        platform: &mut MockPlatform,
    ) -> Result<ExportOutcome> {
        perform(action, state, &mut ViewportOffset::default(), platform)
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
    fn clipboard_copy_preserves_blank_rows_and_trailing_spaces_without_mutating_state() {
        let mut state = EditorState::new(&ThemeConfig::default(), "test");
        state.grid.lines = lines_from_text("ab\n\nz");
        state.move_to(Coord::default());
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Right);
        state.extend_selection(crate::model::Direction::Down);
        state.extend_selection(crate::model::Direction::Down);
        let before = state.clone();
        let mut platform = MockPlatform::default();
        copy_selection(&state, &mut platform).unwrap();
        assert_eq!(platform.clipboard.as_deref(), Some("ab \n   \nz  "));
        assert_eq!(state.grid.lines, before.grid.lines);
        assert_eq!(state.selection, before.selection);
        assert_eq!(state.grid.cursor_pos, before.grid.cursor_pos);
    }

    #[test]
    fn cut_copies_exact_ragged_rectangle_then_clears_only_that_rectangle() {
        let mut state = EditorState::new(&ThemeConfig::default(), "test");
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

        let mut blank = EditorState::new(&ThemeConfig::default(), "test");
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
        let mut state = EditorState::new(&ThemeConfig::default(), "test");
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
        let mut state = EditorState::new(&ThemeConfig::default(), "test");
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
            ExportOutcome::Unchanged
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
    fn project_json_is_human_readable_and_contains_no_cell_styles() {
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
        assert_eq!(value["canvas"]["rows"][1], "  ab   ");
        for forbidden in ["face", "fg", "bg", "underline", "attributes", "contents"] {
            assert!(!json.contains(&format!("\"{forbidden}\"")));
        }
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
            option: 3,
        });
        let source_menu = source.toolbar.durable_selections();
        let source_viewport = ViewportOffset { x: -12, y: 34 };
        save_project_json(&path, &source, source_viewport).unwrap();

        let mut target = EditorState::new(&ThemeConfig::default(), "target");
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
        assert!(
            target
                .grid
                .lines
                .iter()
                .flatten()
                .all(|atom| atom.face == Face::default())
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn project_save_uses_project_filename_and_does_not_mutate_editor_state() {
        assert_eq!(default_file_name(FileKind::Txt), "selection.txt");
        assert_eq!(default_file_name(FileKind::Json), "ascdraw.json");

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
        let source = EditorState::new(&ThemeConfig::default(), "source");
        let mut value =
            serde_json::to_value(project_document(&source, ViewportOffset { x: 4, y: -8 }))
                .unwrap();
        value["canvas"]["rows"] = serde_json::json!(["😀"]);
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
    fn unsupported_project_format_and_version_are_rejected() {
        let state = EditorState::new(&ThemeConfig::default(), "source");
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
            option: 3,
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
