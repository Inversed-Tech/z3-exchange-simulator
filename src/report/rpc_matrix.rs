//! RPC compatibility matrix, derived from real run evidence rather than
//! hand-maintained. `docs/rpc/rpc-coverage-matrix.md`'s `Tested?` column went
//! stale once (it asserted "no method has been tested" against 16 runs that
//! contradicted it) — deriving `Tested?`/success-rate/latency mechanically
//! from `rpc_calls.jsonl` avoids that recurring.
//!
//! The in-scope method roster below is transcribed from
//! `docs/rpc/rpc-coverage-matrix.md` (56 methods; the gRPC-only mempool
//! notification mechanisms and `decoderawtransaction`, which that doc marks
//! "not in the Foundation's confirmed list", are excluded). Columns this
//! module cannot derive from data alone — zcashd equivalence, parity
//! deviations — stay hand-maintained in that doc; this module only produces
//! `Tested?` / success-failure counts / latency.

use std::collections::{HashMap, HashSet};

use crate::data_model::{Phase, RpcCall};

use super::loader::RunData;

/// Collapses same-`(run, intent)` retry groups within one method's calls down
/// to a single representative call each — the attempt with the highest
/// `attempt_number`, whose `success` is that retry sequence's terminal
/// outcome. Without this, aggregating N attempts of one intent's `z_sendmany`
/// (e.g. two anchor-confirmation retries that failed before a third attempt
/// succeeded) counts N-1 spurious failures alongside the one call that
/// actually mattered — reproducing the client's own "101 calls, 28 failed
/// retries → 27.7%" artifact. Calls with `intent_id: None` (not tied to a
/// specific intent — health checks, warmup polling, the funding-phase
/// fan-out, background mining, ...) pass through unchanged, one row each.
///
/// `calls` must already be filtered to one method and one phase scope by the
/// caller; the `run_id` alongside each call keys the grouping so intents from
/// different runs (which may reuse the same intent_id value) never collapse
/// into each other.
fn representative_calls<'a>(
    calls: impl Iterator<Item = (&'a str, &'a RpcCall)>,
) -> Vec<&'a RpcCall> {
    let mut grouped: HashMap<(&'a str, &'a str), &'a RpcCall> = HashMap::new();
    let mut unlinked: Vec<&'a RpcCall> = Vec::new();
    for (run_id, call) in calls {
        match call.intent_id.as_deref() {
            Some(intent_id) => {
                grouped
                    .entry((run_id, intent_id))
                    .and_modify(|existing| {
                        if call.attempt_number > existing.attempt_number {
                            *existing = call;
                        }
                    })
                    .or_insert(call);
            }
            None => unlinked.push(call),
        }
    }
    let mut out: Vec<&RpcCall> = grouped.into_values().collect();
    out.extend(unlinked);
    out
}

/// Aggregate statistics over an already-deduplicated slice of calls (see
/// `representative_calls`), shared by both `build_matrix` (roster methods)
/// and `build_unlisted` (off-roster methods) so the two never compute status/
/// percentile logic two different ways.
struct CallAggregate {
    status: MatrixStatus,
    calls: u64,
    successes: u64,
    observed_backends: Vec<String>,
    error_codes: Vec<i64>,
    p50_ms: Option<f64>,
    p95_ms: Option<f64>,
    p99_ms: Option<f64>,
}

