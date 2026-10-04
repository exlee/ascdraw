//! Editor side of objects: commands from the Objects menu, copy and paste,
//! moving and resizing copies, and the overlay the renderer draws.

use super::*;
use crate::objects::{
    AnchorError, AnchorKind, AnchorTarget, Cells, InstanceId, ObjectCell, ObjectStore, Session,
    bounds_contain, render_definition, resolve_anchors,
};
use crate::toolbar::{ObjectCommand, ObjectMenuState};

/// What the renderer draws over the canvas for objects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectOverlay {
    pub outlines: Vec<SelectionBounds>,
    pub focus: Option<SelectionBounds>,
    pub ghost: Vec<(Coord, ObjectCell)>,
    pub anchors: Vec<AnchorMark>,
    /// Edt: cells still showing the definition, drawn dimmed.
    pub definition_cells: Vec<Coord>,
    /// Edt: local blanks, drawn as dimmed rectangles.
    pub blank_cells: Vec<Coord>,
    /// The copy the cursor sits on outside DfnEdt and Edt.
    pub cursor_object: Option<SelectionBounds>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleSide {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl HandleSide {
    /// The horizontal edge it moves: `Some(true)` left, `Some(false)` right.
    pub fn moves_left(self) -> Option<bool> {
        match self {
            Self::Left | Self::TopLeft | Self::BottomLeft => Some(true),
            Self::Right | Self::TopRight | Self::BottomRight => Some(false),
            Self::Top | Self::Bottom => None,
        }
    }

    /// The vertical edge it moves: `Some(true)` top, `Some(false)` bottom.
    pub fn moves_top(self) -> Option<bool> {
        match self {
            Self::Top | Self::TopLeft | Self::TopRight => Some(true),
            Self::Bottom | Self::BottomLeft | Self::BottomRight => Some(false),
            Self::Left | Self::Right => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnchorMark {
    pub at: Coord,
    pub kind: AnchorKind,
    /// What the anchor is attached to, one entry per line drawn to it.
    pub segments: Vec<AnchorSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorSegment {
    /// From another anchor's cell.
    Anchor(Coord),
    /// From a side of the bounding box: `horizontal` for left/right, `min`
    /// for left/top.
    Edge { horizontal: bool, min: bool },
}

impl Editor {
    pub fn objects(&self) -> &ObjectStore {
        &self.objects
    }

    pub fn restore_objects(&mut self, mut objects: ObjectStore) {
        objects.session = None;
        self.objects = objects;
        self.object_writes.clear();
        self.adopt_saved_paint();
    }

    pub fn object_session(&self) -> Option<Session> {
        self.objects.session
    }

    /// The copy at `coord` that behaves as a single cell: every copy except the
    /// one an open DfnEdt or Edt session edits as normal canvas.
    fn solid_instance_at(&self, coord: Coord) -> Option<InstanceId> {
        let editing = match self.objects.session {
            Some(Session::Define(id) | Session::Edit(id)) => Some(id),
            None => None,
        };
        self.objects
            .instance_at(self.canvas.active_id(), coord)
            .filter(|id| Some(*id) != editing)
    }

    fn cursor_instance(&self) -> Option<InstanceId> {
        self.objects
            .instance_at(self.canvas.active_id(), self.grid.cursor_pos)
    }

    pub fn object_menu_state(&self) -> ObjectMenuState {
        let session = self.objects.session;
        let on_instance = self.cursor_instance().is_some();
        ObjectMenuState {
            define_enabled: !self.selection.is_collapsed(),
            define_edit_enabled: on_instance || matches!(session, Some(Session::Define(_))),
            edit_enabled: on_instance || matches!(session, Some(Session::Edit(_))),
            reset_enabled: on_instance && !matches!(session, Some(Session::Define(_))),
            anchor_enabled: matches!(session, Some(Session::Define(_))),
            define_edit_active: matches!(session, Some(Session::Define(_))),
            edit_active: matches!(session, Some(Session::Edit(_))),
        }
    }

    pub fn sync_object_menu(&mut self) {
        let state = self.object_menu_state();
        self.toolbar.set_object_menu(state);
    }

    fn object_tip(&mut self, text: &str) {
        self.transient_tip = Some((
            text.to_owned(),
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        ));
    }

    /// Runs the Objects menu command the toolbar queued. Returns whether the
    /// document changed.
    pub(super) fn apply_pending_object_command(&mut self) -> bool {
        let Some(command) = self.toolbar.take_object_command() else {
            return false;
        };
        let changed = match command {
            ObjectCommand::Define => self.define_object(),
            ObjectCommand::DefineEdit => self.toggle_object_session(true),
            ObjectCommand::Edit => self.toggle_object_session(false),
            ObjectCommand::Reset => self.reset_object(),
            ObjectCommand::Anchor(kind) => self.toggle_object_anchor(kind),
        };
        self.sync_object_menu();
        changed
    }

    fn define_object(&mut self) -> bool {
        if self.selection.is_collapsed() {
            self.object_tip("Dfn: select the rectangle to save as an object");
            return false;
        }
        let bounds = self.selection.bounds();
        let origin = Coord {
            line: bounds.top,
            column: bounds.left,
        };
        self.exit_object_session();
        let layer = self.canvas.active_id();
        let cells = self.read_cells(layer, bounds, origin);
        let instance = self.objects.define(
            layer,
            origin,
            bounds.right - bounds.left + 1,
            bounds.bottom - bounds.top + 1,
            cells,
        );
        self.objects.session = Some(Session::Define(instance));
        self.collapse_selection();
        true
    }

    /// A rectangle drawn outside DfnEdt and Edt becomes an object with an
    /// anchor in each corner, so its copies stretch from the corners.
    pub(super) fn define_shape_object(&mut self, cells: &[(Coord, String)], face: &Face) {
        if self.objects.session.is_some() || cells.is_empty() {
            return;
        }
        let top = cells.iter().map(|(coord, _)| coord.line).min().unwrap_or(0);
        let bottom = cells.iter().map(|(coord, _)| coord.line).max().unwrap_or(0);
        let left = cells
            .iter()
            .map(|(coord, _)| coord.column)
            .min()
            .unwrap_or(0);
        let right = cells
            .iter()
            .map(|(coord, _)| coord.column)
            .max()
            .unwrap_or(0);
        let origin = Coord {
            line: top,
            column: left,
        };
        let mut local = Cells::default();
        for (coord, contents) in cells {
            let cell = ObjectCell {
                atom: contents.clone(),
                face: face.clone(),
                line: None,
            };
            if !cell.is_blank() {
                local.insert(
                    Coord {
                        line: coord.line - top,
                        column: coord.column - left,
                    },
                    cell,
                );
            }
        }
        let (width, height) = (right - left + 1, bottom - top + 1);
        let id = self
            .objects
            .define(self.canvas.active_id(), origin, width, height, local);
        if let Some(object) = self.objects.instance(id).map(|instance| instance.object)
            && let Some(definition) = self.objects.definition_mut(object)
        {
            for (at, kind) in [
                (Coord { line: 0, column: 0 }, AnchorKind::NW),
                (
                    Coord {
                        line: 0,
                        column: width - 1,
                    },
                    AnchorKind::NE,
                ),
                (
                    Coord {
                        line: height - 1,
                        column: 0,
                    },
                    AnchorKind::SW,
                ),
                (
                    Coord {
                        line: height - 1,
                        column: width - 1,
                    },
                    AnchorKind::SE,
                ),
            ] {
                let _ = definition.toggle_anchor(at, kind);
            }
        }
        self.materialize(id);
    }

    fn toggle_object_session(&mut self, define: bool) -> bool {
        let active = self.objects.session;
        if matches!(
            (active, define),
            (Some(Session::Define(_)), true) | (Some(Session::Edit(_)), false)
        ) {
            return self.exit_object_session();
        }
        let Some(instance) = self.cursor_instance().or(match active {
            Some(Session::Define(id) | Session::Edit(id)) => Some(id),
            None => None,
        }) else {
            self.object_tip("Place the cursor on an object first");
            return false;
        };
        self.exit_object_session();
        self.fold_lone_copy(instance);
        if define {
            self.objects.session = Some(Session::Define(instance));
            self.materialize(instance);
        } else {
            self.objects.session = Some(Session::Edit(instance));
        }
        if let Some(layer) = self
            .objects
            .instance(instance)
            .map(|instance| instance.layer)
        {
            self.select_layer(layer);
        }
        true
    }

    /// Leaves DfnEdt or Edt, showing the edited copy as a regular instance
    /// again. Returns whether a session was active.
    pub fn exit_object_session(&mut self) -> bool {
        let Some(session) = self.objects.session.take() else {
            return false;
        };
        if let Session::Define(id) = session {
            self.materialize(id);
        }
        self.sync_object_menu();
        true
    }

    fn toggle_object_anchor(&mut self, kind: AnchorKind) -> bool {
        let Some(Session::Define(id)) = self.objects.session else {
            self.object_tip("Anchr works in DfnEdt");
            return false;
        };
        let Some(instance) = self.objects.instance(id) else {
            return false;
        };
        let (object, origin) = (instance.object, instance.origin);
        let local = Coord {
            line: self.grid.cursor_pos.line - origin.line,
            column: self.grid.cursor_pos.column - origin.column,
        };
        let Some(definition) = self.objects.definition_mut(object) else {
            return false;
        };
        match definition.toggle_anchor(local, kind) {
            Ok(_) => true,
            Err(AnchorError::OutsideDefinition) => {
                self.object_tip("Anchr: move the cursor inside the object");
                false
            }
            Err(AnchorError::NothingToExtend) => {
                self.object_tip("Anchr ·: add an anchor to extend first");
                false
            }
        }
    }

    /// Cmd-C on a copy: remembers the copy and returns its text for the system
    /// clipboard. `None` when the cursor or selection is not on a copy.
    pub fn copy_object(&mut self) -> Option<String> {
        self.object_clipboard = None;
        // Inside DfnEdt and Edt the copy is canvas: Cmd-C copies cells.
        if self.objects.session.is_some() {
            return None;
        }
        let id = self.cursor_instance()?;
        let instance = self.objects.instance(id)?;
        let bounds = self.objects.bounds(instance);
        if !self.selection.is_collapsed() && self.selection.bounds() != bounds {
            return None;
        }
        let cells = self.instance_cells(id)?;
        let origin = instance.origin;
        let text = (bounds.top..=bounds.bottom)
            .map(|line| {
                (bounds.left..=bounds.right)
                    .map(|column| {
                        cells
                            .get(Coord {
                                line: line - origin.line,
                                column: column - origin.column,
                            })
                            .map_or(" ", |cell| cell.atom.as_str())
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.object_clipboard = Some((instance.clone(), text.clone()));
        Some(text)
    }

    /// Cmd-V after Cmd-C on a copy: places a new copy with the same stretch and
    /// local edits at the cursor, while the system clipboard still holds the
    /// copied text.
    pub fn paste_object(&mut self, clipboard: &str) -> bool {
        let Some((template, _)) = self.object_clipboard.as_ref().filter(|(template, text)| {
            text == clipboard && self.objects.definition(template.object).is_some()
        }) else {
            return false;
        };
        let template = template.clone();
        // The pasted copy covers the same area with its corner at the cursor.
        let origin = Coord {
            line: self.grid.cursor_pos.line + template.extent.top,
            column: self.grid.cursor_pos.column + template.extent.left,
        };
        let id = self
            .objects
            .place(template.object, self.canvas.active_id(), origin);
        if let Some(instance) = self.objects.instance_mut(id) {
            instance.stretch = template.stretch;
            instance.overlay = template.overlay;
            instance.extent = template.extent;
        }
        self.collapse_selection();
        self.materialize(id);
        true
    }

    /// The side whose edge cell `coord` is, for the topmost copy on the
    /// active layer. Edges act as resize handles in Objects mode outside
    /// DfnEdt and Edt; corners resize horizontally.
    pub fn object_handle_at(&self, coord: Coord) -> Option<(InstanceId, HandleSide)> {
        if self.objects.session.is_some() {
            return None;
        }
        let id = self.objects.instance_at(self.canvas.active_id(), coord)?;
        let bounds = self.objects.bounds(self.objects.instance(id)?);
        let horizontal = if coord.column == bounds.left {
            Some(true)
        } else if coord.column == bounds.right {
            Some(false)
        } else {
            None
        };
        let vertical = if coord.line == bounds.top {
            Some(true)
        } else if coord.line == bounds.bottom {
            Some(false)
        } else {
            None
        };
        let side = match (horizontal, vertical) {
            (Some(true), Some(true)) => HandleSide::TopLeft,
            (Some(false), Some(true)) => HandleSide::TopRight,
            (Some(true), Some(false)) => HandleSide::BottomLeft,
            (Some(false), Some(false)) => HandleSide::BottomRight,
            (Some(true), None) => HandleSide::Left,
            (Some(false), None) => HandleSide::Right,
            (None, Some(true)) => HandleSide::Top,
            (None, Some(false)) => HandleSide::Bottom,
            (None, None) => return None,
        };
        Some((id, side))
    }

    /// Mouse drag on an edge or corner: moves those sides of the copy to
    /// `target`, never below one cell. Only the copy changes.
    pub fn drag_object_handle(&mut self, id: InstanceId, side: HandleSide, target: Coord) -> bool {
        let Some(instance) = self.objects.instance(id) else {
            return false;
        };
        let bounds = self.objects.bounds(instance);
        let (width, height) = self.objects.size(instance);
        // (origin shift, size change) per axis.
        let (shift_columns, columns) = match side.moves_left() {
            Some(true) => {
                let grow = (bounds.left - target.column).max(1 - width);
                (-grow, grow)
            }
            Some(false) => (0, (target.column - bounds.right).max(1 - width)),
            None => (0, 0),
        };
        let (shift_lines, lines) = match side.moves_top() {
            Some(true) => {
                let grow = (bounds.top - target.line).max(1 - height);
                (-grow, grow)
            }
            Some(false) => (0, (target.line - bounds.bottom).max(1 - height)),
            None => (0, 0),
        };
        if columns == 0 && lines == 0 {
            return false;
        }
        let Some(instance) = self.objects.instance_mut(id) else {
            return false;
        };
        instance.origin.column += shift_columns;
        instance.origin.line += shift_lines;
        instance.stretch.0 += columns;
        instance.stretch.1 += lines;
        self.materialize(id);
        true
    }

    /// Ctrl-click: the copy at `coord` stops being an object and its glyphs
    /// stay as plain text. The definition goes with its last copy.
    pub fn dissolve_object_at(&mut self, coord: Coord) -> bool {
        let Some(id) = self.objects.instance_at(self.canvas.active_id(), coord) else {
            return false;
        };
        if self.objects.session.is_some() {
            return false;
        }
        let Some(instance) = self.objects.remove_instance(id) else {
            return false;
        };
        let object = instance.object;
        if !self
            .objects
            .instances
            .iter()
            .any(|other| other.object == object)
        {
            self.objects
                .definitions
                .retain(|definition| definition.id != object);
            if self.objects.current == Some(object) {
                self.objects.current = None;
            }
        }
        true
    }

    /// The copy at `coord` on the active layer, if any.
    pub fn object_at(&self, coord: Coord) -> Option<InstanceId> {
        self.objects.instance_at(self.canvas.active_id(), coord)
    }

    /// Double-click on a copy outside DfnEdt and Edt: opens Lcl on it with
    /// the cursor on the clicked cell.
    pub fn open_local_edit_at(&mut self, coord: Coord) -> bool {
        if self.objects.session.is_some() || self.object_at(coord).is_none() {
            return false;
        }
        self.grid.cursor_pos = coord;
        self.selection.collapse(coord);
        let opened = self.toggle_object_session(false);
        self.sync_object_menu();
        opened
    }

    /// Whether `coord` shows text lying over a copy rather than the copy.
    fn text_over_copy_at(&self, coord: Coord) -> bool {
        let shown = self
            .canvas
            .active_cell(coord)
            .map(|data| data.atom.contents().to_owned());
        let painted = self
            .painted_top(self.canvas.active_id())
            .get(&(coord.line, coord.column))
            .filter(|cell| !cell.is_blank())
            .map(|cell| cell.atom.clone());
        shown.is_some() && shown != painted
    }

    /// One Alt step. A gesture decides once, from the cell it starts on:
    /// text there (or no copy) makes it erase, a copy's own cell or its
    /// empty space makes it move the copy. Later steps keep that choice.
    pub fn alt_step(&mut self, direction: Direction) -> bool {
        let cursor = self.grid.cursor_pos;
        let erases = match self.alt_gesture {
            Some((erases, at)) if at == cursor => erases,
            _ => {
                self.text_over_copy_at(cursor)
                    || self.objects.session.is_some()
                    || self.solid_instance_at(cursor).is_none()
            }
        };
        let changed = if erases {
            self.erase(direction)
        } else {
            self.move_object(direction)
        };
        self.alt_gesture = Some((erases, self.grid.cursor_pos));
        changed
    }

    /// Ends an Alt gesture, so the next one decides afresh.
    pub fn end_alt_gesture(&mut self) {
        self.alt_gesture = None;
    }

    pub fn cursor_on_object(&self) -> bool {
        self.cursor_instance().is_some()
    }

    /// Ctrl-direction in Objects stretches the copy under the cursor; the
    /// definition never changes. Toward the far side of the center it expands
    /// that edge; toward the center it contracts the opposite edge.
    pub fn stretch_object(&mut self, direction: Direction) -> bool {
        let Some(id) = self.cursor_instance() else {
            return false;
        };
        // DfnEdt and Edt edit the copy as canvas; stretching is outside them.
        if self.objects.session.is_some() {
            return false;
        }
        let Some(instance) = self.objects.instance(id) else {
            return false;
        };
        let previous = self.objects.bounds(instance);
        let (width, height) = self.objects.size(instance);
        let cursor = self.grid.cursor_pos;
        let center_column = f32::from(previous.left + previous.right) / 2.0;
        let center_line = f32::from(previous.top + previous.bottom) / 2.0;
        let (column, line) = (f32::from(cursor.column), f32::from(cursor.line));
        // Repeating the same stretch keeps expanding or contracting even when
        // the cursor crosses the moving center.
        let expand = match self.stretch_latch {
            Some((latched, latched_direction, expand, at))
                if latched == id && latched_direction == direction && at == cursor =>
            {
                expand
            }
            _ => match direction {
                Direction::Left => column <= center_column,
                Direction::Right => column >= center_column,
                Direction::Up => line <= center_line,
                Direction::Down => line >= center_line,
            },
        };
        // (origin shift, size change) per axis.
        let ((origin_columns, columns), (origin_lines, lines)) = match (direction, expand) {
            (Direction::Left, true) => ((-1, 1), (0, 0)),
            (Direction::Left, false) if width > 1 => ((0, -1), (0, 0)),
            (Direction::Right, true) => ((0, 1), (0, 0)),
            (Direction::Right, false) if width > 1 => ((1, -1), (0, 0)),
            (Direction::Up, true) => ((0, 0), (-1, 1)),
            (Direction::Up, false) if height > 1 => ((0, 0), (0, -1)),
            (Direction::Down, true) => ((0, 0), (0, 1)),
            (Direction::Down, false) if height > 1 => ((0, 0), (1, -1)),
            _ => return false,
        };
        if let Some(instance) = self.objects.instance_mut(id) {
            instance.origin.column += origin_columns;
            instance.origin.line += origin_lines;
            instance.stretch.0 += columns;
            instance.stretch.1 += lines;
        }
        self.materialize(id);
        if let Some(instance) = self.objects.instance(id) {
            let bounds = self.objects.bounds(instance);
            self.grid.cursor_pos = Coord {
                line: cursor.line.clamp(bounds.top, bounds.bottom),
                column: cursor.column.clamp(bounds.left, bounds.right),
            };
            self.selection.collapse(self.grid.cursor_pos);
        }
        self.stretch_latch = Some((id, direction, expand, self.grid.cursor_pos));
        true
    }

    /// Space in Objects: places a copy of the last defined object with its
    /// north-west corner at the cursor.
    pub fn place_object(&mut self) -> bool {
        let Some(object) = self
            .objects
            .current
            .filter(|object| self.objects.definition(*object).is_some())
        else {
            return false;
        };
        let id = self
            .objects
            .place(object, self.canvas.active_id(), self.grid.cursor_pos);
        self.materialize(id);
        true
    }

    /// Backspace in Objects outside DfnEdt and Edt: removes the copy under the
    /// cursor and keeps its definition. Returns whether a copy was removed.
    pub fn remove_object(&mut self) -> bool {
        if self.cursor_mode != CursorMode::Objects
            || self.objects.session.is_some()
            || !self.selection.is_collapsed()
        {
            return false;
        }
        let Some(id) = self.cursor_instance() else {
            return false;
        };
        // Text over the copy goes first; the copy goes once nothing is above.
        if self.text_over_copy_at(self.grid.cursor_pos) {
            return false;
        }
        let Some(instance) = self.objects.remove_instance(id) else {
            return false;
        };
        let layer = instance.layer;
        let mut before = self.painted_top(layer);
        for (coord, cell) in instance.painted.iter() {
            before
                .entry((coord.line, coord.column))
                .or_insert_with(|| cell.clone());
        }
        self.recompose_layer(layer, &before);
        true
    }

    /// Backspace in Edt: drops the local edits under the selection so the
    /// definition shows again. A typed space covers the definition instead.
    /// Returns whether the selection touched the edited copy.
    pub fn revert_local_copy(&mut self) -> bool {
        let Some(Session::Edit(id)) = self.objects.session else {
            return false;
        };
        let Some(instance) = self.objects.instance(id) else {
            return false;
        };
        let (origin, bounds) = (instance.origin, self.objects.bounds(instance));
        let selection = self.selection.bounds();
        let overlap = SelectionBounds {
            top: selection.top.max(bounds.top),
            left: selection.left.max(bounds.left),
            bottom: selection.bottom.min(bounds.bottom),
            right: selection.right.min(bounds.right),
        };
        if overlap.top > overlap.bottom || overlap.left > overlap.right {
            return false;
        }
        if let Some(instance) = self.objects.instance_mut(id) {
            instance.overlay.0.retain(|&(line, column), _| {
                !bounds_contain(
                    overlap,
                    Coord {
                        line: origin.line + line,
                        column: origin.column + column,
                    },
                )
            });
        }
        self.materialize(id);
        true
    }

    /// Edt Res: drops the local edits, stretch, and growth of the copy under the
    /// cursor so it matches its definition again.
    fn reset_object(&mut self) -> bool {
        let Some(id) = self.cursor_instance() else {
            self.object_tip("Place the cursor on an object first");
            return false;
        };
        if self.objects.session == Some(Session::Define(id)) {
            return false;
        }
        let Some(instance) = self.objects.instance_mut(id) else {
            return false;
        };
        instance.overlay = Cells::default();
        instance.stretch = (0, 0);
        instance.extent = crate::objects::Extent::default();
        self.materialize(id);
        true
    }

    /// Canvas cells of `instance` that show the stretched definition rather
    /// than a local edit.
    fn definition_cells(&self, instance: &crate::objects::Instance) -> Vec<Coord> {
        // A lone copy is its own definition: nothing to tell apart.
        let copies = self
            .objects
            .instances
            .iter()
            .filter(|other| other.object == instance.object)
            .count();
        let Some(definition) = self
            .objects
            .definition(instance.object)
            .filter(|_| copies > 1)
        else {
            return Vec::new();
        };
        let (width, height) = self.objects.size(instance);
        render_definition(definition, width, height)
            .iter()
            .filter(|(local, cell)| !cell.is_blank() && instance.overlay.get(*local).is_none())
            .map(|(local, _)| Coord {
                line: instance.origin.line + local.line,
                column: instance.origin.column + local.column,
            })
            .collect()
    }

    /// Backspace in DfnEdt on an anchor removes only the anchor; the next
    /// Backspace clears the cell. Returns whether one was removed.
    pub fn remove_anchor_at_cursor(&mut self) -> bool {
        if !self.selection.is_collapsed() {
            return false;
        }
        let Some(Session::Define(id)) = self.objects.session else {
            return false;
        };
        let Some(instance) = self.objects.instance(id) else {
            return false;
        };
        let (object, origin) = (instance.object, instance.origin);
        let local = Coord {
            line: self.grid.cursor_pos.line - origin.line,
            column: self.grid.cursor_pos.column - origin.column,
        };
        self.objects
            .definition_mut(object)
            .is_some_and(|definition| definition.remove_anchor_at(local))
    }

    /// Alt-direction on a copy outside DfnEdt and Edt moves the copy one cell
    /// and the cursor with it. Returns whether a copy moved.
    pub fn move_object(&mut self, direction: Direction) -> bool {
        let Some(id) = self.solid_instance_at(self.grid.cursor_pos) else {
            return false;
        };
        if self.objects.session.is_some() {
            return false;
        }
        let Some(cursor) = adjacent_coord(self.grid.cursor_pos, direction) else {
            return false;
        };
        let Some(instance) = self.objects.instance_mut(id) else {
            return false;
        };
        instance.origin = Coord {
            line: instance.origin.line + (cursor.line - self.grid.cursor_pos.line),
            column: instance.origin.column + (cursor.column - self.grid.cursor_pos.column),
        };
        self.materialize(id);
        self.grid.cursor_pos = cursor;
        self.selection.collapse(cursor);
        true
    }

    pub fn object_overlay(&self) -> ObjectOverlay {
        let session = self.objects.session;
        let mut overlay = ObjectOverlay {
            cursor_object: self
                .solid_instance_at(self.grid.cursor_pos)
                .and_then(|id| self.objects.instance(id))
                .map(|instance| self.objects.bounds(instance)),
            ..ObjectOverlay::default()
        };
        if self.cursor_mode == CursorMode::Objects || session.is_some() {
            overlay.outlines = self
                .objects
                .instances
                .iter()
                .map(|instance| match session {
                    Some(Session::Define(id)) if id == instance.id => {
                        self.objects.definition_bounds(instance)
                    }
                    _ => self.objects.bounds(instance),
                })
                // The copy under the cursor has its own outline.
                .filter(|bounds| Some(*bounds) != overlay.cursor_object)
                .collect();
        }
        let Some(instance) = self.objects.session_instance() else {
            return overlay;
        };
        let origin = instance.origin;
        let Some(Session::Define(_)) = session else {
            overlay.focus = Some(self.objects.bounds(instance));
            overlay.definition_cells = self.definition_cells(instance);
            overlay.blank_cells = instance
                .overlay
                .iter()
                .filter(|(_, cell)| cell.is_blank())
                .map(|(local, _)| Coord {
                    line: instance.origin.line + local.line,
                    column: instance.origin.column + local.column,
                })
                .collect();
            return overlay;
        };
        let shown = self.objects.definition_bounds(instance);
        overlay.focus = Some(shown);
        overlay.ghost = instance
            .overlay
            .iter()
            .map(|(local, cell)| {
                (
                    Coord {
                        line: origin.line + local.line,
                        column: origin.column + local.column,
                    },
                    cell.clone(),
                )
            })
            .filter(|(coord, _)| bounds_contain(shown, *coord))
            .collect();
        let Some(definition) = self.objects.definition(instance.object) else {
            return overlay;
        };
        let placed = resolve_anchors(definition, definition.width, definition.height);
        let canvas = |local: Coord| Coord {
            line: origin.line + local.line,
            column: origin.column + local.column,
        };
        for anchor in &definition.anchors {
            let at = canvas(anchor.at);
            let mut segments = Vec::new();
            for (target, horizontal, side) in [
                (anchor.horizontal, true, anchor.kind.horizontal_side()),
                (anchor.vertical, false, anchor.kind.vertical_side()),
            ] {
                let segment = match target {
                    Some(AnchorTarget::Anchor(other)) => placed
                        .get(&other)
                        .copied()
                        .map(|coord| AnchorSegment::Anchor(canvas(coord))),
                    Some(AnchorTarget::Bounds) => side.map(|side| AnchorSegment::Edge {
                        horizontal,
                        min: side == crate::objects::Side::Min,
                    }),
                    None => None,
                };
                if let Some(segment) = segment
                    && !segments.contains(&segment)
                {
                    segments.push(segment);
                }
            }
            overlay.anchors.push(AnchorMark {
                at,
                kind: anchor.kind,
                segments,
            });
        }
        overlay
    }
}
