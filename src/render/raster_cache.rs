use std::time::{Duration, Instant};

use crate::model::Coord;

const REFRESH_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Default)]
pub(super) struct RasterRefreshThrottle {
    style_generation: Option<u64>,
    accepted_metrics_generation: Option<u64>,
    pending_metrics_generation: Option<u64>,
    last_refresh: Option<Instant>,
}

impl RasterRefreshThrottle {
    pub(super) fn use_current_metrics(
        &mut self,
        style_generation: u64,
        metrics_generation: u64,
        now: Instant,
    ) -> bool {
        if self.style_generation != Some(style_generation) {
            self.style_generation = Some(style_generation);
            self.accepted_metrics_generation = Some(metrics_generation);
            self.pending_metrics_generation = None;
            self.last_refresh = Some(now);
            return true;
        }
        if self.accepted_metrics_generation == Some(metrics_generation) {
            self.pending_metrics_generation = None;
            return true;
        }
        if self
            .last_refresh
            .is_none_or(|last| now.saturating_duration_since(last) >= REFRESH_INTERVAL)
        {
            self.accepted_metrics_generation = Some(metrics_generation);
            self.pending_metrics_generation = None;
            self.last_refresh = Some(now);
            return true;
        }
        self.pending_metrics_generation = Some(metrics_generation);
        false
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.pending_metrics_generation
            .and_then(|_| self.last_refresh.map(|last| last + REFRESH_INTERVAL))
    }

    pub(super) fn accepts(&self, style_generation: u64, metrics_generation: u64) -> bool {
        self.style_generation == Some(style_generation)
            && self.accepted_metrics_generation == Some(metrics_generation)
    }

    pub(super) fn promote_if_due(&mut self, now: Instant) -> bool {
        if self.deadline().is_none_or(|deadline| now < deadline) {
            return false;
        }
        self.accepted_metrics_generation = self.pending_metrics_generation.take();
        self.last_refresh = Some(now);
        true
    }
}

#[derive(Clone, Default)]
pub(super) struct RasterPrefillCursor {
    generation: Option<u64>,
    layer_index: usize,
    last_coord: Option<Coord>,
    complete: bool,
}

impl RasterPrefillCursor {
    pub(super) fn prepare(&mut self, generation: u64) {
        if self.generation == Some(generation) {
            return;
        }
        self.generation = Some(generation);
        self.layer_index = 0;
        self.last_coord = None;
        self.complete = false;
    }

    pub(super) fn position(&self) -> Option<(usize, Option<Coord>)> {
        (!self.complete).then_some((self.layer_index, self.last_coord))
    }

    pub(super) fn advance(&mut self, coord: Coord) {
        self.last_coord = Some(coord);
    }

    pub(super) fn advance_layer(&mut self) {
        self.layer_index = self.layer_index.saturating_add(1);
        self.last_coord = None;
    }

    pub(super) fn finish(&mut self) {
        self.complete = true;
    }

    pub(super) fn is_pending(&self) -> bool {
        !self.complete
    }
}

#[cfg(test)]
#[path = "../inline_tests/raster_cache_tests.rs"]
mod tests;
