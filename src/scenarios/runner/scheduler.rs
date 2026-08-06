//! Load shape and TPS scheduling for the scenario runner.

use std::time::Duration;

use crate::data_model::FlowConfig;

// ── LoadShape ─────────────────────────────────────────────────────────────────

/// How the load profile changes over time during the run.
#[derive(Debug, Clone)]
pub enum LoadShape {
    /// Constant TPS for the full duration.
    SteadyState,
    /// TPS ramps linearly from 0 to `target_tps` over `ramp_secs`, then stays constant.
    Ramp { ramp_secs: u64 },
    /// Constant TPS for `pre_burst_secs`, then a spike for `burst_secs`, then back to base.
    Burst {
        pre_burst_secs: u64,
        burst_secs: u64,
        spike_multiplier: f64,
    },
    /// 50/50 shielded/transparent mix at constant TPS (overrides per-scenario flow config).
    Mixed,
}

// ── Scheduler ─────────────────────────────────────────────────────────────────

pub struct Scheduler {
    pub shape: LoadShape,
    pub target_tps: f64,
}

impl Scheduler {
    pub fn new(shape: LoadShape, target_tps: f64) -> Self {
        Self { shape, target_tps }
    }

    /// Compute the instantaneous TPS at `elapsed` time into the run.
    pub fn instantaneous_tps(&self, elapsed: Duration) -> f64 {
        match &self.shape {
            LoadShape::SteadyState | LoadShape::Mixed => self.target_tps,
            LoadShape::Ramp { ramp_secs } => {
                let ramp = *ramp_secs as f64;
                let secs = elapsed.as_secs_f64();
                if secs >= ramp || ramp == 0.0 {
                    self.target_tps
                } else {
                    self.target_tps * (secs / ramp)
                }
            }
            LoadShape::Burst {
                pre_burst_secs,
                burst_secs,
                spike_multiplier,
            } => {
                let secs = elapsed.as_secs_f64();
                let pre = *pre_burst_secs as f64;
                let burst_end = pre + *burst_secs as f64;
                if secs >= pre && secs < burst_end {
                    self.target_tps * spike_multiplier
                } else {
                    self.target_tps
                }
            }
        }
    }

    /// TPS to use for the very first interval (avoids zero-delay at start for Ramp).
    pub fn initial_tps(&self) -> f64 {
        match &self.shape {
            LoadShape::Ramp { .. } => (self.target_tps * 0.1).max(0.001),
            _ => self.instantaneous_tps(Duration::ZERO).max(0.001),
        }
    }

    /// The interval the dispatch loop should apply via `ticker.reset_after`
    /// for its next tick, or `None` if it should leave the ticker alone.
    ///
    /// Returns `None` for the very first tick: the caller primes the ticker
    /// with [`initial_tps`](Self::initial_tps)'s interval *before* the loop
    /// starts (via `interval_at`), specifically to avoid a zero-delay start
    /// for `Ramp`. `elapsed` is ~0 at that point, and recomputing from it
    /// would immediately discard that priming with the same near-zero
    /// result `initial_tps` exists to avoid — for `Ramp`,
    /// `instantaneous_tps(~0)` is itself ~0, floored to the 0.001 TPS
    /// minimum, producing a fixed ~1000s period before the very first
    /// dispatch, every time, regardless of scenario config. Every tick
    /// after the first recomputes normally, once `elapsed` is meaningfully
    /// nonzero.
    pub fn dispatch_tick_period(&self, elapsed: Duration, is_first_tick: bool) -> Option<Duration> {
        if is_first_tick {
            return None;
        }
        let tps = self.instantaneous_tps(elapsed).max(0.001);
        Some(Duration::from_secs_f64(1.0 / tps))
    }
}

// ── Mixed flow override ───────────────────────────────────────────────────────

