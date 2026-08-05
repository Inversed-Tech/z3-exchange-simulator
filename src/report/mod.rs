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

pub use charts::ChartError;
pub use error::ReportError;
pub use findings::{flag_candidates, Finding, FindingCategory, Severity};
pub use latency::{windowed_stats, windowed_throughput, ThroughputWindow, WindowStats};
pub use load_curve::{windowed_load_curve, LoadCurvePoint, DEFAULT_WINDOW_SECS};
pub use loader::{load_run, load_runs, RunData};
pub use markdown::{render_report, render_report_with_assets};
pub use rpc_matrix::{
    build_matrix, Category, MatrixRow, MatrixStatus, RosterEntry, IN_SCOPE_METHODS,
};
