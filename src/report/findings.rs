//! Candidate-finding detection: computes objective aggregates, flags
//! statistical outliers as *candidates* for review, and assigns each a
//! simple, rate-based severity plus a reproducibility signal (how many of
//! the provided runs the pattern actually shows up in). Root-cause
//! narratives and remediation recommendations still require judgment and
//! are authored by reading this evidence, the same way the existing
//! crash-loop/spending-bug docs were produced — severity/reproducibility
//! are mechanical enough to compute automatically, root cause is not.

use std::collections::{HashMap, HashSet};

use crate::data_model::Phase;

use super::load_curve::load_degradation_candidates;
use super::loader::RunData;
use super::rpc_matrix::{build_matrix, Category, MatrixStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingCategory {
    RpcFailure,
    Timeout,
    LatencyOutlier,
    FlowTypeDisparity,
    LoadDegradation,
    DataCompleteness,
    KnownLimitation,
}

impl std::fmt::Display for FindingCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindingCategory::RpcFailure => write!(f, "RPC failure"),
            FindingCategory::Timeout => write!(f, "Timeout"),
            FindingCategory::LatencyOutlier => write!(f, "Latency outlier"),
            FindingCategory::FlowTypeDisparity => write!(f, "Flow-type disparity"),
            FindingCategory::LoadDegradation => write!(f, "Load degradation"),
            FindingCategory::DataCompleteness => write!(f, "Data completeness"),
            FindingCategory::KnownLimitation => write!(f, "Known limitation"),
        }
    }
}

/// Rate-based severity tier. Variant declaration order is deliberate: it is
/// also the `Ord` derive's ranking, so `sort_by_key(|f| f.severity)` sorts
/// High-first without a separate comparator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    High,
    Medium,
    Low,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::High => write!(f, "High"),
            Severity::Medium => write!(f, "Medium"),
            Severity::Low => write!(f, "Low"),
        }
    }
}

pub(crate) const HIGH_RATE: f64 = 0.20;
pub(crate) const MEDIUM_RATE: f64 = 0.05;

/// Simple rate-based tiering shared by every category whose signal is
/// "how often did this happen out of how many chances." Thresholds are
/// deliberately coarse — this is meant to triage a long findings list, not
/// to stand in for a human severity assessment.
fn severity_from_rate(rate: f64) -> Severity {
    if rate >= HIGH_RATE {
        Severity::High
    } else if rate >= MEDIUM_RATE {
        Severity::Medium
    } else {
        Severity::Low
    }
}

/// A flagged candidate, not a finished finding. `summary` states only what
/// was observed; `evidence` gives the specific facts a reader needs to
/// confirm severity and judge cause themselves. `severity` and the
/// run-occurrence counts in `evidence` are mechanically derived from rate/
/// ratio thresholds — they triage the list, they do not replace reading it.
#[derive(Debug, Clone)]
pub struct Finding {
    pub category: FindingCategory,
    pub severity: Severity,
    pub summary: String,
    pub evidence: Vec<String>,
    /// A short label distinguishing this finding from what its category
    /// would otherwise suggest — currently used only for a regtest
    /// block-generation latency finding, so a reader cannot mistake it for a
    /// production-relevant latency signal (see `uniformly_slow_candidates`).
    /// `None` for every other finding.
    pub context: Option<String>,
}

pub(crate) const TAIL_LATENCY_MULTIPLE: f64 = 5.0;
pub(crate) const HIGH_TAIL_LATENCY_MULTIPLE: f64 = 10.0;
pub(crate) const MIN_TAIL_LATENCY_SAMPLE: u64 = 5;
/// A P99/P50 ratio alone flags noise for very fast methods (e.g. "P99 11ms is
/// 11x P50 1ms" is not a meaningful tail — it's sub-millisecond jitter). Also
/// requiring the absolute P99 to clear this floor keeps the ratio check
/// meaningful only where the tail itself is large enough to matter.
pub(crate) const MIN_ABSOLUTE_P99_MS: f64 = 50.0;

pub(crate) const HIGH_DISPARITY_GAP: f64 = 0.40;
pub(crate) const MIN_DISPARITY_GAP: f64 = 0.20;

