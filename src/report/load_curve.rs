//! Per-run load-curve analysis: correlates achieved throughput against
//! latency and error rate across a run's own time windows, to surface
//! candidate inflection/degradation points for ramp and burst scenarios —
//! see docs/scope.md's "load-curve results (throughput vs. latency,
//! inflection points, degradation modes)" requirement.
//!
//! Deliberately computed **per run**, never merged across runs the way the
//! RPC matrix is: two runs' time series only make sense to overlay if they
//! share a load shape, and the report has no reliable way to confirm that,
//! so each run gets its own curve and its own candidate detection.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::data_model::RpcCall;

use super::findings::{Finding, FindingCategory, Severity};
use super::loader::RunData;

/// Window width for load-curve bucketing. Independent of a scenario's own
/// `metric_sampling_interval_secs` (that governs `metrics.jsonl` sampling,
/// not this per-call reconstruction) — fixed here so curves from different
/// scenarios are directly comparable window-for-window.
pub const DEFAULT_WINDOW_SECS: i64 = 10;

/// A window's error rate is not considered for degradation detection below
/// this many calls — keeps a two-call window from reading as a "100% error
/// rate" out of pure noise.
pub(crate) const MIN_WINDOW_CALLS: u64 = 5;
/// How many times the run's baseline P99 a window's P99 must reach to be a
/// candidate degradation point.
pub(crate) const DEGRADATION_LATENCY_MULTIPLE: f64 = 3.0;
pub(crate) const DEGRADATION_HIGH_LATENCY_MULTIPLE: f64 = 6.0;
/// An error rate at or above this, on its own, is worth flagging regardless
/// of the latency ratio.
pub(crate) const DEGRADATION_ERROR_RATE: f64 = 0.10;

/// Nearest-rank percentile, matching `src/metrics/latency.rs`'s convention
/// (duplicated rather than reused across a private module boundary, same as
/// `report::latency` and `report::rpc_matrix` already do).
fn percentile_value(sorted: &[u64], p: f64) -> f64 {
    let n = sorted.len();
    let idx = ((p * n as f64).floor() as usize).min(n - 1);
    sorted[idx] as f64
}

/// One time window's throughput, latency, and error rate across every RPC
/// call in the window, all methods combined — the single throughput-vs-latency
/// curve scope.md asks for, as opposed to `report::latency::windowed_stats`'s
/// per-method breakdown. `rpc_calls_per_second` counts every RPC call in the
/// window regardless of method or outcome — it is not a confirmed-transaction
/// rate (see `crate::data_model` and `RunManifest`'s `confirmed_tx_throughput`
/// metric for that).
#[derive(Debug, Clone)]
pub struct LoadCurvePoint {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub calls: u64,
    pub errors: u64,
    pub rpc_calls_per_second: f64,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
}

/// Buckets every RPC call in `calls` into fixed-width windows (anchored to
/// the earliest `request_at`) and computes per-window throughput, latency
/// percentiles, and error rate across all methods combined. Returns windows
/// in chronological order.
pub fn windowed_load_curve(calls: &[RpcCall], window_secs: i64) -> Vec<LoadCurvePoint> {
    if calls.is_empty() || window_secs <= 0 {
        return Vec::new();
    }
    let earliest = calls.iter().map(|c| c.request_at).min().unwrap();

    struct Bucket {
        window_start: DateTime<Utc>,
        calls: u64,
        errors: u64,
        latencies: Vec<u64>,
    }
    let mut buckets: HashMap<i64, Bucket> = HashMap::new();
    for call in calls {
        let offset_secs = (call.request_at - earliest).num_seconds();
        let window_index = offset_secs / window_secs;
        let window_start = earliest + chrono::Duration::seconds(window_index * window_secs);
        let bucket = buckets.entry(window_index).or_insert_with(|| Bucket {
            window_start,
            calls: 0,
            errors: 0,
            latencies: Vec::new(),
        });
        bucket.calls += 1;
        if !call.success {
            bucket.errors += 1;
        }
        if let Some(ms) = call.latency_ms {
            bucket.latencies.push(ms);
        }
    }

    let mut out: Vec<LoadCurvePoint> = buckets
        .into_values()
        .map(|b| {
            let (p50, p95, p99) = if b.latencies.is_empty() {
                (None, None, None)
            } else {
                let mut sorted = b.latencies.clone();
                sorted.sort_unstable();
                (
                    Some(percentile_value(&sorted, 0.50)),
                    Some(percentile_value(&sorted, 0.95)),
                    Some(percentile_value(&sorted, 0.99)),
                )
            };
            LoadCurvePoint {
                window_start: b.window_start,
                window_end: b.window_start + chrono::Duration::seconds(window_secs),
                calls: b.calls,
                errors: b.errors,
                rpc_calls_per_second: b.calls as f64 / window_secs as f64,
                p50_ms: p50,
                p95_ms: p95,
                p99_ms: p99,
            }
        })
        .collect();
    out.sort_by_key(|w| w.window_start);
    out
}