fn aggregate(calls: &[&RpcCall]) -> CallAggregate {
    let total = calls.len() as u64;
    let successes = calls.iter().filter(|c| c.success).count() as u64;
    let mut backends: HashSet<String> = HashSet::new();
    let mut error_codes: HashSet<i64> = HashSet::new();
    let mut latencies: Vec<u64> = Vec::new();
    for call in calls {
        backends.insert(format!("{:?}", call.backend));
        if let Some(code) = call.error_code {
            error_codes.insert(code);
        }
        if let Some(ms) = call.latency_ms {
            latencies.push(ms);
        }
    }

    let status = if total == 0 {
        MatrixStatus::NotTested
    } else if successes == total {
        MatrixStatus::ExercisedAllSuccess
    } else if successes == 0 {
        MatrixStatus::ExercisedAllFailed
    } else {
        MatrixStatus::ExercisedPartialFailure
    };

    let (p50, p95, p99) = if latencies.is_empty() {
        (None, None, None)
    } else {
        latencies.sort_unstable();
        (
            Some(percentile_value(&latencies, 0.50)),
            Some(percentile_value(&latencies, 0.95)),
            Some(percentile_value(&latencies, 0.99)),
        )
    };

    let mut observed_backends: Vec<String> = backends.into_iter().collect();
    observed_backends.sort();
    let mut error_codes: Vec<i64> = error_codes.into_iter().collect();
    error_codes.sort_unstable();

    CallAggregate {
        status,
        calls: total,
        successes,
        observed_backends,
        error_codes,
        p50_ms: p50,
        p95_ms: p95,
        p99_ms: p99,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Stress,
    Smoke,
    RegtestControl,
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Category::Stress => write!(f, "Stress"),
            Category::Smoke => write!(f, "Smoke"),
            Category::RegtestControl => write!(f, "Regtest-control"),
        }
    }
}

pub struct RosterEntry {
    pub method: &'static str,
    /// Documentation label only (e.g. "Zebra + Zallet" for router-dispatched
    /// dual-backend methods) — matching against observed calls is by method
    /// name alone, since the *actually* recorded `backend` for a dual-backend
    /// method has changed at least once (see regtest-funding-plan.md,
    /// z_listunifiedreceivers re-routed to Zallet).
    pub backend_label: &'static str,
    pub category: Category,
}

pub const IN_SCOPE_METHODS: &[RosterEntry] = &[
    RosterEntry {
        method: "getblockchaininfo",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getblockcount",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getbestblockhash",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getbestblockheightandhash",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getblock",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getblockhash",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getblockheader",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getrawtransaction",
        backend_label: "Zebra + Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "gettxout",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getaddressbalance",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getaddresstxids",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getaddressutxos",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getrawmempool",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getmempoolinfo",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getblocktemplate",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "submitblock",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getblocksubsidy",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "getdifficulty",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "getmininginfo",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "getnetworkhashps",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "getnetworksolps",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "getpeerinfo",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getnetworkinfo",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "getinfo",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "addnode",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "ping",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "generate",
        backend_label: "Zebra",
        category: Category::RegtestControl,
    },
    RosterEntry {
        method: "invalidateblock",
        backend_label: "Zebra",
        category: Category::RegtestControl,
    },
    RosterEntry {
        method: "reconsiderblock",
        backend_label: "Zebra",
        category: Category::RegtestControl,
    },
    RosterEntry {
        method: "z_gettreestate",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_getsubtreesbyindex",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "validateaddress",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "z_validateaddress",
        backend_label: "Zebra",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "z_listunifiedreceivers",
        backend_label: "Zebra + Zallet",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "sendrawtransaction",
        backend_label: "Zebra",
        category: Category::Stress,
    },
    RosterEntry {
        method: "rpc.discover",
        backend_label: "Zebra + Zallet",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "stop",
        backend_label: "Zebra + Zallet",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "z_getnewaccount",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_getaddressforaccount",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_listaccounts",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_getaccount",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "listaddresses",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_gettotalbalance",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_sendmany",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_getoperationstatus",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_getoperationresult",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_listoperationids",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_listunspent",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_listtransactions",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_getnotescount",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_viewtransaction",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "z_recoveraccounts",
        backend_label: "Zallet",
        category: Category::Stress,
    },
    RosterEntry {
        method: "getwalletinfo",
        backend_label: "Zallet",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "walletlock",
        backend_label: "Zallet",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "walletpassphrase",
        backend_label: "Zallet",
        category: Category::Smoke,
    },
    RosterEntry {
        method: "help",
        backend_label: "Zallet",
        category: Category::Smoke,
    },
];

/// Absolute path to the hand-maintained coverage-matrix doc, anchored at
/// compile time to this crate checkout — the same pattern already used by
/// this module's own `roster_matches_documented_coverage_matrix_exactly`
/// test, extended here to production code so the rendered report can pull
/// in the doc's zcashd-parity/deviation notes rather than requiring a
/// reader to cross-reference a separate file.
const COVERAGE_MATRIX_DOC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/docs/rpc/rpc-coverage-matrix.md"
);

