use crate::app::CursorMode;
use crate::editor_event::EditorState;

use super::Editor;

impl Editor {
    pub fn state(&self) -> EditorState {
        if self.jump_mode.is_some() {
            return EditorState::JumpMode;
        }
        if self.toolbar.export_menu_open() {
            return EditorState::ExportMode;
        }
        if self.toolbar.pending_shortcut().is_some() {
            return EditorState::ToolbarMode;
        }
        if self.move_lift.is_some() {
            return EditorState::MoveMode;
        }
        if self.line_preview.is_some() {
            return EditorState::LinePreviewMode;
        }
        if self.shape_preview.is_some() {
            return EditorState::ShapePreviewMode;
        }
        if self.single_replace_pending {
            return EditorState::ReplaceOneMode;
        }
        if !self.selection.is_collapsed() {
            return EditorState::SelectionMode(self.cursor_mode);
        }
        match self.cursor_mode {
            CursorMode::Text => EditorState::TextMode,
            CursorMode::Insert => EditorState::InsertMode,
            CursorMode::Replace => EditorState::ReplaceMode,
            CursorMode::MoveDraw => EditorState::LineMode,
            CursorMode::Stamp => EditorState::StampMode,
            CursorMode::Shapes => EditorState::ShapeMode,
            CursorMode::Utilities => EditorState::UtilityMode,
            CursorMode::Navigation => EditorState::NavigationMode,
        }
    }

    pub fn cancel_current_state(&mut self) -> bool {
        match self.state() {
            EditorState::JumpMode => self.cancel_jump(),
            EditorState::ExportMode => {
                self.toolbar.close_export_menu();
                true
            }
            EditorState::ToolbarMode => {
                self.toolbar.cancel_shortcut();
                true
            }
            EditorState::MoveMode => self.cancel_move_lift(),
            EditorState::LinePreviewMode => self.cancel_line_preview(),
            EditorState::ShapePreviewMode => {
                self.shape_preview = None;
                true
            }
            EditorState::ReplaceOneMode
            | EditorState::TextMode
            | EditorState::InsertMode
            | EditorState::ReplaceMode => self.cancel_text_entry(),
            EditorState::SelectionMode(_) => {
                self.collapse_selection();
                true
            }
            EditorState::LineMode
            | EditorState::StampMode
            | EditorState::ShapeMode
            | EditorState::UtilityMode
            | EditorState::NavigationMode => false,
        }
    }
}
