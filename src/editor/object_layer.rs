//! Copies form the lowest layer under ordinary text. Each copy remembers
//! what it painted; a recompose repaints every copy on a layer and keeps
//! any canvas cell that differs from that paint, which is text written over
//! the copy. Edits made inside DfnEdt or Edt fold back into the definition
//! or the local copy.

use super::*;
use crate::objects::{Cells, InstanceId, ObjectCell, Session, bounds_contain, render_definition};

type Key = (i16, i16);

impl Editor {
    /// The local copy composited over the stretched definition.
    pub(super) fn instance_cells(&self, id: InstanceId) -> Option<Cells> {
        let instance = self.objects.instance(id)?;
        let definition = self.objects.definition(instance.object)?;
        let (width, height) = self.objects.size(instance);
        let mut cells = render_definition(definition, width, height);
        cells.0.retain(|_, cell| !cell.is_blank());
        // Local blanks stay: they paint the cell empty over lower copies.
        for (local, cell) in instance.overlay.iter() {
            cells.insert(local, cell.clone());
        }
        Some(cells)
    }

    /// Absolute cells a copy shows: the definition while DfnEdt edits it,
    /// otherwise its stretched local copy. Blank cells are local blanks.
    pub(super) fn view_cells(&self, id: InstanceId) -> Vec<(Key, ObjectCell)> {
        let Some(instance) = self.objects.instance(id) else {
            return Vec::new();
        };
        let cells = if self.objects.session == Some(Session::Define(id)) {
            self.objects
                .definition(instance.object)
                .map(|definition| definition.cells.clone())
                .unwrap_or_default()
        } else {
            self.instance_cells(id).unwrap_or_default()
        };
        let origin = instance.origin;
        cells
            .iter()
            .map(|(local, cell)| {
                (
                    (origin.line + local.line, origin.column + local.column),
                    cell.clone(),
                )
            })
            .collect()
    }

    /// What the copies on `layer` last painted, topmost copy winning.
    pub(super) fn painted_top(&self, layer: LayerId) -> BTreeMap<Key, ObjectCell> {
        let mut top = BTreeMap::new();
        for instance in self
            .objects
            .instances
            .iter()
            .filter(|instance| instance.layer == layer)
        {
            for (coord, cell) in instance.painted.iter() {
                top.insert((coord.line, coord.column), cell.clone());
            }
        }
        top
    }

    /// Repaints the copy and everything it overlaps on its layer.
    pub(super) fn materialize(&mut self, id: InstanceId) {
        let Some(layer) = self.objects.instance(id).map(|instance| instance.layer) else {
            return;
        };
        let before = self.painted_top(layer);
        self.recompose_layer(layer, &before);
    }

