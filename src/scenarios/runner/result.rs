//! Outcome types and run statistics for the scenario runner.

use std::collections::HashMap;

use chrono::Utc;

use crate::data_model::{
    Deposit, ExpectationsConfig, FlowType, IntentFailureClass, IntentRecord, Withdrawal,
};

/// The outcome of a single dispatched transaction intent.
#[derive(Debug)]
pub enum IntentOutcome {
    WithdrawalOk {
        withdrawal: Withdrawal,
        intent_id: String,
        flow_type: FlowType,
    },
    DepositOk {
        deposit: Deposit,
        intent_id: String,
        flow_type: FlowType,
    },
    Failed {
        intent_id: String,
        flow_type: FlowType,
        error: String,
    },
    TimedOut {
        intent_id: String,
        flow_type: FlowType,
        /// Which wait timed out, e.g. "operation <id> did not complete within
        /// the deadline" (async ZK proving) vs. "tx <id> did not reach N
        /// confirmations" — propagated from `ExchangeError::Timeout`'s own
        /// context string. Needed to tell an RPC-layer stall apart from a
        /// confirmation-depth stall when reading the findings report.
        context: String,
    },
}

impl IntentRecord {
    pub fn from_outcome(outcome: &IntentOutcome, run_id: &str) -> Self {
        let recorded_at = Utc::now();
        let run_id = run_id.to_string();
        match outcome {
            IntentOutcome::WithdrawalOk {
                intent_id,
                flow_type,
                ..
            }
            | IntentOutcome::DepositOk {
                intent_id,
                flow_type,
                ..
            } => Self {
                run_id,
                intent_id: intent_id.clone(),
                flow_type: flow_type.clone(),
                outcome: "confirmed".to_string(),
                error: None,
                timeout_context: None,
                recorded_at,
                failure_class: None,
            },
            IntentOutcome::Failed {
                intent_id,
                flow_type,
                error,
            } => Self {
                run_id,
                intent_id: intent_id.clone(),
                flow_type: flow_type.clone(),
                outcome: "failed".to_string(),
                error: Some(error.clone()),
                timeout_context: None,
                recorded_at,
                failure_class: Some(IntentFailureClass::classify(error)),
            },
            IntentOutcome::TimedOut {
                intent_id,
                flow_type,
                context,
            } => Self {
                run_id,
                intent_id: intent_id.clone(),
                flow_type: flow_type.clone(),
                outcome: "timed_out".to_string(),
                error: None,
                timeout_context: Some(context.clone()),
                recorded_at,
                failure_class: Some(IntentFailureClass::Timeout),
            },
        }
    }
}

/// The result of evaluating a run's `RunStats` against its scenario's
/// `ExpectationsConfig`. Empty `violations` iff `passed`.
#[derive(Debug, Clone)]
pub struct AssertionOutcome {
    pub passed: bool,
    /// One line per violated expectation, e.g. `"confirmed 56 < min_confirmed
    /// 60"`. Empty when `passed`.
    pub violations: Vec<String>,
}

/// Counts each terminal-failed intent once, keyed by the `IntentFailureClass`
/// recorded on its `IntentRecord`. Built directly from the run's
/// `IntentRecord`s, not from `RpcCall` rows, so retries never inflate any
/// entry.
///
/// Scoped to `outcome == "failed"` only — `RunStats::evaluate` compares this
/// map's total against `max_terminal_failures`, which is documented (and
/// intended, per `ExpectationsConfig::max_terminal_failures`) as counting
/// terminal *failures*, independently of `max_timeouts`. A `TimedOut`
/// `IntentRecord` always carries `failure_class: Some(IntentFailureClass::Timeout)`
/// too (for informational/reporting purposes), but including it here would
/// double-count every timeout against both thresholds instead of leaving
/// `max_timeouts` as the sole gate for them. Confirmed intents
/// (`failure_class: None`) are never counted regardless.
pub fn terminal_failures_by_class(records: &[IntentRecord]) -> HashMap<IntentFailureClass, u64> {
    let mut counts: HashMap<IntentFailureClass, u64> = HashMap::new();
    for record in records {
        if record.outcome != "failed" {
            continue;
        }
        if let Some(class) = record.failure_class {
            *counts.entry(class).or_insert(0) += 1;
        }
    }
    counts
}