/// For a given method, how many of the provided runs called it at all, and
/// how many saw at least one failure — a cheap reproducibility signal: a
/// failure seen in 4/4 runs is a materially different claim than one seen
/// in 1/4, even at the same aggregate failure rate.
fn run_occurrence(runs: &[RunData], method: &str) -> (usize, usize) {
    let mut with_call = 0;
    let mut with_failure = 0;
    for run in runs {
        let mut called = false;
        let mut failed = false;
        for call in &run.rpc_calls {
            if call.method == method {
                called = true;
                if !call.success {
                    failed = true;
                }
            }
        }
        if called {
            with_call += 1;
        }
        if failed {
            with_failure += 1;
        }
    }
    (with_call, with_failure)
}

/// Flags every non-regtest-control method with at least one recorded
/// failure. Regtest-control methods (`generate` and friends) shape the test
/// rather than being part of the measured workload — see
/// docs/rpc/method-scope.md — so a failed `generate` call is excluded here
/// the same way it is excluded from the stress latency histograms.
fn rpc_failure_candidates(runs: &[RunData]) -> Vec<Finding> {
    build_matrix(runs, Phase::is_workload)
        .into_iter()
        .filter(|row| row.category != Category::RegtestControl)
        .filter(|row| {
            matches!(
                row.status,
                MatrixStatus::ExercisedPartialFailure | MatrixStatus::ExercisedAllFailed
            )
        })
        .map(|row| {
            let failure_rate = (row.calls - row.successes) as f64 / row.calls as f64;
            let (runs_with_call, runs_with_failure) = run_occurrence(runs, row.method);
            Finding {
                category: FindingCategory::RpcFailure,
                severity: severity_from_rate(failure_rate),
                summary: format!(
                    "{}: {}/{} calls succeeded ({}, {:.1}% failure rate)",
                    row.method,
                    row.successes,
                    row.calls,
                    row.status,
                    failure_rate * 100.0
                ),
                evidence: vec![
                    format!(
                        "error codes observed: {:?}; backend(s): {:?}",
                        row.error_codes, row.observed_backends
                    ),
                    format!(
                        "failed in {runs_with_failure}/{runs_with_call} run(s) that called this method"
                    ),
                ],
                context: None,
            }
        })
        .collect()
}

/// Flags any observed timeout, grouped by (flow type, timeout stage), rated
/// against how often that flow type timed out at that stage overall.
fn timeout_candidates(runs: &[RunData]) -> Vec<Finding> {
    let mut by_key: HashMap<(String, String), (u64, HashSet<String>)> = HashMap::new();
    let mut flow_totals: HashMap<String, u64> = HashMap::new();

    for run in runs {
        for intent in &run.intents {
            let flow = format!("{:?}", intent.flow_type);
            *flow_totals.entry(flow.clone()).or_default() += 1;

            if intent.outcome != "timed_out" {
                continue;
            }
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
            let entry = by_key.entry((flow, stage)).or_insert((0, HashSet::new()));
            entry.0 += 1;
            entry.1.insert(run.manifest.run_id.clone());
        }
    }

    let mut out: Vec<Finding> = by_key
        .into_iter()
        .map(|((flow, stage), (count, contributing_runs))| {
            let total = flow_totals.get(&flow).copied().unwrap_or(count).max(1);
            let rate = count as f64 / total as f64;
            Finding {
                category: FindingCategory::Timeout,
                severity: severity_from_rate(rate),
                summary: format!(
                    "{count} timeout(s) for {flow} intents during {stage} \
                     ({:.1}% of {flow} intents)",
                    rate * 100.0
                ),
                evidence: vec![
                    format!("flow_type={flow}, stage={stage}, count={count}, of_total={total}"),
                    format!("observed in {} run(s)", contributing_runs.len()),
                ],
                context: None,
            }
        })
        .collect();
    out.sort_by(|a, b| a.summary.cmp(&b.summary));
    out
}

