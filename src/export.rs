use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::editor::EditorState;
use crate::model::{Atom, Face};
use crate::selection::selected_atoms;

const SELECTION_DOCUMENT_VERSION: u32 = 1;

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
            .set_file_name(format!("selection.{extension}"))
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

pub fn perform(
    action: ExportAction,
    state: &mut EditorState,
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
            save_selection_json(&path, state)?;
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
            state.replace_canvas(lines_from_json(&contents)?);
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
struct SelectionDocument {
    version: u32,
    width: usize,
    height: usize,
    lines: Vec<Vec<Atom>>,
}

#[derive(Deserialize)]
struct NativeJsonDocument {
    lines: Vec<Vec<Atom>>,
}

fn selection_document(state: &EditorState) -> SelectionDocument {
    let bounds = state.selection_bounds();
    SelectionDocument {
        version: SELECTION_DOCUMENT_VERSION,
        width: bounds.width(),
        height: bounds.height(),
        lines: selected_atoms(&state.grid.lines, bounds),
    }
}

fn save_selection_json(path: &Path, state: &EditorState) -> Result<()> {
    let contents = serde_json::to_string_pretty(&selection_document(state))
        .context("failed to serialize selected canvas rectangle")?;
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

fn lines_from_json(contents: &str) -> Result<Vec<Vec<Atom>>> {
    if let Ok(document) = serde_json::from_str::<SelectionDocument>(contents) {
        if document.version != SELECTION_DOCUMENT_VERSION {
            bail!("unsupported selection JSON version {}", document.version);
        }
        if document.height != document.lines.len() {
            bail!(
                "selection JSON height {} does not match {} rows",
                document.height,
                document.lines.len()
            );
        }
        return normalize_lines(document.lines, document.width);
    }
    let document: NativeJsonDocument =
        serde_json::from_str(contents).context("failed to parse canvas JSON")?;
    Ok(nonempty_lines(document.lines))
}

fn normalize_lines(lines: Vec<Vec<Atom>>, width: usize) -> Result<Vec<Vec<Atom>>> {
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
    line.iter()
        .map(|atom| {
            UnicodeWidthStr::width(atom.contents.as_str())
                .max(usize::from(!atom.contents.is_empty()))
        })
        .sum()
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

    #[test]
    fn clipboard_receives_exact_selected_text_only() {
        let mut state = state_with_selection();
        let mut platform = MockPlatform::default();
        assert_eq!(
            perform(ExportAction::ClipboardTxt, &mut state, &mut platform).unwrap(),
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
            perform(ExportAction::LoadTxt, &mut state, &mut platform).unwrap(),
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
            perform(ExportAction::SaveTxt, &mut state, &mut platform).unwrap(),
            ExportOutcome::Unchanged
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "ab\ncd");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn selection_json_round_trip_preserves_dimensions_faces_and_origin() {
        let mut state = state_with_selection();
        state.grid.lines[1][2].face.fg = "#ff0000".to_string();
        let document = selection_document(&state);
        assert_eq!((document.width, document.height), (2, 2));
        assert_eq!(document.lines[0][0].face.fg, "#ff0000");
        let json = serde_json::to_string(&document).unwrap();
        let loaded = lines_from_json(&json).unwrap();
        assert_eq!(loaded, document.lines);
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
        state.toggle_shape_preview();
        let mut platform = MockPlatform {
            open: Some(path.clone()),
            ..MockPlatform::default()
        };

        assert_eq!(
            perform(ExportAction::LoadTxt, &mut state, &mut platform).unwrap(),
            ExportOutcome::DocumentLoaded
        );
        assert_eq!(state.grid.cursor_pos, Coord::default());
        assert!(state.selection.is_collapsed());
        assert!(state.lines_with_shape_preview().is_none());
        assert_eq!(state.cursor_mode, CursorMode::Shapes);
        assert_eq!(state.selected_text(), "n");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn json_load_accepts_saved_selection_and_resets_to_normalized_origin() {
        let path = temp_path("json");
        let mut source = state_with_selection();
        source.grid.lines[1][2].face.fg = "#0000ff".to_string();
        save_selection_json(&path, &source).unwrap();

        let mut target = EditorState::new(&ThemeConfig::default(), "target");
        target.grid.lines = lines_from_text("unrelated outside content");
        target.move_to(Coord { line: 0, column: 5 });
        target.extend_selection(crate::model::Direction::Right);
        let mut platform = MockPlatform {
            open: Some(path.clone()),
            ..MockPlatform::default()
        };
        assert_eq!(
            perform(ExportAction::LoadJson, &mut target, &mut platform).unwrap(),
            ExportOutcome::DocumentLoaded
        );
        assert_eq!(target.grid.cursor_pos, Coord::default());
        assert!(target.selection.is_collapsed());
        assert_eq!(target.grid.lines[0][0].contents, "a");
        assert_eq!(target.grid.lines[0][0].face.fg, "#0000ff");
        assert_eq!(target.grid.lines.len(), 2);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn clear_replaces_the_canvas_without_using_the_platform() {
        let mut state = state_with_selection();
        let mut platform = MockPlatform {
            fail_clipboard_read: true,
            fail_clipboard_write: true,
            ..MockPlatform::default()
        };

        assert_eq!(
            perform(ExportAction::Clear, &mut state, &mut platform).unwrap(),
            ExportOutcome::CanvasCleared
        );
        assert!(state.content_cells().is_empty());
        assert_eq!(state.grid.cursor_pos, Coord { line: 2, column: 3 });
        assert!(state.selection.is_collapsed());
        assert!(platform.save.is_none());
        assert!(platform.open.is_none());
        assert!(platform.clipboard.is_none());
    }
}
