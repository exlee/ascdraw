use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::drawing::LineEnding;
use crate::model::{
    Atom, Coord, Face, LAYER_SYMBOLS, LayerId, LayerSummary, MAX_LAYERS, StyledAtom,
};
use crate::selection::{CanvasRegion, SelectionBounds, TextRectangle};

mod composition;
use composition::overlay_nonblank_atoms;

#[allow(dead_code)]
pub type Raster = Rc<Atom>;

thread_local! {
    static BLANK_ATOM: Rc<Atom> = Rc::new(Atom::new(" ").expect("blank is one cell"));
}

fn blank_atom() -> Rc<Atom> {
    BLANK_ATOM.with(Rc::clone)
}

#[derive(Debug, Clone)]
pub struct Rasterized {
    pub generation: u64,
    pub image: skia_safe::Image,
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

    pub(crate) fn selected_atoms(&self, bounds: SelectionBounds) -> Vec<Vec<StyledAtom>> {
        (bounds.top..=bounds.bottom)
            .map(|line| {
                (bounds.left..=bounds.right)
                    .map(|column| {
                        self.get(line, column)
                            .map_or_else(default_blank, |data| StyledAtom {
                                face: data.face.as_ref().clone(),
                                contents: data.atom.contents().to_owned(),
                            })
                    })
                    .collect()
            })
            .collect()
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
        let (line, column) = coord_i16(coord)?;
        self.get(line, column)?.line.as_ref()
    }

    pub(crate) fn take_line_at(&mut self, coord: Coord) -> Option<LineData> {
        let (line, column) = coord_i16(coord)?;
        self.rows.get_mut(&line)?.get_mut(&column)?.line.take()
    }

    pub(crate) fn set_line_at(&mut self, coord: Coord, line_data: LineData) -> bool {
        let Some((line, column)) = coord_i16(coord) else {
            return false;
        };
        let Some(data) = self
            .rows
            .get_mut(&line)
            .and_then(|row| row.get_mut(&column))
        else {
            return false;
        };
        data.line = Some(line_data);
        true
    }

    pub(crate) fn remove_line_at(&mut self, coord: Coord) -> bool {
        self.take_line_at(coord).is_some()
    }

    fn replace_line_markers(&mut self, markers: &[LineMarker]) {
        for row in self.rows.values_mut() {
            for data in row.values_mut() {
                data.line = None;
            }
        }
        for marker in markers {
            let (line, column) = (marker.coord.line, marker.coord.column);
            if let Some(data) = self
                .rows
                .get_mut(&line)
                .and_then(|row| row.get_mut(&column))
            {
                data.line = Some(LineData {
                    ending: marker.ending,
                    base_glyph: marker.base_glyph.clone(),
                });
            }
        }
    }

