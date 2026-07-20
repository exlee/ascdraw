use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::model::LayerId;

use super::{CoordData, LayerMap, LayerStack};

thread_local! {
    static HISTORY_CAPTURE: RefCell<Option<HistoryCapture>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLayerState {
    pub id: LayerId,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCanvasState {
    pub layers: Vec<HistoryLayerState>,
    pub active: usize,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCellChange {
    pub layer: LayerId,
    pub line: i16,
    pub column: i16,
    pub before: Option<CoordData>,
    pub after: Option<CoordData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCanvasDelta {
    pub before: HistoryCanvasState,
    pub after: HistoryCanvasState,
    pub cells: Vec<HistoryCellChange>,
}

#[derive(Debug)]
struct HistoryCapture {
    before: HistoryCanvasState,
    cells: BTreeMap<(LayerId, i16, i16), Option<CoordData>>,
}

pub(super) fn record_cell_before(layer: LayerId, line: i16, column: i16, data: Option<&CoordData>) {
    HISTORY_CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        let Some(capture) = capture.as_mut() else {
            return;
        };
        capture
            .cells
            .entry((layer, line, column))
            .or_insert_with(|| data.cloned());
    });
}

pub(super) fn record_layer_before(layer: &LayerMap) {
    for (&line, row) in &layer.rows {
        for (&column, data) in row {
            record_cell_before(layer.id, line, column, Some(data));
        }
    }
}

pub(super) fn record_row_before(layer: &LayerMap, line: i16) {
    let Some(row) = layer.rows.get(&line) else {
        return;
    };
    for (&column, data) in row {
        record_cell_before(layer.id, line, column, Some(data));
    }
}

impl HistoryCanvasDelta {
    pub fn is_empty(&self) -> bool {
        self.before == self.after && self.cells.is_empty()
    }

    pub fn merge(&mut self, next: Self) {
        let mut cells = self
            .cells
            .drain(..)
            .map(|change| ((change.layer, change.line, change.column), change))
            .collect::<BTreeMap<_, _>>();
        for change in next.cells {
            let key = (change.layer, change.line, change.column);
            if let Some(existing) = cells.get_mut(&key) {
                existing.after = change.after;
                if existing.before == existing.after {
                    cells.remove(&key);
                }
            } else if change.before != change.after {
                cells.insert(key, change);
            }
        }
        self.after = next.after;
        self.cells = cells.into_values().collect();
    }
}

impl LayerStack {
    pub fn history_state(&self) -> HistoryCanvasState {
        HistoryCanvasState {
            layers: self
                .layers
                .iter()
                .map(|layer| HistoryLayerState {
                    id: layer.id,
                    visible: layer.visible,
                })
                .collect(),
            active: self.active,
            enabled: self.enabled,
        }
    }

    pub fn begin_history_capture(&self) {
        HISTORY_CAPTURE.with(|capture| {
            let mut capture = capture.borrow_mut();
            assert!(capture.is_none(), "history capture is already active");
            *capture = Some(HistoryCapture {
                before: self.history_state(),
                cells: BTreeMap::new(),
            });
        });
    }

    pub fn cancel_history_capture() {
        HISTORY_CAPTURE.with(|capture| {
            capture.borrow_mut().take();
        });
    }

    pub fn record_history_replacement(&self, replacement: &Self) {
        for layer in &self.layers {
            record_layer_before(layer);
        }
        for layer in &replacement.layers {
            for (&line, row) in &layer.rows {
                for &column in row.keys() {
                    record_cell_before(layer.id, line, column, None);
                }
            }
        }
    }

    pub fn finish_history_capture(&self) -> HistoryCanvasDelta {
        let capture = HISTORY_CAPTURE.with(|capture| {
            capture
                .borrow_mut()
                .take()
                .expect("history capture was not started")
        });
        let mut cells = Vec::with_capacity(capture.cells.len());
        for ((layer, line, column), before) in capture.cells {
            let after = self
                .layers
                .iter()
                .find(|candidate| candidate.id == layer)
                .and_then(|layer| layer.get(line, column))
                .cloned();
            if before != after {
                cells.push(HistoryCellChange {
                    layer,
                    line,
                    column,
                    before,
                    after,
                });
            }
        }
        HistoryCanvasDelta {
            before: capture.before,
            after: self.history_state(),
            cells,
        }
    }

    pub fn apply_history_delta(&mut self, delta: &HistoryCanvasDelta, forward: bool) {
        let state = if forward { &delta.after } else { &delta.before };
        let mut current = std::mem::take(&mut self.layers);
        self.layers = state
            .layers
            .iter()
            .map(|target| {
                if let Some(index) = current.iter().position(|layer| layer.id == target.id) {
                    let mut layer = current.remove(index);
                    layer.visible = target.visible;
                    layer
                } else {
                    LayerMap::new(target.id, target.visible)
                }
            })
            .collect();
        self.active = state.active.min(self.layers.len().saturating_sub(1));
        self.enabled = state.enabled;
        for change in &delta.cells {
            let Some(layer) = self
                .layers
                .iter_mut()
                .find(|layer| layer.id == change.layer)
            else {
                continue;
            };
            let data = if forward {
                change.after.as_ref()
            } else {
                change.before.as_ref()
            };
            if let Some(data) = data {
                layer.set_data(change.line, change.column, data.clone());
            } else {
                layer.delete_at(change.column, change.line);
            }
        }
    }
}