/// Flags methods whose P99 latency is disproportionate to their P50 — a
/// signal of tail-latency behavior worth a closer look, not necessarily a
/// defect. Requires at least `MIN_TAIL_LATENCY_SAMPLE` calls to avoid flagging
/// noise from a single slow call, and excludes regtest-control methods for
/// the same reason `rpc_failure_candidates` does.
fn latency_outlier_candidates(runs: &[RunData]) -> Vec<Finding> {
    build_matrix(runs, Phase::is_workload)
        .into_iter()
        .filter(|row| row.category != Category::RegtestControl)
        .filter(|row| row.calls >= MIN_TAIL_LATENCY_SAMPLE)
        .filter_map(|row| match (row.p50_ms, row.p99_ms) {
            (Some(p50), Some(p99))
                if p50 > 0.0
                    && p99 >= p50 * TAIL_LATENCY_MULTIPLE
                    && p99 >= MIN_ABSOLUTE_P99_MS =>
            {
                let ratio = p99 / p50;
                let severity = if ratio >= HIGH_TAIL_LATENCY_MULTIPLE {
                    Severity::High
                } else {
                    Severity::Medium
                };
                let (runs_with_call, _) = run_occurrence(runs, row.method);
                Some(Finding {
                    category: FindingCategory::LatencyOutlier,
                    severity,
                    summary: format!(
                        "{}: P99 ({:.0} ms) is {:.1}x P50 ({:.0} ms) over {} calls",
                        row.method, p99, ratio, p50, row.calls
                    ),
                    evidence: vec![
                        format!(
                            "p50={:.0}ms p95={:.0}ms p99={:.0}ms calls={}",
                            p50,
                            row.p95_ms.unwrap_or(0.0),
                            p99,
                            row.calls
                        ),
                        format!("observed across {runs_with_call} run(s)"),
                    ],
                    context: None,
                })
            }
            _ => None,
        })
        .collect()
}

/// A method's P99/P50 *ratio* is blind to a method that is slow more or
/// less uniformly: "P50 11188ms, P99 25675ms" is only a 2.3x ratio — nowhere
/// near [`TAIL_LATENCY_MULTIPLE`] — even though a call that takes 11+
/// seconds more than half the time is pathological regardless of its tail
/// shape. This floor catches that case directly, on the median alone.
pub(crate) const UNIFORM_SLOWNESS_FLOOR_MS: f64 = 1000.0;