/// Flow configuration used when `LoadShape::Mixed` is active.
pub(super) fn mixed_flow_config() -> FlowConfig {
    FlowConfig {
        transparent_to_transparent: 0.0,
        transparent_to_shielded: 0.5,
        shielded_to_transparent: 0.0,
        shielded_to_shielded: 0.5,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_state_tps_is_constant() {
        let sched = Scheduler::new(LoadShape::SteadyState, 5.0);
        assert_eq!(sched.instantaneous_tps(Duration::ZERO), 5.0);
        assert_eq!(sched.instantaneous_tps(Duration::from_secs(10)), 5.0);
        assert_eq!(sched.instantaneous_tps(Duration::from_secs(100)), 5.0);
    }

    #[test]
    fn ramp_tps_is_zero_at_start() {
        let sched = Scheduler::new(LoadShape::Ramp { ramp_secs: 60 }, 10.0);
        assert_eq!(sched.instantaneous_tps(Duration::ZERO), 0.0);
    }

    #[test]
    fn ramp_initial_tps_is_nonzero() {
        let sched = Scheduler::new(LoadShape::Ramp { ramp_secs: 60 }, 10.0);
        assert!(sched.initial_tps() > 0.0);
    }

    #[test]
    fn ramp_tps_reaches_target_after_ramp_secs() {
        let sched = Scheduler::new(LoadShape::Ramp { ramp_secs: 60 }, 10.0);
        let tps = sched.instantaneous_tps(Duration::from_secs(60));
        assert!((tps - 10.0).abs() < 1e-9);
        // After ramp, stays at target
        let tps2 = sched.instantaneous_tps(Duration::from_secs(90));
        assert!((tps2 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn burst_tps_spikes_during_window() {
        let sched = Scheduler::new(
            LoadShape::Burst {
                pre_burst_secs: 10,
                burst_secs: 5,
                spike_multiplier: 3.0,
            },
            4.0,
        );
        // During burst window (10..15s): 4.0 × 3.0 = 12.0
        let tps = sched.instantaneous_tps(Duration::from_secs(12));
        assert!((tps - 12.0).abs() < 1e-9);
    }

    #[test]
    fn burst_tps_returns_to_base_after_burst() {
        let sched = Scheduler::new(
            LoadShape::Burst {
                pre_burst_secs: 10,
                burst_secs: 5,
                spike_multiplier: 3.0,
            },
            4.0,
        );
        // Before burst (0..10s): base TPS
        let before = sched.instantaneous_tps(Duration::from_secs(5));
        assert!((before - 4.0).abs() < 1e-9);
        // After burst (>=15s): base TPS
        let after = sched.instantaneous_tps(Duration::from_secs(20));
        assert!((after - 4.0).abs() < 1e-9);
    }

    #[test]
    fn dispatch_tick_period_is_none_on_the_first_tick() {
        let ramp = Scheduler::new(LoadShape::Ramp { ramp_secs: 60 }, 5.0);
        assert_eq!(ramp.dispatch_tick_period(Duration::ZERO, true), None);
        assert_eq!(
            ramp.dispatch_tick_period(Duration::from_secs(30), true),
            None
        );

        let steady = Scheduler::new(LoadShape::SteadyState, 5.0);
        assert_eq!(steady.dispatch_tick_period(Duration::ZERO, true), None);
    }

    #[test]
    fn dispatch_tick_period_recomputes_from_elapsed_after_the_first_tick() {
        // Ramp { ramp_secs: 60 }, target 10.0, at elapsed=30s: 10.0 * 30/60 = 5.0 TPS.
        let sched = Scheduler::new(LoadShape::Ramp { ramp_secs: 60 }, 10.0);
        let period = sched
            .dispatch_tick_period(Duration::from_secs(30), false)
            .expect("expected a period once past the first tick");
        assert!((period.as_secs_f64() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn reproduces_the_reported_thousand_second_stall_if_the_first_tick_is_wrongly_recomputed() {
        // Demonstrates the reported bug's exact mechanism: if the first
        // dispatch tick's period were (incorrectly) recomputed from ~0
        // elapsed instead of skipped, Ramp's near-zero instantaneous_tps
        // floors to the 0.001 TPS minimum — a fixed 1000s period, every
        // time, regardless of scenario config. `is_first_tick: true`
        // (tested above) exists specifically to avoid ever taking this path
        // on the first tick.
        let sched = Scheduler::new(LoadShape::Ramp { ramp_secs: 60 }, 5.0);
        let period = sched
            .dispatch_tick_period(Duration::ZERO, false)
            .expect("dispatch_tick_period always returns Some when is_first_tick is false");
        assert_eq!(period, Duration::from_secs(1000));
    }

    #[tokio::test(start_paused = true)]
    async fn first_dispatch_tick_fires_at_the_initial_tps_interval_not_after_1000s() {
        // End-to-end reproduction of mod.rs's actual ticker setup and first
        // loop iteration, using tokio's virtual clock so the test resolves
        // instantly regardless of which interval actually gets applied.
        let sched = Scheduler::new(LoadShape::Ramp { ramp_secs: 60 }, 5.0);

        let initial_tps = sched.initial_tps();
        let initial_interval = Duration::from_secs_f64(1.0 / initial_tps);
        let mut ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + initial_interval,
            initial_interval,
        );
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Mirrors mod.rs's dispatch loop: first tick, elapsed ~0.
        if let Some(period) = sched.dispatch_tick_period(Duration::ZERO, true) {
            ticker.reset_after(period);
        }

        let tick_at = tokio::time::Instant::now();
        ticker.tick().await;
        let waited = tick_at.elapsed();

        assert_eq!(waited, initial_interval);
        assert!(waited < Duration::from_secs(5), "waited {waited:?}");
    }
}