/// Extracts the `## Matrix` ... `## Removed or replaced from zcashd` slice
/// of the coverage-matrix doc — the region whose backtick-quoted first
/// column names are Z3 roster methods, not zcashd-only ones.
fn matrix_section(content: &str) -> Option<&str> {
    let start = content.find("## Matrix")?;
    let end = content.find("## Removed or replaced from zcashd")?;
    Some(&content[start..end])
}

/// Hand-maintained context for one method from `rpc-coverage-matrix.md`:
/// whether a zcashd equivalent exists, its behavioral parity, and any
/// caveats — columns this module cannot derive from run evidence alone (see
/// the module doc comment).
#[derive(Debug, Clone, Default)]
pub struct ParityInfo {
    pub zcashd_equiv: String,
    pub parity: String,
    pub notes: String,
}

fn parse_parity_row(line: &str) -> Option<(String, ParityInfo)> {
    let trimmed = line.trim();
    if !trimmed.starts_with("| `") {
        return None;
    }
    let cells: Vec<&str> = trimmed
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim())
        .collect();
    // | Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
    if cells.len() < 9 {
        return None;
    }
    let method = cells[0].trim_matches('`').to_string();
    Some((
        method,
        ParityInfo {
            zcashd_equiv: cells[3].to_string(),
            parity: cells[7].to_string(),
            notes: cells[8].to_string(),
        },
    ))
}

/// Loads every method's hand-maintained parity/notes context, keyed by
/// method name. Best-effort: the doc is edited by hand and may be missing,
/// moved, or reshaped — callers get an empty map rather than a hard
/// failure, since this is supplementary context, not something the report
/// can verify mechanically (never treat its absence as a correctness
/// problem with the run evidence itself).
pub fn load_parity_annotations() -> HashMap<String, ParityInfo> {
    let content = match std::fs::read_to_string(COVERAGE_MATRIX_DOC) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[report] warning: could not read {COVERAGE_MATRIX_DOC}: {e} — \
                 Parity/Notes columns will be omitted from the RPC compatibility matrix"
            );
            return HashMap::new();
        }
    };
    let Some(section) = matrix_section(&content) else {
        eprintln!(
            "[report] warning: {COVERAGE_MATRIX_DOC} is missing its ## Matrix / \
             ## Removed or replaced from zcashd sections — Parity/Notes columns will be omitted"
        );
        return HashMap::new();
    };
    section.lines().filter_map(parse_parity_row).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixStatus {
    /// No run in the provided evidence recorded any call to this method.
    NotTested,
    /// Every observed call to this method succeeded.
    ExercisedAllSuccess,
    /// At least one call succeeded and at least one failed.
    ExercisedPartialFailure,
    /// Every observed call to this method failed.
    ExercisedAllFailed,
}

impl std::fmt::Display for MatrixStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatrixStatus::NotTested => write!(f, "Not tested"),
            MatrixStatus::ExercisedAllSuccess => write!(f, "Succeeded"),
            MatrixStatus::ExercisedPartialFailure => write!(f, "Partial failure"),
            MatrixStatus::ExercisedAllFailed => write!(f, "Failed"),
        }
    }
}

pub struct MatrixRow {
    pub method: &'static str,
    pub backend_label: &'static str,
    pub category: Category,
    pub status: MatrixStatus,
    pub calls: u64,
    pub successes: u64,
    /// Distinct backends actually recorded for this method across the
    /// provided runs — may differ from `backend_label` (informational only).
    pub observed_backends: Vec<String>,
    /// Distinct non-null error codes observed, sorted ascending.
    pub error_codes: Vec<i64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
}

fn percentile_value(sorted: &[u64], p: f64) -> f64 {
    let n = sorted.len();
    let idx = ((p * n as f64).floor() as usize).min(n - 1);
    sorted[idx] as f64
}