/// Aggregate statistics collected during the load phase.
#[derive(Debug, Default)]
pub struct RunStats {
    pub total_attempted: u64,
    pub confirmed: u64,
    pub failed: u64,
    pub timed_out: u64,
}

impl RunStats {
    /// Evaluates these stats against `expectations`, honoring
    /// `allowed_error_classes` via `failures_by_class` (see
    /// `terminal_failures_by_class`) rather than `self.failed` directly —
    /// `self.failed` counts every terminal failure regardless of class, so a
    /// scenario that has pre-approved a class of failure needs the
    /// per-class breakdown to subtract it before comparing against
    /// `max_terminal_failures`.
    pub fn evaluate(
        &self,
        expectations: &ExpectationsConfig,
        failures_by_class: &HashMap<IntentFailureClass, u64>,
    ) -> AssertionOutcome {
        let mut counted: Vec<(IntentFailureClass, u64)> = Vec::new();
        let mut excluded: Vec<(IntentFailureClass, u64)> = Vec::new();
        for (class, count) in failures_by_class {
            if expectations
                .allowed_error_classes
                .iter()
                .any(|c| c == class.as_str())
            {
                excluded.push((*class, *count));
            } else {
                counted.push((*class, *count));
            }
        }
        let counted_failures: u64 = counted.iter().map(|(_, n)| n).sum();

        let mut violations = Vec::new();
        if self.confirmed < expectations.min_confirmed {
            violations.push(format!(
                "confirmed {} < min_confirmed {}",
                self.confirmed, expectations.min_confirmed
            ));
        }
        if counted_failures > expectations.max_terminal_failures {
            counted.sort_by_key(|(c, _)| c.as_str());
            let breakdown_str = counted
                .iter()
                .map(|(c, n)| format!("{n} {}", c.as_str()))
                .collect::<Vec<_>>()
                .join(", ");
            let excluded_str = if excluded.is_empty() {
                String::new()
            } else {
                excluded.sort_by_key(|(c, _)| c.as_str());
                format!(
                    "; {} excluded by allowed_error_classes",
                    excluded
                        .iter()
                        .map(|(c, n)| format!("{n} {}", c.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            violations.push(format!(
                "terminal failures {counted_failures} ({breakdown_str}{excluded_str}) > max_terminal_failures {}",
                expectations.max_terminal_failures
            ));
        }
        if self.timed_out > expectations.max_timeouts {
            violations.push(format!(
                "timed out {} > max_timeouts {}",
                self.timed_out, expectations.max_timeouts
            ));
        }

        AssertionOutcome {
            passed: violations.is_empty(),
            violations,
        }
    }
}

/// The full result returned by [`super::run`].
#[derive(Debug)]
pub struct RunResult {
    pub run_id: String,
    /// `None` for dry-run (no directory created); `Some(path)` for real runs.
    pub output_dir: Option<std::path::PathBuf>,
    pub dry_run: bool,
    pub stats: RunStats,
    pub outcomes: Vec<IntentOutcome>,
    /// Pass/fail per the scenario's `expectations` block. Always `passed:
    /// true` (no violations) for a dry run — no load phase runs, so there is
    /// nothing to evaluate.
    pub assertion: AssertionOutcome,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_stats_defaults_to_zero() {
        let s = RunStats::default();
        assert_eq!(s.total_attempted, 0);
        assert_eq!(s.confirmed, 0);
        assert_eq!(s.failed, 0);
        assert_eq!(s.timed_out, 0);
    }

    #[test]
    fn run_stats_fields_are_independent() {
        let s = RunStats {
            total_attempted: 10,
            confirmed: 7,
            failed: 2,
            timed_out: 1,
        };
        assert_eq!(s.total_attempted, 10);
        assert_eq!(s.confirmed, 7);
        assert_eq!(s.failed, 2);
        assert_eq!(s.timed_out, 1);
    }

    fn passing_assertion() -> AssertionOutcome {
        AssertionOutcome {
            passed: true,
            violations: vec![],
        }
    }

    #[test]
    fn run_result_output_dir_none_for_dry_run() {
        let r = RunResult {
            run_id: "test-id".into(),
            output_dir: None,
            dry_run: true,
            stats: RunStats::default(),
            outcomes: vec![],
            assertion: passing_assertion(),
        };
        assert!(r.output_dir.is_none());
        assert!(r.dry_run);
    }

    #[test]
    fn run_result_output_dir_some_for_live_run() {
        let path = std::path::PathBuf::from("/tmp/test-run");
        let r = RunResult {
            run_id: "test-id".into(),
            output_dir: Some(path.clone()),
            dry_run: false,
            stats: RunStats::default(),
            outcomes: vec![],
            assertion: passing_assertion(),
        };
        assert_eq!(r.output_dir.as_ref(), Some(&path));
        assert!(!r.dry_run);
    }

    // ── RunStats::evaluate / terminal_failures_by_class ─────────────────────

    fn expectations(
        min_confirmed: u64,
        max_terminal_failures: u64,
        max_timeouts: u64,
        allowed_error_classes: &[&str],
    ) -> ExpectationsConfig {
        ExpectationsConfig {
            min_confirmed,
            max_terminal_failures,
            max_timeouts,
            allowed_error_classes: allowed_error_classes
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    #[test]
    fn evaluate_passes_when_all_thresholds_met() {
        let stats = RunStats {
            total_attempted: 60,
            confirmed: 60,
            failed: 0,
            timed_out: 0,
        };
        let outcome = stats.evaluate(&expectations(60, 0, 0, &[]), &HashMap::new());
        assert!(outcome.passed);
        assert!(outcome.violations.is_empty());
    }

    #[test]
    fn evaluate_fails_and_lists_every_violated_threshold() {
        let stats = RunStats {
            total_attempted: 60,
            confirmed: 50,
            failed: 5,
            timed_out: 3,
        };
        let mut failures_by_class = HashMap::new();
        failures_by_class.insert(IntentFailureClass::MempoolConflict, 5);
        let outcome = stats.evaluate(&expectations(60, 0, 0, &[]), &failures_by_class);
        assert!(!outcome.passed);
        assert_eq!(
            outcome.violations.len(),
            3,
            "all three violated thresholds must be listed: {:?}",
            outcome.violations
        );
        assert!(outcome.violations[0].contains("confirmed 50 < min_confirmed 60"));
        assert!(outcome.violations[1].contains("terminal failures 5"));
        assert!(outcome.violations[2].contains("timed out 3 > max_timeouts 0"));
    }

    #[test]
    fn test_allowed_error_classes_excludes_matching_failures_from_count() {
        let stats = RunStats {
            total_attempted: 3,
            confirmed: 0,
            failed: 3,
            timed_out: 0,
        };
        let mut failures_by_class = HashMap::new();
        failures_by_class.insert(IntentFailureClass::InsufficientBalance, 2);
        failures_by_class.insert(IntentFailureClass::MempoolConflict, 1);

        // allowed_error_classes excludes insufficient_balance: counted
        // failures = 1 (mempool_conflict only) <= max_terminal_failures 1.
        let allowed = expectations(0, 1, 0, &["insufficient_balance"]);
        let outcome = stats.evaluate(&allowed, &failures_by_class);
        assert!(
            outcome.passed,
            "expected pass with insufficient_balance excluded: {:?}",
            outcome.violations
        );

        // Regression guard: without allowed_error_classes, all 3 count and
        // the same max_terminal_failures: 1 must fail.
        let strict = expectations(0, 1, 0, &[]);
        let outcome = stats.evaluate(&strict, &failures_by_class);
        assert!(!outcome.passed, "expected fail with no exclusions");
        assert!(outcome.violations[0].contains("terminal failures 3"));
    }

    fn record_with(outcome: &str, failure_class: Option<IntentFailureClass>) -> IntentRecord {
        IntentRecord {
            run_id: "r".into(),
            intent_id: "i".into(),
            flow_type: FlowType::TToT,
            outcome: outcome.into(),
            error: None,
            timeout_context: None,
            recorded_at: Utc::now(),
            failure_class,
        }
    }

    #[test]
    fn terminal_failures_by_class_counts_only_failed_records_deduplicated_by_class() {
        let records = vec![
            record_with("confirmed", None),
            record_with("failed", Some(IntentFailureClass::InsufficientBalance)),
            record_with("failed", Some(IntentFailureClass::InsufficientBalance)),
            record_with("failed", Some(IntentFailureClass::MempoolConflict)),
        ];
        let counts = terminal_failures_by_class(&records);
        assert_eq!(
            counts.get(&IntentFailureClass::InsufficientBalance),
            Some(&2)
        );
        assert_eq!(counts.get(&IntentFailureClass::MempoolConflict), Some(&1));
        assert_eq!(counts.get(&IntentFailureClass::Timeout), None);
    }

    #[test]
    fn terminal_failures_by_class_excludes_timed_out_records() {
        // A `TimedOut` IntentRecord always carries `failure_class:
        // Some(IntentFailureClass::Timeout)` (informational), but its
        // `outcome` is "timed_out", not "failed" — it must never be counted
        // here, since this map feeds `max_terminal_failures`, a threshold
        // documented and intended to be independent of `max_timeouts`
        // (FINDING-1).
        let records = vec![
            record_with("timed_out", Some(IntentFailureClass::Timeout)),
            record_with("timed_out", Some(IntentFailureClass::Timeout)),
            record_with("failed", Some(IntentFailureClass::InsufficientBalance)),
        ];
        let counts = terminal_failures_by_class(&records);
        assert_eq!(counts.get(&IntentFailureClass::Timeout), None);
        assert_eq!(
            counts.get(&IntentFailureClass::InsufficientBalance),
            Some(&1)
        );
    }

    #[test]
    fn evaluate_max_timeouts_is_independent_of_max_terminal_failures() {
        // FINDING-1 regression: a run with only timeouts (no Failed
        // outcomes) and a generous max_timeouts must pass even when
        // max_terminal_failures is tight (e.g. 0) — timeouts are not
        // terminal failures for this comparison; the two thresholds are
        // documented as independent knobs.
        let stats = RunStats {
            total_attempted: 63,
            confirmed: 60,
            failed: 0,
            timed_out: 3,
        };
        let records = vec![
            record_with("timed_out", Some(IntentFailureClass::Timeout)),
            record_with("timed_out", Some(IntentFailureClass::Timeout)),
            record_with("timed_out", Some(IntentFailureClass::Timeout)),
        ];
        let failures_by_class = terminal_failures_by_class(&records);

        let permissive_timeouts = expectations(60, 0, 5, &[]);
        let outcome = stats.evaluate(&permissive_timeouts, &failures_by_class);
        assert!(
            outcome.passed,
            "expected pass: 3 timeouts <= max_timeouts 5, and 0 terminal \
             failures <= max_terminal_failures 0; got violations: {:?}",
            outcome.violations
        );

        // Regression guard: a tight max_timeouts still catches the same
        // timeouts via its own check, and only that check fires.
        let strict_timeouts = expectations(60, 0, 0, &[]);
        let outcome = stats.evaluate(&strict_timeouts, &failures_by_class);
        assert!(!outcome.passed, "expected fail via max_timeouts alone");
        assert_eq!(
            outcome.violations.len(),
            1,
            "only the timed-out violation should fire, not a duplicate \
             terminal-failures violation: {:?}",
            outcome.violations
        );
        assert!(outcome.violations[0].contains("timed out 3 > max_timeouts 0"));
    }

    #[test]
    fn test_report_recompute_is_deterministic() {
        // FINDING-2: the automated version of the client's own "recompute
        // report totals" acceptance step. Recomputing terminal_failures_by_class
        // and RunStats::evaluate twice from the same fixed IntentRecord set
        // must produce field-identical results both times — a regression
        // guard against HashMap-iteration-order nondeterminism ever leaking
        // into the assertion pass/fail decision or its violation text.
        let records = vec![
            record_with("confirmed", None),
            record_with("confirmed", None),
            record_with("failed", Some(IntentFailureClass::InsufficientBalance)),
            record_with("failed", Some(IntentFailureClass::MempoolConflict)),
            record_with("failed", Some(IntentFailureClass::MempoolConflict)),
            record_with("failed", Some(IntentFailureClass::Other)),
            record_with("timed_out", Some(IntentFailureClass::Timeout)),
        ];
        let stats = RunStats {
            total_attempted: 7,
            confirmed: 2,
            failed: 4,
            timed_out: 1,
        };
        let exp = expectations(5, 1, 0, &["other"]);

        let counts_a = terminal_failures_by_class(&records);
        let outcome_a = stats.evaluate(&exp, &counts_a);
        let counts_b = terminal_failures_by_class(&records);
        let outcome_b = stats.evaluate(&exp, &counts_b);

        assert_eq!(counts_a, counts_b);
        assert_eq!(outcome_a.passed, outcome_b.passed);
        assert_eq!(outcome_a.violations, outcome_b.violations);
    }
}