    pub fn set_at(&mut self, x: i16, y: i16, atom: Atom, face: &Face) -> Result<()> {
        if face == &Face::default() && atom.contents().chars().all(char::is_whitespace) {
            self.delete_at(x, y);
            return Ok(());
        }
        let line = self.get(y, x).and_then(|data| data.line.clone());
        self.set_data(
            y,
            x,
            CoordData {
                face: Rc::new(face.clone()),
                atom: Rc::new(atom),
                raster_cache: RefCell::new(None),
                line,
            },
        );
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_face_at(&mut self, coord: Coord, face: Face) -> bool {
        let Some((line, column)) = coord_i16(coord) else {
            return false;
        };
        let Some(data) = self
            .rows
            .get_mut(&line)
            .and_then(|row| row.get_mut(&column))
        else {
            return false;
        };
        data.face = Rc::new(face);
        *data.raster_cache.borrow_mut() = None;
        true
    }

    pub fn atoms_in_region(&self, region: CanvasRegion) -> Vec<Vec<StyledAtom>> {
        (0..region.height)
            .map(|row_offset| {
                let line = region
                    .top
                    .saturating_add(i64::try_from(row_offset).unwrap_or(i64::MAX));
                (0..region.width)
                    .map(|column_offset| {
                        let column = region
                            .left
                            .saturating_add(i64::try_from(column_offset).unwrap_or(i64::MAX));
                        let (Ok(line), Ok(column)) = (i16::try_from(line), i16::try_from(column))
                        else {
                            return default_blank();
                        };
                        self.get(line, column)
                            .map_or_else(default_blank, |data| StyledAtom {
                                face: data.face.as_ref().clone(),
                                contents: data.atom.contents().to_owned(),
                            })
                    })
                    .collect()
            })
            .collect()
    }

    pub fn delete_at(&mut self, x: i16, y: i16) -> bool {
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

    pub(crate) fn insert_column(&mut self, column: i16, height: usize) -> Result<()> {
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
        let _ = height;
        Ok(())
    }

    pub(crate) fn insert_row(&mut self, line: i16) -> Result<()> {
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

    pub(crate) fn pull_column_left(&mut self, column: i16, affected: &[bool]) -> Result<()> {
        let x = column;
        for (line, is_affected) in affected.iter().copied().enumerate() {
            if !is_affected {
                continue;
            }
            let y = i16::try_from(line).context("line exceeds signed canvas range")?;
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

    pub(crate) fn pull_column_right(&mut self, column: i16, affected: &[bool]) -> Result<()> {
        let x = column;
        for (line, is_affected) in affected.iter().copied().enumerate() {
            if !is_affected {
                continue;
            }
            let y = i16::try_from(line).context("line exceeds signed canvas range")?;
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

    pub(crate) fn remove_row_and_prepend_blank(&mut self, line: i16) -> Result<()> {
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

    pub(crate) fn overwrite_rectangle(
        &mut self,
        origin: Coord,
        rectangle: &TextRectangle,
    ) -> Result<()> {
        for (row_offset, row) in rectangle.rows.iter().enumerate() {
            let row_offset =
                i16::try_from(row_offset).context("rectangle exceeds canvas height")?;
            let line = origin
                .line
                .checked_add(row_offset)
                .context("rectangle exceeds canvas height")?;
            for (column_offset, atom) in row.iter().enumerate() {
                atom.validate_cell()?;
                let column_offset =
                    i16::try_from(column_offset).context("rectangle exceeds canvas width")?;
                let column = origin
                    .column
                    .checked_add(column_offset)
                    .context("rectangle exceeds canvas width")?;
                self.delete_at(column, line);
                self.set_at(column, line, Atom::new(atom.contents.clone())?, &atom.face)?;
            }
        }
        Ok(())
    }

    pub fn from_dense(id: LayerId, visible: bool, lines: &[Vec<StyledAtom>]) -> Result<Self> {
        for atom in lines.iter().flatten() {
            for grapheme in UnicodeSegmentation::graphemes(atom.contents.as_str(), true) {
                if UnicodeWidthStr::width(grapheme) != 1 {
                    bail!("atom {grapheme:?} has display width other than 1");
                }
            }
        }
        Self::from_dense_with_markers(id, visible, lines, &[])
    }

    pub(crate) fn from_dense_with_markers(
        id: LayerId,
        visible: bool,
        lines: &[Vec<StyledAtom>],
        line_markers: &[LineMarker],
    ) -> Result<Self> {
        let mut map = Self::new(id, visible);
        for (line_index, row) in lines.iter().enumerate() {
            let line = i16::try_from(line_index).context("canvas line exceeds signed i16 range")?;
            let mut column = 0i16;
            for atom in row {
                for grapheme in UnicodeSegmentation::graphemes(atom.contents.as_str(), true) {
                    let width = UnicodeWidthStr::width(grapheme);
                    if width == 0 {
                        bail!("atom {grapheme:?} has display width zero");
                    }
                    let cell_atom = Atom::new(grapheme)?;
                    if atom.face != Face::default()
                        || !cell_atom.contents().chars().all(char::is_whitespace)
                    {
                        map.set_data(
                            line,
                            column,
                            CoordData {
                                face: Rc::new(atom.face.clone()),
                                atom: Rc::new(cell_atom),
                                raster_cache: RefCell::new(None),
                                line: None,
                            },
                        );
                    }
                    column = column
                        .checked_add(i16::try_from(width).context("atom width exceeds i16")?)
                        .context("canvas column exceeds signed i16 range")?;
                }
            }
        }
        map.replace_line_markers(line_markers);
        Ok(map)
    }

    pub fn set_line_data(&mut self, x: i16, y: i16, line: Option<LineData>) -> bool {
        let Some(data) = self.rows.get_mut(&y).and_then(|row| row.get_mut(&x)) else {
            return false;
        };
        data.line = line;
        true
    }

    fn set_data(&mut self, y: i16, x: i16, data: CoordData) {
        self.rows.entry(y).or_default().insert(x, data);
    }

    pub fn to_dense(&self) -> Vec<Vec<StyledAtom>> {
        let height = self
            .rows
            .last_key_value()
            .and_then(|(&line, _)| usize::try_from(line).ok())
            .map_or(1, |line| line.saturating_add(1));
        (0..height)
            .map(|line| {
                let row = i16::try_from(line)
                    .ok()
                    .and_then(|line| self.rows.get(&line));
                let width = row.map_or(0, |row| {
                    row.iter()
                        .filter_map(|(&column, data)| {
                            usize::try_from(column).ok().map(|column| {
                                column.saturating_add(
                                    UnicodeWidthStr::width(data.atom.contents()).max(1),
                                )
                            })
                        })
                        .max()
                        .unwrap_or(0)
                });
                let mut atoms = Vec::new();
                let mut column = 0usize;
                while column < width {
                    let data = i16::try_from(column)
                        .ok()
                        .and_then(|column| row.and_then(|row| row.get(&column)));
                    if let Some(data) = data {
                        column = column
                            .saturating_add(UnicodeWidthStr::width(data.atom.contents()).max(1));
                        atoms.push(StyledAtom {
                            face: data.face.as_ref().clone(),
                            contents: data.atom.contents().to_owned(),
                        });
                    } else {
                        atoms.push(default_blank());
                        column += 1;
                    }
                }
                atoms
            })
            .collect()
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
        let (line, column) = coord_i16(coord).context("canvas coordinate exceeds signed range")?;
        self.layers[self.active].set_at(column, line, atom, face)?;
        Ok(())
    }

    pub(crate) fn active_cell(&self, coord: Coord) -> Option<&CoordData> {
        let (line, column) = coord_i16(coord)?;
        self.layers[self.active].get(line, column)
    }

    #[cfg(test)]
    pub(crate) fn set_face_at(&mut self, coord: Coord, face: Face) -> bool {
        self.layers[self.active].set_face_at(coord, face)
    }

    pub(crate) fn delete_at(&mut self, coord: Coord) -> bool {
        let Some((line, column)) = coord_i16(coord) else {
            return false;
        };
        self.layers[self.active].delete_at(column, line)
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

    #[cfg(test)]
    pub(crate) fn active_dense_lines(&self) -> Vec<Vec<StyledAtom>> {
        self.layers[self.active].to_dense()
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

    pub(crate) fn insert_column_in_all_layers(&mut self, column: i16, height: usize) -> Result<()> {
        for layer in &mut self.layers {
            layer.insert_column(column, height)?;
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
        affected: &[bool],
    ) -> Result<()> {
        for layer in &mut self.layers {
            layer.pull_column_left(column, affected)?;
        }
        Ok(())
    }

    pub(crate) fn pull_column_right_in_all_layers(
        &mut self,
        column: i16,
        affected: &[bool],
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

    pub(crate) fn remove_row_and_prepend_blank_in_all_layers(&mut self, line: i16) -> Result<()> {
        for layer in &mut self.layers {
            layer.remove_row_and_prepend_blank(line)?;
        }
        Ok(())
    }

    pub(crate) fn overwrite_active_rectangle(
        &mut self,
        origin: Coord,
        rectangle: &TextRectangle,
    ) -> Result<()> {
        self.layers[self.active].overwrite_rectangle(origin, rectangle)?;
        Ok(())
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

    #[cfg(test)]
    pub(crate) fn commit_active_with_markers(
        &mut self,
        lines: &[Vec<StyledAtom>],
        markers: &[LineMarker],
    ) -> Result<()> {
        let layer = &self.layers[self.active];
        self.layers[self.active] =
            LayerMap::from_dense_with_markers(layer.id, layer.visible, lines, markers)?;
        Ok(())
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
        let source = self.layers.remove(index);
        let target = target.saturating_sub(usize::from(target > index));
        let target_layer = &mut self.layers[target];
        let mut target_lines = target_layer.to_dense();
        let source_lines = source.to_dense();
        let covered = overlay_nonblank_atoms(&mut target_lines, &source_lines);
        let mut markers = target_layer.line_markers();
        markers.retain(|marker| !covered.iter().any(|bounds| bounds.contains(marker.coord)));
        for marker in source.line_markers() {
            markers.retain(|existing| existing.coord != marker.coord);
            markers.push(marker);
        }
        let id = target_layer.id;
        let visible = target_layer.visible;
        self.layers[target] =
            LayerMap::from_dense_with_markers(id, visible, &target_lines, &markers)?;
        self.active = target;
        Ok(true)
    }

    pub(crate) fn delete(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.layers.len() {
            return false;
        }
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

    #[allow(dead_code)]
    pub fn at(&self, line: i16, column: i16) -> Raster {
        let layers = if self.enabled {
            self.layers.as_slice()
        } else {
            &self.layers[..1]
        };
        layers
            .iter()
            .filter(|layer| layer.visible)
            .filter_map(|layer| layer.get(line, column))
            .map(|data| Rc::clone(&data.atom))
            .next_back()
            .unwrap_or_else(blank_atom)
    }

    #[allow(dead_code)]
    pub fn top_at(&self, line: i16, column: i16) -> Raster {
        let layers = if self.enabled {
            self.layers.as_slice()
        } else {
            &self.layers[..1]
        };
        layers
            .iter()
            .filter(|layer| layer.visible)
            .filter_map(|layer| layer.get(line, column))
            .filter(|data| !data.atom.contents().chars().all(char::is_whitespace))
            .map(|data| Rc::clone(&data.atom))
            .next_back()
            .unwrap_or_else(blank_atom)
    }

    pub fn composite_region(&self, region: CanvasRegion) -> Option<Vec<Vec<StyledAtom>>> {
        let left = i16::try_from(region.left).ok()?;
        let top = i16::try_from(region.top).ok()?;
        let width = i16::try_from(region.width).ok()?;
        let height = i16::try_from(region.height).ok()?;
        let mut rows = Vec::with_capacity(region.height);
        for line_offset in 0..height {
            let line = top.checked_add(line_offset)?;
            let mut row = Vec::with_capacity(region.width);
            for column_offset in 0..width {
                let column = left.checked_add(column_offset)?;
                let atom = self
                    .effective_layers()
                    .iter()
                    .filter(|layer| layer.visible)
                    .filter_map(|layer| layer.get(line, column))
                    .filter(|data| !data.atom.contents().chars().all(char::is_whitespace))
                    .next_back()
                    .map_or_else(default_blank, |data| StyledAtom {
                        face: data.face.as_ref().clone(),
                        contents: data.atom.contents().to_owned(),
                    });
                row.push(atom);
            }
            rows.push(row);
        }
        Some(rows)
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

fn coord_i16(coord: Coord) -> Option<(i16, i16)> {
    Some((coord.line, coord.column))
}

fn default_blank() -> StyledAtom {
    StyledAtom {
        face: Face::default(),
        contents: " ".to_owned(),
    }
}

#[cfg(test)]
#[path = "inline_tests/canvas_tests.rs"]
mod tests;