/// Builds one row per roster method, aggregating observed calls to that
/// method across every provided run whose `phase` satisfies `phase_filter`.
/// Pass [`Phase::is_workload`] for the headline Load/Drain-scoped view, or
/// [`Phase::is_setup`] for the setup-phase appendix — see `markdown.rs`'s two
/// call sites. `Phase::Unknown` (a pre-phase-tagging run) matches neither.
pub fn build_matrix(runs: &[RunData], phase_filter: impl Fn(Phase) -> bool) -> Vec<MatrixRow> {
    IN_SCOPE_METHODS
        .iter()
        .map(|entry| {
            let filtered = runs.iter().flat_map(|run| {
                run.rpc_calls
                    .iter()
                    .filter(|call| call.method == entry.method && phase_filter(call.phase))
                    .map(move |call| (run.manifest.run_id.as_str(), call))
            });
            let reps = representative_calls(filtered);
            let agg = aggregate(&reps);

            MatrixRow {
                method: entry.method,
                backend_label: entry.backend_label,
                category: entry.category,
                status: agg.status,
                calls: agg.calls,
                successes: agg.successes,
                observed_backends: agg.observed_backends,
                error_codes: agg.error_codes,
                p50_ms: agg.p50_ms,
                p95_ms: agg.p95_ms,
                p99_ms: agg.p99_ms,
            }
        })
        .collect()
}

/// One method observed in the provided runs whose name is not part of
/// [`IN_SCOPE_METHODS`] — same shape as [`MatrixRow`] minus the roster-only
/// `backend_label`/`category` fields, since there is no roster entry to
/// pull them from.
pub struct UnlistedRow {
    pub method: String,
    pub status: MatrixStatus,
    pub calls: u64,
    pub successes: u64,
    pub observed_backends: Vec<String>,
    pub error_codes: Vec<i64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
}

/// Aggregates RPC calls whose method is *not* in [`IN_SCOPE_METHODS`],
/// scoped by `phase_filter` the same way [`build_matrix`] is — this function
/// is rendered directly under the RPC compatibility matrix, whose stated
/// scope (`Phase::is_workload` by default) it must match, or an unlisted
/// method exercised only during setup would silently appear in a section
/// whose own text claims to exclude setup-phase activity.
/// `build_matrix` silently excludes these (see its own
/// `build_matrix_ignores_calls_to_methods_outside_the_roster` test) — a real
/// method actually exercised against Zebra or Zallet but missing from the
/// Foundation's confirmed roster would otherwise be invisible in this
/// report, not even shown as "Not tested."
pub fn build_unlisted(runs: &[RunData], phase_filter: impl Fn(Phase) -> bool) -> Vec<UnlistedRow> {
    let roster: HashSet<&str> = IN_SCOPE_METHODS.iter().map(|e| e.method).collect();

    let mut by_method: HashMap<&str, Vec<(&str, &RpcCall)>> = HashMap::new();
    for run in runs {
        for call in &run.rpc_calls {
            if roster.contains(call.method.as_str()) {
                continue;
            }
            if !phase_filter(call.phase) {
                continue;
            }
            by_method
                .entry(call.method.as_str())
                .or_default()
                .push((run.manifest.run_id.as_str(), call));
        }
    }

    let mut out: Vec<UnlistedRow> = by_method
        .into_iter()
        .map(|(method, calls)| {
            let reps = representative_calls(calls.into_iter());
            let agg = aggregate(&reps);
            UnlistedRow {
                method: method.to_string(),
                status: agg.status,
                calls: agg.calls,
                successes: agg.successes,
                observed_backends: agg.observed_backends,
                error_codes: agg.error_codes,
                p50_ms: agg.p50_ms,
                p95_ms: agg.p95_ms,
                p99_ms: agg.p99_ms,
            }
        })
        .collect();
    out.sort_by(|a, b| a.method.cmp(&b.method));
    out
}

/// Per-method distribution of how many attempts a load/drain-phase intent
/// needed before its terminal `z_sendmany`-style outcome — the "Retry detail
/// by method" appendix data. Only intents linked via `intent_id` are
/// represented; a group's representative call's own `attempt_number` (see
/// `representative_calls`) already equals how many attempts that intent
/// needed, since attempts are numbered 1..N.
///
/// Computed here (Track 3), rendered by Track 5's appendix restructuring
/// (moving the RPC matrix out of the main report body) — see the Cross-Track
/// Review's "Shared changes" note on this mechanism. `#[allow(dead_code)]`
/// until that rendering call site lands and reads this data.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryDetail {
    pub method: &'static str,
    pub single_attempt: u64,
    pub two_to_three_attempts: u64,
    pub four_plus_attempts: u64,
}

