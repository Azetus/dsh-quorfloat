//! Opt-in CPU-pass measurements. No timer, no repaint requests, no per-pass file IO.
use std::collections::VecDeque;

const CAPACITY: usize = 512;
const REPORT_SECONDS: f64 = 5.0;

#[derive(Clone, Copy, Default)]
pub(super) struct StateTiming {
    pub wait_ms: f64,
    pub copy_ms: f64,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Sample {
    pub cpu_ms: f64,
    pub layout_ms: f64,
    pub state: StateTiming,
}

pub(super) struct Performance {
    enabled: bool,
    samples: VecDeque<Sample>,
    gaps: VecDeque<f64>,
    previous: Option<(u64, f64, bool)>,
    report_at: f64,
    passes: usize,
    resizes: usize,
}

impl Performance {
    pub fn new(enabled: bool) -> Self {
        Self { enabled, samples: VecDeque::new(), gaps: VecDeque::new(), previous: None,
            report_at: 0.0, passes: 0, resizes: 0 }
    }
    pub fn enabled(&self) -> bool { self.enabled }
    pub fn resize(&mut self) { if self.enabled { self.resizes += 1; } }

    /// `active` only covers continuous motion (scroll/animation), never idle or token arrival gaps.
    /// CPU samples are per egui pass; cadence samples are per distinct egui frame, not GPU presents.
    pub fn record(&mut self, now: f64, frame: u64, active: bool, sample: Sample) -> Option<String> {
        if !self.enabled { return None; }
        self.passes += 1;
        push(&mut self.samples, sample);
        if let Some((previous_frame, time, was_active)) = self.previous {
            if previous_frame != frame {
                if was_active && active { push(&mut self.gaps, (now - time) * 1000.0); }
                self.previous = Some((frame, now, active));
            }
        } else { self.previous = Some((frame, now, active)); }
        if now - self.report_at < REPORT_SECONDS || self.samples.is_empty() { return None; }
        self.report_at = now;
        let cpu: Vec<_> = self.samples.iter().map(|s| s.cpu_ms).collect();
        let over = cpu.iter().filter(|&&ms| ms > 1000.0 / 60.0).count();
        let report = format!("perf passes={} sampled={} cpu_ms={} layout_ms={} lock_ms={} snapshot_ms={} active_gap_ms={} active_gaps={} over16.7={}/{} resizes={}",
            self.passes, self.samples.len(), percentiles(cpu),
            percentiles(self.samples.iter().map(|s| s.layout_ms).collect()),
            percentiles(self.samples.iter().map(|s| s.state.wait_ms).collect()),
            percentiles(self.samples.iter().map(|s| s.state.copy_ms).collect()),
            percentiles(self.gaps.iter().copied().collect()), self.gaps.len(), over, self.samples.len(), self.resizes);
        self.samples.clear(); self.gaps.clear(); self.passes = 0; self.resizes = 0;
        Some(report)
    }
}

fn push<T>(samples: &mut VecDeque<T>, sample: T) {
    if samples.len() == CAPACITY { samples.pop_front(); }
    samples.push_back(sample);
}

/// p50/p95/p99/max, in milliseconds. Missing cadence samples are unavailable, not zero.
fn percentiles(mut values: Vec<f64>) -> String {
    if values.is_empty() { return "n/a".into(); }
    values.sort_by(f64::total_cmp);
    let at = |p: usize| values[((values.len() * p).div_ceil(100) - 1).min(values.len() - 1)];
    format!("{:.2}/{:.2}/{:.2}/{:.2}", at(50), at(95), at(99), values.last().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_is_bounded_and_reports_cpu_percentiles_without_counting_idle_as_jank() {
        let mut perf = Performance::new(true);
        for i in 0..600 {
            assert!(perf.record(i as f64 / 1000.0, i, true, Sample { cpu_ms: 20.0, ..Default::default() }).is_none());
        }
        assert_eq!(perf.samples.len(), CAPACITY);
        perf.resize();
        let report = perf.record(6.0, 601, false, Sample::default()).unwrap();
        assert!(report.contains("cpu_ms=20.00/20.00/20.00/20.00"), "{report}");
        assert!(report.contains("active_gap_ms=1.00/1.00/1.00/1.00"), "{report}");
        assert!(report.contains("over16.7=511/512 resizes=1"), "{report}");
        assert!(perf.record(6.0, 601, true, Sample::default()).is_none());
        assert_eq!(perf.gaps.len(), 0, "extra egui passes are not additional frames");
        let report = perf.record(12.0, 602, false, Sample::default()).unwrap();
        assert!(report.contains("active_gap_ms=n/a"), "{report}");
        let mut off = Performance::new(false);
        assert!(off.record(100.0, 1, true, Sample::default()).is_none());
        assert!(off.samples.is_empty());
    }
}
