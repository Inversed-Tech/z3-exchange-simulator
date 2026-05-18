//! Metrics collection, aggregation, and output.
//!
//! Records per-RPC-call latency, success/failure counts, mempool saturation
//! signals, and time-series metric samples. Writes structured output to
//! the experiment run directory for post-run analysis.
