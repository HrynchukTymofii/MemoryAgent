//! Latency instrumentation for stage 1 of the budget.
//!
//! §4 claims hotkey → overlay painted is under 50 ms. That is a contract, so it
//! is measured continuously in the real application rather than benchmarked
//! once — a regression should be visible the moment it is introduced.
//!
//! The measurement spans the hook callback through to the overlay's first
//! animation frame, which means it includes the IPC round-trip. That slightly
//! overstates paint time, and overstating is the right direction to be wrong.

use std::time::Instant;

use parking_lot::Mutex;

#[derive(Default)]
pub struct LatencyTracker {
    pending: Mutex<Option<Instant>>,
    samples: Mutex<Vec<f64>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LatencyReport {
    pub count: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub worst_ms: f64,
    /// Whether p95 is inside the 50 ms ceiling from §4.
    pub within_budget: bool,
}

const CEILING_MS: f64 = 50.0;

impl LatencyTracker {
    /// Called the instant the chord is detected, before the window is shown.
    pub fn begin(&self) {
        *self.pending.lock() = Some(Instant::now());
    }

    /// Called when the overlay reports its first painted frame.
    pub fn complete(&self) -> Option<f64> {
        let started = self.pending.lock().take()?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut s = self.samples.lock();
        s.push(ms);
        // Bounded: this runs for the lifetime of an always-on process.
        if s.len() > 500 {
            s.remove(0);
        }
        Some(ms)
    }

    pub fn report(&self) -> LatencyReport {
        let mut s = self.samples.lock().clone();
        if s.is_empty() {
            return LatencyReport {
                count: 0,
                p50_ms: 0.0,
                p95_ms: 0.0,
                worst_ms: 0.0,
                within_budget: true,
            };
        }
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pick = |q: f64| s[((s.len() as f64 - 1.0) * q).round() as usize];
        let p95 = pick(0.95);
        LatencyReport {
            count: s.len(),
            p50_ms: pick(0.50),
            p95_ms: p95,
            worst_ms: *s.last().unwrap(),
            within_budget: p95 <= CEILING_MS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_empty_before_any_capture() {
        let t = LatencyTracker::default();
        assert_eq!(t.report().count, 0);
        assert!(t.report().within_budget);
    }

    #[test]
    fn complete_without_begin_yields_nothing() {
        // A stray paint report must not fabricate a sample.
        let t = LatencyTracker::default();
        assert!(t.complete().is_none());
    }

    #[test]
    fn records_and_summarises() {
        let t = LatencyTracker::default();
        for _ in 0..5 {
            t.begin();
            assert!(t.complete().is_some());
        }
        assert_eq!(t.report().count, 5);
    }
}