    /// Repaints every copy on `layer`. A canvas cell that still shows what
    /// the copies painted before, or nothing, takes the new paint; any other
    /// content is text over the copies and stays.
    pub(super) fn recompose_layer(&mut self, layer: LayerId, before: &BTreeMap<Key, ObjectCell>) {
        self.fold_lone_copies();
        let Some(index) = self.canvas.index_of(layer) else {
            return;
        };
        let ids = self
            .objects
            .instances
            .iter()
            .filter(|instance| instance.layer == layer)
            .map(|instance| instance.id)
            .collect::<Vec<_>>();
        let mut after = BTreeMap::new();
        let mut views = Vec::with_capacity(ids.len());
        for id in ids {
            let view = self.view_cells(id);
            for (key, cell) in &view {
                after.insert(*key, cell.clone());
            }
            views.push((id, view));
        }
        let keys = before
            .keys()
            .chain(after.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let mut writes = Vec::new();
        for key in keys {
            let current = self.canvas.layers()[index]
                .get(key.0, key.1)
                .map(object_cell);
            let expected = before.get(&key).filter(|cell| !cell.is_blank());
            if current.is_none() || current.as_ref() == expected {
                let paint = after.get(&key).cloned().unwrap_or_else(ObjectCell::blank);
                let unchanged = match &current {
                    Some(cell) => *cell == paint,
                    None => paint.is_blank(),
                };
                if !unchanged {
                    writes.push((key, paint));
                }
            }
        }
        self.write_absolute(layer, &writes);
        for (id, view) in views {
            if let Some(instance) = self.objects.instance_mut(id) {
                let mut painted = Cells::default();
                for ((line, column), cell) in view {
                    painted.insert(Coord { line, column }, cell);
                }
                instance.painted = painted;
            }
        }
    }

    /// An object with a single unstretched copy has no separate definition:
    /// the copy's local edits and growth become the definition.
    fn fold_lone_copies(&mut self) {
        let lone = self
            .objects
            .definitions
            .iter()
            .filter_map(|definition| {
                let mut copies = self
                    .objects
                    .instances
                    .iter()
                    .filter(|instance| instance.object == definition.id);
                let only = copies.next()?;
                // Size stays a stretch of the definition, so resizing never
                // loses structure; only unstretched local edits fold.
                let folded =
                    only.overlay.0.is_empty() && only.extent == crate::objects::Extent::default();
                (copies.next().is_none() && only.stretch == (0, 0) && !folded).then_some(only.id)
            })
            .collect::<Vec<_>>();
        for id in lone {
            self.fold_copy(id);
        }
    }

    /// Opening DfnEdt or Edt on a lone copy makes it its own definition,
    /// size included, so both show the same thing.
    pub(super) fn fold_lone_copy(&mut self, id: InstanceId) {
        let Some(object) = self.objects.instance(id).map(|instance| instance.object) else {
            return;
        };
        let copies = self
            .objects
            .instances
            .iter()
            .filter(|instance| instance.object == object)
            .count();
        if copies == 1 {
            self.fold_copy(id);
        }
    }

    /// Makes the copy's stretched local copy its definition, moving anchors
    /// to where they resolve at the copy's size.
    fn fold_copy(&mut self, id: InstanceId) {
        let Some(instance) = self.objects.instance(id) else {
            return;
        };
        let Some(definition) = self.objects.definition(instance.object) else {
            return;
        };
        let (width, height) = self.objects.size(instance);
        let bounds = self.objects.bounds(instance);
        let (top, left) = (
            instance.origin.line - bounds.top,
            instance.origin.column - bounds.left,
        );
        let resolved = crate::objects::resolve_anchors(definition, width, height);
        let object = instance.object;
        let mut cells = Cells::default();
        for (local, cell) in self.instance_cells(id).unwrap_or_default().iter() {
            if !cell.is_blank() {
                cells.insert(
                    Coord {
                        line: local.line + top,
                        column: local.column + left,
                    },
                    cell.clone(),
                );
            }
        }
        if let Some(definition) = self.objects.definition_mut(object) {
            definition.cells = cells;
            definition.width = bounds.right - bounds.left + 1;
            definition.height = bounds.bottom - bounds.top + 1;
            for anchor in &mut definition.anchors {
                if let Some(at) = resolved.get(&anchor.id) {
                    anchor.at = Coord {
                        line: at.line + top,
                        column: at.column + left,
                    };
                }
            }
        }
        if let Some(instance) = self.objects.instance_mut(id) {
            instance.origin = Coord {
                line: bounds.top,
                column: bounds.left,
            };
            instance.overlay = Cells::default();
            instance.stretch = (0, 0);
            instance.extent = crate::objects::Extent::default();
        }
    }

    pub(super) fn write_absolute(&mut self, layer: LayerId, cells: &[(Key, ObjectCell)]) {
        let Some(index) = self.canvas.index_of(layer) else {
            return;
        };
        for ((line, column), cell) in cells {
            let target = self.canvas.layer_mut(index);
            target.delete_at(*column, *line);
            self.object_writes.insert((layer, *line, *column));
            if cell.is_blank() {
                continue;
            }
            let Ok(atom) = Atom::new(cell.atom.as_str()) else {
                continue;
            };
            let target = self.canvas.layer_mut(index);
            if target.set_at(*column, *line, atom, &cell.face).is_err() {
                continue;
            }
            if cell.line.is_some() {
                target.set_line_data(*column, *line, cell.line.clone());
            }
        }
    }

    pub(super) fn read_cells(
        &self,
        layer: LayerId,
        bounds: SelectionBounds,
        origin: Coord,
    ) -> Cells {
        let mut cells = Cells::default();
        let Some(index) = self.canvas.index_of(layer) else {
            return cells;
        };
        for (&line, row) in self.canvas.layers()[index]
            .rows()
            .range(bounds.top..=bounds.bottom)
        {
            for (&column, data) in row.range(bounds.left..=bounds.right) {
                cells.insert(
                    Coord {
                        line: line - origin.line,
                        column: column - origin.column,
                    },
                    object_cell(data),
                );
            }
        }
        cells
    }

    /// Documents saved before copies recorded their paint only listed the
    /// cells; the canvas still shows what they painted.
    pub(super) fn adopt_saved_paint(&mut self) {
        for index in 0..self.objects.instances.len() {
            let instance = &self.objects.instances[index];
            if !instance.painted.0.is_empty() || instance.footprint.is_empty() {
                continue;
            }
            let Some(layer) = self.canvas.index_of(instance.layer) else {
                continue;
            };
            let mut painted = Cells::default();
            for &(line, column) in &instance.footprint {
                if let Some(data) = self.canvas.layers()[layer].get(line, column) {
                    painted.insert(Coord { line, column }, object_cell(data));
                }
            }
            let instance = &mut self.objects.instances[index];
            instance.painted = painted;
            instance.footprint.clear();
        }
    }

    /// Folds the edits of the open history capture into objects: DfnEdt
    /// rewrites the definition (growing it for edits outside) and Edt the
    /// local copy. Afterwards every touched layer is recomposed, so text
    /// over a copy stays and a copy shows again where its cell was emptied.
    pub fn sync_objects(&mut self) {
        let written = std::mem::take(&mut self.object_writes);
        let changes = self
            .canvas
            .captured_changes()
            .into_iter()
            .filter(|key| !written.contains(key))
            .collect::<Vec<_>>();
        let writes = self
            .canvas
            .captured_writes()
            .into_iter()
            .filter(|key| !written.contains(key))
            .collect::<Vec<_>>();
        if writes.is_empty() {
            return;
        }
        let layers = writes
            .iter()
            .map(|key| key.0)
            .collect::<std::collections::BTreeSet<_>>();
        let before = layers
            .iter()
            .map(|layer| (*layer, self.painted_top(*layer)))
            .collect::<Vec<_>>();
        if let Some(session) = self.objects.session {
            match self
                .objects
                .session_instance()
                .map(|instance| instance.layer)
            {
                Some(layer) => {
                    let on_layer = |keys: &[(LayerId, i16, i16)]| {
                        keys.iter()
                            .filter(|key| key.0 == layer)
                            .map(|&(_, line, column)| Coord { line, column })
                            .collect::<Vec<_>>()
                    };
                    let (changed, typed) = (on_layer(&changes), on_layer(&writes));
                    match session {
                        Session::Define(id) if !changed.is_empty() => {
                            self.sync_definition(id, &changed);
                        }
                        Session::Edit(id) if !typed.is_empty() => {
                            self.sync_local_copy(id, &typed);
                        }
                        _ => {}
                    }
                }
                None => self.objects.session = None,
            }
        }
        for (layer, before) in before {
            self.recompose_layer(layer, &before);
        }
        self.object_writes.clear();
    }

    /// DfnEdt: the changed cells become definition cells, growing the
    /// definition for cells outside it.
    fn sync_definition(&mut self, id: InstanceId, changed: &[Coord]) {
        let Some(instance) = self.objects.instance(id) else {
            return;
        };
        let (object, origin, layer) = (instance.object, instance.origin, instance.layer);
        let mut grown = self.objects.definition_bounds(instance);
        for coord in changed {
            grown.top = grown.top.min(coord.line);
            grown.bottom = grown.bottom.max(coord.line);
            grown.left = grown.left.min(coord.column);
            grown.right = grown.right.max(coord.column);
        }
        let local = SelectionBounds {
            top: grown.top - origin.line,
            bottom: grown.bottom - origin.line,
            left: grown.left - origin.column,
            right: grown.right - origin.column,
        };
        self.objects.grow_definition(object, local);
        let Some(origin) = self.objects.instance(id).map(|instance| instance.origin) else {
            return;
        };
        let Some(index) = self.canvas.index_of(layer) else {
            return;
        };
        let updates = changed
            .iter()
            .map(|coord| {
                let local = Coord {
                    line: coord.line - origin.line,
                    column: coord.column - origin.column,
                };
                let cell = self.canvas.layers()[index]
                    .get(coord.line, coord.column)
                    .map(object_cell);
                (local, cell)
            })
            .collect::<Vec<_>>();
        if let Some(definition) = self.objects.definition_mut(object) {
            for (local, cell) in updates {
                match cell {
                    Some(cell) => definition.cells.insert(local, cell),
                    None => {
                        definition.cells.0.remove(&(local.line, local.column));
                    }
                }
            }
        }
    }

    /// Edt: changed cells inside the copy become local edits. A cell that
    /// matches the definition drops its edit; an emptied cell becomes an
    /// opaque local blank. The boundary never changes.
    fn sync_local_copy(&mut self, id: InstanceId, changed: &[Coord]) {
        let Some(instance) = self.objects.instance(id) else {
            return;
        };
        let Some(definition) = self.objects.definition(instance.object) else {
            return;
        };
        let (width, height) = self.objects.size(instance);
        let base = render_definition(definition, width, height);
        let (layer, origin, bounds) = (
            instance.layer,
            instance.origin,
            self.objects.bounds(instance),
        );
        let current = self.read_cells(layer, bounds, origin);
        let mut overlay = instance.overlay.clone();
        for coord in changed
            .iter()
            .filter(|coord| bounds_contain(bounds, **coord))
        {
            let local = Coord {
                line: coord.line - origin.line,
                column: coord.column - origin.column,
            };
            match (current.get(local), base.get(local)) {
                (Some(now), Some(was)) if now == was => {
                    overlay.0.remove(&(local.line, local.column));
                }
                (Some(now), _) => overlay.insert(local, now.clone()),
                (None, _) => overlay.insert(local, ObjectCell::blank()),
            }
        }
        if let Some(instance) = self.objects.instance_mut(id) {
            instance.overlay = overlay;
        }
    }

    /// A confirmed selection move carries the copies lying wholly inside the
    /// selection: the original goes to `origin`, and each clone stamp places
    /// another copy. Their canvas cells already moved with the selection.
    pub(super) fn move_lifted_objects(
        &mut self,
        source: SelectionBounds,
        origin: Coord,
        clones: &[Coord],
        layers: &[LayerId],
    ) {
        let carried = self
            .objects
            .instances
            .iter()
            .filter(|instance| layers.contains(&instance.layer))
            .filter(|instance| {
                let bounds = self.objects.bounds(instance);
                bounds.top >= source.top
                    && bounds.bottom <= source.bottom
                    && bounds.left >= source.left
                    && bounds.right <= source.right
            })
            .map(|instance| instance.id)
            .collect::<Vec<_>>();
        if carried.is_empty() {
            return;
        }
        let shift = |coord: Coord, to: Coord| Coord {
            line: coord.line + (to.line - source.top),
            column: coord.column + (to.column - source.left),
        };
        let shifted = |cells: &Cells, to: Coord| {
            let mut moved = Cells::default();
            for (coord, cell) in cells.iter() {
                moved.insert(shift(coord, to), cell.clone());
            }
            moved
        };
        let mut touched = std::collections::BTreeSet::new();
        for id in carried {
            let Some(template) = self.objects.instance(id).cloned() else {
                continue;
            };
            touched.insert(template.layer);
            for to in clones {
                let copy = self.objects.place(
                    template.object,
                    template.layer,
                    shift(template.origin, *to),
                );
                if let Some(instance) = self.objects.instance_mut(copy) {
                    instance.stretch = template.stretch;
                    instance.overlay = template.overlay.clone();
                    instance.extent = template.extent;
                    instance.painted = shifted(&template.painted, *to);
                }
            }
            if let Some(instance) = self.objects.instance_mut(id) {
                instance.origin = shift(template.origin, origin);
                instance.painted = shifted(&template.painted, origin);
            }
        }
        for layer in touched {
            let before = self.painted_top(layer);
            self.recompose_layer(layer, &before);
        }
    }
}

fn object_cell(data: &crate::canvas::CoordData) -> ObjectCell {
    ObjectCell {
        atom: data.atom.contents().to_owned(),
        face: data.face.as_ref().clone(),
        line: data.line.clone(),
    }
}

use std::collections::BTreeMap;
