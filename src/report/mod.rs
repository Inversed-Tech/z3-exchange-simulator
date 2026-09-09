//! Findings-report generation: loads completed run directories, computes
//! load/latency/RPC-compatibility aggregates, flags candidate findings, and
//! renders a Markdown report.
//!
//! Deliberately consumes only the current run schema (`intents.jsonl`,
//! `manifest.json` with `timeouts`) — see `loader.rs` and
//! docs/architecture/observability.md for why older runs are rejected rather
//! than silently analyzed with partial data.

mod charts;
mod error;
mod findings;
mod latency;
mod load_curve;
mod loader;
mod markdown;
mod rpc_matrix;
mod system_health;

pub use charts::ChartError;
pub use error::ReportError;
pub use findings::{flag_candidates, Finding, FindingCategory, Severity};
pub use latency::{windowed_stats, windowed_throughput, ThroughputWindow, WindowStats};
pub use load_curve::{
    find_degradation_point, peak_tps_point, windowed_load_curve, DegradationPoint, LoadCurvePoint,
    DEFAULT_WINDOW_SECS,
};
pub use loader::{load_run, load_runs, RunData};
pub use markdown::{render_report, render_report_with_assets};
pub use rpc_matrix::{
    build_matrix, build_unlisted, load_parity_annotations, Category, MatrixRow, MatrixStatus,
    ParityInfo, RosterEntry, UnlistedRow, IN_SCOPE_METHODS,
};
pub use system_health::{
    compute_system_health, ProcessResourcePeak, ProvingTimeStats, SystemHealth,
};

#[cfg(test)]
mod tps_label_hygiene {
    //! `confirmed_tx_throughput` is the only rate this codebase may present
    //! to a reader as "TPS" (see Track 4 of
    //! `foundation-feedback-remediation-plan.md` §Track 4 4e) —
    //! `scheduled_dispatch_rate` and `rpc_calls_per_second` measure different
    //! things and must never be labeled that way. This is a repo-hygiene
    //! regression guard, not a behavioral test: it greps every `.rs` source
    //! file in this module for the literal substring "TPS" and asserts each
    //! occurrence sits within a few lines of a `confirmed_tx_throughput`
    //! mention, so a future edit can't silently reattach the "TPS" label to
    //! the wrong metric.

    const WINDOW: usize = 3;

    fn files_to_scan_for_tps() -> Vec<std::path::PathBuf> {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/report"));
        // Excludes this file (`mod.rs`) itself: the only "TPS" occurrence
        // here is inside this hygiene test's own source (checking for the
        // literal substring "TPS"), which is meta-referential, not a
        // rendered label this rule is meant to police.
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rs"))
            .filter(|p| p.file_name().and_then(|n| n.to_str()) != Some("mod.rs"))
            .collect();
        files.push(std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/docs/architecture/observability.md"
        )));
        files
    }

    #[test]
    fn tps_label_appears_only_near_confirmed_tx_throughput() {
        for path in files_to_scan_for_tps() {
            let content = std::fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !line.contains("TPS") {
                    continue;
                }
                let start = i.saturating_sub(WINDOW);
                let end = (i + WINDOW + 1).min(lines.len());
                let window_has_metric = lines[start..end]
                    .iter()
                    .any(|l| l.contains("confirmed_tx_throughput"));
                assert!(
                    window_has_metric,
                    "{}:{}: \"TPS\" appears with no nearby `confirmed_tx_throughput` mention: {line}",
                    path.display(),
                    i + 1,
                );
            }
        }
    }

    #[test]
    fn tps_label_hygiene_test_actually_finds_tps_occurrences() {
        // Regression guard on the test itself: if every "TPS" string were
        // ever removed from this module, the test above would trivially
        // pass having checked nothing. Confirm it still has real matches to
        // examine.
        let total: usize = files_to_scan_for_tps()
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap().matches("TPS").count())
            .sum();
        assert!(
            total > 0,
            "expected at least one \"TPS\" occurrence to check"
        );
    }
}
