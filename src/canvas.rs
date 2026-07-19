use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::model::{Atom, Face, LayerId};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rasterized {
    pub generation: u64,
    pub scale: u16,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct LineData {
    pub connections: u8,
    pub ending: Option<String>,
    pub base_glyph: Option<String>,
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

    pub fn from_dense(id: LayerId, visible: bool, lines: &[Vec<Atom>]) -> Result<Self> {
        let mut map = Self::new(id, visible);
        for (line_index, row) in lines.iter().enumerate() {
            let line = i16::try_from(line_index).context("canvas line exceeds signed i16 range")?;
            let mut column = 0i16;
            for atom in row {
                for grapheme in UnicodeSegmentation::graphemes(atom.contents.as_str(), true) {
                    if UnicodeWidthStr::width(grapheme) != 1 {
                        bail!("atom {grapheme:?} has display width other than 1");
                    }
                    let cell_atom = Atom {
                        face: atom.face.clone(),
                        contents: grapheme.to_owned(),
                    };
                    map.set_at(column, line, cell_atom, &atom.face)?;
                    column = column
                        .checked_add(1)
                        .context("canvas column exceeds signed i16 range")?;
                }
            }
        }
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
        let Some(max_line) = self.rows.keys().copied().filter(|line| *line >= 0).max() else {
            return vec![Vec::new()];
        };
        let mut lines = vec![Vec::new(); usize::try_from(max_line).unwrap_or(0) + 1];
        for (&line, row) in &self.rows {
            let Ok(line) = usize::try_from(line) else {
                continue;
            };
            let Some(max_column) = row.keys().copied().filter(|column| *column >= 0).max() else {
                continue;
            };
            let mut atoms = vec![default_blank(); usize::try_from(max_column).unwrap_or(0) + 1];
            for (&column, data) in row {
                if let Ok(column) = usize::try_from(column) {
                    atoms[column] = data.atom.as_ref().clone();
                    atoms[column].face = data.face.as_ref().clone();
                }
            }
            while atoms.last().is_some_and(is_default_blank) {
                atoms.pop();
            }
            lines[line] = atoms;
        }
        lines
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerStack {
    layers: Vec<LayerMap>,
    enabled: bool,
    bounds: Option<LayerBounds>,
}

impl LayerStack {
    pub fn new(layers: Vec<LayerMap>, enabled: bool) -> Result<Self> {
        if layers.is_empty() {
            bail!("layer stack cannot be empty");
        }
        let bounds = combined_bounds(&layers);
        Ok(Self {
            layers,
            enabled,
            bounds,
        })
    }

    pub fn layers(&self) -> &[LayerMap] {
        &self.layers
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

fn default_blank() -> Atom {
    Atom {
        face: Face::default(),
        contents: " ".to_owned(),
    }
}

fn is_default_blank(atom: &Atom) -> bool {
    atom.face == Face::default() && atom.contents == " "
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
        *source.raster_cache.borrow_mut() = Some(Rc::new(Rasterized {
            generation: 3,
            scale: 2,
            rgba: vec![0, 1, 2, 3],
        }));

        let cloned = source.clone();

        assert!(cloned.raster_cache.borrow().is_none());
        assert_eq!(cloned, source);
    }
}
