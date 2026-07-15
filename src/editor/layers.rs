use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::{Atom, LAYER_SYMBOLS, LayerId, LayerSummary, MAX_LAYERS};

use super::{PlacedLineMarker, blank_atom};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub struct PersistedLayer {
    pub id: LayerId,
    pub visible: bool,
    pub lines: Vec<Vec<Atom>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanvasLayer {
    pub id: LayerId,
    pub visible: bool,
    pub lines: Vec<Vec<Atom>>,
    pub(super) line_markers: Vec<PlacedLineMarker>,
}

impl CanvasLayer {
    fn placeholder(id: LayerId) -> Self {
        Self {
            id,
            visible: true,
            lines: vec![Vec::new()],
            line_markers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LayerView<'a> {
    pub id: LayerId,
    pub visible: bool,
    pub lines: &'a [Vec<Atom>],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerStack {
    layers: Vec<CanvasLayer>,
    active: usize,
}

impl Default for LayerStack {
    fn default() -> Self {
        Self {
            layers: vec![CanvasLayer::placeholder(LayerId(0))],
            active: 0,
        }
    }
}

impl LayerStack {
    pub(super) fn from_persisted(
        persisted: Vec<PersistedLayer>,
        active_id: LayerId,
    ) -> Result<(Self, Vec<Vec<Atom>>)> {
        if persisted.is_empty() || persisted.len() > MAX_LAYERS {
            bail!("project must contain between 1 and {MAX_LAYERS} layers");
        }
        if persisted[0].id != LayerId(0) {
            bail!("the base layer must be first");
        }
        let mut seen = std::collections::HashSet::new();
        for layer in &persisted {
            if !layer.id.is_valid() {
                bail!(
                    "layer symbol id {} is outside the supported pool",
                    layer.id.0
                );
            }
            if !seen.insert(layer.id) {
                bail!("layer symbol {} is duplicated", layer.id.symbol());
            }
        }
        let active = persisted
            .iter()
            .position(|layer| layer.id == active_id)
            .ok_or_else(|| anyhow::anyhow!("active layer is not present in the layer stack"))?;
        let mut layers = persisted
            .into_iter()
            .map(|layer| CanvasLayer {
                id: layer.id,
                visible: layer.visible,
                lines: if layer.lines.is_empty() {
                    vec![Vec::new()]
                } else {
                    layer.lines
                },
                line_markers: Vec::new(),
            })
            .collect::<Vec<_>>();
        let active_lines = std::mem::replace(&mut layers[active].lines, vec![Vec::new()]);
        Ok((Self { layers, active }, active_lines))
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_id(&self) -> LayerId {
        self.layers[self.active].id
    }

    pub fn layers(&self) -> &[CanvasLayer] {
        &self.layers
    }

    pub(super) fn summaries(&self) -> Vec<LayerSummary> {
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

    pub(super) fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|layer| layer.id == id)
    }

    pub(super) fn activate(
        &mut self,
        index: usize,
        active_lines: &mut Vec<Vec<Atom>>,
        active_markers: &mut Vec<PlacedLineMarker>,
    ) -> bool {
        if index >= self.layers.len() || index == self.active {
            return false;
        }
        std::mem::swap(active_lines, &mut self.layers[self.active].lines);
        std::mem::swap(active_markers, &mut self.layers[self.active].line_markers);
        std::mem::swap(active_lines, &mut self.layers[index].lines);
        std::mem::swap(active_markers, &mut self.layers[index].line_markers);
        self.active = index;
        true
    }

    pub(super) fn add_above(
        &mut self,
        index: usize,
        active_lines: &mut Vec<Vec<Atom>>,
        active_markers: &mut Vec<PlacedLineMarker>,
    ) -> Option<LayerId> {
        if self.layers.len() >= MAX_LAYERS || index >= self.layers.len() {
            return None;
        }
        let id = (0..LAYER_SYMBOLS.len())
            .map(|value| LayerId(value as u8))
            .find(|candidate| self.index_of(*candidate).is_none())?;
        let new_index = index + 1;
        self.layers.insert(new_index, CanvasLayer::placeholder(id));
        if new_index <= self.active {
            self.active += 1;
        }
        self.activate(new_index, active_lines, active_markers);
        Some(id)
    }

    pub(super) fn toggle_visibility(&mut self, index: usize) -> bool {
        let Some(layer) = self.layers.get_mut(index) else {
            return false;
        };
        layer.visible = !layer.visible;
        true
    }

    pub(super) fn move_up(&mut self, index: usize) -> bool {
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

    pub(super) fn move_down(&mut self, index: usize) -> bool {
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

    pub(super) fn delete(
        &mut self,
        index: usize,
        active_lines: &mut Vec<Vec<Atom>>,
        active_markers: &mut Vec<PlacedLineMarker>,
    ) -> bool {
        if index == 0 || index >= self.layers.len() {
            return false;
        }
        if index == self.active {
            self.layers.remove(index);
            self.active = index - 1;
            *active_lines = vec![Vec::new()];
            active_markers.clear();
            std::mem::swap(active_lines, &mut self.layers[self.active].lines);
            std::mem::swap(active_markers, &mut self.layers[self.active].line_markers);
        } else {
            self.layers.remove(index);
            if index < self.active {
                self.active -= 1;
            }
        }
        true
    }

    pub(super) fn prepend_line_to_inactive(&mut self) {
        for (index, layer) in self.layers.iter_mut().enumerate() {
            if index == self.active {
                continue;
            }
            layer.lines.insert(0, Vec::new());
            for marker in &mut layer.line_markers {
                marker.coord.line = marker.coord.line.saturating_add(1);
            }
        }
    }

    pub(super) fn prepend_column_to_inactive(&mut self) {
        for (index, layer) in self.layers.iter_mut().enumerate() {
            if index == self.active {
                continue;
            }
            for line in &mut layer.lines {
                line.insert(0, blank_atom());
            }
            for marker in &mut layer.line_markers {
                marker.coord.column = marker.coord.column.saturating_add(1);
            }
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
}
