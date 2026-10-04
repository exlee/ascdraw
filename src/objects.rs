//! Reusable objects: a definition is a saved rectangle of cells anchored at
//! its north-west corner, and every instance materializes that definition
//! (stretched to the instance size, then covered by its local overlay) into
//! one canvas layer. Positions inside definitions and overlays are local to
//! the north-west corner.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::canvas::LineData;
use crate::model::{Coord, Face, LayerId};
use crate::selection::SelectionBounds;

mod stretch;
#[cfg(test)]
use stretch::implicit_groups;
pub use stretch::{render_definition, resolve_anchors};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct ObjectId(pub u32);

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct InstanceId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectCell {
    pub atom: String,
    #[serde(default)]
    pub face: Face,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<LineData>,
}

impl ObjectCell {
    pub fn blank() -> Self {
        Self {
            atom: " ".to_owned(),
            face: Face::default(),
            line: None,
        }
    }

    pub fn is_blank(&self) -> bool {
        self.atom.chars().all(char::is_whitespace) && self.face == Face::default()
    }
}

/// Sparse local cells keyed by `(line, column)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<CellEntry>", into = "Vec<SavedCellEntry>")]
pub struct Cells(pub BTreeMap<(i16, i16), ObjectCell>);

/// One saved cell. `p` is `[column, line]`, `v` the atom and line data is
/// written as `line-data`. Older documents wrote `line`, `column` and `atom`,
/// with line data as a second `line` key; those are still read.
struct CellEntry {
    line: i16,
    column: i16,
    atom: String,
    face: Face,
    line_data: Option<LineData>,
}

#[derive(Serialize)]
struct SavedCellEntry {
    p: [i16; 2],
    v: String,
    face: Face,
    #[serde(rename = "line-data", skip_serializing_if = "Option::is_none")]
    line_data: Option<LineData>,
}

impl<'de> Deserialize<'de> for CellEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum LineField {
            Position(i16),
            Data(LineData),
        }

        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = CellEntry;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an object cell")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<CellEntry, A::Error> {
                let (mut line, mut column, mut atom) = (None, None, None);
                let (mut face, mut line_data) = (Face::default(), None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "line" => match map.next_value::<LineField>()? {
                            LineField::Position(value) => line = Some(value),
                            LineField::Data(data) => line_data = Some(data),
                        },
                        "line-data" => line_data = map.next_value()?,
                        "p" => {
                            let [x, y] = map.next_value::<[i16; 2]>()?;
                            (column, line) = (Some(x), Some(y));
                        }
                        "v" => atom = Some(map.next_value()?),
                        "column" => column = Some(map.next_value()?),
                        "atom" => atom = Some(map.next_value()?),
                        "face" => face = map.next_value()?,
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(CellEntry {
                    line: line.ok_or_else(|| serde::de::Error::missing_field("p"))?,
                    column: column.ok_or_else(|| serde::de::Error::missing_field("p"))?,
                    atom: atom.ok_or_else(|| serde::de::Error::missing_field("v"))?,
                    face,
                    line_data,
                })
            }
        }

        deserializer.deserialize_map(Visitor)
    }
}

impl From<Vec<CellEntry>> for Cells {
    fn from(entries: Vec<CellEntry>) -> Self {
        Self(
            entries
                .into_iter()
                .map(|entry| {
                    (
                        (entry.line, entry.column),
                        ObjectCell {
                            atom: entry.atom,
                            face: entry.face,
                            line: entry.line_data,
                        },
                    )
                })
                .collect(),
        )
    }
}

impl From<Cells> for Vec<SavedCellEntry> {
    fn from(cells: Cells) -> Self {
        cells
            .0
            .into_iter()
            .map(|((line, column), cell)| SavedCellEntry {
                p: [column, line],
                v: cell.atom,
                face: cell.face,
                line_data: cell.line,
            })
            .collect()
    }
}

impl Cells {
    pub fn get(&self, local: Coord) -> Option<&ObjectCell> {
        self.0.get(&(local.line, local.column))
    }

    pub fn insert(&mut self, local: Coord, cell: ObjectCell) {
        self.0.insert((local.line, local.column), cell);
    }

