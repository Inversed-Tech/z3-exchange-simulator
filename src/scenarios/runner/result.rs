//! Outcome types and run statistics for the scenario runner.

use crate::data_model::{Deposit, FlowType, Withdrawal};

/// The outcome of a single dispatched transaction intent.
#[derive(Debug)]
pub enum IntentOutcome {
    WithdrawalOk(Withdrawal),
    DepositOk(Deposit),
    Failed {
        intent_id: String,
        flow_type: FlowType,
        error: String,
    },
    TimedOut {
        intent_id: String,
        flow_type: FlowType,
    },
}

/// Aggregate statistics collected during the load phase.
#[derive(Debug, Default)]
pub struct RunStats {
    pub total_attempted: u64,
    pub confirmed: u64,
    pub failed: u64,
    pub timed_out: u64,
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

    #[test]
    fn run_result_output_dir_none_for_dry_run() {
        let r = RunResult {
            run_id: "test-id".into(),
            output_dir: None,
            dry_run: true,
            stats: RunStats::default(),
            outcomes: vec![],
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
        };
        assert_eq!(r.output_dir.as_ref(), Some(&path));
        assert!(!r.dry_run);
    }
}