/// Flags methods whose *median* latency alone clears
/// [`UNIFORM_SLOWNESS_FLOOR_MS`] — "uniformly slow," as distinct from
/// [`latency_outlier_candidates`]'s "usually fast, occasionally slow." A
/// method already flagged there is skipped here, so a method that is both
/// uniformly slow and disproportionately tailed is reported once.
///
/// Unlike `latency_outlier_candidates`, regtest-control methods (`generate`
/// and friends) are deliberately NOT excluded: those calls don't count
/// toward the measured workload's own latency, but a control-plane call
/// that is itself pathologically slow is exactly the signal that predicts —
/// and, concurrently, can directly cause — degraded latency for every other
/// RPC method sharing the same backend (see
/// docs/concurrent-generate-pileup.md).
fn uniformly_slow_candidates(runs: &[RunData]) -> Vec<Finding> {
    build_matrix(runs, Phase::is_workload)
        .into_iter()
        .filter(|row| row.calls >= MIN_TAIL_LATENCY_SAMPLE)
        .filter_map(|row| {
            let p50 = row.p50_ms?;
            if p50 < UNIFORM_SLOWNESS_FLOOR_MS {
                return None;
            }
            // Already covered by the ratio-based check above.
            if let Some(p99) = row.p99_ms {
                if p50 > 0.0 && p99 >= p50 * TAIL_LATENCY_MULTIPLE && p99 >= MIN_ABSOLUTE_P99_MS {
                    return None;
                }
            }
            let (runs_with_call, _) = run_occurrence(runs, row.method);
            // Regtest-control methods (`generate` and friends) are the one
            // place this function *includes* rather than excludes the
            // category — a slow `generate` is a real operational signal
            // (see docs/concurrent-generate-pileup.md), but it is not a
            // production latency signal, and a reader must not mistake it
            // for one.
            let context = (row.category == Category::RegtestControl).then(|| {
                "regtest block-generation latency (not a production latency signal — see \
                 docs/concurrent-generate-pileup.md)"
                    .to_string()
            });
            Some(Finding {
                category: FindingCategory::LatencyOutlier,
                severity: Severity::High,
                summary: format!(
                    "{}: uniformly slow — P50 ({:.0} ms) alone clears the {:.0} ms floor over {} calls (P99 {:.0} ms)",
                    row.method,
                    p50,
                    UNIFORM_SLOWNESS_FLOOR_MS,
                    row.calls,
                    row.p99_ms.unwrap_or(p50)
                ),
                evidence: vec![
                    format!(
                        "p50={:.0}ms p95={:.0}ms p99={:.0}ms calls={}",
                        p50,
                        row.p95_ms.unwrap_or(0.0),
                        row.p99_ms.unwrap_or(0.0),
                        row.calls
                    ),
                    format!("observed across {runs_with_call} run(s)"),
                ],
                context,
            })
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
            let gap = overall_rate - rate;
            // Flag if this flow type's rate trails the overall rate by more
            // than MIN_DISPARITY_GAP percentage points.
            if gap > MIN_DISPARITY_GAP {
                let severity = if gap >= HIGH_DISPARITY_GAP {
                    Severity::High
                } else {
                    Severity::Medium
                };
                Some(Finding {
                    category: FindingCategory::FlowTypeDisparity,
                    severity,
                    summary: format!(
                        "{flow}: {confirmed}/{total} confirmed ({:.0}%) vs. {:.0}% overall across all flow types",
                        rate * 100.0,
                        overall_rate * 100.0
                    ),
                    evidence: vec![format!(
                        "flow_type={flow} confirmed={confirmed} total={total} overall_rate={:.3}",
                        overall_rate
                    )],
                    context: None,
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
/// test. Always `Low`: this category flags incomplete evidence, not a
/// defect, so it never competes with real findings for a reader's attention.
fn data_completeness_candidates(runs: &[RunData]) -> Vec<Finding> {
    runs.iter()
        .filter(|r| !r.parse_warnings.is_empty())
        .map(|r| Finding {
            category: FindingCategory::DataCompleteness,
            severity: Severity::Low,
            summary: format!(
                "{}: {} malformed line(s) skipped while loading",
                r.manifest.run_id,
                r.parse_warnings.len()
            ),
            evidence: r.parse_warnings.clone(),
            context: None,
        })
        .collect()
}

/// One known, harness-tolerated defect that must never surface as a fresh
/// `RpcFailure` finding. Matching on method name alone is deliberately not
/// enough: `z_listunspent` is issued from two call sites with materially
/// different risk profiles — the unfiltered call in `lifecycle::warmup`
/// (the actual known defect below) and `z_list_unspent_for_addresses`'s
/// filtered call in `run_sweep` (added specifically to *avoid* that
/// defect), both recorded under the identical method string. A candidate
/// call must match all three of `method`, `phase`, and `error_substring` to
/// be excluded from ordinary scoring — a method-only or phase-only match
/// still flows into `rpc_failure_candidates` unaffected, so a genuinely new
/// failure sharing just the method or the phase is never masked.
///
/// Add to this list only when a defect is (a) already tolerated in runner
/// code, (b) documented in `docs/`, and (c) precise enough that a call
/// matching all three fields is unambiguously *this* known defect.
struct KnownLimitation {
    method: &'static str,
    phase: Phase,
    error_substring: &'static str,
    explanation: &'static str,
}

const KNOWN_LIMITATIONS: &[KnownLimitation] = &[KnownLimitation {
    method: "z_listunspent",
    phase: Phase::Warmup,
    error_substring: "get_memo",
    explanation: "non-UTF8 shielded-coinbase memo bytes fail WalletDb::get_memo wallet-wide \
                  during warmup — the account fan-out that follows still proves spendability; \
                  see docs/regtest-funding-plan.md",
}];

/// Flags every `KNOWN_LIMITATIONS` entry actually observed in the provided
/// runs as a `Low`-severity `KnownLimitation` finding, so the defect is
/// surfaced explicitly rather than silently absent from the report — never
/// as a fresh `High`-severity `RpcFailure` candidate. Operates directly on
/// raw `RpcCall` rows (not `build_matrix`'s aggregates), since matching
/// requires the per-call error message `MatrixRow` does not retain.
fn known_limitation_findings(runs: &[RunData]) -> Vec<Finding> {
    KNOWN_LIMITATIONS
        .iter()
        .filter_map(|limitation| {
            let mut matching_calls = 0u64;
            let mut matching_runs: HashSet<&str> = HashSet::new();
            for run in runs {
                for call in &run.rpc_calls {
                    if call.method != limitation.method || call.phase != limitation.phase {
                        continue;
                    }
                    let Some(msg) = &call.error_message else {
                        continue;
                    };
                    if msg.contains(limitation.error_substring) {
                        matching_calls += 1;
                        matching_runs.insert(run.manifest.run_id.as_str());
                    }
                }
            }
            if matching_calls == 0 {
                return None;
            }
            Some(Finding {
                category: FindingCategory::KnownLimitation,
                severity: Severity::Low,
                summary: format!(
                    "{}: {matching_calls} known, tolerated {:?}-phase failure(s) — {}",
                    limitation.method, limitation.phase, limitation.explanation
                ),
                evidence: vec![format!(
                    "observed in {} run(s), matched on method={}, phase={:?}, error substring=\"{}\"",
                    matching_runs.len(),
                    limitation.method,
                    limitation.phase,
                    limitation.error_substring
                )],
                context: None,
            })
        })
        .collect()
}

/// Runs every candidate-detection rule over the provided runs and returns
/// the combined, unranked (by category) list — each `Finding` carries its
/// own `severity` for the caller to sort/filter by. Root cause and
/// remediation recommendations are still not assigned here; those require
/// judgment and are authored separately, same as before.
pub fn flag_candidates(runs: &[RunData]) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(rpc_failure_candidates(runs));
    out.extend(timeout_candidates(runs));
    out.extend(latency_outlier_candidates(runs));
    out.extend(uniformly_slow_candidates(runs));
    out.extend(flow_type_disparity_candidates(runs));
    out.extend(load_degradation_candidates(runs));
    out.extend(data_completeness_candidates(runs));
    out.extend(known_limitation_findings(runs));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{Backend, FlowType, IntentRecord, RpcCall};
    use crate::metrics::{RunManifest, RunTimeouts, StateIdentifier};
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
            phase_boundaries: Vec::new(),
            load_and_drain_completed_at: None,
            compose_config_hash: String::new(),
            image_digests: Vec::new(),
            host_cpu_count: 0,
            host_memory_limit_bytes: None,
            state: StateIdentifier::default(),
            assertion: None,
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
            phase: crate::data_model::Phase::Load,
            intent_id: None,
            attempt_number: 1,
        }
    }

    fn call_with_phase_and_error(
        method: &str,
        phase: crate::data_model::Phase,
        error_message: &str,
    ) -> RpcCall {
        RpcCall {
            call_id: "c".into(),
            run_id: "r".into(),
            method: method.to_string(),
            backend: Backend::Zallet,
            params_hash: None,
            request_at: Utc::now(),
            response_at: Some(Utc::now()),
            latency_ms: None,
            success: false,
            error_code: Some(-20),
            error_message: Some(error_message.to_string()),
            phase,
            intent_id: None,
            attempt_number: 1,
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
            failure_class: None,
        }
    }

    #[test]
    fn severity_sorts_high_first() {
        let mut sevs = vec![Severity::Low, Severity::High, Severity::Medium];
        sevs.sort();
        assert_eq!(sevs, vec![Severity::High, Severity::Medium, Severity::Low]);
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
    fn rpc_failure_severity_matches_failure_rate() {
        // 1 failure out of 20 calls = 5% => Medium.
        let mut calls = vec![call("z_sendmany", true, Some(10), None); 19];
        calls.push(call("z_sendmany", false, None, Some(-4)));
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        let f = findings
            .iter()
            .find(|f| f.category == FindingCategory::RpcFailure)
            .expect("expected an rpc failure finding");
        assert_eq!(f.severity, Severity::Medium);
    }

    #[test]
    fn rpc_failure_high_severity_above_20_percent() {
        let mut calls = vec![call("z_sendmany", true, Some(10), None); 3];
        calls.push(call("z_sendmany", false, None, Some(-4)));
        let r = run("r1", calls, vec![]); // 1/4 = 25% failure
        let findings = flag_candidates(&[r]);
        let f = findings
            .iter()
            .find(|f| f.category == FindingCategory::RpcFailure)
            .expect("expected an rpc failure finding");
        assert_eq!(f.severity, Severity::High);
    }

    #[test]
    fn rpc_failure_evidence_reports_run_occurrence() {
        let r1 = run(
            "r1",
            vec![call("z_sendmany", false, None, Some(-4))],
            vec![],
        );
        let r2 = run("r2", vec![call("z_sendmany", true, Some(10), None)], vec![]);
        let findings = flag_candidates(&[r1, r2]);
        let f = findings
            .iter()
            .find(|f| f.category == FindingCategory::RpcFailure)
            .expect("expected an rpc failure finding");
        assert!(
            f.evidence.iter().any(|e| e.contains("1/2 run(s)")),
            "evidence: {:?}",
            f.evidence
        );
    }

    #[test]
    fn regtest_control_failures_are_not_flagged() {
        let r = run("r1", vec![call("generate", false, None, Some(-1))], vec![]);
        let findings = flag_candidates(&[r]);
        assert!(!findings
            .iter()
            .any(|f| f.category == FindingCategory::RpcFailure));
    }

    #[test]
    fn known_z_listunspent_warmup_error_produces_low_severity_known_limitation_not_high_rpc_failure(
    ) {
        let r = run(
            "r1",
            vec![call_with_phase_and_error(
                "z_listunspent",
                crate::data_model::Phase::Warmup,
                "WalletDb::get_memo failed / Invalid UTF-8: invalid utf-8 sequence",
            )],
            vec![],
        );
        let findings = flag_candidates(&[r]);
        let known = findings
            .iter()
            .find(|f| f.category == FindingCategory::KnownLimitation)
            .expect("expected a KnownLimitation finding");
        assert_eq!(known.severity, Severity::Low);
        assert!(
            !findings
                .iter()
                .any(|f| f.category == FindingCategory::RpcFailure
                    && f.summary.contains("z_listunspent")),
            "the known warmup defect must not also surface as an RpcFailure finding"
        );
    }

    #[test]
    fn load_phase_z_listunspent_failure_is_not_masked_as_known_limitation() {
        // Same method, but Phase::Load (as run_sweep's filtered call would
        // be) with an unrelated error — must flow into ordinary RpcFailure
        // scoring, not be silently downgraded to a known limitation.
        let r = run(
            "r1",
            vec![call_with_phase_and_error(
                "z_listunspent",
                crate::data_model::Phase::Load,
                "connection refused",
            )],
            vec![],
        );
        let findings = flag_candidates(&[r]);
        assert!(!findings
            .iter()
            .any(|f| f.category == FindingCategory::KnownLimitation));
        assert!(findings
            .iter()
            .any(|f| f.category == FindingCategory::RpcFailure
                && f.summary.contains("z_listunspent")));
    }

    #[test]
    fn load_phase_z_listunspent_failure_with_the_known_error_substring_is_still_not_masked() {
        // Phase alone must disqualify a match even when the error substring
        // happens to coincide — all three of method, phase, and error
        // substring are required together.
        let r = run(
            "r1",
            vec![call_with_phase_and_error(
                "z_listunspent",
                crate::data_model::Phase::Load,
                "WalletDb::get_memo failed / Invalid UTF-8: invalid utf-8 sequence",
            )],
            vec![],
        );
        let findings = flag_candidates(&[r]);
        assert!(!findings
            .iter()
            .any(|f| f.category == FindingCategory::KnownLimitation));
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
        assert_eq!(f.severity, Severity::High); // 1/1 = 100%
    }

    #[test]
    fn flags_latency_outliers_above_threshold() {
        let mut calls = vec![call("z_sendmany", true, Some(10), None); 5];
        calls.push(call("z_sendmany", true, Some(200), None)); // 20x P50-ish outlier
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        let f = findings
            .iter()
            .find(|f| {
                f.category == FindingCategory::LatencyOutlier && f.summary.contains("z_sendmany")
            })
            .expect("expected a latency outlier finding");
        assert_eq!(f.severity, Severity::High); // ratio >= 10x
    }

    #[test]
    fn latency_outlier_medium_severity_below_10x_ratio() {
        let mut calls = vec![call("z_sendmany", true, Some(20), None); 5];
        calls.push(call("z_sendmany", true, Some(150), None)); // 7.5x, >=5 but <10
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        let f = findings
            .iter()
            .find(|f| f.category == FindingCategory::LatencyOutlier)
            .expect("expected a latency outlier finding");
        assert_eq!(f.severity, Severity::Medium);
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
    fn flags_uniformly_slow_methods_even_with_a_low_ratio() {
        // P50 11000ms, P99 12000ms: ~1.1x ratio, nowhere near the 5x tail
        // threshold, but every call takes over 11 seconds — exactly the
        // `generate`-under-concurrency case the ratio check missed.
        let mut calls = vec![call("generate", true, Some(11000), None); 5];
        calls.push(call("generate", true, Some(12000), None));
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        let f = findings
            .iter()
            .find(|f| {
                f.category == FindingCategory::LatencyOutlier && f.summary.contains("generate")
            })
            .expect("expected a latency outlier finding for a uniformly slow method");
        assert_eq!(f.severity, Severity::High);
        assert!(
            f.summary.contains("uniformly slow"),
            "summary: {}",
            f.summary
        );
    }

    #[test]
    fn regtest_control_methods_are_not_exempt_from_the_uniform_slowness_floor() {
        // regtest_control_failures_are_not_flagged (above) confirms `generate`
        // failures are excluded from RpcFailure findings — that exclusion must
        // NOT extend to this check: a pathologically slow `generate` predicts
        // (and can cause) degraded latency for every other RPC method sharing
        // its backend, so it is exactly the signal worth surfacing here.
        let calls = vec![call("generate", true, Some(15000), None); 6];
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        assert!(findings.iter().any(|f| {
            f.category == FindingCategory::LatencyOutlier && f.summary.contains("generate")
        }));
    }

    #[test]
    fn generate_latency_finding_carries_regtest_context_label() {
        let calls = vec![call("generate", true, Some(15000), None); 6];
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        let f = findings
            .iter()
            .find(|f| {
                f.category == FindingCategory::LatencyOutlier && f.summary.contains("generate")
            })
            .expect("expected a latency outlier finding for generate");
        let ctx = f
            .context
            .as_deref()
            .expect("a regtest-control latency finding must carry a context label");
        assert!(
            ctx.contains("not a production latency signal"),
            "context: {ctx}"
        );
    }

    #[test]
    fn non_regtest_control_latency_findings_carry_no_context() {
        let mut calls = vec![call("z_sendmany", true, Some(1200), None); 5];
        calls.push(call("z_sendmany", true, Some(15000), None));
        let r = run("r1", calls, vec![]);
        let findings = flag_candidates(&[r]);
        let f = findings
            .iter()
            .find(|f| {
                f.category == FindingCategory::LatencyOutlier && f.summary.contains("z_sendmany")
            })
            .expect("expected a latency outlier finding for z_sendmany");
        assert!(f.context.is_none());
    }

    #[test]
    fn does_not_double_flag_a_method_caught_by_both_checks() {
        // P50 1200ms (clears the uniform-slowness floor), P99 15000ms (12.5x
        // ratio, clears the tail-latency threshold too) — already reported by
        // latency_outlier_candidates; uniformly_slow_candidates must not add
        // a second, redundant finding for the same method.
        let mut calls = vec![call("z_sendmany", true, Some(1200), None); 5];
        calls.push(call("z_sendmany", true, Some(15000), None));
        let r = run("r1", calls, vec![]);
        let findings: Vec<_> = flag_candidates(&[r])
            .into_iter()
            .filter(|f| {
                f.category == FindingCategory::LatencyOutlier && f.summary.contains("z_sendmany")
            })
            .collect();
        assert_eq!(findings.len(), 1, "findings: {findings:?}");
    }

    #[test]
    fn does_not_flag_uniform_slowness_below_the_floor() {
        let calls = vec![call("z_gettotalbalance", true, Some(500), None); 6];
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
        let f = findings
            .iter()
            .find(|f| {
                f.category == FindingCategory::FlowTypeDisparity && f.summary.contains("ZToT")
            })
            .expect("expected a flow type disparity finding");
        assert_eq!(f.severity, Severity::High); // 100% vs 50% overall = 50pp gap
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
        let f = findings
            .iter()
            .find(|f| f.category == FindingCategory::DataCompleteness && f.summary.contains("r1"))
            .expect("expected a data completeness finding");
        assert_eq!(f.severity, Severity::Low);
    }

    #[test]
    fn empty_runs_produce_no_findings() {
        assert!(flag_candidates(&[]).is_empty());
    }
}
