use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::diagnostics::log_info;

const SAMPLE_LIMIT: usize = 1_024;
const REPORT_INTERVAL: usize = 120;

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTiming {
    pub submitted: bool,
    pub buffer_acquisition: Duration,
    pub rasterization: Duration,
    pub presentation: Duration,
    pub toolbar: Duration,
    pub grid: Duration,
    pub minimap: Duration,
}

impl FrameTiming {
    pub fn total(self) -> Duration {
        self.buffer_acquisition + self.rasterization + self.presentation
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct DurationSummary {
    pub samples: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PerfSnapshot {
    pub frames: DurationSummary,
    pub frame_rate: f64,
    pub over_8_33_ms_percent: f64,
    pub over_16_67_ms_percent: f64,
    pub key_handling: DurationSummary,
    pub state_history: DurationSummary,
    pub buffer_acquisition: DurationSummary,
    pub rasterization: DurationSummary,
    pub presentation: DurationSummary,
    #[serde(rename = "event_to_submit")]
    pub event_to_present: DurationSummary,
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
    frames: VecDeque<Duration>,
    frame_times: VecDeque<Instant>,
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
            frames: VecDeque::new(),
            frame_times: VecDeque::new(),
            samples_since_report: 0,
        }
    }

    pub fn enable(&mut self) {
        self.enabled = true;
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
        push_sample(&mut self.frames, timing.total());
        push_sample(&mut self.frame_times, now);
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

    pub fn snapshot(&self) -> PerfSnapshot {
        let frame_rate = match (self.frame_times.front(), self.frame_times.back()) {
            (Some(first), Some(last)) if self.frame_times.len() > 1 => {
                (self.frame_times.len() - 1) as f64
                    / last.saturating_duration_since(*first).as_secs_f64()
            }
            _ => 0.0,
        };
        PerfSnapshot {
            frames: duration_summary(&self.frames),
            frame_rate,
            over_8_33_ms_percent: over_budget_percent(&self.frames, Duration::from_micros(8_333)),
            over_16_67_ms_percent: over_budget_percent(&self.frames, Duration::from_micros(16_667)),
            key_handling: duration_summary(&self.key_handling),
            state_history: duration_summary(&self.state_history),
            buffer_acquisition: duration_summary(&self.buffer_acquisition),
            rasterization: duration_summary(&self.rasterization),
            presentation: duration_summary(&self.presentation),
            event_to_present: duration_summary(&self.event_to_present),
        }
    }

    pub fn reset(&mut self) {
        self.key_handling.clear();
        self.state_history.clear();
        self.buffer_acquisition.clear();
        self.rasterization.clear();
        self.presentation.clear();
        self.event_to_present.clear();
        self.frames.clear();
        self.frame_times.clear();
        self.samples_since_report = 0;
    }
}

fn push_sample<T>(samples: &mut VecDeque<T>, sample: T) {
    if samples.len() == SAMPLE_LIMIT {
        samples.pop_front();
    }
    samples.push_back(sample);
}

fn duration_summary(samples: &VecDeque<Duration>) -> DurationSummary {
    let mut micros = samples.iter().map(Duration::as_micros).collect::<Vec<_>>();
    micros.sort_unstable();
    DurationSummary {
        samples: micros.len(),
        p50_ms: percentile(&micros, 50) as f64 / 1_000.0,
        p95_ms: percentile(&micros, 95) as f64 / 1_000.0,
        p99_ms: percentile(&micros, 99) as f64 / 1_000.0,
    }
}

fn over_budget_percent(samples: &VecDeque<Duration>, budget: Duration) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().filter(|sample| **sample > budget).count() as f64 * 100.0 / samples.len() as f64
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

    #[test]
    fn snapshot_reports_frame_rate_and_budget_misses() {
        let started = Instant::now();
        let mut perf = PerfDiagnostics::new(true);
        perf.record_present(
            FrameTiming {
                rasterization: Duration::from_millis(5),
                ..FrameTiming::default()
            },
            started,
        );
        perf.record_present(
            FrameTiming {
                rasterization: Duration::from_millis(20),
                ..FrameTiming::default()
            },
            started + Duration::from_millis(10),
        );

        let snapshot = perf.snapshot();
        assert_eq!(snapshot.frames.samples, 2);
        assert_eq!(snapshot.frame_rate, 100.0);
        assert_eq!(snapshot.over_8_33_ms_percent, 50.0);
        assert_eq!(snapshot.over_16_67_ms_percent, 50.0);
        assert_eq!(snapshot.frames.p95_ms, 20.0);
    }
}
