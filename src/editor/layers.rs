use crate::model::Atom;

use super::{PlacedLineMarker, blank_atom};

pub const MAX_LAYERS: usize = 6;
pub const LAYER_SYMBOLS: [&str; 10] = ["⍺", "β", "γ", "δ", "ε", "ζ", "η", "θ", "ι", "κ"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(pub u8);

impl LayerId {
    pub fn symbol(self) -> &'static str {
        LAYER_SYMBOLS[usize::from(self.0)]
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerSummary {
    pub id: LayerId,
    pub visible: bool,
    pub active: bool,
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
