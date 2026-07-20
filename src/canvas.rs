use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;

use crate::drawing::LineEnding;
use crate::model::{Atom, Coord, Face, LAYER_SYMBOLS, LayerId, LayerSummary, MAX_LAYERS};
use crate::selection::SelectionBounds;

mod history;
pub use history::HistoryCanvasDelta;
use history::{record_cell_before, record_layer_before, record_row_before};

#[derive(Debug, Clone)]
pub struct Rasterized {
    pub generation: u64,
    pub image: skia_safe::Image,
    pub cell_width: f32,
    pub cell_height: f32,
    pub overflow: f32,
    pub atlas_safe: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LineData {
    pub ending: LineEnding,
    pub base_glyph: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineMarker {
    pub(crate) coord: Coord,
    pub(crate) ending: LineEnding,
    pub(crate) base_glyph: String,
}

#[derive(Debug)]
pub struct CoordData {
    pub face: Rc<Face>,
    pub atom: Rc<Atom>,
    pub raster_cache: RefCell<Option<Rc<Rasterized>>>,
    pub line: Option<LineData>,
}

impl Clone for CoordData {
    fn clone(&self) -> Self {
        Self {
            face: Rc::clone(&self.face),
            atom: Rc::clone(&self.atom),
            raster_cache: RefCell::new(None),
            line: self.line.clone(),
        }
    }
}

impl PartialEq for CoordData {
    fn eq(&self, other: &Self) -> bool {
        self.face == other.face && self.atom == other.atom && self.line == other.line
    }
}

impl Eq for CoordData {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerMap {
    pub id: LayerId,
    pub visible: bool,
    rows: BTreeMap<i16, BTreeMap<i16, CoordData>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerBounds {
    pub min_x: i16,
    pub min_y: i16,
    pub max_x: i16,
    pub max_y: i16,
}

impl LayerMap {
    pub fn new(id: LayerId, visible: bool) -> Self {
        Self {
            id,
            visible,
            rows: BTreeMap::new(),
        }
    }

    pub fn rows(&self) -> &BTreeMap<i16, BTreeMap<i16, CoordData>> {
        &self.rows
    }

    pub fn get(&self, line: i16, column: i16) -> Option<&CoordData> {
        self.rows.get(&line)?.get(&column)
    }

    pub fn bounds(&self) -> Option<LayerBounds> {
        let (&min_y, _) = self.rows.first_key_value()?;
        let (&max_y, _) = self.rows.last_key_value()?;
        let mut columns = self.rows.values().flat_map(|row| row.keys().copied());
        let min_x = columns.next()?;
        let (min_x, max_x) = columns.fold((min_x, min_x), |(min_x, max_x), x| {
            (min_x.min(x), max_x.max(x))
        });
        Some(LayerBounds {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    }

    pub(crate) fn row_width(&self, line: i16) -> usize {
        let Some(row) = self.rows.get(&line) else {
            return 0;
        };
        row.iter()
            .filter_map(|(&column, data)| {
                usize::try_from(column).ok().map(|column| {
                    column.saturating_add(UnicodeWidthStr::width(data.atom.contents()).max(1))
                })
            })
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn line_markers(&self) -> Vec<LineMarker> {
        self.rows
            .iter()
            .flat_map(|(&line, row)| {
                row.iter().filter_map(move |(&column, data)| {
                    let line_data = data.line.as_ref()?;
                    Some(LineMarker {
                        coord: Coord { line, column },
                        ending: line_data.ending,
                        base_glyph: line_data.base_glyph.clone(),
                    })
                })
            })
            .collect()
    }

    pub(crate) fn line_at(&self, coord: Coord) -> Option<&LineData> {
        self.get(coord.line, coord.column)?.line.as_ref()
    }

    pub(crate) fn take_line_at(&mut self, coord: Coord) -> Option<LineData> {
        let (line, column) = (coord.line, coord.column);
        record_cell_before(self.id, line, column, self.get(line, column));
        self.rows.get_mut(&line)?.get_mut(&column)?.line.take()
    }

    pub(crate) fn set_line_at(&mut self, coord: Coord, line_data: LineData) -> bool {
        let (line, column) = (coord.line, coord.column);
        let Some(data) = self
            .rows
            .get_mut(&line)
            .and_then(|row| row.get_mut(&column))
        else {
            return false;
        };
        record_cell_before(self.id, line, column, Some(data));
        data.line = Some(line_data);
        true
    }

    pub(crate) fn remove_line_at(&mut self, coord: Coord) -> bool {
        self.take_line_at(coord).is_some()
    }

    pub fn set_at(&mut self, x: i16, y: i16, atom: Atom, face: &Face) -> Result<()> {
        self.set_at_inner(x, y, atom, face, true)
    }

    pub(crate) fn set_at_untracked(
        &mut self,
        x: i16,
        y: i16,
        atom: Atom,
        face: &Face,
    ) -> Result<()> {
        self.set_at_inner(x, y, atom, face, false)
    }

    fn set_at_inner(
        &mut self,
        x: i16,
        y: i16,
        atom: Atom,
        face: &Face,
        tracked: bool,
    ) -> Result<()> {
        if face == &Face::default() && atom.contents().chars().all(char::is_whitespace) {
            if tracked {
                self.delete_at(x, y);
            } else {
                self.delete_at_untracked(x, y);
            }
            return Ok(());
        }
        let line = self.get(y, x).and_then(|data| data.line.clone());
        let data = CoordData {
            face: Rc::new(face.clone()),
            atom: Rc::new(atom),
            raster_cache: RefCell::new(None),
            line,
        };
        if tracked {
            self.set_data(y, x, data);
        } else {
            self.set_data_untracked(y, x, data);
        }
        Ok(())
    }

    pub fn delete_at(&mut self, x: i16, y: i16) -> bool {
        record_cell_before(self.id, y, x, self.get(y, x));
        self.delete_at_untracked(x, y)
    }

    fn delete_at_untracked(&mut self, x: i16, y: i16) -> bool {
        let mut removed = false;
        let remove_row = self.rows.get_mut(&y).is_some_and(|row| {
            removed = row.remove(&x).is_some();
            row.is_empty()
        });
        if remove_row {
            self.rows.remove(&y);
        }
        removed
    }

    pub(crate) fn insert_cells(
        &mut self,
        line: i16,
        column: i16,
        cells: Vec<(Atom, Face)>,
    ) -> Result<()> {
        if cells.is_empty() {
            return Ok(());
        }
        record_row_before(self, line);
        let y = line;
        let x = column;
        let shift = i16::try_from(cells.len()).context("insert width exceeds signed range")?;
        if let Some(row) = self.rows.remove(&y) {
            let mut shifted = BTreeMap::new();
            for (existing_x, data) in row {
                let target = if existing_x >= x {
                    existing_x
                        .checked_add(shift)
                        .context("insert exceeds signed canvas range")?
                } else {
                    existing_x
                };
                shifted.insert(target, data);
            }
            self.rows.insert(y, shifted);
        }
        for (offset, (atom, face)) in cells.into_iter().enumerate() {
            let target = x
                .checked_add(i16::try_from(offset).context("insert offset exceeds signed range")?)
                .context("insert exceeds signed canvas range")?;
            self.set_at(target, y, atom, &face)?;
        }
        Ok(())
    }

    pub(crate) fn remove_cells(&mut self, line: i16, column: i16, count: usize) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        record_row_before(self, line);
        let y = line;
        let start = column;
        let count_i16 = i16::try_from(count).context("remove width exceeds signed range")?;
        let end = start
            .checked_add(count_i16)
            .context("remove exceeds signed canvas range")?;
        if let Some(row) = self.rows.remove(&y) {
            let mut shifted = BTreeMap::new();
            for (x, data) in row {
                if (start..end).contains(&x) {
                    continue;
                }
                shifted.insert(if x >= end { x - count_i16 } else { x }, data);
            }
            if !shifted.is_empty() {
                self.rows.insert(y, shifted);
            }
        }
        Ok(())
    }

    pub(crate) fn split_row(&mut self, line: i16, column: i16) -> Result<()> {
        record_layer_before(self);
        let y = line;
        let x = column;
        let next_y = y
            .checked_add(1)
            .context("split exceeds signed canvas range")?;
        self.rows = std::mem::take(&mut self.rows)
            .into_iter()
            .map(|(row_y, row)| {
                let shifted_y = if row_y > y {
                    row_y
                        .checked_add(1)
                        .expect("validated canvas rows fit signed range")
                } else {
                    row_y
                };
                (shifted_y, row)
            })
            .collect();
        let mut remainder = BTreeMap::new();
        if let Some(row) = self.rows.get_mut(&y) {
            let moved = row
                .range(x..)
                .map(|(&column, _)| column)
                .collect::<Vec<_>>();
            for column in moved {
                if let Some(data) = row.remove(&column) {
                    remainder.insert(column - x, data);
                }
            }
        }
        if !remainder.is_empty() {
            self.rows.insert(next_y, remainder);
        }
        Ok(())
    }

    pub(crate) fn insert_column(&mut self, column: i16) -> Result<()> {
        record_layer_before(self);
        let x = column;
        for row in self.rows.values_mut() {
            *row = std::mem::take(row)
                .into_iter()
                .map(|(existing_x, data)| {
                    let target = if existing_x >= x {
                        existing_x
                            .checked_add(1)
                            .expect("validated canvas columns fit signed range")
                    } else {
                        existing_x
                    };
                    (target, data)
                })
                .collect();
        }
        Ok(())
    }

    pub(crate) fn insert_row(&mut self, line: i16) -> Result<()> {
        record_layer_before(self);
        let y = line;
        self.rows = std::mem::take(&mut self.rows)
            .into_iter()
            .map(|(existing_y, row)| {
                let target = if existing_y >= y {
                    existing_y
                        .checked_add(1)
                        .expect("validated canvas rows fit signed range")
                } else {
                    existing_y
                };
                (target, row)
            })
            .collect();
        Ok(())
    }

    pub(crate) fn pull_column_left(&mut self, column: i16, affected: &BTreeSet<i16>) -> Result<()> {
        let x = column;
        for &y in affected {
            record_row_before(self, y);
            if let Some(row) = self.rows.remove(&y) {
                let shifted = row
                    .into_iter()
                    .filter_map(|(existing_x, data)| {
                        (existing_x != x).then_some((
                            if existing_x > x {
                                existing_x - 1
                            } else {
                                existing_x
                            },
                            data,
                        ))
                    })
                    .collect::<BTreeMap<_, _>>();
                if !shifted.is_empty() {
                    self.rows.insert(y, shifted);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn pull_column_right(
        &mut self,
        column: i16,
        affected: &BTreeSet<i16>,
    ) -> Result<()> {
        let x = column;
        for &y in affected {
            record_row_before(self, y);
            if let Some(row) = self.rows.remove(&y) {
                let shifted = row
                    .into_iter()
                    .filter_map(|(existing_x, data)| {
                        (existing_x != x).then_some((
                            if existing_x < x {
                                existing_x
                                    .checked_add(1)
                                    .expect("validated columns fit signed range")
                            } else {
                                existing_x
                            },
                            data,
                        ))
                    })
                    .collect::<BTreeMap<_, _>>();
                if !shifted.is_empty() {
                    self.rows.insert(y, shifted);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn remove_row(&mut self, line: i16) -> Result<()> {
        record_layer_before(self);
        let y = line;
        self.rows.remove(&y);
        self.rows = std::mem::take(&mut self.rows)
            .into_iter()
            .map(|(existing_y, row)| {
                (
                    if existing_y > y {
                        existing_y - 1
                    } else {
                        existing_y
                    },
                    row,
                )
            })
            .collect();
        Ok(())
    }

    pub(crate) fn remove_row_and_shift_earlier_rows_down(&mut self, line: i16) -> Result<()> {
        record_layer_before(self);
        let y = line;
        self.rows.remove(&y);
        self.rows = std::mem::take(&mut self.rows)
            .into_iter()
            .map(|(existing_y, row)| {
                let target = if existing_y < y {
                    existing_y
                        .checked_add(1)
                        .expect("validated rows fit signed range")
                } else {
                    existing_y
                };
                (target, row)
            })
            .collect();
        Ok(())
    }

    pub(crate) fn join_row_with_next(&mut self, line: i16) -> Result<bool> {
        let Some(next_y) = line.checked_add(1) else {
            return Ok(false);
        };
        if !self.rows.contains_key(&next_y) {
            return Ok(false);
        }
        record_layer_before(self);
        let y = line;
        let offset =
            i16::try_from(self.row_width(line)).context("row width exceeds signed canvas range")?;
        let next = self.rows.remove(&next_y).unwrap_or_default();
        let row = self.rows.entry(y).or_default();
        for (x, data) in next {
            row.insert(
                x.checked_add(offset)
                    .context("joined row exceeds signed canvas range")?,
                data,
            );
        }
        self.rows = std::mem::take(&mut self.rows)
            .into_iter()
            .map(|(row_y, row)| (if row_y > next_y { row_y - 1 } else { row_y }, row))
            .collect();
        Ok(true)
    }

    pub(crate) fn replace_bounds(
        &mut self,
        bounds: SelectionBounds,
        replacement: Option<(Atom, Face)>,
    ) -> Result<()> {
        for line in bounds.top..=bounds.bottom {
            let y = line;
            for column in bounds.left..=bounds.right {
                let x = column;
                self.delete_at(x, y);
                if let Some((atom, face)) = &replacement {
                    self.set_at(x, y, atom.clone(), face)?;
                }
            }
        }
        Ok(())
    }

    pub fn set_line_data(&mut self, x: i16, y: i16, line: Option<LineData>) -> bool {
        record_cell_before(self.id, y, x, self.get(y, x));
        let Some(data) = self.rows.get_mut(&y).and_then(|row| row.get_mut(&x)) else {
            return false;
        };
        data.line = line;
        true
    }

    pub(crate) fn set_line_data_untracked(
        &mut self,
        x: i16,
        y: i16,
        line: Option<LineData>,
    ) -> bool {
        let Some(data) = self.rows.get_mut(&y).and_then(|row| row.get_mut(&x)) else {
            return false;
        };
        data.line = line;
        true
    }

    fn set_data(&mut self, y: i16, x: i16, data: CoordData) {
        record_cell_before(self.id, y, x, self.get(y, x));
        self.set_data_untracked(y, x, data);
    }

    fn set_data_untracked(&mut self, y: i16, x: i16, data: CoordData) {
        self.rows.entry(y).or_default().insert(x, data);
    }

    fn overlay_nonblank_from(&mut self, source: &Self) {
        for (&line, row) in &source.rows {
            for (&column, source_data) in row {
                if !source_data.atom.contents().chars().all(char::is_whitespace) {
                    self.set_data(line, column, source_data.clone());
                } else if let Some(line_data) = source_data.line.as_ref()
                    && let Some(target_data) = self
                        .rows
                        .get_mut(&line)
                        .and_then(|row| row.get_mut(&column))
                {
                    record_cell_before(self.id, line, column, Some(target_data));
                    target_data.line = Some(line_data.clone());
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerStack {
    layers: Vec<LayerMap>,
    active: usize,
    enabled: bool,
}

impl LayerStack {
    pub fn new(layers: Vec<LayerMap>, enabled: bool) -> Result<Self> {
        Self::with_active(layers, LayerId(0), enabled)
    }

    pub fn with_active(layers: Vec<LayerMap>, active_id: LayerId, enabled: bool) -> Result<Self> {
        if layers.is_empty() {
            bail!("layer stack cannot be empty");
        }
        let active = layers
            .iter()
            .position(|layer| layer.id == active_id)
            .context("active layer is not present in the layer stack")?;
        Ok(Self {
            layers,
            active,
            enabled,
        })
    }

    pub fn layers(&self) -> &[LayerMap] {
        &self.layers
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_id(&self) -> LayerId {
        self.layers[self.active].id
    }

    pub(crate) fn set_at(&mut self, coord: Coord, atom: Atom, face: &Face) -> Result<()> {
        let (line, column) = (coord.line, coord.column);
        self.layers[self.active].set_at(column, line, atom, face)?;
        Ok(())
    }

    pub(crate) fn active_cell(&self, coord: Coord) -> Option<&CoordData> {
        self.layers[self.active].get(coord.line, coord.column)
    }

    pub(crate) fn delete_at(&mut self, coord: Coord) -> bool {
        self.layers[self.active].delete_at(coord.column, coord.line)
    }

    pub(crate) fn insert_cells(
        &mut self,
        line: i16,
        column: i16,
        cells: Vec<(Atom, Face)>,
    ) -> Result<()> {
        self.layers[self.active].insert_cells(line, column, cells)?;
        Ok(())
    }

    pub(crate) fn remove_cells(&mut self, line: i16, column: i16, count: usize) -> Result<()> {
        self.layers[self.active].remove_cells(line, column, count)?;
        Ok(())
    }

    pub(crate) fn split_row(&mut self, line: i16, column: i16) -> Result<()> {
        self.layers[self.active].split_row(line, column)?;
        Ok(())
    }

    pub(crate) fn join_row_with_next(&mut self, line: i16) -> Result<bool> {
        let joined = self.layers[self.active].join_row_with_next(line)?;
        Ok(joined)
    }

    pub(crate) fn replace_active_bounds(
        &mut self,
        bounds: SelectionBounds,
        replacement: Option<(Atom, Face)>,
    ) -> Result<()> {
        self.layers[self.active].replace_bounds(bounds, replacement)?;
        Ok(())
    }

    pub(crate) fn clear_bounds_in_all_layers(&mut self, bounds: SelectionBounds) -> Result<()> {
        for layer in &mut self.layers {
            layer.replace_bounds(bounds, None)?;
        }
        Ok(())
    }

    pub(crate) fn insert_column_in_all_layers(&mut self, column: i16) -> Result<()> {
        for layer in &mut self.layers {
            layer.insert_column(column)?;
        }
        Ok(())
    }

    pub(crate) fn insert_row_in_all_layers(&mut self, line: i16) -> Result<()> {
        for layer in &mut self.layers {
            layer.insert_row(line)?;
        }
        Ok(())
    }

    pub(crate) fn pull_column_left_in_all_layers(
        &mut self,
        column: i16,
        affected: &BTreeSet<i16>,
    ) -> Result<()> {
        for layer in &mut self.layers {
            layer.pull_column_left(column, affected)?;
        }
        Ok(())
    }

    pub(crate) fn pull_column_right_in_all_layers(
        &mut self,
        column: i16,
        affected: &BTreeSet<i16>,
    ) -> Result<()> {
        for layer in &mut self.layers {
            layer.pull_column_right(column, affected)?;
        }
        Ok(())
    }

    pub(crate) fn remove_row_in_all_layers(&mut self, line: i16) -> Result<()> {
        for layer in &mut self.layers {
            layer.remove_row(line)?;
        }
        Ok(())
    }

    pub(crate) fn remove_row_and_shift_earlier_rows_down_in_all_layers(
        &mut self,
        line: i16,
    ) -> Result<()> {
        for layer in &mut self.layers {
            layer.remove_row_and_shift_earlier_rows_down(line)?;
        }
        Ok(())
    }

    pub(crate) fn active_layer_mut(&mut self) -> &mut LayerMap {
        &mut self.layers[self.active]
    }

    pub(crate) fn active_row_width(&self, line: i16) -> usize {
        self.layers[self.active].row_width(line)
    }

    pub(crate) fn active_row_exists(&self, line: i16) -> bool {
        self.layers[self.active].rows().contains_key(&line)
    }

    pub(crate) fn active_line_markers(&self) -> Vec<LineMarker> {
        self.layers[self.active].line_markers()
    }

    pub(crate) fn line_at(&self, coord: Coord) -> Option<&LineData> {
        self.layers[self.active].line_at(coord)
    }

    pub(crate) fn take_line_at(&mut self, coord: Coord) -> Option<LineData> {
        self.layers[self.active].take_line_at(coord)
    }

    pub(crate) fn set_line_at(&mut self, coord: Coord, data: LineData) -> bool {
        self.layers[self.active].set_line_at(coord, data)
    }

    pub(crate) fn remove_line_at(&mut self, coord: Coord) -> bool {
        self.layers[self.active].remove_line_at(coord)
    }

    pub fn summaries(&self) -> Vec<LayerSummary> {
        self.layers
            .iter()
            .enumerate()
            .map(|(index, layer)| LayerSummary {
                id: layer.id,
                visible: layer.visible,
                active: index == self.active,
            })
            .collect()
    }

    pub fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|layer| layer.id == id)
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(crate) fn activate(&mut self, index: usize) -> bool {
        if index >= self.layers.len() || index == self.active {
            return false;
        }
        self.active = index;
        true
    }

    pub(crate) fn add_above(&mut self, index: usize) -> Result<Option<LayerId>> {
        if self.layers.len() >= MAX_LAYERS || index >= self.layers.len() {
            return Ok(None);
        }
        let id = (0..LAYER_SYMBOLS.len())
            .map(|value| LayerId(value as u8))
            .find(|candidate| self.index_of(*candidate).is_none())
            .context("no unused layer id")?;
        let new_index = index + 1;
        self.layers.insert(new_index, LayerMap::new(id, true));
        self.active = new_index;
        Ok(Some(id))
    }

    pub fn toggle_visibility(&mut self, index: usize) -> bool {
        let Some(layer) = self.layers.get_mut(index) else {
            return false;
        };
        layer.visible = !layer.visible;
        true
    }

    pub fn move_up(&mut self, index: usize) -> bool {
        if index == 0 || index + 1 >= self.layers.len() {
            return false;
        }
        self.layers.swap(index, index + 1);
        self.active = match self.active {
            active if active == index => index + 1,
            active if active == index + 1 => index,
            active => active,
        };
        true
    }

    pub fn move_down(&mut self, index: usize) -> bool {
        if index <= 1 || index >= self.layers.len() {
            return false;
        }
        self.layers.swap(index - 1, index);
        self.active = match self.active {
            active if active == index => index - 1,
            active if active == index - 1 => index,
            active => active,
        };
        true
    }

    pub(crate) fn merge_into(&mut self, index: usize, target: usize) -> Result<bool> {
        if index == 0
            || index >= self.layers.len()
            || target >= self.layers.len()
            || index.abs_diff(target) != 1
        {
            return Ok(false);
        }
        record_layer_before(&self.layers[index]);
        let source = self.layers.remove(index);
        let target = target.saturating_sub(usize::from(target > index));
        self.layers[target].overlay_nonblank_from(&source);
        self.active = target;
        Ok(true)
    }

    pub(crate) fn delete(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.layers.len() {
            return false;
        }
        record_layer_before(&self.layers[index]);
        self.layers.remove(index);
        if index == self.active {
            self.active = index - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        true
    }

    pub(crate) fn mutate_layers(&mut self, mut apply: impl FnMut(LayerId, &mut LayerMap)) {
        for layer in &mut self.layers {
            apply(layer.id, layer);
        }
    }

    pub(crate) fn clear_contents(&mut self) {
        for layer in &mut self.layers {
            record_layer_before(layer);
            *layer = LayerMap::new(layer.id, layer.visible);
        }
    }

    pub fn bounds(&self) -> Option<LayerBounds> {
        combined_bounds(&self.layers)
    }

    pub fn effective_layers(&self) -> &[LayerMap] {
        if self.enabled {
            &self.layers
        } else {
            &self.layers[..1]
        }
    }
}

fn combined_bounds(layers: &[LayerMap]) -> Option<LayerBounds> {
    layers
        .iter()
        .filter_map(LayerMap::bounds)
        .reduce(|left, right| LayerBounds {
            min_x: left.min_x.min(right.min_x),
            min_y: left.min_y.min(right.min_y),
            max_x: left.max_x.max(right.max_x),
            max_y: left.max_y.max(right.max_y),
        })
}

#[cfg(test)]
#[path = "inline_tests/canvas_tests.rs"]
mod tests;
