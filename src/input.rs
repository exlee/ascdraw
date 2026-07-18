use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::app::AppConfig;
use crate::app::CursorMode;
use crate::layout::{PADDING, ViewportOffset, content_top_padding};
use crate::model::{Coord, Direction};
use crate::render::Renderer;
use crate::toolbar::{ToolbarState, UtilityKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    Move(Direction),
    Draw(Direction),
    DrawStamp(Direction),
    ApplyUtility(Direction),
    ExtendSelection(Direction),
    Erase(Direction),
    Clear,
    ClearAndBack,
    ToggleTextEntry,
    ToggleReplaceMode,
    BeginSingleReplace,
    CancelTextEntry,
    PlaceStamp,
    StartOrConfirmShape,
    Home,
    End,
    Backspace,
    Delete,
    Newline,
    InsertTab,
    ConfirmOrTextEntry,
    ConfirmOrReplace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewCommand {
    Pan(Direction),
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveSelectionCommand {
    BeginAndStep(Direction),
    BeginCloneAndStep(Direction, u64),
    Step(Direction),
    CloneAndStep(Direction, u64),
    ConfirmAndMove(Direction),
    Confirm,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinePreviewCommand {
    StartOrAdvance,
    Move(Direction),
    RemoveAnchor,
    Cancel,
}

pub fn line_preview_command(
    key: &Key,
    modifiers: ModifiersState,
    mode: CursorMode,
    active: bool,
) -> Option<LinePreviewCommand> {
    if mode != CursorMode::MoveDraw {
        return None;
    }
    if active
        && (matches!(key, Key::Named(NamedKey::Escape))
            || (modifiers == ModifiersState::CONTROL
                && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("g"))))
    {
        return Some(LinePreviewCommand::Cancel);
    }
    if modifiers.shift_key() || modifiers.alt_key() || modifiers.super_key() {
        return None;
    }
    if active && matches!(key, Key::Named(NamedKey::Backspace)) {
        return Some(LinePreviewCommand::RemoveAnchor);
    }
    if is_space_key(key) {
        return Some(LinePreviewCommand::StartOrAdvance);
    }
    active
        .then(|| direction_for_key(key))
        .flatten()
        .map(LinePreviewCommand::Move)
}

/// Resolves the modal keyboard interaction for moving an expanded selection.
/// Clipboard/history shortcuts are routed before this command by the runtime.
pub fn move_selection_command(
    key: &Key,
    modifiers: ModifiersState,
    active: bool,
    selection_expanded: bool,
    clone_press: Option<u64>,
) -> Option<MoveSelectionCommand> {
    if active {
        if matches!(key, Key::Named(NamedKey::Escape))
            || (modifiers == ModifiersState::CONTROL
                && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("g")))
        {
            return Some(MoveSelectionCommand::Cancel);
        }
        if is_space_key(key) || matches!(key, Key::Named(NamedKey::Enter)) {
            return (modifiers == ModifiersState::empty()).then_some(MoveSelectionCommand::Confirm);
        }
        let direction = direction_for_key(key)?;
        return move_selection_direction_command(
            direction,
            modifiers,
            active,
            selection_expanded,
            clone_press,
        );
    }
    let direction = direction_for_key(key)?;
    move_selection_direction_command(
        direction,
        modifiers,
        active,
        selection_expanded,
        clone_press,
    )
}

pub fn move_selection_direction_command(
    direction: Direction,
    modifiers: ModifiersState,
    active: bool,
    selection_expanded: bool,
    clone_press: Option<u64>,
) -> Option<MoveSelectionCommand> {
    if active {
        return match modifiers {
            _ if modifiers == (ModifiersState::ALT | ModifiersState::SHIFT) => {
                clone_press.map(|press| MoveSelectionCommand::CloneAndStep(direction, press))
            }
            _ if modifiers == ModifiersState::ALT => Some(MoveSelectionCommand::Step(direction)),
            _ if modifiers == ModifiersState::empty() => {
                Some(MoveSelectionCommand::ConfirmAndMove(direction))
            }
            _ => None,
        };
    }
    if !selection_expanded {
        return None;
    }
    if modifiers == (ModifiersState::ALT | ModifiersState::SHIFT) {
        return clone_press.map(|press| MoveSelectionCommand::BeginCloneAndStep(direction, press));
    }
    (modifiers == ModifiersState::ALT).then_some(MoveSelectionCommand::BeginAndStep(direction))
}

/// Resolves viewport-only commands for the Utilities View tool. These are
/// handled by the window runtime because they must never mutate the document.
pub fn view_command(
    key: &Key,
    modifiers: ModifiersState,
    mode: CursorMode,
    utility: UtilityKind,
) -> Option<ViewCommand> {
    if mode != CursorMode::Utilities
        || utility != UtilityKind::View
        || modifiers != ModifiersState::empty()
    {
        return None;
    }
    if is_space_key(key) {
        Some(ViewCommand::Center)
    } else {
        direction_for_key(key).map(ViewCommand::Pan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardCommand {
    Copy,
    Cut,
    Paste,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryCommand {
    Undo,
    Redo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionModifier {
    Shift,
    Alt,
    Control,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderedModifierTracker {
    held: Vec<DirectionModifier>,
    shift_press: u64,
}

impl OrderedModifierTracker {
    /// Updates the held modifier order from a winit state transition. Normal
    /// one-at-a-time transitions preserve press order. If a platform reports
    /// multiple additions together, Shift, Alt, Control is the stable fallback.
    pub fn update(&mut self, state: ModifiersState) {
        self.held.retain(|modifier| modifier.is_held(state));
        for modifier in [
            DirectionModifier::Shift,
            DirectionModifier::Alt,
            DirectionModifier::Control,
        ] {
            if modifier.is_held(state) && !self.held.contains(&modifier) {
                self.held.push(modifier);
                if modifier == DirectionModifier::Shift {
                    self.shift_press = self.shift_press.wrapping_add(1);
                }
            }
        }
    }

    pub fn clone_move_press(&self, modifiers: ModifiersState) -> Option<u64> {
        (modifiers == (ModifiersState::ALT | ModifiersState::SHIFT)
            && self.primary_and_secondary()
                == Some((DirectionModifier::Alt, Some(DirectionModifier::Shift))))
        .then_some(self.shift_press)
    }

    fn primary_and_secondary(&self) -> Option<(DirectionModifier, Option<DirectionModifier>)> {
        Some((*self.held.first()?, self.held.get(1).copied()))
    }
}

impl DirectionModifier {
    fn is_held(self, state: ModifiersState) -> bool {
        match self {
            Self::Shift => state.shift_key(),
            Self::Alt => state.alt_key(),
            Self::Control => state.control_key(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderedDirectionCommand {
    pub command: EditCommand,
    pub steps: usize,
}

pub fn ordered_direction_command(
    key: &Key,
    modifiers: ModifiersState,
    ordered: &OrderedModifierTracker,
    mode: CursorMode,
) -> Option<OrderedDirectionCommand> {
    let direction = direction_for_key(key)?;
    ordered_direction_command_for_direction(direction, modifiers, ordered, mode)
}

pub fn ordered_direction_command_for_direction(
    direction: Direction,
    modifiers: ModifiersState,
    ordered: &OrderedModifierTracker,
    mode: CursorMode,
) -> Option<OrderedDirectionCommand> {
    if mode.accepts_text()
        || modifiers.super_key()
        || (mode == CursorMode::Navigation && modifiers.control_key())
    {
        return None;
    }
    let (primary, secondary) = ordered.primary_and_secondary()?;
    let command = match primary {
        DirectionModifier::Shift => EditCommand::ExtendSelection(direction),
        DirectionModifier::Alt => EditCommand::Erase(direction),
        DirectionModifier::Control => tool_direction_command(direction, mode),
    };
    let steps = match (primary, secondary) {
        (_, None) => 1,
        (DirectionModifier::Shift, Some(DirectionModifier::Control))
        | (DirectionModifier::Alt, Some(DirectionModifier::Control))
        | (DirectionModifier::Control, Some(DirectionModifier::Alt)) => 5,
        (DirectionModifier::Shift, Some(DirectionModifier::Alt))
        | (DirectionModifier::Alt, Some(DirectionModifier::Shift))
        | (DirectionModifier::Control, Some(DirectionModifier::Shift)) => 10,
        (primary, Some(secondary)) if primary == secondary => 1,
        _ => 1,
    };
    Some(OrderedDirectionCommand { command, steps })
}

fn tool_direction_command(direction: Direction, mode: CursorMode) -> EditCommand {
    match mode {
        CursorMode::MoveDraw => EditCommand::Draw(direction),
        CursorMode::Stamp => EditCommand::DrawStamp(direction),
        CursorMode::Utilities => EditCommand::ApplyUtility(direction),
        CursorMode::Shapes => EditCommand::Move(direction),
        _ => EditCommand::Move(direction),
    }
}

/// Returns history commands before every mode-specific or configurable shortcut.
#[cfg(test)]
pub fn history_command(
    key: &Key,
    modifiers: ModifiersState,
    mode: CursorMode,
) -> Option<HistoryCommand> {
    if !modifiers.alt_key() && (modifiers.control_key() || modifiers.super_key()) {
        return match key {
            Key::Character(text) if text.eq_ignore_ascii_case("z") => Some(HistoryCommand::Undo),
            Key::Character(text) if text.eq_ignore_ascii_case("r") => Some(HistoryCommand::Redo),
            _ => None,
        };
    }
    if mode.accepts_text()
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

/// Returns global clipboard commands before mode-specific key handling.
#[cfg(test)]
pub fn clipboard_command(key: &Key, modifiers: ModifiersState) -> Option<ClipboardCommand> {
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

pub fn edit_command(
    key: &Key,
    repeat: bool,
    modifiers: ModifiersState,
    mode: CursorMode,
) -> Option<EditCommand> {
    if repeat && matches!(key, Key::Named(NamedKey::Escape)) {
        return None;
    }
    edit_command_for_key(key, modifiers, mode)
}

fn edit_command_for_key(
    key: &Key,
    modifiers: ModifiersState,
    mode: CursorMode,
) -> Option<EditCommand> {
    if mode.accepts_text()
        && (matches!(key, Key::Named(NamedKey::Escape))
            || (modifiers.control_key()
                && !modifiers.alt_key()
                && !modifiers.super_key()
                && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("g"))))
    {
        return Some(EditCommand::CancelTextEntry);
    }

    if !mode.accepts_text() && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("i"))
    {
        return Some(EditCommand::ToggleTextEntry);
    }

    if !mode.accepts_text()
        && modifiers.shift_key()
        && matches!(key, Key::Character(text) if text.eq_ignore_ascii_case("r"))
    {
        return Some(EditCommand::ToggleReplaceMode);
    }

    if matches!(key, Key::Named(NamedKey::Enter)) {
        let in_shape = mode == CursorMode::Shapes;
        return Some(match (in_shape, modifiers.shift_key()) {
            (true, true) => EditCommand::ConfirmOrTextEntry,
            (true, false) => EditCommand::ConfirmOrReplace,
            (false, true) => EditCommand::ToggleTextEntry,
            (false, false) => EditCommand::ToggleReplaceMode,
        });
    }

    if modifiers == ModifiersState::empty()
        && matches!(key, Key::Character(text) if text == "r")
        && !matches!(
            mode,
            CursorMode::Text | CursorMode::Insert | CursorMode::Replace
        )
    {
        return Some(EditCommand::BeginSingleReplace);
    }

    if mode == CursorMode::Navigation {
        if modifiers.shift_key()
            && !modifiers.alt_key()
            && !modifiers.control_key()
            && !modifiers.super_key()
        {
            return direction_for_key(key).map(EditCommand::ExtendSelection);
        }
        return (modifiers == ModifiersState::empty())
            .then(|| direction_for_key(key).map(EditCommand::Move))
            .flatten();
    }

    if matches!(key, Key::Named(NamedKey::Backspace)) {
        return match mode {
            CursorMode::Insert | CursorMode::Text => Some(EditCommand::Backspace),
            CursorMode::Replace => Some(EditCommand::ClearAndBack),
            _ => Some(EditCommand::Clear),
        };
    }

    if let Some(direction) = cursor_direction_for_key(key, mode)
        && let Some(command) = edit_direction_command(direction, modifiers, mode, false)
    {
        return Some(command);
    }

    if modifiers.super_key()
        || modifiers.alt_key()
        || (mode.accepts_text() && modifiers.control_key())
        || (!mode.accepts_text() && modifiers.shift_key())
    {
        return None;
    }

    if mode == CursorMode::MoveDraw {
        return None;
    }

    if mode == CursorMode::Text {
        return match key {
            Key::Named(NamedKey::Delete) => Some(EditCommand::Delete),
            Key::Named(NamedKey::Tab) => Some(EditCommand::InsertTab),
            _ => None,
        };
    }

    if mode == CursorMode::Stamp {
        return match key {
            _ if is_space_key(key) => Some(EditCommand::PlaceStamp),
            _ => None,
        };
    }

    if mode == CursorMode::Shapes {
        return match key {
            _ if is_space_key(key) => Some(EditCommand::StartOrConfirmShape),
            _ => None,
        };
    }

    if mode == CursorMode::Utilities {
        return None;
    }

    match key {
        Key::Named(NamedKey::Home) => Some(EditCommand::Home),
        Key::Named(NamedKey::End) => Some(EditCommand::End),
        Key::Named(NamedKey::Backspace) => Some(EditCommand::Backspace),
        Key::Named(NamedKey::Delete) => Some(EditCommand::Delete),
        Key::Named(NamedKey::Enter) => Some(EditCommand::Newline),
        Key::Named(NamedKey::Tab) => Some(EditCommand::InsertTab),
        _ => None,
    }
}

pub fn cursor_direction_for_key(key: &Key, mode: CursorMode) -> Option<Direction> {
    if mode.accepts_text() {
        arrow_direction_for_key(key)
    } else {
        direction_for_key(key)
    }
}

pub fn direction_key_for_event<'a>(key: &'a Key, key_without_modifiers: &'a Key) -> &'a Key {
    if direction_for_key(key_without_modifiers).is_some() {
        key_without_modifiers
    } else {
        key
    }
}

pub fn edit_direction_command(
    direction: Direction,
    modifiers: ModifiersState,
    mode: CursorMode,
    space_held: bool,
) -> Option<EditCommand> {
    if mode == CursorMode::Navigation && modifiers.control_key() {
        return None;
    }
    if modifiers.shift_key()
        && !modifiers.alt_key()
        && !modifiers.control_key()
        && !modifiers.super_key()
    {
        return Some(EditCommand::ExtendSelection(direction));
    }
    if !mode.accepts_text()
        && modifiers.alt_key()
        && !modifiers.control_key()
        && !modifiers.shift_key()
        && !modifiers.super_key()
    {
        return Some(EditCommand::Erase(direction));
    }
    if modifiers.super_key()
        || modifiers.alt_key()
        || (mode.accepts_text() && modifiers.control_key())
        || (!mode.accepts_text() && modifiers.shift_key())
    {
        return None;
    }
    if space_held && mode == CursorMode::Stamp {
        return Some(EditCommand::DrawStamp(direction));
    }
    Some(if modifiers.control_key() {
        tool_direction_command(direction, mode)
    } else {
        EditCommand::Move(direction)
    })
}

fn is_space_key(key: &Key) -> bool {
    match key {
        Key::Named(NamedKey::Space) => true,
        Key::Character(text) => text == " ",
        _ => false,
    }
}

pub(crate) fn direction_for_key(key: &Key) -> Option<Direction> {
    arrow_direction_for_key(key).or_else(|| match key {
        Key::Character(text) if text.eq_ignore_ascii_case("ķ") => Some(Direction::Left),
        Key::Character(text) if text.eq_ignore_ascii_case("∆") => Some(Direction::Down),
        Key::Character(text) if text.eq_ignore_ascii_case("Ż") => Some(Direction::Up),
        Key::Character(text) if text.eq_ignore_ascii_case("ł") => Some(Direction::Right),

        Key::Character(text) if text.eq_ignore_ascii_case("h") => Some(Direction::Left),
        Key::Character(text) if text.eq_ignore_ascii_case("j") => Some(Direction::Down),
        Key::Character(text) if text.eq_ignore_ascii_case("k") => Some(Direction::Up),
        Key::Character(text) if text.eq_ignore_ascii_case("l") => Some(Direction::Right),
        _ => None,
    })
}

fn arrow_direction_for_key(key: &Key) -> Option<Direction> {
    match key {
        Key::Named(NamedKey::ArrowLeft) => Some(Direction::Left),
        Key::Named(NamedKey::ArrowRight) => Some(Direction::Right),
        Key::Named(NamedKey::ArrowUp) => Some(Direction::Up),
        Key::Named(NamedKey::ArrowDown) => Some(Direction::Down),
        _ => None,
    }
}

pub fn pointer_position_to_coord(
    position: (f64, f64),
    viewport_width: usize,
    renderer: &Renderer,
    scale_factor: f64,
    config: &AppConfig,
    toolbar: &ToolbarState,
    viewport: ViewportOffset,
) -> Option<Coord> {
    let metrics = renderer.metrics(scale_factor);
    let toolbar_metrics = renderer.title_metrics(scale_factor);
    let box_width = (viewport_width.saturating_sub(PADDING * 2) as f32
        / toolbar_metrics.cell_width.max(1.0)) as usize;
    let grid_top = content_top_padding(scale_factor, config.transparent_menubar)
        + crate::toolbar::toolbar_height_for_width(toolbar, box_width, toolbar_metrics.cell_height);
    if crate::layout::minimap_rect(
        viewport_width,
        grid_top,
        (toolbar_metrics.cell_width, toolbar_metrics.cell_height),
    )
    .contains(position.0, position.1)
    {
        return None;
    }
    pointer_position_to_coord_with_metrics(
        position.0,
        position.1,
        grid_top,
        metrics.cell_width,
        metrics.cell_height,
        viewport,
    )
}

fn pointer_position_to_coord_with_metrics(
    x: f64,
    y: f64,
    grid_top: f32,
    cell_width: f32,
    cell_height: f32,
    viewport: ViewportOffset,
) -> Option<Coord> {
    if y < grid_top as f64 {
        return None;
    }
    let grid_x = x - PADDING as f64 - viewport.x as f64;
    let grid_y = y - grid_top as f64 - viewport.y as f64;
    if grid_x < 0.0 || grid_y < 0.0 {
        return None;
    }
    let column = (grid_x / cell_width.max(1.0) as f64).floor() as usize;
    let line = (grid_y / cell_height.max(1.0) as f64).floor() as usize;
    Some(Coord { line, column })
}

pub fn pointer_position_to_toolbar_position(
    x: f64,
    y: f64,
    viewport_width: usize,
    renderer: &Renderer,
    scale_factor: f64,
    config: &AppConfig,
    toolbar: &ToolbarState,
) -> Option<(usize, usize, usize)> {
    let metrics = renderer.title_metrics(scale_factor);
    let box_width =
        (viewport_width.saturating_sub(PADDING * 2) as f32 / metrics.cell_width.max(1.0)) as usize;
    toolbar_position(
        x,
        y,
        viewport_width,
        metrics.cell_width,
        metrics.cell_height,
        content_top_padding(scale_factor, config.transparent_menubar),
        toolbar.rows_for_width(box_width),
    )
}

fn toolbar_position(
    x: f64,
    y: f64,
    viewport_width: usize,
    cell_width: f32,
    cell_height: f32,
    top_padding: f32,
    toolbar_rows: usize,
) -> Option<(usize, usize, usize)> {
    let toolbar_x = x - PADDING as f64;
    let toolbar_y = y - top_padding as f64;
    if toolbar_x < 0.0 || toolbar_y < 0.0 {
        return None;
    }
    let stride = cell_height + crate::toolbar::TOOLBAR_ROW_GAP as f32;
    let row = (toolbar_y / stride as f64).floor() as usize;
    let within_row = toolbar_y - row as f64 * stride as f64;
    if row == 0 || row + 1 >= toolbar_rows || within_row >= cell_height as f64 {
        return None;
    }
    let column = (toolbar_x / cell_width.max(1.0) as f64).floor() as usize;
    let box_width =
        (viewport_width.saturating_sub(PADDING * 2) as f32 / cell_width.max(1.0)) as usize;
    if column < 2 || column >= box_width.saturating_sub(2) {
        return None;
    }
    Some((row - 1, column, box_width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxed_toolbar_hit_testing_skips_borders_and_translates_content_offsets() {
        let top = 30;
        let cell_width = 10;
        let cell_height = 16;
        let stride = cell_height + crate::toolbar::TOOLBAR_ROW_GAP;
        let viewport_width = PADDING * 2 + 20 * cell_width;
        let toolbar_rows = ToolbarState::default().rows();

        assert_eq!(
            toolbar_position(
                (PADDING + 8 * cell_width + 1) as f64,
                top as f64,
                viewport_width,
                cell_width as f32,
                cell_height as f32,
                top as f32,
                toolbar_rows,
            ),
            None,
            "top border is inert"
        );
        assert_eq!(
            toolbar_position(
                (PADDING + 8 * cell_width + 1) as f64,
                (top + cell_height) as f64,
                viewport_width,
                cell_width as f32,
                cell_height as f32,
                top as f32,
                toolbar_rows,
            ),
            None,
            "inter-row gap is inert"
        );
        assert_eq!(
            toolbar_position(
                (PADDING + 8 * cell_width + 1) as f64,
                (top + stride + 1) as f64,
                viewport_width,
                cell_width as f32,
                cell_height as f32,
                top as f32,
                toolbar_rows,
            ),
            Some((0, 8, 20))
        );
        assert_eq!(
            crate::toolbar::ToolbarState::default().action_at(0, 8, 20),
            Some(crate::toolbar::ToolbarAction::SelectMain(
                crate::toolbar::MainMode::Stamp
            ))
        );
        for border_column in [0, 1, 18, 19] {
            assert_eq!(
                toolbar_position(
                    (PADDING + border_column * cell_width + 1) as f64,
                    (top + stride + 1) as f64,
                    viewport_width,
                    cell_width as f32,
                    cell_height as f32,
                    top as f32,
                    toolbar_rows,
                ),
                None,
                "box border or padding column {border_column} is inert"
            );
        }
        assert_eq!(
            toolbar_position(
                (PADDING + 2 * cell_width) as f64,
                (top + (toolbar_rows - 1) * stride) as f64,
                viewport_width,
                cell_width as f32,
                cell_height as f32,
                top as f32,
                toolbar_rows,
            ),
            None,
            "bottom border is inert"
        );
    }

    #[test]
    fn too_narrow_toolbar_has_no_clickable_interior() {
        assert_eq!(
            toolbar_position(
                (PADDING + 2) as f64,
                17.0,
                PADDING * 2 + 3,
                1.0,
                16.0,
                0.0,
                ToolbarState::default().rows(),
            ),
            None
        );
    }

    #[test]
    fn pointer_mapping_uses_the_active_dynamic_toolbar_height() {
        let top = 20;
        let cell_width = 8;
        let cell_height = 16;
        let stride = cell_height + crate::toolbar::TOOLBAR_ROW_GAP;
        let viewport_width = PADDING * 2 + 40 * cell_width;
        let mut toolbar = ToolbarState::default();
        toolbar.apply_action(crate::toolbar::ToolbarAction::SelectMain(
            crate::toolbar::MainMode::Shapes,
        ));
        let line_rows = toolbar.rows();
        assert_eq!(
            toolbar_position(
                (PADDING + 4 * cell_width) as f64,
                (top + (line_rows - 1) * stride) as f64,
                viewport_width,
                cell_width as f32,
                cell_height as f32,
                top as f32,
                line_rows,
            ),
            None
        );

        toolbar.apply_action(crate::toolbar::ToolbarAction::SelectMain(
            crate::toolbar::MainMode::Stamp,
        ));
        assert!(toolbar.rows() > line_rows);
        let last_content_row = toolbar.content_rows() - 1;
        assert_eq!(
            toolbar_position(
                (PADDING + 4 * cell_width) as f64,
                (top + crate::toolbar::toolbar_content_row(last_content_row) * stride + 1) as f64,
                viewport_width,
                cell_width as f32,
                cell_height as f32,
                top as f32,
                toolbar.rows(),
            ),
            Some((last_content_row, 4, 40))
        );
    }

    #[test]
    fn pointer_mapping_keeps_an_anchored_cell_across_grid_top_changes() {
        let cell_width = 8;
        let cell_height = 16;
        let coord = Coord {
            line: 20,
            column: 9,
        };
        let old_grid_top = 44;
        let new_grid_top = 172;
        let mut viewport = ViewportOffset { x: -5, y: 11 };
        let screen_x = PADDING as i64
            + coord.column as i64 * cell_width as i64
            + viewport.x
            + cell_width as i64 / 2;
        let screen_y = old_grid_top as i64
            + coord.line as i64 * cell_height as i64
            + viewport.y
            + cell_height as i64 / 2;

        assert_eq!(
            pointer_position_to_coord_with_metrics(
                screen_x as f64,
                screen_y as f64,
                old_grid_top as f32,
                cell_width as f32,
                cell_height as f32,
                viewport,
            ),
            Some(coord)
        );

        viewport.reanchor_grid_top(old_grid_top as f32, new_grid_top as f32);
        assert_eq!(
            pointer_position_to_coord_with_metrics(
                screen_x as f64,
                screen_y as f64,
                new_grid_top as f32,
                cell_width as f32,
                cell_height as f32,
                viewport,
            ),
            Some(coord)
        );
    }

    #[test]
    fn pointer_mapping_does_not_expose_canvas_behind_toolbar() {
        let grid_top = 100.0;
        let viewport = ViewportOffset { x: -80, y: -160 };

        assert_eq!(
            pointer_position_to_coord_with_metrics(
                (PADDING + 10) as f64,
                grid_top as f64 - 1.0,
                grid_top,
                8.0,
                16.0,
                viewport,
            ),
            None,
            "toolbar remains inert when the canvas is panned upward"
        );
        assert!(
            pointer_position_to_coord_with_metrics(
                PADDING as f64,
                grid_top as f64,
                grid_top,
                8.0,
                16.0,
                viewport,
            )
            .is_some(),
            "the visible canvas origin remains interactive"
        );
    }

    #[test]
    fn maps_editor_navigation_keys() {
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowLeft),
                ModifiersState::empty(),
                CursorMode::Insert,
            ),
            Some(EditCommand::Move(Direction::Left))
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Backspace),
                ModifiersState::empty(),
                CursorMode::Insert,
            ),
            Some(EditCommand::Backspace)
        );
    }

    #[test]
    fn move_draw_ignores_non_directional_editing_keys() {
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Delete),
                ModifiersState::empty(),
                CursorMode::MoveDraw,
            ),
            None
        );
    }

    #[test]
    fn maps_backspace_to_clear_in_every_mode() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            assert_eq!(
                edit_command_for_key(
                    &Key::Named(NamedKey::Backspace),
                    ModifiersState::empty(),
                    mode,
                ),
                Some(EditCommand::Clear)
            );
        }
        for mode in [CursorMode::Text, CursorMode::Insert] {
            assert_eq!(
                edit_command_for_key(
                    &Key::Named(NamedKey::Backspace),
                    ModifiersState::empty(),
                    mode,
                ),
                Some(EditCommand::Backspace)
            );
        }
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Backspace),
                ModifiersState::empty(),
                CursorMode::Replace,
            ),
            Some(EditCommand::ClearAndBack)
        );

        assert_eq!(
            edit_command_for_key(
                &Key::Character(" ".into()),
                ModifiersState::empty(),
                CursorMode::MoveDraw,
            ),
            None
        );
    }

    #[test]
    fn line_preview_routes_space_movement_backspace_and_cancel() {
        let mode = CursorMode::MoveDraw;
        assert_eq!(
            line_preview_command(
                &Key::Named(NamedKey::Space),
                ModifiersState::empty(),
                mode,
                false,
            ),
            Some(LinePreviewCommand::StartOrAdvance)
        );
        assert_eq!(
            line_preview_command(
                &Key::Named(NamedKey::ArrowRight),
                ModifiersState::empty(),
                mode,
                true,
            ),
            Some(LinePreviewCommand::Move(Direction::Right))
        );
        assert_eq!(
            line_preview_command(
                &Key::Named(NamedKey::ArrowRight),
                ModifiersState::CONTROL,
                mode,
                true,
            ),
            Some(LinePreviewCommand::Move(Direction::Right))
        );
        assert_eq!(
            line_preview_command(
                &Key::Named(NamedKey::ArrowRight),
                ModifiersState::SHIFT,
                mode,
                true,
            ),
            None
        );
        assert_eq!(
            line_preview_command(
                &Key::Named(NamedKey::Backspace),
                ModifiersState::empty(),
                mode,
                true,
            ),
            Some(LinePreviewCommand::RemoveAnchor)
        );
        for (key, modifiers) in [
            (Key::Named(NamedKey::Escape), ModifiersState::empty()),
            (Key::Character("g".into()), ModifiersState::CONTROL),
        ] {
            assert_eq!(
                line_preview_command(&key, modifiers, mode, true),
                Some(LinePreviewCommand::Cancel)
            );
        }
        assert_eq!(
            line_preview_command(
                &Key::Named(NamedKey::Space),
                ModifiersState::empty(),
                CursorMode::Stamp,
                false,
            ),
            None
        );
    }

    #[test]
    fn maps_hjkl_and_control_movement_to_move_draw_commands() {
        assert_eq!(
            edit_command_for_key(
                &Key::Character("h".into()),
                ModifiersState::empty(),
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::Move(Direction::Left))
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowDown),
                ModifiersState::CONTROL,
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::Draw(Direction::Down))
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Character("l".into()),
                ModifiersState::CONTROL,
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::Draw(Direction::Right))
        );
    }

    #[test]
    fn maps_shift_directions_to_selection_and_alt_directions_to_erasing_in_canvas_modes() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            for (key, direction) in [
                (Key::Character("h".into()), Direction::Left),
                (Key::Character("j".into()), Direction::Down),
                (Key::Character("k".into()), Direction::Up),
                (Key::Character("l".into()), Direction::Right),
                (Key::Named(NamedKey::ArrowLeft), Direction::Left),
                (Key::Named(NamedKey::ArrowDown), Direction::Down),
                (Key::Named(NamedKey::ArrowUp), Direction::Up),
                (Key::Named(NamedKey::ArrowRight), Direction::Right),
            ] {
                assert_eq!(
                    edit_command_for_key(&key, ModifiersState::SHIFT, mode),
                    Some(EditCommand::ExtendSelection(direction))
                );
                assert_eq!(
                    edit_command_for_key(&key, ModifiersState::ALT, mode),
                    Some(EditCommand::Erase(direction))
                );
            }
        }
        for mode in [CursorMode::Insert, CursorMode::Replace, CursorMode::Text] {
            assert_eq!(
                edit_command_for_key(
                    &Key::Named(NamedKey::ArrowLeft),
                    ModifiersState::SHIFT,
                    mode,
                ),
                Some(EditCommand::ExtendSelection(Direction::Left))
            );
            assert_eq!(
                edit_command_for_key(&Key::Character("H".into()), ModifiersState::SHIFT, mode,),
                None
            );
            assert_ne!(
                edit_command_for_key(&Key::Character("h".into()), ModifiersState::ALT, mode,),
                Some(EditCommand::Erase(Direction::Left))
            );
        }
    }

    #[test]
    fn direction_modifiers_are_single_modifier_commands_only() {
        for modifiers in [
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            ModifiersState::CONTROL | ModifiersState::ALT,
            ModifiersState::ALT | ModifiersState::SHIFT,
        ] {
            assert_eq!(
                edit_command_for_key(
                    &Key::Named(NamedKey::ArrowRight),
                    modifiers,
                    CursorMode::MoveDraw,
                ),
                None
            );
        }
    }

    fn tracker(states: &[ModifiersState]) -> OrderedModifierTracker {
        let mut tracker = OrderedModifierTracker::default();
        for state in states {
            tracker.update(*state);
        }
        tracker
    }

    #[test]
    fn ordered_modifier_tracker_preserves_press_release_and_repress_order() {
        let mut ordered = tracker(&[
            ModifiersState::ALT,
            ModifiersState::ALT | ModifiersState::CONTROL,
            ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT,
        ]);
        assert_eq!(
            ordered.held,
            vec![
                DirectionModifier::Alt,
                DirectionModifier::Control,
                DirectionModifier::Shift
            ]
        );

        ordered.update(ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(
            ordered.held,
            vec![DirectionModifier::Control, DirectionModifier::Shift]
        );
        ordered.update(ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        assert_eq!(
            ordered.held,
            vec![
                DirectionModifier::Control,
                DirectionModifier::Shift,
                DirectionModifier::Alt
            ]
        );
        ordered.update(ModifiersState::CONTROL | ModifiersState::ALT);
        assert_eq!(
            ordered.held,
            vec![DirectionModifier::Control, DirectionModifier::Alt]
        );
        ordered.update(ModifiersState::CONTROL | ModifiersState::ALT);
        assert_eq!(
            ordered.held,
            vec![DirectionModifier::Control, DirectionModifier::Alt]
        );
        ordered.update(ModifiersState::empty());
        assert!(ordered.held.is_empty());
    }

    #[test]
    fn clone_move_press_changes_only_after_shift_is_repressed_after_alt() {
        let mut ordered = OrderedModifierTracker::default();
        ordered.update(ModifiersState::ALT);
        assert_eq!(ordered.clone_move_press(ModifiersState::ALT), None);

        let combined = ModifiersState::ALT | ModifiersState::SHIFT;
        ordered.update(combined);
        let first = ordered.clone_move_press(combined).unwrap();
        ordered.update(combined);
        assert_eq!(ordered.clone_move_press(combined), Some(first));

        ordered.update(ModifiersState::ALT);
        ordered.update(combined);
        assert_ne!(ordered.clone_move_press(combined), Some(first));

        let shift_first = tracker(&[ModifiersState::SHIFT, combined]);
        assert_eq!(shift_first.clone_move_press(combined), None);
    }

    #[test]
    fn simultaneous_additions_have_a_stable_fallback_and_three_use_earliest_secondary() {
        let ordered = tracker(&[ModifiersState::SHIFT | ModifiersState::ALT]);
        assert_eq!(
            ordered.held,
            vec![DirectionModifier::Shift, DirectionModifier::Alt]
        );
        assert_eq!(
            ordered_direction_command(
                &Key::Character("l".into()),
                ModifiersState::SHIFT | ModifiersState::ALT,
                &ordered,
                CursorMode::MoveDraw,
            ),
            Some(OrderedDirectionCommand {
                command: EditCommand::ExtendSelection(Direction::Right),
                steps: 10,
            })
        );

        let all = tracker(&[
            ModifiersState::CONTROL,
            ModifiersState::CONTROL | ModifiersState::ALT,
            ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT,
        ]);
        assert_eq!(
            ordered_direction_command(
                &Key::Named(NamedKey::ArrowDown),
                ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT,
                &all,
                CursorMode::MoveDraw,
            ),
            Some(OrderedDirectionCommand {
                command: EditCommand::Draw(Direction::Down),
                steps: 5,
            })
        );
    }

    #[test]
    fn every_ordered_pair_selects_its_primary_operation_and_step_size() {
        let cases = [
            (
                DirectionModifier::Shift,
                DirectionModifier::Control,
                EditCommand::ExtendSelection(Direction::Right),
                5,
            ),
            (
                DirectionModifier::Control,
                DirectionModifier::Shift,
                EditCommand::Draw(Direction::Right),
                10,
            ),
            (
                DirectionModifier::Shift,
                DirectionModifier::Alt,
                EditCommand::ExtendSelection(Direction::Right),
                10,
            ),
            (
                DirectionModifier::Alt,
                DirectionModifier::Shift,
                EditCommand::Erase(Direction::Right),
                10,
            ),
            (
                DirectionModifier::Alt,
                DirectionModifier::Control,
                EditCommand::Erase(Direction::Right),
                5,
            ),
            (
                DirectionModifier::Control,
                DirectionModifier::Alt,
                EditCommand::Draw(Direction::Right),
                5,
            ),
        ];
        for (primary, secondary, command, steps) in cases {
            let ordered = OrderedModifierTracker {
                held: vec![primary, secondary],
                shift_press: 0,
            };
            let modifiers = match (primary, secondary) {
                (DirectionModifier::Shift, DirectionModifier::Control)
                | (DirectionModifier::Control, DirectionModifier::Shift) => {
                    ModifiersState::SHIFT | ModifiersState::CONTROL
                }
                (DirectionModifier::Shift, DirectionModifier::Alt)
                | (DirectionModifier::Alt, DirectionModifier::Shift) => {
                    ModifiersState::SHIFT | ModifiersState::ALT
                }
                _ => ModifiersState::ALT | ModifiersState::CONTROL,
            };
            assert_eq!(
                ordered_direction_command(
                    &Key::Named(NamedKey::ArrowRight),
                    modifiers,
                    &ordered,
                    CursorMode::MoveDraw,
                ),
                Some(OrderedDirectionCommand { command, steps })
            );
        }
    }

    #[test]
    fn ordered_directions_support_hjkl_mode_routing_and_exclude_text_modes() {
        let ordered = tracker(&[
            ModifiersState::CONTROL,
            ModifiersState::CONTROL | ModifiersState::SHIFT,
        ]);
        for (mode, command) in [
            (CursorMode::Stamp, EditCommand::DrawStamp(Direction::Left)),
            (CursorMode::Shapes, EditCommand::Move(Direction::Left)),
            (
                CursorMode::Utilities,
                EditCommand::ApplyUtility(Direction::Left),
            ),
        ] {
            assert_eq!(
                ordered_direction_command(
                    &Key::Character("h".into()),
                    ModifiersState::SHIFT | ModifiersState::CONTROL,
                    &ordered,
                    mode,
                ),
                Some(OrderedDirectionCommand { command, steps: 10 })
            );
        }
        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            assert_eq!(
                ordered_direction_command(
                    &Key::Character("h".into()),
                    ModifiersState::SHIFT | ModifiersState::CONTROL,
                    &ordered,
                    mode,
                ),
                None
            );
        }

        let single = tracker(&[ModifiersState::ALT]);
        assert_eq!(
            ordered_direction_command(
                &Key::Named(NamedKey::ArrowUp),
                ModifiersState::ALT,
                &single,
                CursorMode::MoveDraw,
            ),
            Some(OrderedDirectionCommand {
                command: EditCommand::Erase(Direction::Up),
                steps: 1,
            })
        );
        assert_eq!(
            ordered_direction_command(
                &Key::Named(NamedKey::ArrowUp),
                ModifiersState::ALT | ModifiersState::SUPER,
                &single,
                CursorMode::MoveDraw,
            ),
            None
        );
    }

    #[test]
    fn canvas_escape_cancels_transients_and_shape_space_confirms() {
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::Space),
                ModifiersState::empty(),
                CursorMode::Shapes,
            ),
            Some(EditCommand::StartOrConfirmShape)
        );
    }

    #[test]
    fn leaves_modified_keys_for_app_shortcuts() {
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowLeft),
                ModifiersState::SUPER,
                CursorMode::MoveDraw,
            ),
            None
        );
    }

    #[test]
    fn return_toggles_text_mode_from_every_canvas_mode() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Text,
            CursorMode::Stamp,
            CursorMode::Utilities,
            CursorMode::Navigation,
        ] {
            assert_eq!(
                edit_command_for_key(&Key::Named(NamedKey::Enter), ModifiersState::empty(), mode,),
                Some(EditCommand::ToggleReplaceMode)
            );
        }
    }

    #[test]
    fn utilities_route_control_directions_to_tools_without_changing_other_modes() {
        for key in [
            Key::Character("h".into()),
            Key::Character("j".into()),
            Key::Character("k".into()),
            Key::Character("l".into()),
            Key::Named(NamedKey::ArrowLeft),
            Key::Named(NamedKey::ArrowDown),
            Key::Named(NamedKey::ArrowUp),
            Key::Named(NamedKey::ArrowRight),
        ] {
            let direction = direction_for_key(&key).unwrap();
            assert_eq!(
                edit_command_for_key(&key, ModifiersState::CONTROL, CursorMode::Utilities),
                Some(EditCommand::ApplyUtility(direction))
            );
            assert_eq!(
                edit_command_for_key(&key, ModifiersState::empty(), CursorMode::Utilities),
                Some(EditCommand::Move(direction))
            );
        }
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowRight),
                ModifiersState::CONTROL,
                CursorMode::MoveDraw,
            ),
            Some(EditCommand::Draw(Direction::Right))
        );
    }

    #[test]
    fn utilities_view_resolves_plain_pan_and_center_only() {
        for (key, direction) in [
            (Key::Character("h".into()), Direction::Left),
            (Key::Character("j".into()), Direction::Down),
            (Key::Character("k".into()), Direction::Up),
            (Key::Character("l".into()), Direction::Right),
            (Key::Named(NamedKey::ArrowLeft), Direction::Left),
            (Key::Named(NamedKey::ArrowDown), Direction::Down),
            (Key::Named(NamedKey::ArrowUp), Direction::Up),
            (Key::Named(NamedKey::ArrowRight), Direction::Right),
        ] {
            assert_eq!(
                view_command(
                    &key,
                    ModifiersState::empty(),
                    CursorMode::Utilities,
                    UtilityKind::View,
                ),
                Some(ViewCommand::Pan(direction))
            );
        }
        assert_eq!(
            view_command(
                &Key::Named(NamedKey::Space),
                ModifiersState::empty(),
                CursorMode::Utilities,
                UtilityKind::View,
            ),
            Some(ViewCommand::Center)
        );
        for modifiers in [
            ModifiersState::SHIFT,
            ModifiersState::ALT,
            ModifiersState::CONTROL,
            ModifiersState::SUPER,
        ] {
            assert_eq!(
                view_command(
                    &Key::Named(NamedKey::ArrowRight),
                    modifiers,
                    CursorMode::Utilities,
                    UtilityKind::View,
                ),
                None
            );
        }
        assert_eq!(
            view_command(
                &Key::Named(NamedKey::Space),
                ModifiersState::empty(),
                CursorMode::Utilities,
                UtilityKind::Pull,
            ),
            None
        );
    }

    #[test]
    fn active_selection_move_resolves_navigation_confirmation_and_cancel_only() {
        assert_eq!(
            move_selection_command(
                &Key::Named(NamedKey::ArrowRight),
                ModifiersState::empty(),
                true,
                false,
                None,
            ),
            Some(MoveSelectionCommand::ConfirmAndMove(Direction::Right))
        );
        for key in [Key::Named(NamedKey::Space), Key::Named(NamedKey::Enter)] {
            assert_eq!(
                move_selection_command(&key, ModifiersState::empty(), true, false, None),
                Some(MoveSelectionCommand::Confirm)
            );
        }
        for (key, modifiers) in [
            (Key::Named(NamedKey::Escape), ModifiersState::empty()),
            (Key::Character("g".into()), ModifiersState::CONTROL),
        ] {
            assert_eq!(
                move_selection_command(&key, modifiers, true, false, None),
                Some(MoveSelectionCommand::Cancel)
            );
        }
        assert_eq!(
            move_selection_command(
                &Key::Character("c".into()),
                ModifiersState::CONTROL,
                true,
                false,
                None,
            ),
            None,
            "Ctrl-C is handled by the cancel-key classifier"
        );
        assert_eq!(
            move_selection_command(
                &Key::Named(NamedKey::Enter),
                ModifiersState::empty(),
                false,
                false,
                None,
            ),
            None,
            "Enter keeps its existing meaning until a lift is active"
        );
        assert_eq!(
            move_selection_command(
                &Key::Named(NamedKey::Space),
                ModifiersState::empty(),
                false,
                false,
                None,
            ),
            None
        );
    }

    #[test]
    fn alt_direction_starts_and_continues_selection_move_in_every_mode() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
            CursorMode::Text,
            CursorMode::Replace,
        ] {
            assert_eq!(
                move_selection_command(
                    &Key::Named(NamedKey::ArrowDown),
                    ModifiersState::ALT,
                    false,
                    true,
                    None,
                ),
                Some(MoveSelectionCommand::BeginAndStep(Direction::Down)),
                "mode={mode:?}"
            );
            assert_eq!(
                move_selection_command(
                    &Key::Named(NamedKey::ArrowDown),
                    ModifiersState::ALT,
                    true,
                    true,
                    None,
                ),
                Some(MoveSelectionCommand::Step(Direction::Down)),
                "mode={mode:?}"
            );
        }

        assert_eq!(
            move_selection_command(
                &Key::Named(NamedKey::ArrowDown),
                ModifiersState::ALT,
                false,
                false,
                None,
            ),
            None
        );

        assert_eq!(
            move_selection_command(
                &Key::Named(NamedKey::ArrowRight),
                ModifiersState::empty(),
                true,
                true,
                None,
            ),
            Some(MoveSelectionCommand::ConfirmAndMove(Direction::Right))
        );
    }

    #[test]
    fn alt_first_shift_press_starts_or_clones_an_active_selection_move() {
        let modifiers = ModifiersState::ALT | ModifiersState::SHIFT;
        assert_eq!(
            move_selection_direction_command(Direction::Right, modifiers, false, true, Some(7),),
            Some(MoveSelectionCommand::BeginCloneAndStep(Direction::Right, 7))
        );
        assert_eq!(
            move_selection_direction_command(Direction::Right, modifiers, true, true, Some(7),),
            Some(MoveSelectionCommand::CloneAndStep(Direction::Right, 7))
        );
        assert_eq!(
            move_selection_direction_command(Direction::Right, modifiers, true, true, None),
            None,
            "Shift-first Alt remains available to ordered long movement"
        );
    }

    #[test]
    fn modifier_transformed_hjkl_still_resolve_clone_move_directions() {
        let modifiers = ModifiersState::ALT | ModifiersState::SHIFT;
        for (key, direction) in [
            ("h", Direction::Left),
            ("j", Direction::Down),
            ("k", Direction::Up),
            ("l", Direction::Right),
        ] {
            let logical_key = Key::Character(format!("modified-{key}").into());
            let unmodified_key = Key::Character(key.into());
            assert_eq!(
                move_selection_command(
                    direction_key_for_event(&logical_key, &unmodified_key),
                    modifiers,
                    false,
                    true,
                    Some(7),
                ),
                Some(MoveSelectionCommand::BeginCloneAndStep(direction, 7))
            );
        }
    }

    #[test]
    fn entering_replace_mode() {
        for mode in [CursorMode::MoveDraw, CursorMode::Navigation] {
            assert_eq!(
                edit_command_for_key(&Key::Named(NamedKey::Enter), ModifiersState::SHIFT, mode,),
                Some(EditCommand::ToggleTextEntry)
            );
            assert_eq!(
                edit_command_for_key(&Key::Character("r".into()), ModifiersState::SHIFT, mode,),
                Some(EditCommand::ToggleReplaceMode)
            );
        }
    }

    #[test]
    fn lowercase_r_starts_single_replace_only_outside_text_and_replace_modes() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
            CursorMode::Navigation,
        ] {
            assert_eq!(
                edit_command_for_key(&Key::Character("r".into()), ModifiersState::empty(), mode,),
                Some(EditCommand::BeginSingleReplace)
            );
        }

        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            assert_eq!(
                edit_command_for_key(&Key::Character("r".into()), ModifiersState::empty(), mode,),
                None
            );
        }
    }

    #[test]
    fn navigation_keeps_mode_entry_keys_but_ignores_tool_keys() {
        for (key, modifiers, expected) in [
            (
                Key::Character("i".into()),
                ModifiersState::empty(),
                Some(EditCommand::ToggleTextEntry),
            ),
            (
                Key::Named(NamedKey::Enter),
                ModifiersState::SHIFT,
                Some(EditCommand::ToggleTextEntry),
            ),
            (
                Key::Named(NamedKey::Enter),
                ModifiersState::empty(),
                Some(EditCommand::ToggleReplaceMode),
            ),
            (
                Key::Character("R".into()),
                ModifiersState::SHIFT,
                Some(EditCommand::ToggleReplaceMode),
            ),
            (
                Key::Character("r".into()),
                ModifiersState::empty(),
                Some(EditCommand::BeginSingleReplace),
            ),
            (Key::Named(NamedKey::Space), ModifiersState::empty(), None),
            (
                Key::Named(NamedKey::ArrowRight),
                ModifiersState::CONTROL,
                None,
            ),
        ] {
            assert_eq!(
                edit_command_for_key(&key, modifiers, CursorMode::Navigation),
                expected,
                "key={key:?}, modifiers={modifiers:?}"
            );
        }
    }

    #[test]
    fn cancel_keys_exit_every_text_accepting_mode() {
        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            for (key, modifiers) in [
                (Key::Named(NamedKey::Escape), ModifiersState::empty()),
                (Key::Character("g".into()), ModifiersState::CONTROL),
            ] {
                assert_eq!(
                    edit_command_for_key(&key, modifiers, mode),
                    Some(EditCommand::CancelTextEntry),
                    "mode={mode:?}, key={key:?}"
                );
            }
        }
    }

    #[test]
    fn command_copy_and_control_or_command_cut_paste_are_global() {
        for modifiers in [
            ModifiersState::SUPER,
            ModifiersState::SUPER | ModifiersState::SHIFT,
        ] {
            assert_eq!(
                clipboard_command(&Key::Character("c".into()), modifiers),
                Some(ClipboardCommand::Copy)
            );
            assert_eq!(
                clipboard_command(&Key::Character("C".into()), modifiers),
                Some(ClipboardCommand::Copy)
            );
        }
        for modifiers in [
            ModifiersState::CONTROL,
            ModifiersState::SUPER,
            ModifiersState::CONTROL | ModifiersState::SHIFT,
            ModifiersState::SUPER | ModifiersState::SHIFT,
        ] {
            assert_eq!(
                clipboard_command(&Key::Character("x".into()), modifiers),
                Some(ClipboardCommand::Cut)
            );
            assert_eq!(
                clipboard_command(&Key::Character("X".into()), modifiers),
                Some(ClipboardCommand::Cut)
            );
            assert_eq!(
                clipboard_command(&Key::Character("v".into()), modifiers),
                Some(ClipboardCommand::Paste)
            );
            assert_eq!(
                clipboard_command(&Key::Character("V".into()), modifiers),
                Some(ClipboardCommand::Paste)
            );
        }
        assert_eq!(
            clipboard_command(&Key::Character("c".into()), ModifiersState::CONTROL),
            None
        );
        assert_eq!(
            clipboard_command(
                &Key::Character("c".into()),
                ModifiersState::CONTROL | ModifiersState::ALT
            ),
            None
        );
        assert_eq!(
            clipboard_command(
                &Key::Character("x".into()),
                ModifiersState::SUPER | ModifiersState::ALT
            ),
            None
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Character("c".into()),
                ModifiersState::CONTROL,
                CursorMode::Text
            ),
            None,
            "Ctrl-C is handled by the cancel-key classifier before edit commands"
        );
    }

    #[test]
    fn control_and_command_history_shortcuts_are_global_and_alt_is_excluded() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
            CursorMode::Text,
            CursorMode::Insert,
            CursorMode::Replace,
        ] {
            for modifiers in [
                ModifiersState::CONTROL,
                ModifiersState::SUPER,
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                ModifiersState::SUPER | ModifiersState::SHIFT,
            ] {
                assert_eq!(
                    history_command(&Key::Character("z".into()), modifiers, mode),
                    Some(HistoryCommand::Undo)
                );
                assert_eq!(
                    history_command(&Key::Character("R".into()), modifiers, mode),
                    Some(HistoryCommand::Redo)
                );
            }
        }
        assert_eq!(
            history_command(
                &Key::Character("r".into()),
                ModifiersState::CONTROL | ModifiersState::ALT,
                CursorMode::Stamp,
            ),
            None
        );
        assert_eq!(
            history_command(
                &Key::Character("r".into()),
                ModifiersState::empty(),
                CursorMode::Stamp,
            ),
            None
        );
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
            CursorMode::Text,
            CursorMode::Insert,
            CursorMode::Replace,
        ] {
            assert_eq!(
                edit_command_for_key(&Key::Character("r".into()), ModifiersState::CONTROL, mode),
                None,
                "Ctrl-R must never start or type Replace in {mode:?}"
            );
        }
    }

    #[test]
    fn plain_vim_history_shortcuts_use_logical_case_only_in_canvas_modes() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            assert_eq!(
                history_command(&Key::Character("u".into()), ModifiersState::empty(), mode,),
                Some(HistoryCommand::Undo),
                "mode {mode:?}"
            );
            for modifiers in [ModifiersState::empty(), ModifiersState::SHIFT] {
                assert_eq!(
                    history_command(&Key::Character("U".into()), modifiers, mode),
                    Some(HistoryCommand::Redo),
                    "mode {mode:?}, modifiers {modifiers:?}"
                );
            }
        }

        for mode in [CursorMode::Text, CursorMode::Insert, CursorMode::Replace] {
            for key in ["u", "U"] {
                assert_eq!(
                    history_command(&Key::Character(key.into()), ModifiersState::empty(), mode,),
                    None,
                    "mode {mode:?}, key {key}"
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
                history_command(&Key::Character("U".into()), modifiers, CursorMode::Stamp),
                None,
                "modifiers {modifiers:?}"
            );
        }
    }

    #[test]
    fn control_cancel_keys_do_nothing_in_ordinary_drawing_modes() {
        for mode in [
            CursorMode::MoveDraw,
            CursorMode::Stamp,
            CursorMode::Shapes,
            CursorMode::Utilities,
        ] {
            for key in [Key::Character("c".into()), Key::Character("g".into())] {
                assert_eq!(
                    edit_command_for_key(&key, ModifiersState::CONTROL, mode),
                    None,
                    "mode={mode:?}, key={key:?}"
                );
            }
        }
    }

    #[test]
    fn text_mode_types_hjkl_and_moves_only_with_arrows() {
        assert_eq!(
            edit_command_for_key(
                &Key::Character("h".into()),
                ModifiersState::empty(),
                CursorMode::Text,
            ),
            None
        );
        assert_eq!(
            edit_command_for_key(
                &Key::Named(NamedKey::ArrowLeft),
                ModifiersState::empty(),
                CursorMode::Text,
            ),
            Some(EditCommand::Move(Direction::Left))
        );
    }

    #[test]
    fn space_places_the_active_stamp() {
        for key in [Key::Character(" ".into()), Key::Named(NamedKey::Space)] {
            assert_eq!(
                edit_command_for_key(&key, ModifiersState::empty(), CursorMode::Stamp),
                Some(EditCommand::PlaceStamp)
            );
        }
    }

    #[test]
    fn control_direction_draws_stamps() {
        for (key, direction) in [
            (Key::Character("l".into()), Direction::Right),
            (Key::Named(NamedKey::ArrowDown), Direction::Down),
        ] {
            assert_eq!(
                edit_command_for_key(&key, ModifiersState::CONTROL, CursorMode::Stamp),
                Some(EditCommand::DrawStamp(direction))
            );
        }
    }
}
