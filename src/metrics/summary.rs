use std::collections::HashMap;
use std::io::{BufRead, BufReader};

use crate::data_model::{MetricSample, RpcCall};

use super::error::MetricsError;
use super::latency::percentile_value;
use super::manifest::RunManifest;
use super::run_dir::RunDir;

struct RpcCallAgg {
    count: u64,
    success_count: u64,
    latencies: Vec<u64>,
    error_counts: HashMap<i64, u64>,
}

struct MetricAgg {
    peak_mempool: f64,
    confirmed_total: f64,
    failed_total: f64,
    saturation_events: u64,
    proving_ms: Vec<u64>,
    tps_achieved: Option<f64>,
}

pub fn generate_summary(run_dir: &RunDir, manifest: &RunManifest) -> Result<String, MetricsError> {
    let mut rpc_aggs: HashMap<(String, String), RpcCallAgg> = HashMap::new();

    let rpc_path = run_dir.rpc_calls_path();
    if rpc_path.exists() {
        let file = std::fs::File::open(&rpc_path).map_err(MetricsError::Io)?;
        for line in BufReader::new(file).lines() {
            let line = line.map_err(MetricsError::Io)?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<RpcCall>(&line) {
                Ok(call) => {
                    let backend_str = format!("{:?}", call.backend);
                    let agg = rpc_aggs
                        .entry((call.method.clone(), backend_str))
                        .or_insert_with(|| RpcCallAgg {
                            count: 0,
                            success_count: 0,
                            latencies: vec![],
                            error_counts: HashMap::new(),
                        });
                    agg.count += 1;
                    if call.success {
                        agg.success_count += 1;
                    }
                    if let Some(ms) = call.latency_ms {
                        agg.latencies.push(ms);
                    }
                    if let Some(code) = call.error_code {
                        *agg.error_counts.entry(code).or_default() += 1;
                    }
                }
                Err(e) => eprintln!("[metrics] summary: malformed rpc_calls line: {e}"),
            }
        }
    }

    let mut magg = MetricAgg {
        peak_mempool: 0.0,
        confirmed_total: 0.0,
        failed_total: 0.0,
        saturation_events: 0,
        proving_ms: vec![],
        tps_achieved: None,
    };

    let metrics_path = run_dir.metrics_path();
    if metrics_path.exists() {
        let file = std::fs::File::open(&metrics_path).map_err(MetricsError::Io)?;
        for line in BufReader::new(file).lines() {
            let line = line.map_err(MetricsError::Io)?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MetricSample>(&line) {
                Ok(sample) => match sample.metric_name.as_str() {
                    "mempool_tx_count" => {
                        if sample.value > magg.peak_mempool {
                            magg.peak_mempool = sample.value;
                        }
                    }
                    "confirmed_txs_total" => magg.confirmed_total = sample.value,
                    "failed_txs_total" => magg.failed_total = sample.value,
                    "mempool_saturated" => {
                        if sample.value == 1.0 {
                            magg.saturation_events += 1;
                        }
                    }
                    "withdrawal_proving_time_ms" => {
                        magg.proving_ms.push(sample.value as u64);
                    }
                    "tps_achieved" => magg.tps_achieved = Some(sample.value),
                    _ => {}
                },
                Err(e) => eprintln!("[metrics] summary: malformed metrics line: {e}"),
            }
        }
    }

    let duration_str = match manifest.run_completed_at {
        Some(completed) => {
            let secs = (completed - manifest.run_started_at).num_seconds().max(0);
            format!("{secs} s")
        }
        None => "incomplete".to_string(),
    };

    let total_attempted = magg.confirmed_total + magg.failed_total;
    let (confirmed_pct, failed_pct) = if total_attempted > 0.0 {
        (
            magg.confirmed_total / total_attempted * 100.0,
            magg.failed_total / total_attempted * 100.0,
        )
    } else {
        (0.0, 0.0)
    };

    let achieved_tps_str = match magg.tps_achieved {
        Some(v) => format!("{v:.1}"),
        None => "N/A".to_string(),
    };

    let mut md = String::new();

    md.push_str(&format!("# Run Summary: {}\n\n", manifest.run_id));

    md.push_str("## Run metadata\n");
    md.push_str(&format!("- Scenario: {}\n", manifest.scenario_name));
    md.push_str(&format!(
        "- Started: {}\n",
        manifest.run_started_at.format("%Y-%m-%dT%H:%M:%SZ")
    ));
    md.push_str(&format!("- Duration: {duration_str}\n"));
    md.push_str(&format!(
        "- Simulator commit: {}\n",
        manifest.simulator_commit
    ));
    md.push_str(&format!(
        "- Z3 commits: Zebra {}, Zaino {}, Zallet {}\n\n",
        manifest.zebra_commit, manifest.zaino_commit, manifest.zallet_commit
    ));

    md.push_str("## Load results\n");
    md.push_str(&format!("- Target TPS: {}\n", manifest.target_tps));
    md.push_str(&format!("- Achieved TPS: {achieved_tps_str}\n"));
    md.push_str(&format!(
        "- Total transactions attempted: {}\n",
        total_attempted as u64
    ));
    md.push_str(&format!(
        "- Confirmed: {} ({:.1}%)\n",
        magg.confirmed_total as u64, confirmed_pct
    ));
    md.push_str(&format!(
        "- Failed: {} ({:.1}%)\n\n",
        magg.failed_total as u64, failed_pct
    ));

    md.push_str("## RPC latency (P50 / P95 / P99)\n");
    md.push_str("| Method | Backend | P50 ms | P95 ms | P99 ms | Calls | Errors |\n");
    md.push_str("|---|---|---|---|---|---|---|\n");

    let mut agg_entries: Vec<_> = rpc_aggs.iter().collect();
    agg_entries.sort_by(|a, b| a.0.cmp(b.0));

    for ((method, backend), agg) in &agg_entries {
        let error_count = agg.count - agg.success_count;
        let mut latencies = agg.latencies.clone();
        let (p50_str, p95_str, p99_str) = if latencies.is_empty() {
            ("N/A".to_string(), "N/A".to_string(), "N/A".to_string())
        } else {
            latencies.sort_unstable();
            (
                format!("{:.0}", percentile_value(&latencies, 0.50)),
                format!("{:.0}", percentile_value(&latencies, 0.95)),
                format!("{:.0}", percentile_value(&latencies, 0.99)),
            )
        };
        md.push_str(&format!(
            "| {method} | {backend} | {p50_str} | {p95_str} | {p99_str} | {} | {error_count} |\n",
            agg.count
        ));
    }
    md.push('\n');

    md.push_str("## Mempool\n");
    md.push_str(&format!(
        "- Peak mempool size: {} transactions\n",
        magg.peak_mempool as u64
    ));
    md.push_str(&format!(
        "- Mempool saturation events: {}\n\n",
        magg.saturation_events
    ));

    md.push_str("## Shielded transaction proving times\n");
    if magg.proving_ms.is_empty() {
        md.push_str("- P50: N/A ms, P95: N/A ms, P99: N/A ms\n\n");
    } else {
        let mut sorted = magg.proving_ms.clone();
        sorted.sort_unstable();
        md.push_str(&format!(
            "- P50: {:.0} ms, P95: {:.0} ms, P99: {:.0} ms\n\n",
            percentile_value(&sorted, 0.50),
            percentile_value(&sorted, 0.95),
            percentile_value(&sorted, 0.99),
        ));
    }

    md.push_str("## Notable errors and findings\n");
    let has_errors = agg_entries
        .iter()
        .any(|(_, agg)| !agg.error_counts.is_empty());
    if has_errors {
        for ((method, _backend), agg) in &agg_entries {
            let mut codes: Vec<_> = agg.error_counts.iter().collect();
            codes.sort_by_key(|(code, _)| *code);
            for (code, count) in codes {
                md.push_str(&format!("- {method}: error {code} × {count}\n"));
            }
        }
    } else {
        md.push_str("- No errors recorded\n");
    }

    std::fs::write(run_dir.summary_path(), &md).map_err(MetricsError::Io)?;
    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::manifest::RunManifest;
    use crate::metrics::run_dir::RunDir;
    use crate::metrics::writers::JsonlWriter;

    #[test]
    fn generate_summary_produces_summary_md() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;

        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "sumtest").unwrap();

        let writer = JsonlWriter::<RpcCall>::open(&rd.rpc_calls_path()).unwrap();
        writer.write_record(&RpcCall {
            call_id: "c1".into(),
            run_id: rd.run_id.clone(),
            method: "getblockcount".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: Some(10),
            success: true,
            error_code: None,
            error_message: None,
        });
        std::fs::write(rd.metrics_path(), "").unwrap();

        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "abc".into(),
            zebra_commit: "z".into(),
            zaino_commit: "i".into(),
            zallet_commit: "t".into(),
            scenario_name: "sumtest".into(),
            scenario_config_hash: "sha:0".into(),
            target_tps: 10.0,
        };

        generate_summary(&rd, &manifest).unwrap();
        let md = std::fs::read_to_string(rd.summary_path()).unwrap();

        assert!(md.contains("# Run Summary"), "missing title");
        assert!(md.contains(&rd.run_id), "missing run_id");
        assert!(md.contains("## Run metadata"), "missing metadata section");
        assert!(
            md.contains("## Load results"),
            "missing load results section"
        );
        assert!(md.contains("Target TPS"), "missing target TPS line");
        assert!(md.contains("## RPC latency"), "missing latency section");
        assert!(
            md.contains("| getblockcount"),
            "getblockcount must appear in latency table"
        );
        assert!(md.contains("P50"), "missing P50 in latency table");
        assert!(md.contains("## Mempool"), "missing mempool section");
        assert!(
            md.contains("## Shielded transaction proving times"),
            "missing proving times section"
        );
    }

    #[test]
    fn generate_summary_shows_tps_and_confirmed_from_metrics_jsonl() {
        use chrono::Utc;

        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "tpstest").unwrap();
        std::fs::write(rd.rpc_calls_path(), "").unwrap();

        let mwriter = JsonlWriter::<MetricSample>::open(&rd.metrics_path()).unwrap();
        mwriter.write_record(&MetricSample {
            run_id: rd.run_id.clone(),
            timestamp: Utc::now(),
            metric_name: "tps_achieved".into(),
            value: 42.5,
            labels: HashMap::new(),
        });
        mwriter.write_record(&MetricSample {
            run_id: rd.run_id.clone(),
            timestamp: Utc::now(),
            metric_name: "confirmed_txs_total".into(),
            value: 100.0,
            labels: HashMap::new(),
        });

        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "tpstest".into(),
            scenario_config_hash: "".into(),
            target_tps: 50.0,
        };
        generate_summary(&rd, &manifest).unwrap();
        let md = std::fs::read_to_string(rd.summary_path()).unwrap();
        assert!(
            md.contains("42.5") || md.contains("42"),
            "achieved TPS value must appear in summary"
        );
        assert!(
            md.contains("100"),
            "confirmed tx count must appear in summary"
        );
    }

    #[test]
    fn generate_summary_handles_empty_jsonl_gracefully() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "empty").unwrap();
        std::fs::write(rd.rpc_calls_path(), "").unwrap();
        std::fs::write(rd.metrics_path(), "").unwrap();
        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: None,
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "empty".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        generate_summary(&rd, &manifest).unwrap();
        assert!(rd.summary_path().exists());
    }

    #[test]
    fn generate_summary_skips_malformed_jsonl_lines() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "malformed").unwrap();
        std::fs::write(
            rd.rpc_calls_path(),
            "{\"call_id\":\"c1\",broken}\n{invalid json\n",
        )
        .unwrap();
        std::fs::write(rd.metrics_path(), "").unwrap();
        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: None,
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "malformed".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        generate_summary(&rd, &manifest).unwrap();
        assert!(rd.summary_path().exists());
    }

    #[test]
    fn generate_summary_incomplete_duration_when_no_completed_at() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "incomplete").unwrap();
        std::fs::write(rd.rpc_calls_path(), "").unwrap();
        std::fs::write(rd.metrics_path(), "").unwrap();
        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: None,
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "incomplete".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        let md = generate_summary(&rd, &manifest).unwrap();
        assert!(
            md.contains("incomplete"),
            "summary must say 'incomplete' when run_completed_at is None; got:\n{md}"
        );
    }

    #[test]
    fn generate_summary_proving_times_non_na_when_samples_present() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "proving").unwrap();
        std::fs::write(rd.rpc_calls_path(), "").unwrap();

        let mwriter = JsonlWriter::<MetricSample>::open(&rd.metrics_path()).unwrap();
        for ms in [100u64, 200, 300, 400, 500] {
            mwriter.write_record(&MetricSample {
                run_id: rd.run_id.clone(),
                timestamp: Utc::now(),
                metric_name: "withdrawal_proving_time_ms".into(),
                value: ms as f64,
                labels: HashMap::new(),
            });
        }
        drop(mwriter);

        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "proving".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        let md = generate_summary(&rd, &manifest).unwrap();
        assert!(
            !md.contains("P50: N/A"),
            "proving times must not be N/A when samples are present"
        );
        // sorted=[100,200,300,400,500] n=5; p50: idx floor(0.5*5)=2 → 300
        assert!(
            md.contains("300"),
            "p50 of [100..500] must be 300; summary:\n{md}"
        );
    }

    #[test]
    fn generate_summary_mempool_peak_and_saturation_events() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "mempool").unwrap();
        std::fs::write(rd.rpc_calls_path(), "").unwrap();

        let mwriter = JsonlWriter::<MetricSample>::open(&rd.metrics_path()).unwrap();
        for val in [10.0f64, 55.0, 30.0] {
            mwriter.write_record(&MetricSample {
                run_id: rd.run_id.clone(),
                timestamp: Utc::now(),
                metric_name: "mempool_tx_count".into(),
                value: val,
                labels: HashMap::new(),
            });
        }
        for _ in 0..3 {
            mwriter.write_record(&MetricSample {
                run_id: rd.run_id.clone(),
                timestamp: Utc::now(),
                metric_name: "mempool_saturated".into(),
                value: 1.0,
                labels: HashMap::new(),
            });
        }
        drop(mwriter);

        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "mempool".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        let md = generate_summary(&rd, &manifest).unwrap();
        assert!(
            md.contains("55"),
            "peak mempool (55) must appear in summary; got:\n{md}"
        );
        assert!(
            md.contains("3"),
            "saturation event count (3) must appear in summary; got:\n{md}"
        );
    }

    #[test]
    fn generate_summary_error_code_appears_in_notable_errors() {
        use crate::data_model::{Backend, RpcCall};
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "errcode").unwrap();
        std::fs::write(rd.metrics_path(), "").unwrap();

        let rwriter = JsonlWriter::<RpcCall>::open(&rd.rpc_calls_path()).unwrap();
        rwriter.write_record(&RpcCall {
            call_id: "c-err".into(),
            run_id: rd.run_id.clone(),
            method: "sendrawtransaction".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: None,
            latency_ms: None,
            success: false,
            error_code: Some(-3),
            error_message: Some("invalid tx".into()),
        });
        drop(rwriter);

        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "errcode".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        let md = generate_summary(&rd, &manifest).unwrap();
        assert!(
            md.contains("## Notable errors"),
            "notable errors section must be present"
        );
        assert!(
            md.contains("-3") || md.contains("error -3"),
            "error code -3 must appear in notable errors; got:\n{md}"
        );
        assert!(
            md.contains("sendrawtransaction"),
            "method name must appear with error code; got:\n{md}"
        );
    }

    #[test]
    fn generate_summary_return_value_matches_file_content() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "retval").unwrap();
        std::fs::write(rd.rpc_calls_path(), "").unwrap();
        std::fs::write(rd.metrics_path(), "").unwrap();
        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "retval".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        let returned = generate_summary(&rd, &manifest).unwrap();
        let on_disk = std::fs::read_to_string(rd.summary_path()).unwrap();
        assert_eq!(
            returned, on_disk,
            "generate_summary return value must match summary.md content"
        );
    }

    #[test]
    fn generate_summary_missing_rpc_calls_file_still_succeeds() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "norpc").unwrap();
        // Deliberately do NOT create rpc_calls.jsonl.
        std::fs::write(rd.metrics_path(), "").unwrap();
        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "norpc".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        let md = generate_summary(&rd, &manifest).unwrap();
        assert!(
            md.contains("# Run Summary"),
            "summary must be generated even without rpc_calls.jsonl"
        );
        assert!(rd.summary_path().exists());
    }

    #[test]
    fn generate_summary_zero_total_txs_no_division_by_zero() {
        use chrono::Utc;
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "zerotx").unwrap();
        std::fs::write(rd.rpc_calls_path(), "").unwrap();
        std::fs::write(rd.metrics_path(), "").unwrap();
        let manifest = RunManifest {
            run_id: rd.run_id.clone(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "zerotx".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
        };
        // Must not panic or return Err.
        let md = generate_summary(&rd, &manifest).unwrap();
        assert!(md.contains("0%") || md.contains("0.0%"));
    }
}
