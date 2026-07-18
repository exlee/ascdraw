use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::diagnostics::log_info;

const SAMPLE_LIMIT: usize = 1_024;
const REPORT_INTERVAL: usize = 120;

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTiming {
    pub buffer_acquisition: Duration,
    pub rasterization: Duration,
    pub presentation: Duration,
    pub toolbar: Duration,
    pub grid: Duration,
    pub minimap: Duration,
}

#[derive(Clone, Copy, Debug)]
struct PendingKeypress {
    started: Instant,
    handled: Option<Instant>,
    state_history: Duration,
}

#[derive(Debug)]
pub struct PerfDiagnostics {
    enabled: bool,
    pending: Option<PendingKeypress>,
    key_handling: VecDeque<Duration>,
    state_history: VecDeque<Duration>,
    buffer_acquisition: VecDeque<Duration>,
    rasterization: VecDeque<Duration>,
    presentation: VecDeque<Duration>,
    event_to_present: VecDeque<Duration>,
    samples_since_report: usize,
}

impl PerfDiagnostics {
    pub fn from_env() -> Self {
        Self::new(std::env::var_os("ASCDRAW_PERF").is_some_and(|value| value == "1"))
    }

    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            pending: None,
            key_handling: VecDeque::new(),
            state_history: VecDeque::new(),
            buffer_acquisition: VecDeque::new(),
            rasterization: VecDeque::new(),
            presentation: VecDeque::new(),
            event_to_present: VecDeque::new(),
            samples_since_report: 0,
        }
    }

    pub fn begin_keypress(&mut self, now: Instant) {
        if self.enabled {
            self.pending = Some(PendingKeypress {
                started: now,
                handled: None,
                state_history: Duration::ZERO,
            });
        }
    }

    pub fn record_state_history(&mut self, duration: Duration) {
        if let Some(pending) = self.pending.as_mut() {
            pending.state_history += duration;
        }
    }

    pub fn finish_keypress(&mut self, now: Instant) {
        if let Some(pending) = self.pending.as_mut() {
            pending.handled = Some(now);
        }
    }

    pub fn record_present(&mut self, timing: FrameTiming, now: Instant) {
        if !self.enabled {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        let handled = pending.handled.unwrap_or(now);
        push_sample(
            &mut self.key_handling,
            handled.saturating_duration_since(pending.started),
        );
        push_sample(&mut self.state_history, pending.state_history);
        push_sample(&mut self.buffer_acquisition, timing.buffer_acquisition);
        push_sample(&mut self.rasterization, timing.rasterization);
        push_sample(&mut self.presentation, timing.presentation);
        push_sample(
            &mut self.event_to_present,
            now.saturating_duration_since(pending.started),
        );
        self.samples_since_report += 1;
        if self.samples_since_report >= REPORT_INTERVAL {
            self.samples_since_report = 0;
            log_info(format!(
                "ASCDRAW_PERF samples={} key={} state_history={} buffer={} raster={} present={} event_to_present={}",
                self.event_to_present.len(),
                percentiles(&self.key_handling),
                percentiles(&self.state_history),
                percentiles(&self.buffer_acquisition),
                percentiles(&self.rasterization),
                percentiles(&self.presentation),
                percentiles(&self.event_to_present),
            ));
        }
    }
}

fn push_sample(samples: &mut VecDeque<Duration>, sample: Duration) {
    if samples.len() == SAMPLE_LIMIT {
        samples.pop_front();
    }
    samples.push_back(sample);
}

fn percentiles(samples: &VecDeque<Duration>) -> String {
    let mut micros = samples.iter().map(Duration::as_micros).collect::<Vec<_>>();
    micros.sort_unstable();
    format!(
        "p50={:.3}ms,p95={:.3}ms,p99={:.3}ms",
        percentile(&micros, 50) as f64 / 1_000.0,
        percentile(&micros, 95) as f64 / 1_000.0,
        percentile(&micros, 99) as f64 / 1_000.0,
    )
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = sorted
        .len()
        .saturating_mul(percentile)
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_summary_uses_nearest_rank() {
        let samples = (1..=100)
            .map(Duration::from_micros)
            .collect::<VecDeque<_>>();
        assert_eq!(percentiles(&samples), "p50=0.050ms,p95=0.095ms,p99=0.099ms");
    }
}
