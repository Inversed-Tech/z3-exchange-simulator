//! Candidate-finding detection: computes objective aggregates and flags
//! statistical outliers as *candidates* for review. Deliberately does not
//! generate severity ratings, root-cause narratives, or recommendations —
//! those require judgment and are authored by reading this evidence, the
//! same way the existing crash-loop/spending-bug docs were produced. This
//! module's job stops at "here is what the data shows and why it's worth a
//! second look," with every claim traceable to specific evidence.

use std::collections::HashMap;

use super::loader::RunData;
use super::rpc_matrix::{build_matrix, MatrixStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingCategory {
    RpcFailure,
    Timeout,
    LatencyOutlier,
    FlowTypeDisparity,
    DataCompleteness,
}

impl std::fmt::Display for FindingCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindingCategory::RpcFailure => write!(f, "RPC failure"),
            FindingCategory::Timeout => write!(f, "Timeout"),
            FindingCategory::LatencyOutlier => write!(f, "Latency outlier"),
            FindingCategory::FlowTypeDisparity => write!(f, "Flow-type disparity"),
            FindingCategory::DataCompleteness => write!(f, "Data completeness"),
        }
    }
}

/// A flagged candidate, not a finished finding. `summary` states only what
/// was observed; `evidence` gives the specific facts a reader needs to judge
/// severity and cause themselves.
#[derive(Debug, Clone)]
pub struct Finding {
    pub category: FindingCategory,
    pub summary: String,
    pub evidence: Vec<String>,
}

const TAIL_LATENCY_MULTIPLE: f64 = 5.0;
const MIN_TAIL_LATENCY_SAMPLE: u64 = 5;
/// A P99/P50 ratio alone flags noise for very fast methods (e.g. "P99 11ms is
/// 11x P50 1ms" is not a meaningful tail — it's sub-millisecond jitter). Also
/// requiring the absolute P99 to clear this floor keeps the ratio check
/// meaningful only where the tail itself is large enough to matter.
const MIN_ABSOLUTE_P99_MS: f64 = 50.0;

/// Flags every method with at least one recorded failure.
fn rpc_failure_candidates(runs: &[RunData]) -> Vec<Finding> {
    build_matrix(runs)
        .into_iter()
        .filter(|row| {
            matches!(
                row.status,
                MatrixStatus::ExercisedPartialFailure | MatrixStatus::ExercisedAllFailed
            )
        })
        .map(|row| Finding {
            category: FindingCategory::RpcFailure,
            summary: format!(
                "{}: {}/{} calls succeeded ({})",
                row.method, row.successes, row.calls, row.status
            ),
            evidence: vec![format!(
                "error codes observed: {:?}; backend(s): {:?}",
                row.error_codes, row.observed_backends
            )],
        })
        .collect()
}

/// Flags any observed timeout, grouped by (flow type, timeout stage).
fn timeout_candidates(runs: &[RunData]) -> Vec<Finding> {
    let mut by_key: HashMap<(String, String), u64> = HashMap::new();
    for run in runs {
        for intent in &run.intents {
            if intent.outcome != "timed_out" {
                continue;
            }
            let flow = format!("{:?}", intent.flow_type);
            let stage = intent
                .timeout_context
                .as_deref()
                .map(|c| {
                    if c.starts_with("operation ") {
                        "async operation (ZK proving) wait".to_string()
                    } else if c.starts_with("tx ") {
                        "on-chain confirmation wait".to_string()
                    } else {
                        "other".to_string()
                    }
                })
                .unwrap_or_else(|| "unknown".to_string());
            *by_key.entry((flow, stage)).or_default() += 1;
        }
    }
    let mut out: Vec<Finding> = by_key
        .into_iter()
        .map(|((flow, stage), count)| Finding {
            category: FindingCategory::Timeout,
            summary: format!("{count} timeout(s) for {flow} intents during {stage}"),
            evidence: vec![format!("flow_type={flow}, stage={stage}, count={count}")],
        })
        .collect();
    out.sort_by(|a, b| a.summary.cmp(&b.summary));
    out
}

