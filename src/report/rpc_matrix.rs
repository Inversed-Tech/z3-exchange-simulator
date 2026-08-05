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

use std::collections::HashSet;

use super::loader::RunData;

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
/// method across every provided run.
pub fn build_matrix(runs: &[RunData]) -> Vec<MatrixRow> {
    IN_SCOPE_METHODS
        .iter()
        .map(|entry| {
            let mut calls = 0u64;
            let mut successes = 0u64;
            let mut backends: HashSet<String> = HashSet::new();
            let mut error_codes: HashSet<i64> = HashSet::new();
            let mut latencies: Vec<u64> = Vec::new();

            for run in runs {
                for call in &run.rpc_calls {
                    if call.method != entry.method {
                        continue;
                    }
                    calls += 1;
                    if call.success {
                        successes += 1;
                    }
                    backends.insert(format!("{:?}", call.backend));
                    if let Some(code) = call.error_code {
                        error_codes.insert(code);
                    }
                    if let Some(ms) = call.latency_ms {
                        latencies.push(ms);
                    }
                }
            }

            let status = if calls == 0 {
                MatrixStatus::NotTested
            } else if successes == calls {
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

            MatrixRow {
                method: entry.method,
                backend_label: entry.backend_label,
                category: entry.category,
                status,
                calls,
                successes,
                observed_backends,
                error_codes,
                p50_ms: p50,
                p95_ms: p95,
                p99_ms: p99,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{Backend, IntentRecord, RpcCall};
    use crate::metrics::{RunManifest, RunTimeouts};
    use chrono::Utc;
    use std::path::Path;

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
        }
    }

    #[test]
    fn build_matrix_has_one_row_per_roster_method() {
        let matrix = build_matrix(&[]);
        assert_eq!(matrix.len(), IN_SCOPE_METHODS.len());
    }

    #[test]
    fn build_matrix_marks_unobserved_methods_not_tested() {
        let matrix = build_matrix(&[]);
        assert!(matrix.iter().all(|r| r.status == MatrixStatus::NotTested));
        assert!(matrix.iter().all(|r| r.calls == 0));
    }

    #[test]
    fn build_matrix_marks_all_success_correctly() {
        let run = run_with_calls(vec![call("getblockcount", true, Some(5), None)]);
        let matrix = build_matrix(&[run]);
        let row = matrix.iter().find(|r| r.method == "getblockcount").unwrap();
        assert_eq!(row.status, MatrixStatus::ExercisedAllSuccess);
        assert_eq!(row.calls, 1);
        assert_eq!(row.successes, 1);
        assert_eq!(row.p50_ms, Some(5.0));
    }

    #[test]
    fn build_matrix_marks_partial_failure_correctly() {
        let run = run_with_calls(vec![
            call("z_sendmany", true, Some(10), None),
            call("z_sendmany", false, None, Some(-4)),
        ]);
        let matrix = build_matrix(&[run]);
        let row = matrix.iter().find(|r| r.method == "z_sendmany").unwrap();
        assert_eq!(row.status, MatrixStatus::ExercisedPartialFailure);
        assert_eq!(row.calls, 2);
        assert_eq!(row.successes, 1);
        assert_eq!(row.error_codes, vec![-4]);
    }

    #[test]
    fn build_matrix_marks_all_failed_correctly() {
        let run = run_with_calls(vec![call("z_listunspent", false, None, Some(-20))]);
        let matrix = build_matrix(&[run]);
        let row = matrix.iter().find(|r| r.method == "z_listunspent").unwrap();
        assert_eq!(row.status, MatrixStatus::ExercisedAllFailed);
    }

    #[test]
    fn build_matrix_aggregates_across_multiple_runs() {
        let run1 = run_with_calls(vec![call("getblockcount", true, Some(5), None)]);
        let run2 = run_with_calls(vec![call("getblockcount", true, Some(15), None)]);
        let matrix = build_matrix(&[run1, run2]);
        let row = matrix.iter().find(|r| r.method == "getblockcount").unwrap();
        assert_eq!(row.calls, 2);
        assert_eq!(row.p50_ms, Some(15.0));
    }

    #[test]
    fn build_matrix_ignores_calls_to_methods_outside_the_roster() {
        let run = run_with_calls(vec![call("some_unknown_method", true, Some(1), None)]);
        let matrix = build_matrix(&[run]);
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
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/docs/rpc/rpc-coverage-matrix.md"
        ));
        let content = std::fs::read_to_string(path)
            .expect("docs/rpc/rpc-coverage-matrix.md must be readable");
        let matrix_start = content
            .find("## Matrix")
            .expect("rpc-coverage-matrix.md must have a ## Matrix section");
        let matrix_end = content
            .find("## Removed or replaced from zcashd")
            .expect("rpc-coverage-matrix.md must have a ## Removed or replaced section");
        let section = &content[matrix_start..matrix_end];

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
        let roster: HashSet<String> = IN_SCOPE_METHODS.iter().map(|e| e.method.to_string()).collect();

        let missing_from_roster: Vec<&String> = documented.difference(&roster).collect();
        let extra_in_roster: Vec<&String> = roster.difference(&documented).collect();

        assert!(
            missing_from_roster.is_empty() && extra_in_roster.is_empty(),
            "IN_SCOPE_METHODS has drifted from docs/rpc/rpc-coverage-matrix.md — \
             documented but missing from roster: {missing_from_roster:?}; \
             in roster but not documented: {extra_in_roster:?}"
        );
    }
}
