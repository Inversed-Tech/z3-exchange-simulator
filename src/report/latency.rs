//! Time-windowed latency, throughput, and failure-rate analysis.
//!
//! `metrics.jsonl` only ever records `rpc_latency_ms` percentiles and
//! confirmed/failed/tps totals *once*, at the end of a run's load phase (see
//! docs/architecture/observability.md) — it cannot show how those numbers
//! evolved over the run. `rpc_calls.jsonl` retains a `request_at` timestamp
//! per call, which is enough to reconstruct time-windowed curves here; this
//! is the normal way to derive such curves from a raw per-call log rather
//! than relying on pre-aggregated samples, and needed no simulator change.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::data_model::RpcCall;

/// Nearest-rank percentile, matching `src/metrics/latency.rs`'s convention
/// (duplicated rather than reused across a private module boundary).
fn percentile_value(sorted: &[u64], p: f64) -> f64 {
    let n = sorted.len();
    let idx = ((p * n as f64).floor() as usize).min(n - 1);
    sorted[idx] as f64
}

/// Aggregate stats for one (method, backend) pair within one time window.
#[derive(Debug, Clone)]
pub struct WindowStats {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub method: String,
    pub backend: String,
    pub calls: u64,
    pub successes: u64,
    pub errors: u64,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
}