/// One run's candidate degradation window, structured rather than a
/// pre-formatted `Finding` — shared by [`load_degradation_candidates`] and
/// the per-run load-curve digest in `markdown.rs`, so the two can never
/// drift apart on what counts as "the" inflection point.
#[derive(Debug, Clone)]
pub struct DegradationPoint {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub offset_secs: i64,
    pub rpc_calls_per_second: f64,
    pub calls: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub baseline_p99_ms: Option<f64>,
    pub severity: Severity,
}

/// Finds the first candidate degradation window (past the noise floor)
/// whose P99 has grown disproportionately relative to the run's own
/// baseline window, or whose error rate alone is high enough to be worth a
/// look. Stopping at the first hit is a deliberate simplification — this
/// flags *that* degradation was observed in the run, not a catalogue of
/// every window past it.
pub fn find_degradation_point(points: &[LoadCurvePoint]) -> Option<DegradationPoint> {
    if points.is_empty() {
        return None;
    }
    let baseline_p99 = points
        .iter()
        .find(|p| p.calls >= MIN_WINDOW_CALLS)
        .and_then(|p| p.p99_ms);
    let run_start = points[0].window_start;

    for point in points {
        if point.calls < MIN_WINDOW_CALLS {
            continue;
        }
        let error_rate = point.errors as f64 / point.calls as f64;
        let latency_ratio = match (baseline_p99, point.p99_ms) {
            (Some(base), Some(p99)) if base > 0.0 => Some(p99 / base),
            _ => None,
        };

        let is_latency_degradation = latency_ratio
            .map(|r| r >= DEGRADATION_LATENCY_MULTIPLE)
            .unwrap_or(false);
        let is_error_degradation = error_rate >= DEGRADATION_ERROR_RATE;
        if !is_latency_degradation && !is_error_degradation {
            continue;
        }

        let severity = if latency_ratio.unwrap_or(0.0) >= DEGRADATION_HIGH_LATENCY_MULTIPLE
            || error_rate >= 2.0 * DEGRADATION_ERROR_RATE
        {
            Severity::High
        } else {
            Severity::Medium
        };

        return Some(DegradationPoint {
            window_start: point.window_start,
            window_end: point.window_end,
            offset_secs: (point.window_start - run_start).num_seconds(),
            rpc_calls_per_second: point.rpc_calls_per_second,
            calls: point.calls,
            errors: point.errors,
            error_rate,
            p50_ms: point.p50_ms,
            p95_ms: point.p95_ms,
            p99_ms: point.p99_ms,
            baseline_p99_ms: baseline_p99,
            severity,
        });
    }
    None
}

/// The window with the highest RPC-call rate (all methods combined) in a
/// run's load curve — used for the per-run "peak throughput" digest line,
/// distinct from the candidate degradation point (a run can peak well
/// before or after it degrades).
pub fn peak_tps_point(points: &[LoadCurvePoint]) -> Option<&LoadCurvePoint> {
    points.iter().max_by(|a, b| {
        a.rpc_calls_per_second
            .partial_cmp(&b.rpc_calls_per_second)
            .unwrap()
    })
}

