//! Renders the aggregated run data, RPC matrix, and candidate findings into
//! a single Markdown document. Candidate findings are rendered as flagged
//! evidence, not finished prose — see `findings.rs` for why authorship is a
//! deliberately separate step from this pipeline.

use super::findings::{flag_candidates, FindingCategory};
use super::loader::RunData;
use super::rpc_matrix::{build_matrix, Category};

fn render_run_metadata(runs: &[RunData], md: &mut String) {
    md.push_str("## Runs included in this report\n\n");
    md.push_str("| Run ID | Scenario | Target TPS | Zebra | Zaino | Zallet | Simulator commit |\n");
    md.push_str("|---|---|---|---|---|---|---|\n");
    for run in runs {
        let m = &run.manifest;
        md.push_str(&format!(
            "| {} | {} | {} | `{}` | `{}` | `{}` | `{}` |\n",
            m.run_id,
            m.scenario_name,
            m.target_tps,
            short_sha(&m.zebra_commit),
            short_sha(&m.zaino_commit),
            short_sha(&m.zallet_commit),
            short_sha(&m.simulator_commit),
        ));
    }
    md.push('\n');
}

fn short_sha(s: &str) -> String {
    s.chars().take(10).collect()
}

fn render_load_results(runs: &[RunData], md: &mut String) {
    md.push_str("## Load results by run\n\n");
    md.push_str("| Run ID | Attempted | Confirmed | Failed | Timed out |\n");
    md.push_str("|---|---|---|---|---|\n");
    for run in runs {
        let attempted = run.intents.len();
        let confirmed = run
            .intents
            .iter()
            .filter(|i| i.outcome == "confirmed")
            .count();
        let failed = run.intents.iter().filter(|i| i.outcome == "failed").count();
        let timed_out = run
            .intents
            .iter()
            .filter(|i| i.outcome == "timed_out")
            .count();
        md.push_str(&format!(
            "| {} | {attempted} | {confirmed} | {failed} | {timed_out} |\n",
            run.manifest.run_id
        ));
    }
    md.push('\n');
}

fn render_rpc_matrix(runs: &[RunData], md: &mut String) {
    md.push_str("## RPC compatibility matrix\n\n");
    md.push_str(
        "Derived mechanically from the `rpc_calls.jsonl` of every run listed above. \
         `Not tested` means no observed call, not \"known to fail\" — see \
         docs/rpc/rpc-coverage-matrix.md for zcashd-equivalence and parity notes, \
         which this table does not attempt to reproduce.\n\n",
    );
    let matrix = build_matrix(runs);
    for category in [Category::Stress, Category::RegtestControl, Category::Smoke] {
        let rows: Vec<_> = matrix.iter().filter(|r| r.category == category).collect();
        if rows.is_empty() {
            continue;
        }
        md.push_str(&format!("### {category}\n\n"));
        md.push_str("| Method | Backend | Status | Calls | Successes | P50 ms | P95 ms | P99 ms | Error codes |\n");
        md.push_str("|---|---|---|---|---|---|---|---|---|\n");
        for row in rows {
            let fmt_ms =
                |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or_else(|| "—".into());
            let codes = if row.error_codes.is_empty() {
                "—".to_string()
            } else {
                format!("{:?}", row.error_codes)
            };
            md.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                row.method,
                row.backend_label,
                row.status,
                row.calls,
                row.successes,
                fmt_ms(row.p50_ms),
                fmt_ms(row.p95_ms),
                fmt_ms(row.p99_ms),
                codes,
            ));
        }
        md.push('\n');
    }
}

fn render_findings(runs: &[RunData], md: &mut String) {
    md.push_str("## Candidate findings\n\n");
    md.push_str(
        "**These are flagged candidates, not finished findings.** Each is a mechanically \
         detected statistical outlier or observed failure, stated with only what the data \
         shows. Severity, reproducibility, root cause, and component attribution require \
         reading the underlying evidence and are not assigned here.\n\n",
    );
    let findings = flag_candidates(runs);
    if findings.is_empty() {
        md.push_str("No candidates flagged for the runs included in this report.\n\n");
        return;
    }
    for category in [
        FindingCategory::RpcFailure,
        FindingCategory::Timeout,
        FindingCategory::LatencyOutlier,
        FindingCategory::FlowTypeDisparity,
        FindingCategory::DataCompleteness,
    ] {
        let items: Vec<_> = findings.iter().filter(|f| f.category == category).collect();
        if items.is_empty() {
            continue;
        }
        md.push_str(&format!("### {category}\n\n"));
        for item in items {
            md.push_str(&format!("- {}\n", item.summary));
            for ev in &item.evidence {
                md.push_str(&format!("  - {ev}\n"));
            }
        }
        md.push('\n');
    }
}