/// Flags methods whose P99 latency is disproportionate to their P50 — a
/// signal of tail-latency behavior worth a closer look, not necessarily a
/// defect. Requires at least `MIN_TAIL_LATENCY_SAMPLE` calls to avoid flagging
/// noise from a single slow call.
fn latency_outlier_candidates(runs: &[RunData]) -> Vec<Finding> {
    build_matrix(runs)
        .into_iter()
        .filter(|row| row.calls >= MIN_TAIL_LATENCY_SAMPLE)
        .filter_map(|row| match (row.p50_ms, row.p99_ms) {
            (Some(p50), Some(p99))
                if p50 > 0.0
                    && p99 >= p50 * TAIL_LATENCY_MULTIPLE
                    && p99 >= MIN_ABSOLUTE_P99_MS =>
            {
                Some(Finding {
                    category: FindingCategory::LatencyOutlier,
                    summary: format!(
                        "{}: P99 ({:.0} ms) is {:.1}x P50 ({:.0} ms) over {} calls",
                        row.method,
                        p99,
                        p99 / p50,
                        p50,
                        row.calls
                    ),
                    evidence: vec![format!(
                        "p50={:.0}ms p95={:.0}ms p99={:.0}ms calls={}",
                        p50,
                        row.p95_ms.unwrap_or(0.0),
                        p99,
                        row.calls
                    )],
                })
            }
            _ => None,
        })
        .collect()
}

/// Flags a flow type whose confirmation rate is markedly below the overall
/// average across all flow types with intents recorded.
fn flow_type_disparity_candidates(runs: &[RunData]) -> Vec<Finding> {
    let mut by_flow: HashMap<String, (u64, u64)> = HashMap::new(); // (confirmed, total)
    for run in runs {
        for intent in &run.intents {
            let flow = format!("{:?}", intent.flow_type);
            let entry = by_flow.entry(flow).or_insert((0, 0));
            entry.1 += 1;
            if intent.outcome == "confirmed" {
                entry.0 += 1;
            }
        }
    }
    if by_flow.len() < 2 {
        return Vec::new(); // nothing to compare against
    }
    let overall_confirmed: u64 = by_flow.values().map(|(c, _)| c).sum();
    let overall_total: u64 = by_flow.values().map(|(_, t)| t).sum();
    if overall_total == 0 {
        return Vec::new();
    }
    let overall_rate = overall_confirmed as f64 / overall_total as f64;

    let mut out: Vec<Finding> = by_flow
        .into_iter()
        .filter(|(_, (_, total))| *total > 0)
        .filter_map(|(flow, (confirmed, total))| {
            let rate = confirmed as f64 / total as f64;
            // Flag if this flow type's rate trails the overall rate by more
            // than 20 percentage points.
            if overall_rate - rate > 0.20 {
                Some(Finding {
                    category: FindingCategory::FlowTypeDisparity,
                    summary: format!(
                        "{flow}: {confirmed}/{total} confirmed ({:.0}%) vs. {:.0}% overall across all flow types",
                        rate * 100.0,
                        overall_rate * 100.0
                    ),
                    evidence: vec![format!(
                        "flow_type={flow} confirmed={confirmed} total={total} overall_rate={:.3}",
                        overall_rate
                    )],
                })
            } else {
                None
            }
        })
        .collect();
    out.sort_by(|a, b| a.summary.cmp(&b.summary));
    out
}

/// Surfaces parse warnings (malformed JSONL lines) recorded while loading —
/// evidence gaps in the underlying data, not a claim about the system under
/// test.
fn data_completeness_candidates(runs: &[RunData]) -> Vec<Finding> {
    runs.iter()
        .filter(|r| !r.parse_warnings.is_empty())
        .map(|r| Finding {
            category: FindingCategory::DataCompleteness,
            summary: format!(
                "{}: {} malformed line(s) skipped while loading",
                r.manifest.run_id,
                r.parse_warnings.len()
            ),
            evidence: r.parse_warnings.clone(),
        })
        .collect()
}