#[allow(dead_code)]
pub fn build_retry_detail(
    runs: &[RunData],
    phase_filter: impl Fn(Phase) -> bool,
) -> Vec<RetryDetail> {
    IN_SCOPE_METHODS
        .iter()
        .filter_map(|entry| {
            let filtered = runs.iter().flat_map(|run| {
                run.rpc_calls
                    .iter()
                    .filter(|call| call.method == entry.method && phase_filter(call.phase))
                    .map(move |call| (run.manifest.run_id.as_str(), call))
            });
            let reps = representative_calls(filtered);
            let linked: Vec<&RpcCall> =
                reps.into_iter().filter(|c| c.intent_id.is_some()).collect();
            if linked.is_empty() {
                return None;
            }
            let mut single_attempt = 0u64;
            let mut two_to_three_attempts = 0u64;
            let mut four_plus_attempts = 0u64;
            for call in &linked {
                match call.attempt_number {
                    0 | 1 => single_attempt += 1,
                    2 | 3 => two_to_three_attempts += 1,
                    _ => four_plus_attempts += 1,
                }
            }
            Some(RetryDetail {
                method: entry.method,
                single_attempt,
                two_to_three_attempts,
                four_plus_attempts,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{Backend, IntentRecord, RpcCall};
    use crate::metrics::{RunManifest, RunTimeouts};
    use chrono::Utc;

    fn run_with_calls(calls: Vec<RpcCall>) -> RunData {
        RunData {
            run_dir: "/tmp/test".into(),
            manifest: RunManifest {
                run_id: "r".into(),
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
            },
            rpc_calls: calls,
            intents: Vec::<IntentRecord>::new(),
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
            backend: Backend::Zebra,
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

    fn call_with_phase(method: &str, success: bool, phase: Phase) -> RpcCall {
        RpcCall {
            phase,
            ..call(method, success, Some(5), None)
        }
    }

    fn call_attempt(method: &str, success: bool, intent_id: &str, attempt_number: u32) -> RpcCall {
        RpcCall {
            intent_id: Some(intent_id.to_string()),
            attempt_number,
            ..call(
                method,
                success,
                Some(5),
                if success { None } else { Some(-4) },
            )
        }
    }

    #[test]
    fn build_matrix_has_one_row_per_roster_method() {
        let matrix = build_matrix(&[], Phase::is_workload);
        assert_eq!(matrix.len(), IN_SCOPE_METHODS.len());
    }

    #[test]
    fn build_matrix_marks_unobserved_methods_not_tested() {
        let matrix = build_matrix(&[], Phase::is_workload);
        assert!(matrix.iter().all(|r| r.status == MatrixStatus::NotTested));
        assert!(matrix.iter().all(|r| r.calls == 0));
    }

    #[test]
    fn build_matrix_marks_all_success_correctly() {
        let run = run_with_calls(vec![call("getblockcount", true, Some(5), None)]);
        let matrix = build_matrix(&[run], Phase::is_workload);
        let row = matrix.iter().find(|r| r.method == "getblockcount").unwrap();
        assert_eq!(row.status, MatrixStatus::ExercisedAllSuccess);
        assert_eq!(row.calls, 1);
        assert_eq!(row.successes, 1);
        assert_eq!(row.p50_ms, Some(5.0));
    }

    #[test]
    fn build_matrix_default_scope_excludes_setup_phases() {
        // A mix of failed Funding-phase calls (the funding-fan-out's own
        // anchor-confirmation retries) and successful Load-phase calls for
        // the same method — the exact shape that would otherwise inflate
        // the headline failure rate with setup-phase retry noise.
        let run = run_with_calls(vec![
            call_with_phase("z_sendmany", false, Phase::Funding),
            call_with_phase("z_sendmany", false, Phase::Funding),
            call_with_phase("z_sendmany", true, Phase::Load),
        ]);
        let workload_matrix = build_matrix(&[run], Phase::is_workload);
        let row = workload_matrix
            .iter()
            .find(|r| r.method == "z_sendmany")
            .unwrap();
        assert_eq!(row.calls, 1, "only the Load-phase call should count");
        assert_eq!(row.status, MatrixStatus::ExercisedAllSuccess);
    }

    #[test]
    fn build_matrix_setup_scope_includes_only_setup_phases() {
        let run = run_with_calls(vec![
            call_with_phase("z_sendmany", false, Phase::Funding),
            call_with_phase("z_sendmany", true, Phase::Load),
            call_with_phase("z_sendmany", false, Phase::Unknown),
        ]);
        let setup_matrix = build_matrix(&[run], Phase::is_setup);
        let row = setup_matrix
            .iter()
            .find(|r| r.method == "z_sendmany")
            .unwrap();
        assert_eq!(row.calls, 1, "only the Funding-phase call should count");
        assert_eq!(row.status, MatrixStatus::ExercisedAllFailed);
    }

    #[test]
    fn build_matrix_unknown_phase_excluded_from_both_scopes() {
        let run = run_with_calls(vec![call_with_phase("getblockcount", true, Phase::Unknown)]);
        assert_eq!(
            build_matrix(&[run], Phase::is_workload)
                .iter()
                .find(|r| r.method == "getblockcount")
                .unwrap()
                .calls,
            0
        );
        let run = run_with_calls(vec![call_with_phase("getblockcount", true, Phase::Unknown)]);
        assert_eq!(
            build_matrix(&[run], Phase::is_setup)
                .iter()
                .find(|r| r.method == "getblockcount")
                .unwrap()
                .calls,
            0
        );
    }

    #[test]
    fn build_matrix_marks_partial_failure_correctly() {
        let run = run_with_calls(vec![
            call("z_sendmany", true, Some(10), None),
            call("z_sendmany", false, None, Some(-4)),
        ]);
        let matrix = build_matrix(&[run], Phase::is_workload);
        let row = matrix.iter().find(|r| r.method == "z_sendmany").unwrap();
        assert_eq!(row.status, MatrixStatus::ExercisedPartialFailure);
        assert_eq!(row.calls, 2);
        assert_eq!(row.successes, 1);
        assert_eq!(row.error_codes, vec![-4]);
    }

    #[test]
    fn build_matrix_marks_all_failed_correctly() {
        let run = run_with_calls(vec![call("z_listunspent", false, None, Some(-20))]);
        let matrix = build_matrix(&[run], Phase::is_workload);
        let row = matrix.iter().find(|r| r.method == "z_listunspent").unwrap();
        assert_eq!(row.status, MatrixStatus::ExercisedAllFailed);
    }

    #[test]
    fn build_matrix_aggregates_across_multiple_runs() {
        let run1 = run_with_calls(vec![call("getblockcount", true, Some(5), None)]);
        let run2 = run_with_calls(vec![call("getblockcount", true, Some(15), None)]);
        let matrix = build_matrix(&[run1, run2], Phase::is_workload);
        let row = matrix.iter().find(|r| r.method == "getblockcount").unwrap();
        assert_eq!(row.calls, 2);
        assert_eq!(row.p50_ms, Some(15.0));
    }

    #[test]
    fn test_retry_attempts_do_not_inflate_failure_rate() {
        // One intent's z_sendmany retry sequence: attempts 1-2 fail with
        // "Insufficient balance", attempt 3 succeeds — the exact shape
        // send_many_with_anchor_retries produces. Plus 3 more unlinked,
        // successful calls (health checks / other intents). The computed
        // failure rate for the method must be 0/4 (four terminal outcomes,
        // zero failures) — not 2/6 (six raw attempts, two of them failed).
        let run = run_with_calls(vec![
            call_attempt("z_sendmany", false, "i1", 1),
            call_attempt("z_sendmany", false, "i1", 2),
            call_attempt("z_sendmany", true, "i1", 3),
            call("z_sendmany", true, Some(5), None),
            call("z_sendmany", true, Some(5), None),
            call("z_sendmany", true, Some(5), None),
        ]);
        let matrix = build_matrix(&[run], Phase::is_workload);
        let row = matrix.iter().find(|r| r.method == "z_sendmany").unwrap();
        assert_eq!(
            row.calls, 4,
            "6 raw attempts must collapse to 4 terminal outcomes"
        );
        assert_eq!(row.successes, 4);
        assert_eq!(row.status, MatrixStatus::ExercisedAllSuccess);
    }

    #[test]
    fn build_matrix_representative_call_is_the_highest_attempt_number_not_first_or_last_seen() {
        // Regression guard: the representative must be selected by
        // attempt_number, not by insertion order — feed the terminal
        // (successful) attempt first to rule out "last write wins" being
        // coincidentally correct only because it's also the highest attempt.
        let run = run_with_calls(vec![
            call_attempt("z_sendmany", true, "i1", 3),
            call_attempt("z_sendmany", false, "i1", 1),
            call_attempt("z_sendmany", false, "i1", 2),
        ]);
        let matrix = build_matrix(&[run], Phase::is_workload);
        let row = matrix.iter().find(|r| r.method == "z_sendmany").unwrap();
        assert_eq!(row.calls, 1);
        assert_eq!(row.status, MatrixStatus::ExercisedAllSuccess);
    }

    #[test]
    fn build_retry_detail_buckets_by_terminal_attempt_number() {
        let run = run_with_calls(vec![
            // 1 attempt.
            call_attempt("z_sendmany", true, "i1", 1),
            // 2-3 attempts.
            call_attempt("z_sendmany", false, "i2", 1),
            call_attempt("z_sendmany", true, "i2", 2),
            // 4+ attempts.
            call_attempt("z_sendmany", false, "i3", 1),
            call_attempt("z_sendmany", false, "i3", 2),
            call_attempt("z_sendmany", false, "i3", 3),
            call_attempt("z_sendmany", true, "i3", 4),
            // Unlinked call — must not appear in any bucket.
            call("z_sendmany", true, Some(5), None),
        ]);
        let detail = build_retry_detail(&[run], Phase::is_workload);
        let row = detail.iter().find(|r| r.method == "z_sendmany").unwrap();
        assert_eq!(row.single_attempt, 1);
        assert_eq!(row.two_to_three_attempts, 1);
        assert_eq!(row.four_plus_attempts, 1);
    }

    #[test]
    fn build_retry_detail_omits_methods_with_no_linked_intents() {
        let run = run_with_calls(vec![call("getblockcount", true, Some(5), None)]);
        let detail = build_retry_detail(&[run], Phase::is_workload);
        assert!(detail.iter().all(|r| r.method != "getblockcount"));
    }

    #[test]
    fn build_matrix_ignores_calls_to_methods_outside_the_roster() {
        let run = run_with_calls(vec![call("some_unknown_method", true, Some(1), None)]);
        let matrix = build_matrix(&[run], Phase::is_workload);
        assert!(matrix.iter().all(|r| r.method != "some_unknown_method"));
    }

    #[test]
    fn roster_has_no_duplicate_methods() {
        let mut seen = HashSet::new();
        for entry in IN_SCOPE_METHODS {
            assert!(
                seen.insert(entry.method),
                "duplicate roster entry: {}",
                entry.method
            );
        }
    }

    /// Extracts every `| `method_name` | ...` first-column entry from the
    /// "## Matrix" section of `rpc-coverage-matrix.md`, stopping before
    /// "## Removed or replaced from zcashd" (whose first-column entries are
    /// zcashd methods, not Z3 ones) so those rows can't spuriously count as
    /// roster methods.
    fn methods_documented_in_coverage_matrix() -> HashSet<String> {
        let content = std::fs::read_to_string(COVERAGE_MATRIX_DOC)
            .expect("docs/rpc/rpc-coverage-matrix.md must be readable");
        let section = matrix_section(&content)
            .expect("rpc-coverage-matrix.md must have ## Matrix and ## Removed sections");

        // Explicitly excluded per the module doc comment: gRPC-only mempool
        // notification mechanisms (not JSON-RPC methods at all) and
        // `decoderawtransaction`, which the doc marks "not in the
        // Foundation's confirmed list."
        let excluded: HashSet<&str> = [
            "decoderawtransaction",
            "GetMempoolStream",
            "Indexer.mempool_change()",
        ]
        .into_iter()
        .collect();

        section
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("| `")?;
                let end = rest.find('`')?;
                Some(rest[..end].to_string())
            })
            .filter(|m| !excluded.contains(m.as_str()))
            .collect()
    }

    #[test]
    fn roster_matches_documented_coverage_matrix_exactly() {
        let documented = methods_documented_in_coverage_matrix();
        let roster: HashSet<String> = IN_SCOPE_METHODS
            .iter()
            .map(|e| e.method.to_string())
            .collect();

        let missing_from_roster: Vec<&String> = documented.difference(&roster).collect();
        let extra_in_roster: Vec<&String> = roster.difference(&documented).collect();

        assert!(
            missing_from_roster.is_empty() && extra_in_roster.is_empty(),
            "IN_SCOPE_METHODS has drifted from docs/rpc/rpc-coverage-matrix.md — \
             documented but missing from roster: {missing_from_roster:?}; \
             in roster but not documented: {extra_in_roster:?}"
        );
    }

    #[test]
    fn build_unlisted_finds_methods_outside_the_roster() {
        let run = run_with_calls(vec![
            call("z_getbalanceforaccount", true, Some(5), None),
            call("z_getbalanceforaccount", true, Some(7), None),
            call("getblockcount", true, Some(2), None), // in-roster, must not appear
        ]);
        let unlisted = build_unlisted(&[run], Phase::is_workload);
        assert_eq!(unlisted.len(), 1);
        let row = &unlisted[0];
        assert_eq!(row.method, "z_getbalanceforaccount");
        assert_eq!(row.calls, 2);
        assert_eq!(row.successes, 2);
        assert_eq!(row.status, MatrixStatus::ExercisedAllSuccess);
    }

    #[test]
    fn build_unlisted_empty_when_every_call_is_in_roster() {
        let run = run_with_calls(vec![call("getblockcount", true, Some(2), None)]);
        assert!(build_unlisted(&[run], Phase::is_workload).is_empty());
    }

    #[test]
    fn build_unlisted_default_scope_excludes_setup_phases() {
        // Regression guard: an off-roster method exercised only during setup
        // (e.g. z_getbalanceforaccount, called from warmup()/fund_accounts())
        // must not appear in the Load/Drain-scoped "Observed outside the
        // tracked roster" section nested under the headline RPC matrix.
        let run = run_with_calls(vec![call_with_phase(
            "z_getbalanceforaccount",
            true,
            Phase::Funding,
        )]);
        assert!(build_unlisted(&[run], Phase::is_workload).is_empty());
    }

    #[test]
    fn build_unlisted_setup_scope_finds_setup_only_methods() {
        // The symmetric fix: the same off-roster method must still surface
        // somewhere — in the setup-phase unlisted listing — so it is never
        // silently invisible just because it wasn't exercised during Load/Drain.
        let run = run_with_calls(vec![call_with_phase(
            "z_getbalanceforaccount",
            true,
            Phase::Funding,
        )]);
        let unlisted = build_unlisted(&[run], Phase::is_setup);
        assert_eq!(unlisted.len(), 1);
        assert_eq!(unlisted[0].method, "z_getbalanceforaccount");
    }

    #[test]
    fn build_unlisted_tracks_partial_failure_and_error_codes() {
        let run = run_with_calls(vec![
            call("z_getbalanceforaccount", true, Some(5), None),
            call("z_getbalanceforaccount", false, None, Some(-1)),
        ]);
        let unlisted = build_unlisted(&[run], Phase::is_workload);
        let row = unlisted
            .iter()
            .find(|r| r.method == "z_getbalanceforaccount")
            .unwrap();
        assert_eq!(row.status, MatrixStatus::ExercisedPartialFailure);
        assert_eq!(row.error_codes, vec![-1]);
    }

    #[test]
    fn load_parity_annotations_finds_a_known_method() {
        let annotations = load_parity_annotations();
        let entry = annotations
            .get("getblockchaininfo")
            .expect("getblockchaininfo should be documented in rpc-coverage-matrix.md");
        assert!(!entry.parity.is_empty());
    }

    #[test]
    fn parse_parity_row_extracts_expected_columns() {
        let line =
            "| `getblockchaininfo` | Zebra | Stress | Yes | N/A | Yes | Yes | TBD | Some note. |";
        let (method, info) = parse_parity_row(line).expect("should parse a matrix row");
        assert_eq!(method, "getblockchaininfo");
        assert_eq!(info.zcashd_equiv, "Yes");
        assert_eq!(info.parity, "TBD");
        assert_eq!(info.notes, "Some note.");
    }

    #[test]
    fn parse_parity_row_ignores_header_and_separator_lines() {
        assert!(parse_parity_row("| Method | Backend | Test category |").is_none());
        assert!(parse_parity_row("|---|---|---|").is_none());
    }
}
