//! Metrics collection, aggregation, and output.

use crate::data_model::{MetricSample, RpcCall};

/// Contract C — implemented here in T6, consumed by T2 (resource sampling) and T3 (RPC calls).
pub trait MetricsRecorder: Send + Sync {
    fn record_rpc_call(&self, call: RpcCall);
    fn record_metric(&self, sample: MetricSample);
}
