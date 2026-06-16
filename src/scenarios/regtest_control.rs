//! Regtest-control workflows.
//!
//! Deterministic chain-manipulation routines that shape the test environment
//! rather than forming part of the measured workload: block production and
//! chain reorganization. They use the regtest-only `generate`, `invalidateblock`,
//! and `reconsiderblock` RPCs and are excluded from the stress latency histograms.
//!
//! Like the exchange workflows, these are fully unit-testable against a mock RPC
//! server. End-to-end reorg behavior (actual height rollback and restoration) is
//! asserted in the integration suite against a live Z3 regtest stack.

use std::sync::Arc;

use chrono::Utc;

use crate::data_model::MetricSample;
use crate::metrics::MetricsRecorder;
use crate::rpc::{RpcClient, RpcError};

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RegtestControlError {
    Rpc(RpcError),
    EmptyResult { context: String },
}

impl std::fmt::Display for RegtestControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegtestControlError::Rpc(e) => write!(f, "RPC error: {e}"),
            RegtestControlError::EmptyResult { context } => write!(f, "empty result: {context}"),
        }
    }
}

impl std::error::Error for RegtestControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RegtestControlError::Rpc(e) => Some(e),
            _ => None,
        }
    }
}

// ── Reorg result ────────────────────────────────────────────────────────────

/// Observations recorded while driving a reorg. The integration suite asserts the
/// expected relationships (e.g. `height_after_invalidate == start_height`); unit
/// tests against mocks only verify the call sequence and that the result is filled.
#[derive(Debug, Clone)]
pub struct ReorgResult {
    /// Chain height before mining the branch that will be reorged.
    pub start_height: u64,
    /// Number of blocks mined to form the branch.
    pub mined_blocks: u32,
    /// Hash of the first mined block — the invalidation point that rolls back the branch.
    pub invalidated_hash: String,
    /// Height observed after `invalidateblock`.
    pub height_after_invalidate: u64,
    /// Height observed after `reconsiderblock`.
    pub height_after_reconsider: u64,
}

/// Drive a chain reorganization in regtest:
///
/// 1. Record the current height.
/// 2. Mine `depth` blocks to form a branch.
/// 3. `invalidateblock` the first mined block, rolling the branch back.
/// 4. `reconsiderblock` the same block, restoring it.
///
/// Heights are recorded before and after each step. Emits a `reorg_branch_blocks`
/// metric sample. Returns an error if mining produces no blocks.
pub async fn run_reorg(
    rpc: &RpcClient,
    depth: u32,
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
) -> Result<ReorgResult, RegtestControlError> {
    let start_height = rpc
        .get_block_count()
        .await
        .map_err(RegtestControlError::Rpc)?;

    // Mine the branch. `generate` returns the hashes of the blocks it produced,
    // in order; the first is our invalidation point.
    let mined = rpc
        .generate(depth)
        .await
        .map_err(RegtestControlError::Rpc)?;
    let invalidated_hash = mined
        .first()
        .ok_or_else(|| RegtestControlError::EmptyResult {
            context: format!("generate({depth}) returned no block hashes"),
        })?
        .clone();

    // Roll the branch back by invalidating its first block.
    rpc.invalidate_block(&invalidated_hash)
        .await
        .map_err(RegtestControlError::Rpc)?;
    let height_after_invalidate = rpc
        .get_block_count()
        .await
        .map_err(RegtestControlError::Rpc)?;

    // Restore the branch.
    rpc.reconsider_block(&invalidated_hash)
        .await
        .map_err(RegtestControlError::Rpc)?;
    let height_after_reconsider = rpc
        .get_block_count()
        .await
        .map_err(RegtestControlError::Rpc)?;

    if let Some(m) = &metrics {
        m.record_metric(MetricSample {
            run_id: run_id.to_string(),
            timestamp: Utc::now(),
            metric_name: "reorg_branch_blocks".to_string(),
            value: mined.len() as f64,
            labels: Default::default(),
        });
    }

    Ok(ReorgResult {
        start_height,
        mined_blocks: depth,
        invalidated_hash,
        height_after_invalidate,
        height_after_reconsider,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::data_model::RpcCall;

    struct MockRecorder {
        samples: Mutex<Vec<MetricSample>>,
    }

    impl MockRecorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                samples: Mutex::new(vec![]),
            })
        }
        fn samples(&self) -> Vec<MetricSample> {
            self.samples.lock().unwrap().clone()
        }
    }

    impl MetricsRecorder for MockRecorder {
        fn record_rpc_call(&self, _: RpcCall) {}
        fn record_metric(&self, s: MetricSample) {
            self.samples.lock().unwrap().push(s);
        }
    }

    fn rpc(url: &str) -> RpcClient {
        RpcClient::new(url, "test-run", None, None)
    }

    #[tokio::test]
    async fn run_reorg_drives_full_sequence_and_records_metric() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getblockcount" }),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": 10, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "generate" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["first", "second", "third"], "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "invalidateblock", "params": ["first"] }),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": null, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "reconsiderblock", "params": ["first"] }),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": null, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let result = run_reorg(
            &rpc(&server.uri()),
            3,
            "run-1",
            Some(rec.clone() as Arc<dyn MetricsRecorder>),
        )
        .await
        .unwrap();

        // The invalidation point is the first mined block.
        assert_eq!(result.invalidated_hash, "first");
        assert_eq!(result.mined_blocks, 3);
        assert_eq!(result.start_height, 10);

        let samples = rec.samples();
        assert!(samples
            .iter()
            .any(|s| s.metric_name == "reorg_branch_blocks" && s.value == 3.0));
    }

    #[tokio::test]
    async fn run_reorg_errors_when_generate_returns_no_blocks() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getblockcount" }),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": 5, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "generate" }),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": [], "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;

        let err = run_reorg(&rpc(&server.uri()), 1, "run-1", None)
            .await
            .unwrap_err();
        assert!(matches!(err, RegtestControlError::EmptyResult { .. }));
    }
}