fn render_limitations(runs: &[RunData], md: &mut String) {
    md.push_str("## Limitations and threats to validity\n\n");
    md.push_str(&format!(
        "- This report covers {} run(s). Scenario coverage may be incomplete — \
         see the scenario library's design doc \
         (`docs/scenarios/scenario-design.md`) for the full intended battery.\n",
        runs.len()
    ));
    md.push_str(
        "- The RPC compatibility matrix's `Not tested` methods have not been exercised by \
         any run in this report; that is a coverage gap, not evidence of a defect.\n",
    );
    let warning_runs: Vec<_> = runs
        .iter()
        .filter(|r| !r.parse_warnings.is_empty())
        .collect();
    if !warning_runs.is_empty() {
        md.push_str(&format!(
            "- {} run(s) had malformed lines skipped during loading — see \
             \"Data completeness\" findings above for exactly which lines and which runs.\n",
            warning_runs.len()
        ));
    }
    md.push('\n');
}

/// Renders the full Markdown report for the given runs.
pub fn render_report(runs: &[RunData]) -> String {
    let mut md = String::new();
    md.push_str("# Z3 Exchange Simulator — Findings Report\n\n");
    render_run_metadata(runs, &mut md);
    render_load_results(runs, &mut md);
    render_rpc_matrix(runs, &mut md);
    render_findings(runs, &mut md);
    render_limitations(runs, &mut md);
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{Backend, FlowType, IntentRecord, RpcCall};
    use crate::metrics::{RunManifest, RunTimeouts};
    use chrono::Utc;

    fn sample_run() -> RunData {
        RunData {
            run_dir: "/tmp/r1".into(),
            manifest: RunManifest {
                run_id: "20260803T084825Z-smoke".into(),
                run_started_at: Utc::now(),
                run_completed_at: Some(Utc::now()),
                simulator_commit: "5d6340a2bbcf938a80fe2db661dd70f93bd6c9ea".into(),
                zebra_commit: "bb41d69013edbfa8594bb097fa751f47eeb31445".into(),
                zaino_commit: "17963672d0c2cad97dd12bd38bbf1b6fd232c8c5".into(),
                zallet_commit: "bd7f020eb9e1de6f79da947e8102281832b05f83".into(),
                scenario_name: "smoke".into(),
                scenario_config_hash: "sha256:x".into(),
                target_tps: 1.0,
                timeouts: RunTimeouts::default(),
            },
            rpc_calls: vec![
                RpcCall {
                    call_id: "c1".into(),
                    run_id: "r".into(),
                    method: "getblockcount".into(),
                    backend: Backend::Zebra,
                    params_hash: None,
                    request_at: Utc::now(),
                    response_at: Some(Utc::now()),
                    latency_ms: Some(5),
                    success: true,
                    error_code: None,
                    error_message: None,
                },
                RpcCall {
                    call_id: "c2".into(),
                    run_id: "r".into(),
                    method: "z_listunspent".into(),
                    backend: Backend::Zallet,
                    params_hash: None,
                    request_at: Utc::now(),
                    response_at: Some(Utc::now()),
                    latency_ms: None,
                    success: false,
                    error_code: Some(-20),
                    error_message: Some("WalletDb::get_memo failed".into()),
                },
            ],
            intents: vec![
                IntentRecord {
                    run_id: "r".into(),
                    intent_id: "i1".into(),
                    flow_type: FlowType::TToT,
                    outcome: "confirmed".into(),
                    error: None,
                    timeout_context: None,
                    recorded_at: Utc::now(),
                },
                IntentRecord {
                    run_id: "r".into(),
                    intent_id: "i2".into(),
                    flow_type: FlowType::TToT,
                    outcome: "failed".into(),
                    error: Some("insufficient balance".into()),
                    timeout_context: None,
                    recorded_at: Utc::now(),
                },
            ],
            metrics: Vec::new(),
            parse_warnings: Vec::new(),
        }
    }

    #[test]
    fn render_report_includes_run_id_and_commits() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("20260803T084825Z-smoke"));
        assert!(md.contains("bb41d69013"));
    }

    #[test]
    fn render_report_includes_load_results() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Load results by run"));
        assert!(md.contains("| 20260803T084825Z-smoke | 2 | 1 | 1 | 0 |"));
    }

    #[test]
    fn render_report_includes_rpc_matrix_with_status() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## RPC compatibility matrix"));
        assert!(md.contains("`z_listunspent`"));
        assert!(md.contains("Failed"));
    }

    #[test]
    fn render_report_includes_candidate_findings_with_disclaimer() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Candidate findings"));
        assert!(md.contains("not finished findings"));
        assert!(md.contains("z_listunspent"));
    }

    #[test]
    fn render_report_no_findings_states_so_explicitly() {
        let mut run = sample_run();
        run.rpc_calls.retain(|c| c.success);
        run.intents = vec![IntentRecord {
            run_id: "r".into(),
            intent_id: "i1".into(),
            flow_type: FlowType::TToT,
            outcome: "confirmed".into(),
            error: None,
            timeout_context: None,
            recorded_at: Utc::now(),
        }];
        let md = render_report(&[run]);
        assert!(md.contains("No candidates flagged"));
    }

    #[test]
    fn render_report_includes_limitations_section() {
        let md = render_report(&[sample_run()]);
        assert!(md.contains("## Limitations and threats to validity"));
        assert!(md.contains("1 run(s)"));
    }

    #[test]
    fn render_report_empty_runs_does_not_panic() {
        let md = render_report(&[]);
        assert!(md.contains("# Z3 Exchange Simulator"));
    }
}