/// Flags at most one candidate degradation point per run — see
/// [`find_degradation_point`] for the exact rule. Scoped to `Load`/`Drain`
/// RPC calls only (see `crate::data_model::Phase::is_workload`): setup-phase
/// activity (bootstrap, warmup mining, funding fan-out) is not the measured
/// workload and must not feed degradation detection.
pub fn load_degradation_candidates(runs: &[RunData]) -> Vec<Finding> {
    let fmt_ms = |v: Option<f64>| {
        v.map(|v| format!("{v:.0}ms"))
            .unwrap_or_else(|| "N/A".into())
    };
    let mut out = Vec::new();
    for run in runs {
        let workload_calls: Vec<RpcCall> = run
            .rpc_calls
            .iter()
            .filter(|c| c.phase.is_workload())
            .cloned()
            .collect();
        let points = windowed_load_curve(&workload_calls, DEFAULT_WINDOW_SECS);
        let Some(d) = find_degradation_point(&points) else {
            continue;
        };
        out.push(Finding {
            category: FindingCategory::LoadDegradation,
            severity: d.severity,
            summary: format!(
                "{}: candidate inflection point at +{}s — {:.1} RPC calls/s, P99 {}, error rate {:.0}%",
                run.manifest.run_id,
                d.offset_secs,
                d.rpc_calls_per_second,
                fmt_ms(d.p99_ms),
                d.error_rate * 100.0,
            ),
            evidence: vec![format!(
                "window={}..{} calls={} errors={} p50={} p95={} p99={} baseline_p99={}",
                d.window_start.format("%H:%M:%S"),
                d.window_end.format("%H:%M:%S"),
                d.calls,
                d.errors,
                fmt_ms(d.p50_ms),
                fmt_ms(d.p95_ms),
                fmt_ms(d.p99_ms),
                fmt_ms(d.baseline_p99_ms),
            )],
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::Backend;
    use chrono::TimeZone;

    fn call_at(secs_offset: i64, latency_ms: Option<u64>, success: bool) -> RpcCall {
        let base = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        RpcCall {
            call_id: format!("c-{secs_offset}"),
            run_id: "r".into(),
            method: "z_sendmany".into(),
            backend: Backend::Zallet,
            params_hash: None,
            request_at: base + chrono::Duration::seconds(secs_offset),
            response_at: Some(base + chrono::Duration::seconds(secs_offset)),
            latency_ms,
            success,
            error_code: if success { None } else { Some(-1) },
            error_message: None,
            phase: crate::data_model::Phase::Load,
        }
    }

    fn call_at_phase(
        secs_offset: i64,
        latency_ms: Option<u64>,
        success: bool,
        phase: crate::data_model::Phase,
    ) -> RpcCall {
        RpcCall {
            phase,
            ..call_at(secs_offset, latency_ms, success)
        }
    }

    #[test]
    fn windowed_load_curve_empty_input_returns_empty() {
        assert!(windowed_load_curve(&[], 10).is_empty());
    }

    #[test]
    fn windowed_load_curve_computes_tps_and_percentiles_per_window() {
        let calls = vec![
            call_at(0, Some(10), true),
            call_at(1, Some(20), true),
            call_at(11, Some(10), true), // next 10s window
        ];
        let points = windowed_load_curve(&calls, 10);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].calls, 2);
        assert_eq!(points[1].calls, 1);
    }

    #[test]
    fn windowed_load_curve_tracks_errors() {
        let calls = vec![call_at(0, Some(10), true), call_at(1, None, false)];
        let points = windowed_load_curve(&calls, 10);
        assert_eq!(points[0].errors, 1);
    }

    fn run_with_calls(run_id: &str, calls: Vec<RpcCall>) -> RunData {
        use crate::metrics::{RunManifest, RunTimeouts};
        RunData {
            run_dir: format!("/tmp/{run_id}").into(),
            manifest: RunManifest {
                run_id: run_id.into(),
                run_started_at: Utc::now(),
                run_completed_at: Some(Utc::now()),
                simulator_commit: "abc".into(),
                zebra_commit: "z".into(),
                zaino_commit: "i".into(),
                zallet_commit: "t".into(),
                scenario_name: "ramp".into(),
                scenario_config_hash: "sha256:x".into(),
                target_tps: 10.0,
                timeouts: RunTimeouts::default(),
                phase_boundaries: Vec::new(),
                load_and_drain_completed_at: None,
            },
            rpc_calls: calls,
            intents: Vec::new(),
            metrics: Vec::new(),
            parse_warnings: Vec::new(),
        }
    }

    #[test]
    fn flags_degradation_when_latency_jumps_past_baseline() {
        let mut calls: Vec<RpcCall> = Vec::new();
        // Baseline window (0-9s): fast, healthy.
        for i in 0..6 {
            calls.push(call_at(i, Some(20), true));
        }
        // Degraded window (10-19s): P99 far above baseline.
        for i in 10..16 {
            calls.push(call_at(i, Some(400), true));
        }
        let r = run_with_calls("r1", calls);
        let findings = load_degradation_candidates(&[r]);
        assert!(
            findings
                .iter()
                .any(|f| f.category == FindingCategory::LoadDegradation),
            "expected a load degradation finding"
        );
    }

    #[test]
    fn no_degradation_finding_for_a_flat_healthy_run() {
        let mut calls: Vec<RpcCall> = Vec::new();
        for i in 0..20 {
            calls.push(call_at(i, Some(15), true));
        }
        let r = run_with_calls("r1", calls);
        let findings = load_degradation_candidates(&[r]);
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_degradation_from_error_rate_alone() {
        let mut calls: Vec<RpcCall> = Vec::new();
        for i in 0..5 {
            calls.push(call_at(i, Some(15), true));
        }
        for i in 5..10 {
            calls.push(call_at(i, Some(15), false)); // 100% errors this window
        }
        let r = run_with_calls("r1", calls);
        let findings = load_degradation_candidates(&[r]);
        assert!(!findings.is_empty());
        assert_eq!(findings[0].severity, Severity::High);
    }

    #[test]
    fn empty_run_produces_no_degradation_finding() {
        let r = run_with_calls("r1", vec![]);
        assert!(load_degradation_candidates(&[r]).is_empty());
    }

    #[test]
    fn load_degradation_candidates_excludes_setup_phase_calls() {
        // The exact same latency-jump shape that
        // flags_degradation_when_latency_jumps_past_baseline detects — but
        // every call is tagged Funding (the funding fan-out's own mining),
        // not Load/Drain. It must not be flagged: setup-phase activity is
        // not the measured workload.
        let mut calls: Vec<RpcCall> = Vec::new();
        for i in 0..6 {
            calls.push(call_at_phase(
                i,
                Some(20),
                true,
                crate::data_model::Phase::Funding,
            ));
        }
        for i in 10..16 {
            calls.push(call_at_phase(
                i,
                Some(400),
                true,
                crate::data_model::Phase::Funding,
            ));
        }
        let r = run_with_calls("r1", calls);
        assert!(load_degradation_candidates(&[r]).is_empty());
    }

    #[test]
    fn load_degradation_candidates_still_fires_when_drain_phase_calls_are_present() {
        // Drain-phase calls are part of the measured workload alongside
        // Load — a degradation that only shows up in the drain tail must
        // still be caught.
        let mut calls: Vec<RpcCall> = Vec::new();
        for i in 0..6 {
            calls.push(call_at_phase(
                i,
                Some(20),
                true,
                crate::data_model::Phase::Load,
            ));
        }
        for i in 10..16 {
            calls.push(call_at_phase(
                i,
                Some(400),
                true,
                crate::data_model::Phase::Drain,
            ));
        }
        let r = run_with_calls("r1", calls);
        assert!(!load_degradation_candidates(&[r]).is_empty());
    }

    #[test]
    fn at_most_one_degradation_finding_per_run() {
        let mut calls: Vec<RpcCall> = Vec::new();
        for i in 0..6 {
            calls.push(call_at(i, Some(20), true));
        }
        for i in 10..40 {
            calls.push(call_at(i, Some(500), true));
        }
        let r = run_with_calls("r1", calls);
        let findings = load_degradation_candidates(&[r]);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn peak_tps_point_finds_the_highest_tps_window() {
        let calls = vec![
            call_at(0, Some(10), true),
            call_at(1, Some(10), true),
            call_at(11, Some(10), true),
            call_at(12, Some(10), true),
            call_at(13, Some(10), true),
        ];
        let points = windowed_load_curve(&calls, 10);
        let peak = peak_tps_point(&points).expect("expected a peak point");
        assert_eq!(peak.calls, 3);
    }

    #[test]
    fn peak_tps_point_empty_input_returns_none() {
        assert!(peak_tps_point(&[]).is_none());
    }

    #[test]
    fn find_degradation_point_matches_load_degradation_candidates() {
        let mut calls: Vec<RpcCall> = Vec::new();
        for i in 0..6 {
            calls.push(call_at(i, Some(20), true));
        }
        for i in 10..16 {
            calls.push(call_at(i, Some(400), true));
        }
        let points = windowed_load_curve(&calls, 10);
        let direct = find_degradation_point(&points).expect("expected a degradation point");

        let r = run_with_calls("r1", calls);
        let findings = load_degradation_candidates(&[r]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0]
            .summary
            .contains(&format!("+{}s", direct.offset_secs)));
    }
}
