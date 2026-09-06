//! Aggregates `metrics.jsonl` samples per run into the system-health figures
//! the rest of this pipeline never touches — mempool depth/saturation,
//! per-process CPU/memory, and shielded proving time. `RunData.metrics` is
//! parsed by `loader.rs` but was otherwise unused: `findings.rs`,
//! `load_curve.rs`, `latency.rs`, and `rpc_matrix.rs` all derive their
//! numbers from `rpc_calls.jsonl` / `intents.jsonl` only. This module is the
//! first consumer of the metric samples themselves.

use std::collections::HashMap;

use super::loader::RunData;

/// Nearest-rank percentile, matching the convention duplicated across every
/// other `report::*` module (see `latency.rs`'s doc comment for why it's
/// duplicated rather than shared across a private module boundary).
fn percentile_value(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let idx = ((p * n as f64).floor() as usize).min(n - 1);
    sorted[idx]
}

#[derive(Debug, Clone)]
pub struct ProcessResourcePeak {
    pub process: String,
    pub peak_cpu_percent: Option<f64>,
    pub peak_memory_mb: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ProvingTimeStats {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub samples: usize,
}

#[derive(Debug, Clone, Default)]
pub struct SystemHealth {
    /// From the `scheduled_dispatch_rate` metric — how many intents per
    /// second the scheduler actually dispatched, over real load-phase
    /// elapsed time. Not a confirmed-transaction rate — see
    /// `confirmed_tx_throughput` for that, the only figure this report may
    /// label "TPS".
    pub scheduled_dispatch_rate: Option<f64>,
    /// From the `confirmed_tx_throughput` metric — confirmed transactions
    /// per second of load+drain wall-clock time. This is the run's
    /// confirmed_tx_throughput figure; the report labels it "TPS".
    pub confirmed_tx_throughput: Option<f64>,
    pub peak_mempool_tx_count: Option<f64>,
    pub peak_mempool_bytes: Option<f64>,
    /// Count of `mempool_saturated` samples — emitted once every sampling
    /// interval that observed mempool depth is at or above the scenario's
    /// saturation threshold (see the mempool watcher in
    /// `src/scenarios/exchange.rs`).
    pub saturation_events: u64,
    pub proving_time: Option<ProvingTimeStats>,
    /// Sorted by process name for deterministic report output (`metrics`
    /// iteration order is file order, not grouped by process).
    pub process_peaks: Vec<ProcessResourcePeak>,
}

/// Computes one run's system-health figures from its `metrics.jsonl`
/// samples. Every field degrades to `None`/empty/zero rather than erroring
/// when a metric was never emitted (e.g. a run with no shielded sends has no
/// `withdrawal_proving_time_ms` samples) — this is supplementary context on
/// top of the RPC-log-derived sections, not something a run can fail to
/// produce.
pub fn compute_system_health(run: &RunData) -> SystemHealth {
    let mut scheduled_dispatch_rate = None;
    let mut confirmed_tx_throughput = None;
    let mut peak_mempool_tx_count: Option<f64> = None;
    let mut peak_mempool_bytes: Option<f64> = None;
    let mut saturation_events = 0u64;
    let mut proving_samples: Vec<f64> = Vec::new();
    let mut process_cpu: HashMap<String, f64> = HashMap::new();
    let mut process_mem: HashMap<String, f64> = HashMap::new();

    for sample in &run.metrics {
        match sample.metric_name.as_str() {
            "scheduled_dispatch_rate" => scheduled_dispatch_rate = Some(sample.value),
            "confirmed_tx_throughput" => confirmed_tx_throughput = Some(sample.value),
            "mempool_tx_count" => {
                peak_mempool_tx_count =
                    Some(peak_mempool_tx_count.unwrap_or(0.0).max(sample.value));
            }
            "mempool_bytes" => {
                peak_mempool_bytes = Some(peak_mempool_bytes.unwrap_or(0.0).max(sample.value));
            }
            "mempool_saturated" => {
                if sample.value == 1.0 {
                    saturation_events += 1;
                }
            }
            "withdrawal_proving_time_ms" => proving_samples.push(sample.value),
            "process_cpu_percent" => {
                if let Some(process) = sample.labels.get("process") {
                    let entry = process_cpu.entry(process.clone()).or_insert(sample.value);
                    *entry = entry.max(sample.value);
                }
            }
            "process_memory_mb" => {
                if let Some(process) = sample.labels.get("process") {
                    let entry = process_mem.entry(process.clone()).or_insert(sample.value);
                    *entry = entry.max(sample.value);
                }
            }
            _ => {}
        }
    }

    let proving_time = if proving_samples.is_empty() {
        None
    } else {
        proving_samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
        Some(ProvingTimeStats {
            p50_ms: percentile_value(&proving_samples, 0.50),
            p95_ms: percentile_value(&proving_samples, 0.95),
            p99_ms: percentile_value(&proving_samples, 0.99),
            samples: proving_samples.len(),
        })
    };

    let mut processes: Vec<String> = process_cpu
        .keys()
        .chain(process_mem.keys())
        .cloned()
        .collect();
    processes.sort();
    processes.dedup();
    let process_peaks = processes
        .into_iter()
        .map(|process| ProcessResourcePeak {
            peak_cpu_percent: process_cpu.get(&process).copied(),
            peak_memory_mb: process_mem.get(&process).copied(),
            process,
        })
        .collect();

    SystemHealth {
        scheduled_dispatch_rate,
        confirmed_tx_throughput,
        peak_mempool_tx_count,
        peak_mempool_bytes,
        saturation_events,
        proving_time,
        process_peaks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::MetricSample;
    use crate::metrics::{RunManifest, RunTimeouts, StateIdentifier};
    use chrono::Utc;

    fn run_with_metrics(metrics: Vec<MetricSample>) -> RunData {
        RunData {
            run_dir: "/tmp/r".into(),
            manifest: RunManifest {
                run_id: "r".into(),
                run_started_at: Utc::now(),
                run_completed_at: Some(Utc::now()),
                simulator_commit: "abc".into(),
                zebra_commit: "z".into(),
                zaino_commit: "i".into(),
                zallet_commit: "t".into(),
                scenario_name: "ramp".into(),
                scenario_config_hash: "sha256:x".into(),
                target_tps: 8.0,
                timeouts: RunTimeouts::default(),
                phase_boundaries: Vec::new(),
                load_and_drain_completed_at: None,
                compose_config_hash: String::new(),
                image_digests: Vec::new(),
                host_cpu_count: 0,
                host_memory_limit_bytes: None,
                state: StateIdentifier::default(),
            },
            rpc_calls: Vec::new(),
            intents: Vec::new(),
            metrics,
            parse_warnings: Vec::new(),
        }
    }

    fn sample(name: &str, value: f64, labels: &[(&str, &str)]) -> MetricSample {
        MetricSample {
            run_id: "r".into(),
            timestamp: Utc::now(),
            metric_name: name.into(),
            value,
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn empty_metrics_produce_empty_health() {
        let h = compute_system_health(&run_with_metrics(Vec::new()));
        assert!(h.scheduled_dispatch_rate.is_none());
        assert!(h.confirmed_tx_throughput.is_none());
        assert!(h.peak_mempool_tx_count.is_none());
        assert!(h.proving_time.is_none());
        assert!(h.process_peaks.is_empty());
        assert_eq!(h.saturation_events, 0);
    }

    #[test]
    fn scheduled_dispatch_rate_reads_the_metric_value() {
        let h = compute_system_health(&run_with_metrics(vec![sample(
            "scheduled_dispatch_rate",
            6.6,
            &[],
        )]));
        assert_eq!(h.scheduled_dispatch_rate, Some(6.6));
    }

    #[test]
    fn confirmed_tx_throughput_reads_the_metric_value() {
        let h = compute_system_health(&run_with_metrics(vec![sample(
            "confirmed_tx_throughput",
            4.2,
            &[],
        )]));
        assert_eq!(h.confirmed_tx_throughput, Some(4.2));
    }

    #[test]
    fn mempool_peaks_take_the_max_across_samples() {
        let h = compute_system_health(&run_with_metrics(vec![
            sample("mempool_tx_count", 10.0, &[]),
            sample("mempool_tx_count", 63.0, &[]),
            sample("mempool_tx_count", 30.0, &[]),
            sample("mempool_bytes", 5000.0, &[]),
            sample("mempool_bytes", 9000.0, &[]),
        ]));
        assert_eq!(h.peak_mempool_tx_count, Some(63.0));
        assert_eq!(h.peak_mempool_bytes, Some(9000.0));
    }

    #[test]
    fn saturation_events_count_only_positive_samples() {
        let h = compute_system_health(&run_with_metrics(vec![
            sample("mempool_saturated", 1.0, &[]),
            sample("mempool_saturated", 1.0, &[]),
            sample("mempool_saturated", 0.0, &[]),
        ]));
        assert_eq!(h.saturation_events, 2);
    }

    #[test]
    fn proving_time_percentiles_from_samples() {
        let samples: Vec<MetricSample> = (1..=10)
            .map(|i| sample("withdrawal_proving_time_ms", (i * 100) as f64, &[]))
            .collect();
        let h = compute_system_health(&run_with_metrics(samples));
        let p = h.proving_time.expect("expected proving time stats");
        assert_eq!(p.samples, 10);
        assert_eq!(p.p50_ms, 600.0);
    }

    #[test]
    fn process_peaks_grouped_by_label_and_sorted() {
        let h = compute_system_health(&run_with_metrics(vec![
            sample("process_cpu_percent", 12.0, &[("process", "zallet")]),
            sample("process_cpu_percent", 40.0, &[("process", "zallet")]),
            sample("process_memory_mb", 200.0, &[("process", "zallet")]),
            sample("process_cpu_percent", 5.0, &[("process", "zaino")]),
        ]));
        assert_eq!(h.process_peaks.len(), 2);
        assert_eq!(h.process_peaks[0].process, "zaino");
        assert_eq!(h.process_peaks[0].peak_cpu_percent, Some(5.0));
        assert_eq!(h.process_peaks[0].peak_memory_mb, None);
        assert_eq!(h.process_peaks[1].process, "zallet");
        assert_eq!(h.process_peaks[1].peak_cpu_percent, Some(40.0));
        assert_eq!(h.process_peaks[1].peak_memory_mb, Some(200.0));
    }
}