    pub fn iter(&self) -> impl Iterator<Item = (Coord, &ObjectCell)> {
        self.0
            .iter()
            .map(|(&(line, column), cell)| (Coord { line, column }, cell))
    }

    fn shifted(&self, lines: i16, columns: i16) -> Self {
        Self(
            self.0
                .iter()
                .map(|(&(line, column), cell)| ((line + lines, column + columns), cell.clone()))
                .collect(),
        )
    }
}

/// Anchor directions offered by the Anchr menu, in menu order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnchorKind {
    W,
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    Extension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Min,
    Max,
}

impl AnchorKind {
    pub const ALL: [Self; 9] = [
        Self::W,
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::Extension,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::W => "W",
            Self::NW => "NW",
            Self::N => "N",
            Self::NE => "NE",
            Self::E => "E",
            Self::SE => "SE",
            Self::S => "S",
            Self::SW => "SW",
            Self::Extension => "·",
        }
    }

    /// The bounding-box side the horizontal offset is measured from.
    pub fn horizontal_side(self) -> Option<Side> {
        match self {
            Self::W | Self::NW | Self::SW => Some(Side::Min),
            Self::E | Self::NE | Self::SE => Some(Side::Max),
            Self::N | Self::S | Self::Extension => None,
        }
    }

    /// The bounding-box side the vertical offset is measured from.
    pub fn vertical_side(self) -> Option<Side> {
        match self {
            Self::N | Self::NW | Self::NE => Some(Side::Min),
            Self::S | Self::SW | Self::SE => Some(Side::Max),
            Self::W | Self::E | Self::Extension => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnchorTarget {
    Bounds,
    Anchor(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub id: u32,
    pub at: Coord,
    pub kind: AnchorKind,
    /// Reference that fixes the column; `None` scales proportionally.
    pub horizontal: Option<AnchorTarget>,
    /// Reference that fixes the line; `None` scales proportionally.
    pub vertical: Option<AnchorTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectDefinition {
    pub id: ObjectId,
    pub width: i16,
    pub height: i16,
    pub cells: Cells,
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    #[serde(default)]
    pub next_anchor: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instance {
    pub id: InstanceId,
    pub object: ObjectId,
    pub layer: LayerId,
    pub origin: Coord,
    /// Size difference from the definition; non-zero means stretched.
    #[serde(default)]
    pub stretch: (i16, i16),
    /// Local copy cells laid over the stretched definition. Blank cells
    /// clear what the definition shows underneath.
    #[serde(default)]
    pub overlay: Cells,
    /// Cells the local copy grew beyond the stretched definition, per side.
    #[serde(default)]
    pub extent: Extent,
    /// What this copy last painted, by absolute `(line, column)`. A canvas
    /// cell that differs is text written over the copy.
    #[serde(default)]
    pub painted: Cells,
    /// Cells older documents listed instead of `painted`; read on load only.
    #[serde(default, skip_serializing)]
    pub footprint: Vec<(i16, i16)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extent {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Define(InstanceId),
    Edit(InstanceId),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectStore {
    #[serde(default)]
    pub definitions: Vec<ObjectDefinition>,
    #[serde(default)]
    pub instances: Vec<Instance>,
    #[serde(skip)]
    pub session: Option<Session>,
    #[serde(default)]
    pub current: Option<ObjectId>,
    #[serde(default)]
    next_object: u32,
    #[serde(default)]
    next_instance: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorError {
    OutsideDefinition,
    NothingToExtend,
}

impl ObjectStore {
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty() && self.instances.is_empty()
    }

    pub fn definition(&self, id: ObjectId) -> Option<&ObjectDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.id == id)
    }

    pub fn definition_mut(&mut self, id: ObjectId) -> Option<&mut ObjectDefinition> {
        self.definitions
            .iter_mut()
            .find(|definition| definition.id == id)
    }

    pub fn instance(&self, id: InstanceId) -> Option<&Instance> {
        self.instances.iter().find(|instance| instance.id == id)
    }

    pub fn instance_mut(&mut self, id: InstanceId) -> Option<&mut Instance> {
        self.instances.iter_mut().find(|instance| instance.id == id)
    }

    pub fn session_instance(&self) -> Option<&Instance> {
        match self.session? {
            Session::Define(id) | Session::Edit(id) => self.instance(id),
        }
    }

    /// Creates a definition from `cells` and its first instance at `origin`.
    pub fn define(
        &mut self,
        layer: LayerId,
        origin: Coord,
        width: i16,
        height: i16,
        cells: Cells,
    ) -> InstanceId {
        let object = ObjectId(self.next_object);
        self.next_object += 1;
        self.definitions.push(ObjectDefinition {
            id: object,
            width: width.max(1),
            height: height.max(1),
            cells,
            anchors: Vec::new(),
            next_anchor: 0,
        });
        self.current = Some(object);
        self.place(object, layer, origin)
    }

    pub fn place(&mut self, object: ObjectId, layer: LayerId, origin: Coord) -> InstanceId {
        let id = InstanceId(self.next_instance);
        self.next_instance += 1;
        self.instances.push(Instance {
            id,
            object,
            layer,
            origin,
            stretch: (0, 0),
            overlay: Cells::default(),
            extent: Extent::default(),
            painted: Cells::default(),
            footprint: Vec::new(),
        });
        id
    }

    pub fn remove_instance(&mut self, id: InstanceId) -> Option<Instance> {
        let index = self
            .instances
            .iter()
            .position(|instance| instance.id == id)?;
        Some(self.instances.remove(index))
    }

    /// Instance size: the definition size plus its stretch, never below one cell.
    pub fn size(&self, instance: &Instance) -> (i16, i16) {
        let Some(definition) = self.definition(instance.object) else {
            return (1, 1);
        };
        (
            (definition.width + instance.stretch.0).max(1),
            (definition.height + instance.stretch.1).max(1),
        )
    }

    /// Everything the copy covers: the stretched definition plus its extent.
    pub fn bounds(&self, instance: &Instance) -> SelectionBounds {
        let (width, height) = self.size(instance);
        let core = rect_bounds(instance.origin, width, height);
        let extent = instance.extent;
        SelectionBounds {
            top: core.top - extent.top,
            left: core.left - extent.left,
            bottom: core.bottom + extent.bottom,
            right: core.right + extent.right,
        }
    }

    pub fn definition_bounds(&self, instance: &Instance) -> SelectionBounds {
        let (width, height) = self
            .definition(instance.object)
            .map_or((1, 1), |definition| (definition.width, definition.height));
        rect_bounds(instance.origin, width, height)
    }

    /// The topmost (latest placed) instance on `layer` covering `coord`.
    pub fn instance_at(&self, layer: LayerId, coord: Coord) -> Option<InstanceId> {
        self.instances
            .iter()
            .rev()
            .filter(|instance| instance.layer == layer)
            .find(|instance| bounds_contain(self.bounds(instance), coord))
            .map(|instance| instance.id)
    }

    /// Grows a definition so local `bounds` fit, keeping existing content in
    /// place on the canvas. Returns the north-west shift `(lines, columns)`.
    pub fn grow_definition(&mut self, object: ObjectId, local: SelectionBounds) -> (i16, i16) {
        let Some(definition) = self.definition_mut(object) else {
            return (0, 0);
        };
        let shift_lines = (-local.top).max(0);
        let shift_columns = (-local.left).max(0);
        let bottom = local.bottom.max(definition.height - 1) + shift_lines;
        let right = local.right.max(definition.width - 1) + shift_columns;
        definition.height = bottom + 1;
        definition.width = right + 1;
        if shift_lines == 0 && shift_columns == 0 {
            return (0, 0);
        }
        definition.cells = definition.cells.shifted(shift_lines, shift_columns);
        for anchor in &mut definition.anchors {
            anchor.at.line += shift_lines;
            anchor.at.column += shift_columns;
        }
        for instance in self
            .instances
            .iter_mut()
            .filter(|instance| instance.object == object)
        {
            instance.origin.line -= shift_lines;
            instance.origin.column -= shift_columns;
            instance.overlay = instance.overlay.shifted(shift_lines, shift_columns);
        }
        (shift_lines, shift_columns)
    }
}

impl ObjectDefinition {
    /// Adds an anchor of `kind` at local `at`, or removes an identical one.
    /// Directional references hit the nearest anchor on the same line or
    /// column in that direction, falling back to the bounding box.
    pub fn toggle_anchor(&mut self, at: Coord, kind: AnchorKind) -> Result<bool, AnchorError> {
        if at.line < 0 || at.column < 0 || at.line >= self.height || at.column >= self.width {
            return Err(AnchorError::OutsideDefinition);
        }
        if self
            .anchors
            .iter()
            .any(|anchor| anchor.at == at && anchor.kind == kind)
        {
            self.remove_anchor_at(at);
            return Ok(false);
        }
        let (horizontal, vertical) = if kind == AnchorKind::Extension {
            let nearest = self
                .anchors
                .iter()
                .filter(|anchor| anchor.at != at)
                .min_by_key(|anchor| {
                    let distance = (anchor.at.line - at.line)
                        .abs()
                        .max((anchor.at.column - at.column).abs());
                    (distance, std::cmp::Reverse(anchor.id))
                })
                .ok_or(AnchorError::NothingToExtend)?;
            let target = Some(AnchorTarget::Anchor(nearest.id));
            (target, target)
        } else {
            (
                kind.horizontal_side()
                    .map(|side| self.ray_target(at, side, true)),
                kind.vertical_side()
                    .map(|side| self.ray_target(at, side, false)),
            )
        };
        self.anchors.retain(|anchor| anchor.at != at);
        let id = self.next_anchor;
        self.next_anchor += 1;
        self.anchors.push(Anchor {
            id,
            at,
            kind,
            horizontal,
            vertical,
        });
        Ok(true)
    }

    /// Removes the anchor at local `at`. References to it fall back to the
    /// bounding box; extensions of it go too.
    pub fn remove_anchor_at(&mut self, at: Coord) -> bool {
        let Some(index) = self.anchors.iter().position(|anchor| anchor.at == at) else {
            return false;
        };
        let removed = self.anchors.remove(index).id;
        for anchor in &mut self.anchors {
            for target in [&mut anchor.horizontal, &mut anchor.vertical] {
                if *target == Some(AnchorTarget::Anchor(removed)) {
                    *target =
                        (anchor.kind != AnchorKind::Extension).then_some(AnchorTarget::Bounds);
                }
            }
        }
        // An extension has no meaning without the anchor it extends.
        self.anchors
            .retain(|anchor| anchor.kind != AnchorKind::Extension || anchor.horizontal.is_some());
        true
    }

    fn ray_target(&self, at: Coord, side: Side, horizontal: bool) -> AnchorTarget {
        self.anchors
            .iter()
            .filter(|anchor| anchor.at != at)
            .filter_map(|anchor| {
                let (same, offset) = if horizontal {
                    (anchor.at.line == at.line, anchor.at.column - at.column)
                } else {
                    (anchor.at.column == at.column, anchor.at.line - at.line)
                };
                let toward = match side {
                    Side::Min => offset < 0,
                    Side::Max => offset > 0,
                };
                (same && toward).then_some((offset.abs(), anchor.id))
            })
            .min()
            .map_or(AnchorTarget::Bounds, |(_, id)| AnchorTarget::Anchor(id))
    }

    pub fn anchor(&self, id: u32) -> Option<&Anchor> {
        self.anchors.iter().find(|anchor| anchor.id == id)
    }
}

pub fn rect_bounds(origin: Coord, width: i16, height: i16) -> SelectionBounds {
    SelectionBounds {
        top: origin.line,
        left: origin.column,
        bottom: origin.line.saturating_add(height.max(1) - 1),
        right: origin.column.saturating_add(width.max(1) - 1),
    }
}

pub fn bounds_contain(bounds: SelectionBounds, coord: Coord) -> bool {
    (bounds.top..=bounds.bottom).contains(&coord.line)
        && (bounds.left..=bounds.right).contains(&coord.column)
}

#[cfg(test)]
#[path = "inline_tests/objects_tests.rs"]
mod tests;
