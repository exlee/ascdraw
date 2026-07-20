use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::CursorMode;
use crate::input::{ClipboardCommand, HistoryCommand, direction_for_key};
use crate::model::Direction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum EditorState {
    JumpMode,
    ToolbarMode,
    ExportMode,
    ReplaceOneMode,
    MoveMode,
    LinePreviewMode,
    ShapePreviewMode,
    SelectionMode(CursorMode),
    TextMode,
    InsertMode,
    ReplaceMode,
    LineMode,
    StampMode,
    ShapeMode,
    UtilityMode,
    NavigationMode,
}

impl EditorState {
    pub fn accepts_text(self) -> bool {
        match self {
            Self::SelectionMode(mode) => mode.accepts_text(),
            Self::ReplaceOneMode | Self::TextMode | Self::InsertMode | Self::ReplaceMode => true,
            _ => false,
        }
    }

    pub fn can_start_jump(self) -> bool {
        !matches!(
            self,
            Self::ReplaceOneMode | Self::TextMode | Self::InsertMode | Self::ReplaceMode
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KeyInput<'a> {
    pub key: &'a Key,
    pub text: Option<&'a str>,
    pub repeat: bool,
    pub modifiers: ModifiersState,
}

#[derive(Debug, Clone, Copy)]
#[allow(clippy::enum_variant_names)]
pub enum KeyType<'a> {
    CancelKey(KeyInput<'a>),
    CutKey(KeyInput<'a>),
    CopyKey(KeyInput<'a>),
    PasteKey(KeyInput<'a>),
    UndoKey(KeyInput<'a>),
    RedoKey(KeyInput<'a>),
    DirectionKey {
        input: KeyInput<'a>,
        direction: Direction,
    },
    TextKey(KeyInput<'a>),
    OtherKey(KeyInput<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAction {
    Clear,
    ReplaceOne,
    Move(Direction),
}

impl<'a> KeyType<'a> {
    pub fn input(self) -> KeyInput<'a> {
        match self {
            Self::CancelKey(input)
            | Self::CutKey(input)
            | Self::CopyKey(input)
            | Self::PasteKey(input)
            | Self::UndoKey(input)
            | Self::RedoKey(input)
            | Self::TextKey(input)
            | Self::OtherKey(input)
            | Self::DirectionKey { input, .. } => input,
        }
    }

    pub fn clipboard_command(self) -> Option<ClipboardCommand> {
        match self {
            Self::CopyKey(_) => Some(ClipboardCommand::Copy),
            Self::CutKey(_) => Some(ClipboardCommand::Cut),
            Self::PasteKey(_) => Some(ClipboardCommand::Paste),
            _ => None,
        }
    }

    pub fn history_command(self) -> Option<HistoryCommand> {
        match self {
            Self::UndoKey(_) => Some(HistoryCommand::Undo),
            Self::RedoKey(_) => Some(HistoryCommand::Redo),
            _ => None,
        }
    }

    pub fn is_cancel(self) -> bool {
        matches!(self, Self::CancelKey(_))
    }
}

pub fn selection_action(state: EditorState, key_type: KeyType<'_>) -> Option<SelectionAction> {
    if !matches!(state, EditorState::SelectionMode(_)) {
        return None;
    }
    let input = key_type.input();
    if matches!(input.key, Key::Named(NamedKey::Backspace)) {
        return Some(SelectionAction::Clear);
    }
    if input.modifiers == ModifiersState::empty()
        && matches!(input.key, Key::Character(text) if text == "r")
    {
        return Some(SelectionAction::ReplaceOne);
    }
    match key_type {
        KeyType::DirectionKey { direction, .. } if input.modifiers == ModifiersState::ALT => {
            Some(SelectionAction::Move(direction))
        }
        _ => None,
    }
}

pub fn classify_key(
    state: EditorState,
    cursor_accepts_text: bool,
    input: KeyInput<'_>,
) -> KeyType<'_> {
    if is_cancel_key(input.key, input.modifiers) {
        return KeyType::CancelKey(input);
    }
    if let Some(command) = clipboard_key(input.key, input.modifiers) {
        return match command {
            ClipboardCommand::Copy => KeyType::CopyKey(input),
            ClipboardCommand::Cut => KeyType::CutKey(input),
            ClipboardCommand::Paste => KeyType::PasteKey(input),
        };
    }
    if let Some(command) = history_key(state, cursor_accepts_text, input.key, input.modifiers) {
        return match command {
            HistoryCommand::Undo => KeyType::UndoKey(input),
            HistoryCommand::Redo => KeyType::RedoKey(input),
        };
    }
    if let Some(direction) = direction_for_key(input.key) {
        return KeyType::DirectionKey { input, direction };
    }
    if input
        .text
        .is_some_and(|text| !text.chars().all(char::is_control))
    {
        return KeyType::TextKey(input);
    }
    KeyType::OtherKey(input)
}

fn is_cancel_key(key: &Key, modifiers: ModifiersState) -> bool {
    if matches!(key, Key::Named(NamedKey::Escape)) {
        return true;
    }
    modifiers.control_key()
        && !modifiers.alt_key()
        && !modifiers.super_key()
        && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("c") || text.eq_ignore_ascii_case("g"))
}

fn clipboard_key(key: &Key, modifiers: ModifiersState) -> Option<ClipboardCommand> {
    if modifiers.alt_key() || !(modifiers.control_key() || modifiers.super_key()) {
        return None;
    }
    match key {
        Key::Character(text)
            if text.eq_ignore_ascii_case("c")
                && modifiers.super_key()
                && !modifiers.control_key() =>
        {
            Some(ClipboardCommand::Copy)
        }
        Key::Character(text) if text.eq_ignore_ascii_case("x") => Some(ClipboardCommand::Cut),
        Key::Character(text) if text.eq_ignore_ascii_case("v") => Some(ClipboardCommand::Paste),
        _ => None,
    }
}

fn history_key(
    state: EditorState,
    cursor_accepts_text: bool,
    key: &Key,
    modifiers: ModifiersState,
) -> Option<HistoryCommand> {
    if !modifiers.alt_key() && (modifiers.control_key() || modifiers.super_key()) {
        return match key {
            Key::Character(text) if text.eq_ignore_ascii_case("z") => Some(HistoryCommand::Undo),
            Key::Character(text) if text.eq_ignore_ascii_case("r") => Some(HistoryCommand::Redo),
            _ => None,
        };
    }
    if state.accepts_text()
        || cursor_accepts_text
        || (modifiers != ModifiersState::empty() && modifiers != ModifiersState::SHIFT)
    {
        return None;
    }
    match key {
        Key::Character(text) if text == "u" => Some(HistoryCommand::Undo),
        Key::Character(text) if text == "U" => Some(HistoryCommand::Redo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(key: &Key, modifiers: ModifiersState) -> KeyInput<'_> {
        KeyInput {
            key,
            text: None,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn classifies_cancel_keys_before_clipboard_keys() {
        for (key, modifiers) in [
            (Key::Named(NamedKey::Escape), ModifiersState::empty()),
            (Key::Character("c".into()), ModifiersState::CONTROL),
            (Key::Character("g".into()), ModifiersState::CONTROL),
        ] {
            assert!(matches!(
                classify_key(EditorState::LineMode, false, input(&key, modifiers)),
                KeyType::CancelKey(_)
            ));
        }
    }

    #[test]
    fn classifies_cut_copy_and_paste_with_the_actual_key_attached() {
        for (key, modifiers, expected) in [
            (
                Key::Character("x".into()),
                ModifiersState::CONTROL,
                ClipboardCommand::Cut,
            ),
            (
                Key::Character("c".into()),
                ModifiersState::SUPER,
                ClipboardCommand::Copy,
            ),
            (
                Key::Character("v".into()),
                ModifiersState::CONTROL,
                ClipboardCommand::Paste,
            ),
        ] {
            let classified = classify_key(EditorState::LineMode, false, input(&key, modifiers));
            assert_eq!(classified.clipboard_command(), Some(expected));
            assert_eq!(classified.input().key, &key);
        }

        for modifiers in [
            ModifiersState::SUPER,
            ModifiersState::SUPER | ModifiersState::SHIFT,
        ] {
            for key in ["c", "C"] {
                assert_eq!(
                    classify_key(
                        EditorState::LineMode,
                        false,
                        input(&Key::Character(key.into()), modifiers),
                    )
                    .clipboard_command(),
                    Some(ClipboardCommand::Copy)
                );
            }
        }
        for modifiers in [
            ModifiersState::CONTROL,
            ModifiersState::SUPER,
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            ModifiersState::SUPER | ModifiersState::SHIFT,
        ] {
            for (key, expected) in [
                ("x", ClipboardCommand::Cut),
                ("X", ClipboardCommand::Cut),
                ("v", ClipboardCommand::Paste),
                ("V", ClipboardCommand::Paste),
            ] {
                assert_eq!(
                    classify_key(
                        EditorState::LineMode,
                        false,
                        input(&Key::Character(key.into()), modifiers),
                    )
                    .clipboard_command(),
                    Some(expected)
                );
            }
        }
        for (key, modifiers) in [
            ("c", ModifiersState::CONTROL),
            ("c", ModifiersState::CONTROL | ModifiersState::ALT),
            ("x", ModifiersState::SUPER | ModifiersState::ALT),
        ] {
            assert_eq!(
                classify_key(
                    EditorState::LineMode,
                    false,
                    input(&Key::Character(key.into()), modifiers),
                )
                .clipboard_command(),
                None
            );
        }
    }

    #[test]
    fn classifies_global_and_plain_history_shortcuts_in_their_valid_states() {
        for state in [
            EditorState::LineMode,
            EditorState::StampMode,
            EditorState::ShapeMode,
            EditorState::UtilityMode,
            EditorState::TextMode,
            EditorState::InsertMode,
            EditorState::ReplaceMode,
        ] {
            for modifiers in [
                ModifiersState::CONTROL,
                ModifiersState::SUPER,
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                ModifiersState::SUPER | ModifiersState::SHIFT,
            ] {
                assert_eq!(
                    classify_key(
                        state,
                        state.accepts_text(),
                        input(&Key::Character("z".into()), modifiers),
                    )
                    .history_command(),
                    Some(HistoryCommand::Undo)
                );
                assert_eq!(
                    classify_key(
                        state,
                        state.accepts_text(),
                        input(&Key::Character("R".into()), modifiers),
                    )
                    .history_command(),
                    Some(HistoryCommand::Redo)
                );
            }
        }

        for state in [
            EditorState::LineMode,
            EditorState::StampMode,
            EditorState::ShapeMode,
            EditorState::UtilityMode,
        ] {
            assert_eq!(
                classify_key(
                    state,
                    false,
                    input(&Key::Character("u".into()), ModifiersState::empty()),
                )
                .history_command(),
                Some(HistoryCommand::Undo)
            );
            for modifiers in [ModifiersState::empty(), ModifiersState::SHIFT] {
                assert_eq!(
                    classify_key(state, false, input(&Key::Character("U".into()), modifiers),)
                        .history_command(),
                    Some(HistoryCommand::Redo)
                );
            }
        }

        for state in [
            EditorState::TextMode,
            EditorState::InsertMode,
            EditorState::ReplaceMode,
        ] {
            for key in ["u", "U"] {
                assert_eq!(
                    classify_key(
                        state,
                        true,
                        input(&Key::Character(key.into()), ModifiersState::empty()),
                    )
                    .history_command(),
                    None
                );
            }
        }
        for modifiers in [
            ModifiersState::ALT,
            ModifiersState::CONTROL,
            ModifiersState::SUPER,
            ModifiersState::SHIFT | ModifiersState::ALT,
            ModifiersState::SHIFT | ModifiersState::CONTROL,
        ] {
            assert_eq!(
                classify_key(
                    EditorState::StampMode,
                    false,
                    input(&Key::Character("U".into()), modifiers),
                )
                .history_command(),
                None
            );
        }
        assert_eq!(
            classify_key(
                EditorState::StampMode,
                false,
                input(
                    &Key::Character("r".into()),
                    ModifiersState::CONTROL | ModifiersState::ALT,
                ),
            )
            .history_command(),
            None
        );
    }

    #[test]
    fn plain_history_keys_remain_text_in_text_accepting_states() {
        let key = Key::Character("u".into());
        let classified = classify_key(
            EditorState::TextMode,
            true,
            KeyInput {
                key: &key,
                text: Some("u"),
                repeat: false,
                modifiers: ModifiersState::empty(),
            },
        );

        assert!(matches!(classified, KeyType::TextKey(_)));
    }

    #[test]
    fn plain_history_keys_remain_text_under_non_text_overlays() {
        let key = Key::Character("u".into());
        let classified = classify_key(
            EditorState::ExportMode,
            true,
            KeyInput {
                key: &key,
                text: Some("u"),
                repeat: false,
                modifiers: ModifiersState::empty(),
            },
        );

        assert!(matches!(classified, KeyType::TextKey(_)));
    }

    #[test]
    fn selection_actions_ignore_the_base_mode() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Text,
            CursorMode::Insert,
            CursorMode::Replace,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
            CursorMode::Navigation,
        ] {
            let state = EditorState::SelectionMode(mode);
            let backspace = Key::Named(NamedKey::Backspace);
            assert_eq!(
                selection_action(
                    state,
                    classify_key(
                        state,
                        mode.accepts_text(),
                        input(&backspace, ModifiersState::empty())
                    )
                ),
                Some(SelectionAction::Clear)
            );

            let replace = Key::Character("r".into());
            assert_eq!(
                selection_action(
                    state,
                    classify_key(
                        state,
                        mode.accepts_text(),
                        input(&replace, ModifiersState::empty())
                    )
                ),
                Some(SelectionAction::ReplaceOne)
            );

            let right = Key::Named(NamedKey::ArrowRight);
            assert_eq!(
                selection_action(
                    state,
                    classify_key(
                        state,
                        mode.accepts_text(),
                        input(&right, ModifiersState::ALT)
                    )
                ),
                Some(SelectionAction::Move(Direction::Right))
            );
        }
    }
}
