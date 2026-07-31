//! Metrics collection, aggregation, and output.

mod error;
mod latency;
mod manifest;
mod recorder;
mod run_dir;
mod summary;
mod writers;

use crate::data_model::{MetricSample, RpcCall};

pub use error::MetricsError;
pub use manifest::{
    read_manifest, read_simulator_commit, read_z3_commits, write_manifest, RunManifest, RunTimeouts,
};
pub use recorder::{JsonlRecorder, NullRecorder};
pub use run_dir::RunDir;
pub use summary::generate_summary;

/// Contract C — implemented in recorder.rs, consumed by T2, T3, T5.
pub trait MetricsRecorder: Send + Sync {
    fn record_rpc_call(&self, call: RpcCall);
    fn record_metric(&self, sample: MetricSample);
}