/// Buckets `calls` into fixed-width time windows (anchored to the earliest
/// `request_at`), grouped by (method, backend), and computes per-window
/// call/success/error counts and latency percentiles.
///
/// Returns windows in chronological order, method/backend then
/// window-start order. Calls with no `request_at` cannot occur (the field is
/// required), but calls with `latency_ms: None` (timeouts) are counted
/// towards `calls`/`errors` without contributing to the percentiles.
pub fn windowed_stats(calls: &[RpcCall], window_secs: i64) -> Vec<WindowStats> {
    if calls.is_empty() || window_secs <= 0 {
        return Vec::new();
    }

    let earliest = calls.iter().map(|c| c.request_at).min().unwrap();

    struct Bucket {
        window_start: DateTime<Utc>,
        calls: u64,
        successes: u64,
        latencies: Vec<u64>,
    }

    let mut buckets: HashMap<(String, String, i64), Bucket> = HashMap::new();

    for call in calls {
        let backend = format!("{:?}", call.backend);
        let offset_secs = (call.request_at - earliest).num_seconds();
        let window_index = offset_secs / window_secs;
        let window_start = earliest + chrono::Duration::seconds(window_index * window_secs);
        let key = (call.method.clone(), backend.clone(), window_index);

        let bucket = buckets.entry(key).or_insert_with(|| Bucket {
            window_start,
            calls: 0,
            successes: 0,
            latencies: Vec::new(),
        });
        bucket.calls += 1;
        if call.success {
            bucket.successes += 1;
        }
        if let Some(ms) = call.latency_ms {
            bucket.latencies.push(ms);
        }
    }

    let mut out: Vec<WindowStats> = buckets
        .into_iter()
        .map(|((method, backend, _), b)| {
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
            WindowStats {
                window_start: b.window_start,
                window_end: b.window_start + chrono::Duration::seconds(window_secs),
                method,
                backend,
                calls: b.calls,
                successes: b.successes,
                errors: b.calls - b.successes,
                p50_ms: p50,
                p95_ms: p95,
                p99_ms: p99,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        (&a.method, &a.backend, a.window_start).cmp(&(&b.method, &b.backend, b.window_start))
    });
    out
}

/// Achieved throughput per window across *all* methods combined — useful for
/// a single rate-vs-time curve rather than one per method.
#[derive(Debug, Clone)]
pub struct ThroughputWindow {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub calls: u64,
    pub tps: f64,
}

pub fn windowed_throughput(calls: &[RpcCall], window_secs: i64) -> Vec<ThroughputWindow> {
    if calls.is_empty() || window_secs <= 0 {
        return Vec::new();
    }
    let earliest = calls.iter().map(|c| c.request_at).min().unwrap();
    let mut counts: HashMap<i64, u64> = HashMap::new();
    for call in calls {
        let offset_secs = (call.request_at - earliest).num_seconds();
        let window_index = offset_secs / window_secs;
        *counts.entry(window_index).or_default() += 1;
    }
    let mut out: Vec<ThroughputWindow> = counts
        .into_iter()
        .map(|(idx, calls)| {
            let window_start = earliest + chrono::Duration::seconds(idx * window_secs);
            ThroughputWindow {
                window_start,
                window_end: window_start + chrono::Duration::seconds(window_secs),
                calls,
                tps: calls as f64 / window_secs as f64,
            }
        })
        .collect();
    out.sort_by_key(|w| w.window_start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::Backend;
    use chrono::TimeZone;

    fn call_at(method: &str, secs_offset: i64, latency_ms: Option<u64>, success: bool) -> RpcCall {
        let base = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
        RpcCall {
            call_id: format!("{method}-{secs_offset}"),
            run_id: "r".into(),
            method: method.to_string(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: base + chrono::Duration::seconds(secs_offset),
            response_at: Some(base + chrono::Duration::seconds(secs_offset)),
            latency_ms,
            success,
            error_code: if success { None } else { Some(-1) },
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
            intent_id: None,
            attempt_number: 1,
        }
    }

    #[test]
    fn windowed_stats_empty_input_returns_empty() {
        assert!(windowed_stats(&[], 10).is_empty());
    }

    #[test]
    fn windowed_stats_groups_by_window_and_method() {
        let calls = vec![
            call_at("getblockcount", 0, Some(10), true),
            call_at("getblockcount", 5, Some(20), true),
            call_at("getblockcount", 12, Some(30), true), // next 10s window
        ];
        let windows = windowed_stats(&calls, 10);
        assert_eq!(windows.len(), 2, "expected two 10s windows");
        assert_eq!(windows[0].calls, 2);
        assert_eq!(windows[1].calls, 1);
    }

    #[test]
    fn windowed_stats_counts_successes_and_errors_separately() {
        let calls = vec![
            call_at("z_sendmany", 0, Some(10), true),
            call_at("z_sendmany", 1, None, false),
        ];
        let windows = windowed_stats(&calls, 10);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].calls, 2);
        assert_eq!(windows[0].successes, 1);
        assert_eq!(windows[0].errors, 1);
    }

    #[test]
    fn windowed_stats_none_latency_excluded_from_percentiles_but_counted_in_calls() {
        let calls = vec![
            call_at("z_sendmany", 0, Some(100), true),
            call_at("z_sendmany", 1, None, false), // timeout: no latency
        ];
        let windows = windowed_stats(&calls, 10);
        assert_eq!(windows[0].calls, 2);
        assert_eq!(windows[0].p50_ms, Some(100.0));
    }

    #[test]
    fn windowed_stats_separates_different_methods_into_different_rows() {
        let calls = vec![
            call_at("getblockcount", 0, Some(10), true),
            call_at("z_sendmany", 0, Some(200), true),
        ];
        let windows = windowed_stats(&calls, 10);
        assert_eq!(windows.len(), 2);
        assert!(windows.iter().any(|w| w.method == "getblockcount"));
        assert!(windows.iter().any(|w| w.method == "z_sendmany"));
    }

    #[test]
    fn windowed_throughput_computes_tps_per_window() {
        let calls = vec![
            call_at("a", 0, Some(1), true),
            call_at("b", 1, Some(1), true),
            call_at("c", 2, Some(1), true),
        ];
        let windows = windowed_throughput(&calls, 1);
        assert_eq!(windows.len(), 3, "one call per 1s window");
        assert!(windows.iter().all(|w| w.tps == 1.0));
    }

    #[test]
    fn windowed_throughput_empty_input_returns_empty() {
        assert!(windowed_throughput(&[], 10).is_empty());
    }
}
