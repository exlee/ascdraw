use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::drawing::LineEnding;
use crate::model::{Atom, Coord, Face, LAYER_SYMBOLS, LayerId, LayerSummary, MAX_LAYERS};
use crate::selection::{CanvasRegion, SelectionBounds, TextRectangle, overwrite_rectangle};

pub type Raster = Rc<Atom>;

thread_local! {
    static BLANK_ATOM: Rc<Atom> = Rc::new(Atom {
        face: Face::default(),
        contents: " ".to_owned(),
    });
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
    bounds: Option<LayerBounds>,
    dense_widths: Vec<usize>,
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
            bounds: None,
            dense_widths: vec![0],
        }
    }

    pub fn rows(&self) -> &BTreeMap<i16, BTreeMap<i16, CoordData>> {
        &self.rows
    }

    pub fn get(&self, line: i16, column: i16) -> Option<&CoordData> {
        self.rows.get(&line)?.get(&column)
    }

    pub fn at(&self, x: i16, y: i16) -> Rc<Atom> {
        self.get(y, x)
            .map(|data| Rc::clone(&data.atom))
            .unwrap_or_else(blank_atom)
    }

    pub fn bounds(&self) -> Option<LayerBounds> {
        self.bounds
    }

    pub(crate) fn row_width(&self, line: usize) -> usize {
        self.dense_widths.get(line).copied().unwrap_or(0)
    }

    pub(crate) fn ensure_row_width(&mut self, line: usize, width: usize) {
        if self.dense_widths.len() <= line {
            self.dense_widths.resize(line + 1, 0);
        }
        self.dense_widths[line] = self.dense_widths[line].max(width);
    }

    pub(crate) fn line_markers(&self) -> Vec<LineMarker> {
        self.rows
            .iter()
            .flat_map(|(&line, row)| {
                row.iter().filter_map(move |(&column, data)| {
                    let line_data = data.line.as_ref()?;
                    Some(LineMarker {
                        coord: Coord {
                            line: usize::try_from(line).ok()?,
                            column: usize::try_from(column).ok()?,
                        },
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

    pub(crate) fn remap_line_data(&mut self, mut map: impl FnMut(Coord) -> Option<Coord>) {
        let markers = self
            .line_markers()
            .into_iter()
            .filter_map(|marker| map(marker.coord).map(|coord| LineMarker { coord, ..marker }))
            .collect::<Vec<_>>();
        self.replace_line_markers(&markers);
    }

    fn replace_line_markers(&mut self, markers: &[LineMarker]) {
        for row in self.rows.values_mut() {
            for data in row.values_mut() {
                data.line = None;
            }
        }
        for marker in markers {
            let (Ok(line), Ok(column)) = (
                i16::try_from(marker.coord.line),
                i16::try_from(marker.coord.column),
            ) else {
                continue;
            };
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
        atom.validate_cell()?;
        if face == &Face::default() && atom.contents.chars().all(char::is_whitespace) {
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
        let mut atom = data.atom.as_ref().clone();
        atom.face = face.clone();
        data.face = Rc::new(face);
        data.atom = Rc::new(atom);
        *data.raster_cache.borrow_mut() = None;
        true
    }

    pub fn matches_dense(&self, lines: &[Vec<Atom>]) -> bool {
        Self::from_dense(self.id, self.visible, lines).is_ok_and(|dense| dense == *self)
    }

    pub fn atoms_in_region(&self, region: CanvasRegion) -> Vec<Vec<Atom>> {
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
                        self.at(column, line).as_ref().clone()
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
        if removed
            && self.bounds.is_some_and(|bounds| {
                x == bounds.min_x || x == bounds.max_x || y == bounds.min_y || y == bounds.max_y
            })
        {
            self.recalculate_bounds();
        }
        removed
    }

    pub(crate) fn insert_cells(
        &mut self,
        line: usize,
        column: usize,
        cells: Vec<(Atom, Face)>,
    ) -> Result<()> {
        if cells.is_empty() {
            return Ok(());
        }
        let y = i16::try_from(line).context("canvas line exceeds signed range")?;
        let x = i16::try_from(column).context("canvas column exceeds signed range")?;
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
        if self.dense_widths.len() <= line {
            self.dense_widths.resize(line + 1, 0);
        }
        self.dense_widths[line] = self.dense_widths[line].saturating_add(cells.len());
        for (offset, (atom, face)) in cells.into_iter().enumerate() {
            let target = x
                .checked_add(i16::try_from(offset).context("insert offset exceeds signed range")?)
                .context("insert exceeds signed canvas range")?;
            self.set_at(target, y, atom, &face)?;
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn remove_cells(&mut self, line: usize, column: usize, count: usize) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let y = i16::try_from(line).context("canvas line exceeds signed range")?;
        let start = i16::try_from(column).context("canvas column exceeds signed range")?;
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
        if let Some(width) = self.dense_widths.get_mut(line) {
            *width = width.saturating_sub(count);
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn split_row(&mut self, line: usize, column: usize) -> Result<()> {
        let y = i16::try_from(line).context("canvas line exceeds signed range")?;
        let x = i16::try_from(column).context("canvas column exceeds signed range")?;
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
        let width = self.dense_widths.get(line).copied().unwrap_or(0);
        let split = column.min(width);
        if self.dense_widths.len() <= line {
            self.dense_widths.resize(line + 1, 0);
        }
        self.dense_widths[line] = split;
        self.dense_widths.insert(line + 1, width - split);
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn insert_column(&mut self, column: usize, height: usize) -> Result<()> {
        let x = i16::try_from(column).context("column exceeds signed canvas range")?;
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
        self.dense_widths
            .resize(self.dense_widths.len().max(height), 0);
        for width in &mut self.dense_widths {
            *width = (*width).max(column).saturating_add(1);
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn insert_row(&mut self, line: usize) -> Result<()> {
        let y = i16::try_from(line).context("line exceeds signed canvas range")?;
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
        while self.dense_widths.len() < line {
            self.dense_widths.push(0);
        }
        self.dense_widths.insert(line, 0);
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn pull_column_left(&mut self, column: usize, affected: &[bool]) -> Result<()> {
        let x = i16::try_from(column).context("column exceeds signed canvas range")?;
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
            if let Some(width) = self.dense_widths.get_mut(line) {
                *width = width.saturating_sub(1);
            }
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn pull_column_right(&mut self, column: usize, affected: &[bool]) -> Result<()> {
        let x = i16::try_from(column).context("column exceeds signed canvas range")?;
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
            if self.dense_widths.len() <= line {
                self.dense_widths.resize(line + 1, 0);
            }
            if column >= self.dense_widths[line] {
                self.dense_widths[line] = self.dense_widths[line].saturating_add(1);
            }
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn remove_row(&mut self, line: usize) -> Result<()> {
        let y = i16::try_from(line).context("line exceeds signed canvas range")?;
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
        if line < self.dense_widths.len() {
            self.dense_widths.remove(line);
        }
        if self.dense_widths.is_empty() {
            self.dense_widths.push(0);
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn remove_row_and_prepend_blank(&mut self, line: usize) -> Result<()> {
        let y = i16::try_from(line).context("line exceeds signed canvas range")?;
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
        if line < self.dense_widths.len() {
            self.dense_widths.remove(line);
        }
        self.dense_widths.insert(0, 0);
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn join_row_with_next(&mut self, line: usize) -> Result<bool> {
        if line + 1 >= self.dense_widths.len() {
            return Ok(false);
        }
        let y = i16::try_from(line).context("canvas line exceeds signed range")?;
        let next_y = y
            .checked_add(1)
            .context("join exceeds signed canvas range")?;
        let offset = i16::try_from(self.dense_widths[line])
            .context("row width exceeds signed canvas range")?;
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
        let next_width = self.dense_widths.remove(line + 1);
        self.dense_widths[line] = self.dense_widths[line].saturating_add(next_width);
        self.recalculate_bounds();
        Ok(true)
    }

    pub(crate) fn replace_bounds(
        &mut self,
        bounds: SelectionBounds,
        replacement: Option<(Atom, Face)>,
    ) -> Result<()> {
        for line in bounds.top..=bounds.bottom {
            self.ensure_row_width(line, bounds.right.saturating_add(1));
            let y = i16::try_from(line).context("selection line exceeds signed range")?;
            for column in bounds.left..=bounds.right {
                let x = i16::try_from(column).context("selection column exceeds signed range")?;
                self.delete_at(x, y);
                if let Some((atom, face)) = &replacement {
                    self.set_at(x, y, atom.clone(), face)?;
                }
            }
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn overwrite_rectangle(
        &mut self,
        origin: Coord,
        rectangle: &TextRectangle,
    ) -> Result<()> {
        for (row_offset, row) in rectangle.rows.iter().enumerate() {
            let line = origin
                .line
                .checked_add(row_offset)
                .context("rectangle exceeds canvas height")?;
            self.ensure_row_width(line, origin.column.saturating_add(rectangle.width));
            for (column_offset, atom) in row.iter().enumerate() {
                atom.validate_cell()?;
                let column = origin
                    .column
                    .checked_add(column_offset)
                    .context("rectangle exceeds canvas width")?;
                let x = i16::try_from(column).context("rectangle column exceeds signed range")?;
                let y = i16::try_from(line).context("rectangle line exceeds signed range")?;
                self.delete_at(x, y);
                self.set_at(x, y, atom.clone(), &atom.face)?;
            }
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub fn from_dense(id: LayerId, visible: bool, lines: &[Vec<Atom>]) -> Result<Self> {
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
        lines: &[Vec<Atom>],
        line_markers: &[LineMarker],
    ) -> Result<Self> {
        let mut map = Self::new(id, visible);
        map.dense_widths = lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|atom| UnicodeWidthStr::width(atom.contents.as_str()).max(1))
                    .fold(0usize, usize::saturating_add)
            })
            .collect();
        if map.dense_widths.is_empty() {
            map.dense_widths.push(0);
        }
        for (line_index, row) in lines.iter().enumerate() {
            let line = i16::try_from(line_index).context("canvas line exceeds signed i16 range")?;
            let mut column = 0i16;
            for atom in row {
                for grapheme in UnicodeSegmentation::graphemes(atom.contents.as_str(), true) {
                    let width = UnicodeWidthStr::width(grapheme);
                    if width == 0 {
                        bail!("atom {grapheme:?} has display width zero");
                    }
                    let cell_atom = Atom {
                        face: atom.face.clone(),
                        contents: grapheme.to_owned(),
                    };
                    if atom.face != Face::default()
                        || !cell_atom.contents.chars().all(char::is_whitespace)
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

    fn prepend_line(&mut self) {
        self.rows = std::mem::take(&mut self.rows)
            .into_iter()
            .map(|(line, row)| (line.saturating_add(1), row))
            .collect();
        self.dense_widths.insert(0, 0);
        self.recalculate_bounds();
    }

    fn prepend_column(&mut self) {
        for row in self.rows.values_mut() {
            *row = std::mem::take(row)
                .into_iter()
                .map(|(column, data)| (column.saturating_add(1), data))
                .collect();
        }
        for width in &mut self.dense_widths {
            *width = width.saturating_add(1);
        }
        self.recalculate_bounds();
    }

    pub fn set_line_data(&mut self, x: i16, y: i16, line: Option<LineData>) -> bool {
        let Some(data) = self.rows.get_mut(&y).and_then(|row| row.get_mut(&x)) else {
            return false;
        };
        data.line = line;
        true
    }

    fn set_data(&mut self, y: i16, x: i16, data: CoordData) {
        if let (Ok(line), Ok(column)) = (usize::try_from(y), usize::try_from(x)) {
            if self.dense_widths.len() <= line {
                self.dense_widths.resize(line + 1, 0);
            }
            self.dense_widths[line] =
                self.dense_widths[line]
                    .max(column.saturating_add(
                        UnicodeWidthStr::width(data.atom.contents.as_str()).max(1),
                    ));
        }
        self.rows.entry(y).or_default().insert(x, data);
        self.bounds = Some(match self.bounds {
            Some(bounds) => LayerBounds {
                min_x: bounds.min_x.min(x),
                min_y: bounds.min_y.min(y),
                max_x: bounds.max_x.max(x),
                max_y: bounds.max_y.max(y),
            },
            None => LayerBounds {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
            },
        });
    }

    fn recalculate_bounds(&mut self) {
        let Some((&min_y, _)) = self.rows.first_key_value() else {
            self.bounds = None;
            return;
        };
        let max_y = *self.rows.last_key_value().expect("nonempty rows").0;
        let mut columns = self.rows.values().flat_map(|row| row.keys().copied());
        let min_x = columns.next().expect("nonempty rows contain cells");
        let (min_x, max_x) = columns.fold((min_x, min_x), |(min_x, max_x), x| {
            (min_x.min(x), max_x.max(x))
        });
        self.bounds = Some(LayerBounds {
            min_x,
            min_y,
            max_x,
            max_y,
        });
    }

    pub fn to_dense(&self) -> Vec<Vec<Atom>> {
        self.dense_widths
            .iter()
            .enumerate()
            .map(|(line, &width)| {
                let row = i16::try_from(line)
                    .ok()
                    .and_then(|line| self.rows.get(&line));
                let mut atoms = Vec::new();
                let mut column = 0usize;
                while column < width {
                    let data = i16::try_from(column)
                        .ok()
                        .and_then(|column| row.and_then(|row| row.get(&column)));
                    if let Some(data) = data {
                        let mut atom = data.atom.as_ref().clone();
                        atom.face = data.face.as_ref().clone();
                        column = column
                            .saturating_add(UnicodeWidthStr::width(atom.contents.as_str()).max(1));
                        atoms.push(atom);
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
    bounds: Option<LayerBounds>,
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
        let bounds = combined_bounds(&layers);
        Ok(Self {
            layers,
            active,
            enabled,
            bounds,
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
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn active_cell(&self, coord: Coord) -> Option<&CoordData> {
        let (line, column) = coord_i16(coord)?;
        self.layers[self.active].get(line, column)
    }

    pub(crate) fn set_face_at(&mut self, coord: Coord, face: Face) -> bool {
        self.layers[self.active].set_face_at(coord, face)
    }

    pub(crate) fn delete_at(&mut self, coord: Coord) -> bool {
        let Some((line, column)) = coord_i16(coord) else {
            return false;
        };
        let deleted = self.layers[self.active].delete_at(column, line);
        if deleted {
            self.recalculate_bounds();
        }
        deleted
    }

    pub(crate) fn insert_cells(
        &mut self,
        line: usize,
        column: usize,
        cells: Vec<(Atom, Face)>,
    ) -> Result<()> {
        self.layers[self.active].insert_cells(line, column, cells)?;
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn remove_cells(&mut self, line: usize, column: usize, count: usize) -> Result<()> {
        self.layers[self.active].remove_cells(line, column, count)?;
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn split_row(&mut self, line: usize, column: usize) -> Result<()> {
        self.layers[self.active].split_row(line, column)?;
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn join_row_with_next(&mut self, line: usize) -> Result<bool> {
        let joined = self.layers[self.active].join_row_with_next(line)?;
        self.recalculate_bounds();
        Ok(joined)
    }

    pub(crate) fn active_dense_lines(&self) -> Vec<Vec<Atom>> {
        self.layers[self.active].to_dense()
    }

    pub(crate) fn replace_active_bounds(
        &mut self,
        bounds: SelectionBounds,
        replacement: Option<(Atom, Face)>,
    ) -> Result<()> {
        self.layers[self.active].replace_bounds(bounds, replacement)?;
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn clear_bounds_in_all_layers(&mut self, bounds: SelectionBounds) -> Result<()> {
        for layer in &mut self.layers {
            layer.replace_bounds(bounds, None)?;
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn insert_column_in_all_layers(
        &mut self,
        column: usize,
        height: usize,
    ) -> Result<()> {
        for layer in &mut self.layers {
            layer.insert_column(column, height)?;
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn insert_row_in_all_layers(&mut self, line: usize) -> Result<()> {
        for layer in &mut self.layers {
            layer.insert_row(line)?;
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn pull_column_left_in_all_layers(
        &mut self,
        column: usize,
        affected: &[bool],
    ) -> Result<()> {
        for layer in &mut self.layers {
            layer.pull_column_left(column, affected)?;
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn pull_column_right_in_all_layers(
        &mut self,
        column: usize,
        affected: &[bool],
    ) -> Result<()> {
        for layer in &mut self.layers {
            layer.pull_column_right(column, affected)?;
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn remove_row_in_all_layers(&mut self, line: usize) -> Result<()> {
        for layer in &mut self.layers {
            layer.remove_row(line)?;
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn remove_row_and_prepend_blank_in_all_layers(&mut self, line: usize) -> Result<()> {
        for layer in &mut self.layers {
            layer.remove_row_and_prepend_blank(line)?;
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn overwrite_active_rectangle(
        &mut self,
        origin: Coord,
        rectangle: &TextRectangle,
    ) -> Result<()> {
        self.layers[self.active].overwrite_rectangle(origin, rectangle)?;
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn active_row_width(&self, line: usize) -> usize {
        self.layers[self.active].row_width(line)
    }

    pub(crate) fn ensure_active_row_width(&mut self, line: usize, width: usize) {
        self.layers[self.active].ensure_row_width(line, width);
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

    pub(crate) fn remap_active_line_data(&mut self, map: impl FnMut(Coord) -> Option<Coord>) {
        self.layers[self.active].remap_line_data(map);
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

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn has_legacy_wide_atoms(&self) -> bool {
        self.layers.iter().any(|layer| {
            layer.rows.values().any(|row| {
                row.values()
                    .any(|data| UnicodeWidthStr::width(data.atom.contents.as_str()) != 1)
            })
        })
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(crate) fn commit_active(&mut self, lines: &[Vec<Atom>]) -> Result<()> {
        let layer = &self.layers[self.active];
        let markers = layer.line_markers();
        self.commit_active_with_markers(lines, &markers)
    }

    pub(crate) fn commit_active_with_markers(
        &mut self,
        lines: &[Vec<Atom>],
        markers: &[LineMarker],
    ) -> Result<()> {
        let layer = &self.layers[self.active];
        self.layers[self.active] =
            LayerMap::from_dense_with_markers(layer.id, layer.visible, lines, markers)?;
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn activate(
        &mut self,
        index: usize,
        active_lines: &mut Vec<Vec<Atom>>,
    ) -> Result<bool> {
        if index >= self.layers.len() || index == self.active {
            return Ok(false);
        }
        self.commit_active(active_lines)?;
        self.active = index;
        *active_lines = self.layers[index].to_dense();
        Ok(true)
    }

    pub(crate) fn add_above(
        &mut self,
        index: usize,
        active_lines: &mut Vec<Vec<Atom>>,
    ) -> Result<Option<LayerId>> {
        if self.layers.len() >= MAX_LAYERS || index >= self.layers.len() {
            return Ok(None);
        }
        self.commit_active(active_lines)?;
        let id = (0..LAYER_SYMBOLS.len())
            .map(|value| LayerId(value as u8))
            .find(|candidate| self.index_of(*candidate).is_none())
            .context("no unused layer id")?;
        let new_index = index + 1;
        self.layers.insert(new_index, LayerMap::new(id, true));
        self.active = new_index;
        *active_lines = vec![Vec::new()];
        self.recalculate_bounds();
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

    pub(crate) fn merge_into(
        &mut self,
        index: usize,
        target: usize,
        active_lines: &mut Vec<Vec<Atom>>,
    ) -> Result<bool> {
        if index == 0
            || index >= self.layers.len()
            || target >= self.layers.len()
            || index.abs_diff(target) != 1
        {
            return Ok(false);
        }
        self.commit_active(active_lines)?;
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
        *active_lines = target_lines;
        self.recalculate_bounds();
        Ok(true)
    }

    pub(crate) fn delete(
        &mut self,
        index: usize,
        active_lines: &mut Vec<Vec<Atom>>,
    ) -> Result<bool> {
        if index == 0 || index >= self.layers.len() {
            return Ok(false);
        }
        self.commit_active(active_lines)?;
        self.layers.remove(index);
        if index == self.active {
            self.active = index - 1;
            *active_lines = self.layers[self.active].to_dense();
        } else if index < self.active {
            self.active -= 1;
        }
        self.recalculate_bounds();
        Ok(true)
    }

    pub(crate) fn for_each_layer_dense_mut(
        &mut self,
        active_lines: &mut Vec<Vec<Atom>>,
        mut apply: impl FnMut(LayerId, &mut Vec<Vec<Atom>>, &mut Vec<LineMarker>),
    ) -> Result<()> {
        self.commit_active(active_lines)?;
        let active_id = self.active_id();
        for layer in &mut self.layers {
            let mut lines = layer.to_dense();
            let mut markers = layer.line_markers();
            apply(layer.id, &mut lines, &mut markers);
            *layer = LayerMap::from_dense_with_markers(layer.id, layer.visible, &lines, &markers)?;
            if layer.id == active_id {
                *active_lines = lines;
            }
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn for_each_layer_mut(
        &mut self,
        active_lines: &[Vec<Atom>],
        mut apply: impl FnMut(LayerId, &mut LayerMap),
    ) -> Result<()> {
        self.commit_active(active_lines)?;
        for layer in &mut self.layers {
            apply(layer.id, layer);
        }
        self.recalculate_bounds();
        Ok(())
    }

    pub(crate) fn prepend_line_to_inactive(&mut self) {
        for (index, layer) in self.layers.iter_mut().enumerate() {
            if index != self.active {
                layer.prepend_line();
            }
        }
        self.recalculate_bounds();
    }

    pub(crate) fn prepend_column_to_inactive(&mut self) {
        for (index, layer) in self.layers.iter_mut().enumerate() {
            if index != self.active {
                layer.prepend_column();
            }
        }
        self.recalculate_bounds();
    }

    pub(crate) fn clear_contents(&mut self, active_lines: &mut Vec<Vec<Atom>>, cursor: Coord) {
        for layer in &mut self.layers {
            *layer = LayerMap::new(layer.id, layer.visible);
        }
        *active_lines = (0..=cursor.line).map(|_| Vec::new()).collect();
        active_lines[cursor.line] = (0..cursor.column).map(|_| default_blank()).collect();
        self.bounds = None;
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new(vec![LayerMap::new(LayerId(0), true)], self.enabled)
            .expect("the default stack has a base layer");
    }

    pub fn recalculate_bounds(&mut self) {
        self.bounds = combined_bounds(&self.layers);
    }

    pub fn bounds(&self) -> Option<LayerBounds> {
        self.bounds
    }

    pub fn effective_layers(&self) -> &[LayerMap] {
        if self.enabled {
            &self.layers
        } else {
            &self.layers[..1]
        }
    }

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
            .filter(|data| !data.atom.contents.chars().all(char::is_whitespace))
            .map(|data| Rc::clone(&data.atom))
            .next_back()
            .unwrap_or_else(blank_atom)
    }

    pub fn composite_region(&self, region: CanvasRegion) -> Option<Vec<Vec<Atom>>> {
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
                row.push(self.top_at(line, column).as_ref().clone());
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
    Some((
        i16::try_from(coord.line).ok()?,
        i16::try_from(coord.column).ok()?,
    ))
}

fn overlay_nonblank_atoms(
    target: &mut Vec<Vec<Atom>>,
    source: &[Vec<Atom>],
) -> Vec<SelectionBounds> {
    let mut covered = Vec::new();
    for (line, atoms) in source.iter().enumerate() {
        let mut column = 0usize;
        for atom in atoms {
            let width = UnicodeWidthStr::width(atom.contents.as_str()).max(1);
            if atom.contents.chars().all(char::is_whitespace) {
                column = column.saturating_add(width);
                continue;
            }
            let bounds = SelectionBounds {
                left: column,
                right: column.saturating_add(width.saturating_sub(1)),
                top: line,
                bottom: line,
            };
            overwrite_rectangle(
                target,
                Coord { line, column },
                &TextRectangle {
                    rows: vec![vec![atom.clone()]],
                    width,
                },
            );
            covered.push(bounds);
            column = column.saturating_add(width);
        }
    }
    covered
}

fn default_blank() -> Atom {
    Atom {
        face: Face::default(),
        contents: " ".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(contents: &str, face: Face) -> CoordData {
        CoordData {
            atom: Rc::new(Atom {
                face: face.clone(),
                contents: contents.to_owned(),
            }),
            face: Rc::new(face),
            raster_cache: RefCell::new(None),
            line: None,
        }
    }

    #[test]
    fn validates_atoms_and_canonicalizes_default_whitespace() {
        let mut layer = LayerMap::new(LayerId(0), true);
        assert!(
            layer
                .set_at(
                    0,
                    0,
                    data("界", Face::default()).atom.as_ref().clone(),
                    &Face::default()
                )
                .is_err()
        );
        assert!(
            layer
                .set_at(
                    0,
                    0,
                    data("ab", Face::default()).atom.as_ref().clone(),
                    &Face::default()
                )
                .is_err()
        );
        layer
            .set_at(
                0,
                0,
                data(" ", Face::default()).atom.as_ref().clone(),
                &Face::default(),
            )
            .unwrap();
        assert!(layer.rows().is_empty());
    }

    #[test]
    fn top_at_ignores_whitespace_and_disabled_stack_uses_base_only() {
        let mut base = LayerMap::new(LayerId(0), true);
        base.set_data(2, -3, data("a", Face::default()));
        let mut top = LayerMap::new(LayerId(1), true);
        let styled = Face {
            bg: "selection".to_owned(),
            ..Face::default()
        };
        top.set_data(2, -3, data(" ", styled));
        let stack = LayerStack::new(vec![base.clone(), top], true).unwrap();
        assert_eq!(stack.at(2, -3).contents, " ");
        assert_eq!(stack.top_at(2, -3).contents, "a");

        let mut overlay = LayerMap::new(LayerId(1), true);
        overlay.set_data(2, -3, data("b", Face::default()));
        let stack = LayerStack::new(vec![base, overlay], false).unwrap();
        assert_eq!(stack.top_at(2, -3).contents, "a");
    }

    #[test]
    fn bounds_follow_insertions_and_edge_deletions() {
        let mut layer = LayerMap::new(LayerId(0), true);
        layer
            .set_at(
                -4,
                7,
                data("a", Face::default()).atom.as_ref().clone(),
                &Face::default(),
            )
            .unwrap();
        layer
            .set_at(
                9,
                -2,
                data("b", Face::default()).atom.as_ref().clone(),
                &Face::default(),
            )
            .unwrap();
        assert_eq!(
            layer.bounds(),
            Some(LayerBounds {
                min_x: -4,
                min_y: -2,
                max_x: 9,
                max_y: 7,
            })
        );

        assert!(layer.delete_at(9, -2));
        assert_eq!(
            layer.bounds(),
            Some(LayerBounds {
                min_x: -4,
                min_y: 7,
                max_x: -4,
                max_y: 7,
            })
        );
        assert!(layer.delete_at(-4, 7));
        assert_eq!(layer.bounds(), None);
    }

    #[test]
    fn cloning_coordinate_data_drops_raster_cache() {
        let source = data("a", Face::default());
        let image = skia_safe::surfaces::raster_n32_premul((1, 1))
            .unwrap()
            .image_snapshot();
        *source.raster_cache.borrow_mut() = Some(Rc::new(Rasterized {
            generation: 3,
            image,
        }));

        let cloned = source.clone();

        assert!(cloned.raster_cache.borrow().is_none());
        assert_eq!(cloned, source);
    }

    #[test]
    fn line_markers_are_stored_with_their_coordinate_data() {
        let lines = vec![vec![Atom {
            face: Face::default(),
            contents: "◆".to_owned(),
        }]];
        let marker = LineMarker {
            coord: Coord::default(),
            ending: LineEnding::Fixed('◆'),
            base_glyph: "╴".to_owned(),
        };

        let map = LayerMap::from_dense_with_markers(
            LayerId(0),
            true,
            &lines,
            std::slice::from_ref(&marker),
        )
        .unwrap();

        assert_eq!(
            map.get(0, 0).and_then(|data| data.line.as_ref()),
            Some(&LineData {
                ending: marker.ending,
                base_glyph: marker.base_glyph.clone(),
            })
        );
        assert_eq!(map.line_markers(), vec![marker]);
    }
}
