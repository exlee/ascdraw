use std::time::{Duration, Instant};

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

    pub(super) fn promote_if_due(&mut self, now: Instant) -> bool {
        if !self.deadline().is_some_and(|deadline| now >= deadline) {
            return false;
        }
        self.accepted_metrics_generation = self.pending_metrics_generation.take();
        self.last_refresh = Some(now);
        true
    }
}

#[cfg(test)]
#[path = "../inline_tests/raster_cache_tests.rs"]
mod tests;
