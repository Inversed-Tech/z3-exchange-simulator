//! Tracks which lifecycle phase (see [`crate::data_model::Phase`]) a run is
//! currently in, and shares that state with the run's [`crate::rpc::RpcClient`]
//! so every `RpcCall` it records — from any concurrently-running task — is
//! tagged with the phase active at the moment the call was made.
//!
//! [`PhaseTracker`] owns two things: the `AtomicU8` handed to `RpcClient` via
//! [`RpcClient::attach_phase_tracker`](crate::rpc::RpcClient::attach_phase_tracker)
//! (so `mark()` retags calls with no per-call-site plumbing), and an
//! append-only log of phase start times, read at the end of a run into
//! [`crate::metrics::RunManifest::phase_boundaries`].

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;

use crate::data_model::Phase;
use crate::metrics::PhaseBoundary;

pub struct PhaseTracker {
    current: Arc<AtomicU8>,
    log: Arc<Mutex<Vec<PhaseBoundary>>>,
}

impl PhaseTracker {
    /// Starts at `Phase::Bootstrap`, logging that boundary immediately — the
    /// tracker must be constructed (and this initial boundary recorded)
    /// before `Z3Stack::start()` runs, so `phase_boundaries[Bootstrap]`
    /// genuinely predates stack startup rather than being recorded only once
    /// an `RpcClient` happens to exist partway through it.
    pub fn new() -> Self {
        let current = Arc::new(AtomicU8::new(Phase::Bootstrap as u8));
        let log = Arc::new(Mutex::new(vec![PhaseBoundary {
            phase: Phase::Bootstrap,
            started_at: Utc::now(),
        }]));
        Self { current, log }
    }

    /// Advances to `phase`, immediately visible to every `RpcClient` sharing
    /// this tracker's atomic (via [`Self::shared_atomic`]) regardless of
    /// which task next issues a call.
    pub fn mark(&self, phase: Phase) {
        self.current.store(phase as u8, Ordering::Relaxed);
        self.log.lock().unwrap().push(PhaseBoundary {
            phase,
            started_at: Utc::now(),
        });
    }

    /// The shared atomic, handed to an `RpcClient` via
    /// `RpcClient::attach_phase_tracker` so it reads this tracker's current
    /// phase instead of its own default.
    pub fn shared_atomic(&self) -> Arc<AtomicU8> {
        self.current.clone()
    }

    /// Every phase boundary logged so far, in the order `mark()` (and the
    /// initial `Bootstrap` boundary from `new()`) recorded them.
    pub fn boundaries(&self) -> Vec<PhaseBoundary> {
        self.log.lock().unwrap().clone()
    }
}

impl Default for PhaseTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_at_bootstrap_with_one_boundary_logged() {
        let tracker = PhaseTracker::new();
        let boundaries = tracker.boundaries();
        assert_eq!(boundaries.len(), 1);
        assert_eq!(boundaries[0].phase, Phase::Bootstrap);
    }

    #[test]
    fn mark_updates_the_shared_atomic_and_appends_a_boundary() {
        let tracker = PhaseTracker::new();
        let atomic = tracker.shared_atomic();
        assert_eq!(atomic.load(Ordering::Relaxed), Phase::Bootstrap as u8);

        tracker.mark(Phase::Warmup);
        assert_eq!(atomic.load(Ordering::Relaxed), Phase::Warmup as u8);

        let boundaries = tracker.boundaries();
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[1].phase, Phase::Warmup);
    }

    #[test]
    fn boundaries_are_in_mark_order() {
        let tracker = PhaseTracker::new();
        for phase in [
            Phase::Readiness,
            Phase::Warmup,
            Phase::Funding,
            Phase::Load,
            Phase::Drain,
        ] {
            tracker.mark(phase);
        }
        let boundaries = tracker.boundaries();
        let phases: Vec<Phase> = boundaries.iter().map(|b| b.phase).collect();
        assert_eq!(
            phases,
            vec![
                Phase::Bootstrap,
                Phase::Readiness,
                Phase::Warmup,
                Phase::Funding,
                Phase::Load,
                Phase::Drain,
            ]
        );
    }

    #[test]
    fn boundaries_timestamps_are_monotonically_non_decreasing() {
        let tracker = PhaseTracker::new();
        tracker.mark(Phase::Readiness);
        tracker.mark(Phase::Warmup);
        let boundaries = tracker.boundaries();
        for pair in boundaries.windows(2) {
            assert!(pair[1].started_at >= pair[0].started_at);
        }
    }

    #[test]
    fn shared_atomic_clones_observe_the_same_updates() {
        let tracker = PhaseTracker::new();
        let a = tracker.shared_atomic();
        let b = tracker.shared_atomic();
        tracker.mark(Phase::Load);
        assert_eq!(a.load(Ordering::Relaxed), Phase::Load as u8);
        assert_eq!(b.load(Ordering::Relaxed), Phase::Load as u8);
    }
}