/// Runs every candidate-detection rule over the provided runs and returns
/// the combined, unranked list. Severity, reproducibility, and attribution
/// are deliberately not assigned here.
pub fn flag_candidates(runs: &[RunData]) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(rpc_failure_candidates(runs));
    out.extend(timeout_candidates(runs));
    out.extend(latency_outlier_candidates(runs));
    out.extend(flow_type_disparity_candidates(runs));
    out.extend(data_completeness_candidates(runs));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{Backend, FlowType, IntentRecord, RpcCall};
    use crate::metrics::{RunManifest, RunTimeouts};
    use chrono::Utc;

    fn base_manifest(run_id: &str) -> RunManifest {
        RunManifest {
            run_id: run_id.into(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "abc".into(),
            zebra_commit: "z".into(),
            zaino_commit: "i".into(),
            zallet_commit: "t".into(),
            scenario_name: "smoke".into(),
            scenario_config_hash: "sha256:x".into(),
            target_tps: 1.0,
            timeouts: RunTimeouts::default(),
        }
    }

    fn run(run_id: &str, calls: Vec<RpcCall>, intents: Vec<IntentRecord>) -> RunData {
        RunData {
            run_dir: format!("/tmp/{run_id}").into(),
            manifest: base_manifest(run_id),
            rpc_calls: calls,
            intents,
            metrics: Vec::new(),
            parse_warnings: Vec::new(),
        }
    }

    fn call(
        method: &str,
        success: bool,
        latency_ms: Option<u64>,
        error_code: Option<i64>,
    ) -> RpcCall {
        RpcCall {
            call_id: "c".into(),
            run_id: "r".into(),
            method: method.to_string(),
            backend: Backend::Zallet,
            params_hash: None,
            request_at: Utc::now(),
            response_at: Some(Utc::now()),
            latency_ms,
            success,
            error_code,
            error_message: None,
        }
    }

    fn intent(flow_type: FlowType, outcome: &str, timeout_context: Option<&str>) -> IntentRecord {
        IntentRecord {
            run_id: "r".into(),
            intent_id: "i".into(),
            flow_type,
            outcome: outcome.into(),
            error: None,
            timeout_context: timeout_context.map(String::from),
            recorded_at: Utc::now(),
        }
    }

    #[test]
    fn flags_rpc_failures() {
        let r = run(
            "r1",
            vec![call("z_listunspent", false, None, Some(-20))],
            vec![],
        );
        let findings = flag_candidates(&[r]);
        assert!(findings
            .iter()
            .any(|f| f.category == FindingCategory::RpcFailure
                && f.summary.contains("z_listunspent")));
    }

    #[test]
    fn no_failure_no_rpc_finding() {
        let r = run(
            "r1",
            vec![call("getblockcount", true, Some(5), None)],
            vec![],
        );
        let findings = flag_candidates(&[r]);
        assert!(!findings
            .iter()
            .any(|f| f.category == FindingCategory::RpcFailure));
    }

    #[test]
    fn flags_timeouts_by_flow_and_stage() {
        let r = run(
            "r1",
            vec![],
            vec![intent(
                FlowType::TToZ,
                "timed_out",
                Some("operation op-1 did not complete within the deadline"),
            )],
        );
        let findings = flag_candidates(&[r]);
        let f = findings
            .iter()
            .find(|f| f.category == FindingCategory::Timeout)
            .expect("expected a timeout finding");
        assert!(f.summary.contains("async operation"));
    }

    #[test]
    fn flags_latency_outliers_above_threshold() {
        let mut calls = vec![call("z_sendmany", true, Some(10), None); 5];
        calls.push(call("z_sendmany", true, Some(200), None)); // 20x P50-ish outlier
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        assert!(findings
            .iter()
            .any(|f| f.category == FindingCategory::LatencyOutlier
                && f.summary.contains("z_sendmany")));
    }

    #[test]
    fn does_not_flag_high_ratio_when_absolute_latency_is_trivial() {
        // 1ms P50, 11ms P99: an 11x ratio, but sub-10ms jitter is not a
        // meaningful tail — must not be flagged as a latency outlier.
        let mut calls = vec![call("getrawmempool", true, Some(1), None); 12];
        calls.push(call("getrawmempool", true, Some(11), None));
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        assert!(!findings
            .iter()
            .any(|f| f.category == FindingCategory::LatencyOutlier));
    }

    #[test]
    fn does_not_flag_latency_below_sample_threshold() {
        let calls = vec![
            call("z_sendmany", true, Some(10), None),
            call("z_sendmany", true, Some(200), None),
        ];
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        assert!(!findings
            .iter()
            .any(|f| f.category == FindingCategory::LatencyOutlier));
    }

    #[test]
    fn flags_flow_type_disparity() {
        let mut intents = vec![];
        for _ in 0..10 {
            intents.push(intent(FlowType::TToT, "confirmed", None));
        }
        for _ in 0..10 {
            intents.push(intent(FlowType::ZToT, "failed", None));
        }
        let r = run("r1", vec![], intents);
        let findings = flag_candidates(&[r]);
        assert!(findings.iter().any(
            |f| f.category == FindingCategory::FlowTypeDisparity && f.summary.contains("ZToT")
        ));
    }

    #[test]
    fn no_disparity_when_only_one_flow_type_present() {
        let intents = vec![intent(FlowType::TToT, "failed", None); 5];
        let r = run("r1", vec![], intents);
        let findings = flag_candidates(&[r]);
        assert!(!findings
            .iter()
            .any(|f| f.category == FindingCategory::FlowTypeDisparity));
    }

    #[test]
    fn surfaces_parse_warnings_as_data_completeness_findings() {
        let mut r = run("r1", vec![], vec![]);
        r.parse_warnings
            .push("intents.jsonl:3: malformed line skipped: x".into());
        let findings = flag_candidates(&[r]);
        assert!(findings
            .iter()
            .any(|f| f.category == FindingCategory::DataCompleteness && f.summary.contains("r1")));
    }

    #[test]
    fn empty_runs_produce_no_findings() {
        assert!(flag_candidates(&[]).is_empty());
    }
}
