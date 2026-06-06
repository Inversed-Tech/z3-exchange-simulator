//! Exchange workflow implementations.
//!
//! Four independently callable async functions that drive deposit, withdrawal,
//! sweep, and balance-check workflows against a live Z3 regtest stack via the
//! RPC client. All four are fully unit-testable with a mock RPC server and
//! require no live Z3 stack.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::time::sleep;
use uuid::Uuid;

use crate::data_model::{
    Balance, Deposit, DepositStatus, MetricSample, Sweep, SweepStatus, Withdrawal, WithdrawalStatus,
};
use crate::metrics::MetricsRecorder;
use crate::rpc::{OperationResult, OperationStatus, Recipient, RpcClient, RpcError};

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ExchangeError {
    Rpc(RpcError),
    OperationFailed { op_id: String, message: String },
    Timeout { context: String },
    EmptyResult { context: String },
}

impl std::fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExchangeError::Rpc(e) => write!(f, "RPC error: {e}"),
            ExchangeError::OperationFailed { op_id, message } => {
                write!(f, "operation {op_id} failed: {message}")
            }
            ExchangeError::Timeout { context } => write!(f, "timeout: {context}"),
            ExchangeError::EmptyResult { context } => write!(f, "empty result: {context}"),
        }
    }
}

impl std::error::Error for ExchangeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let ExchangeError::Rpc(e) = self {
            Some(e)
        } else {
            None
        }
    }
}

// ── Polling config ────────────────────────────────────────────────────────────

/// Controls poll rates and timeouts used by all four workflows.
pub struct PollingConfig {
    /// How often to call `z_get_operation_status` while ZK proving is in progress.
    pub operation_poll_interval: Duration,
    /// Upper bound on how long to wait for an async operation to complete.
    pub max_operation_wait: Duration,
    /// How often to call `get_raw_transaction` while waiting for confirmation depth.
    pub confirmation_poll_interval: Duration,
    /// Upper bound on how long to wait for the required confirmation depth.
    pub max_confirmation_wait: Duration,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            operation_poll_interval: Duration::from_secs(2),
            max_operation_wait: Duration::from_secs(120),
            confirmation_poll_interval: Duration::from_secs(1),
            max_confirmation_wait: Duration::from_secs(60),
        }
    }
}

// ── Constants ─────────────────────────────────────────────────────────────────

// Conservative ZIP-317 fee buffer used when sweeping the full balance.
// Prevents submitting an output amount that exceeds available inputs minus fee.
const SWEEP_FEE_BUFFER_ZAT: u64 = 5_000;

// ── Shared utilities ──────────────────────────────────────────────────────────

/// Poll `z_get_operation_status` until the operation completes or the deadline passes.
/// Returns the completed status on success; errors on operation failure or timeout.
async fn poll_operation_until_complete(
    rpc: &RpcClient,
    op_id: &str,
    polling: &PollingConfig,
) -> Result<OperationStatus, ExchangeError> {
    let deadline = Instant::now() + polling.max_operation_wait;
    loop {
        let mut statuses = rpc
            .z_get_operation_status(&[op_id])
            .await
            .map_err(ExchangeError::Rpc)?;

        let status = statuses.pop().ok_or_else(|| ExchangeError::EmptyResult {
            context: format!("z_get_operation_status returned empty list for {op_id}"),
        })?;

        if status.is_complete() {
            if status.status == "success" {
                return Ok(status);
            }
            let msg = status
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| status.status.clone());
            return Err(ExchangeError::OperationFailed {
                op_id: op_id.to_string(),
                message: msg,
            });
        }

        if Instant::now() >= deadline {
            return Err(ExchangeError::Timeout {
                context: format!("operation {op_id} did not complete within the deadline"),
            });
        }
        sleep(polling.operation_poll_interval).await;
    }
}

/// Poll `get_raw_transaction` until the tx has at least `required` confirmations.
/// Returns the final confirmation count.
async fn wait_for_tx_confirmations(
    rpc: &RpcClient,
    txid: &str,
    required: u64,
    polling: &PollingConfig,
) -> Result<u64, ExchangeError> {
    let deadline = Instant::now() + polling.max_confirmation_wait;
    loop {
        let tx = rpc
            .get_raw_transaction(txid, true)
            .await
            .map_err(ExchangeError::Rpc)?;

        if let Some(confs) = tx.confirmations {
            if confs >= required {
                return Ok(confs);
            }
        }

        if Instant::now() >= deadline {
            return Err(ExchangeError::Timeout {
                context: format!(
                    "tx {txid} did not reach {required} confirmations within the deadline"
                ),
            });
        }
        sleep(polling.confirmation_poll_interval).await;
    }
}

/// Emit one `MetricSample` if a recorder is present.
fn emit(
    metrics: &Option<Arc<dyn MetricsRecorder>>,
    run_id: &str,
    name: &str,
    value: f64,
    labels: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
) {
    if let Some(m) = metrics {
        m.record_metric(MetricSample {
            run_id: run_id.to_string(),
            timestamp: Utc::now(),
            metric_name: name.to_string(),
            value,
            labels: labels
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        });
    }
}

// ── Workflow: deposit ─────────────────────────────────────────────────────────

/// Simulate an inbound deposit for a Zallet account in regtest.
///
/// Funds are sent from `from_account` (the simulator's pre-funded hot wallet or
/// miner account) to the Unified Address derived for `zallet_uuid`. After the
/// ZK async operation completes, `required_confirmations` regtest blocks are
/// mined and the deposit is credited.
///
/// Emits `deposit_confirmation_time_ms` to the metrics recorder.
pub async fn run_deposit(
    rpc: &RpcClient,
    account_id: &str,
    zallet_uuid: &str,
    from_account: &str,
    amount_zatoshis: u64,
    required_confirmations: u64,
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
    polling: &PollingConfig,
) -> Result<Deposit, ExchangeError> {
    let start = Instant::now();

    // Step 1: derive the deposit address for this account.
    let ua = rpc
        .z_get_address_for_account(zallet_uuid)
        .await
        .map_err(ExchangeError::Rpc)?;
    let deposit_address = ua.address;

    let mut deposit = Deposit {
        deposit_id: Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        deposit_address: deposit_address.clone(),
        amount_zatoshis,
        txid: None,
        status: DepositStatus::Pending,
        required_confirmations,
        current_confirmations: 0,
        created_at: Utc::now(),
        credited_at: None,
    };

    // Step 2: send funds from the hot wallet to the deposit address.
    let amount_zec = amount_zatoshis as f64 / 1e8;
    let recipients = [Recipient {
        address: deposit_address.clone(),
        amount: amount_zec,
        memo: None,
    }];
    let op_id = rpc
        .z_send_many(from_account, &recipients)
        .await
        .map_err(ExchangeError::Rpc)?;

    // Step 3: wait for ZK proving to complete and extract the txid.
    let op = poll_operation_until_complete(rpc, &op_id, polling).await?;
    let txid = op
        .txid()
        .ok_or_else(|| ExchangeError::EmptyResult {
            context: format!("operation {op_id} succeeded but carried no txid"),
        })?
        .to_string();

    // Step 4: mine blocks to reach the required confirmation depth.
    // Safe cast: required_confirmations will never approach u32::MAX in regtest.
    rpc.generate(required_confirmations as u32)
        .await
        .map_err(ExchangeError::Rpc)?;

    // Step 5: detect the incoming transaction via address history (`get_address_txids`).
    // In production an exchange would derive the txid from this scan; here we already
    // have it from the operation result and use the scan to verify on-chain presence.
    let address_txids = rpc
        .get_address_txids(&[deposit_address.as_str()], None)
        .await
        .map_err(ExchangeError::Rpc)?;

    if !address_txids.contains(&txid) {
        return Err(ExchangeError::EmptyResult {
            context: format!(
                "txid {txid} not found in address history for {deposit_address} after mining"
            ),
        });
    }

    deposit.txid = Some(txid.clone());
    deposit.status = DepositStatus::Detected;
    deposit.status = DepositStatus::Confirming;

    // Step 6: verify confirmation depth.
    let confs = wait_for_tx_confirmations(rpc, &txid, required_confirmations, polling).await?;

    deposit.current_confirmations = confs;
    deposit.status = DepositStatus::Credited;
    deposit.credited_at = Some(Utc::now());

    emit(
        &metrics,
        run_id,
        "deposit_confirmation_time_ms",
        start.elapsed().as_millis() as f64,
        [("account_id", account_id)],
    );

    Ok(deposit)
}

// ── Workflow: withdrawal ──────────────────────────────────────────────────────

/// Execute an outbound withdrawal from a Zallet account.
///
/// Sends `amount_zatoshis` from `from_account` to `destination_address` via
/// `z_send_many`. Mines one regtest block after the operation proves, then waits
/// for confirmation. Emits `withdrawal_proving_time_ms` and
/// `withdrawal_broadcast_latency_ms` to the metrics recorder.
pub async fn run_withdrawal(
    rpc: &RpcClient,
    account_id: &str,
    from_account: &str,
    destination_address: &str,
    amount_zatoshis: u64,
    intent_id: Option<&str>,
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
    polling: &PollingConfig,
) -> Result<Withdrawal, ExchangeError> {
    let mut withdrawal = Withdrawal {
        withdrawal_id: Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        destination_address: destination_address.to_string(),
        amount_zatoshis,
        fee_zatoshis: 0,
        status: WithdrawalStatus::Requested,
        txid: None,
        intent_id: intent_id.map(str::to_string),
        created_at: Utc::now(),
        broadcast_at: None,
    };

    let amount_zec = amount_zatoshis as f64 / 1e8;
    let recipients = [Recipient {
        address: destination_address.to_string(),
        amount: amount_zec,
        memo: None,
    }];

    withdrawal.status = WithdrawalStatus::Processing;

    // Measure proving time from z_send_many call to operation completion.
    let prove_start = Instant::now();
    let op_id = rpc
        .z_send_many(from_account, &recipients)
        .await
        .map_err(ExchangeError::Rpc)?;

    // poll_operation_until_complete confirms success via z_get_operation_status.
    // Then z_get_operation_result retrieves the authoritative final result with
    // txid, as specified in the work track.
    poll_operation_until_complete(rpc, &op_id, polling).await?;
    let proving_ms = prove_start.elapsed().as_millis() as u64;

    let mut results = rpc
        .z_get_operation_result(&[&op_id])
        .await
        .map_err(ExchangeError::Rpc)?;
    let final_result: OperationResult = results.pop().ok_or_else(|| ExchangeError::EmptyResult {
        context: format!("z_get_operation_result returned empty list for {op_id}"),
    })?;
    let txid = final_result
        .result
        .ok_or_else(|| ExchangeError::EmptyResult {
            context: format!("operation result for {op_id} carried no txid detail"),
        })?
        .txid;

    withdrawal.txid = Some(txid.clone());
    withdrawal.status = WithdrawalStatus::Broadcast;
    withdrawal.broadcast_at = Some(Utc::now());

    // Mine one block to include the transaction, then verify on-chain.
    let broadcast_start = Instant::now();
    rpc.generate(1).await.map_err(ExchangeError::Rpc)?;
    wait_for_tx_confirmations(rpc, &txid, 1, polling).await?;
    let broadcast_ms = broadcast_start.elapsed().as_millis() as u64;

    withdrawal.status = WithdrawalStatus::Confirmed;

    emit(
        &metrics,
        run_id,
        "withdrawal_proving_time_ms",
        proving_ms as f64,
        [("account_id", account_id)],
    );
    emit(
        &metrics,
        run_id,
        "withdrawal_broadcast_latency_ms",
        broadcast_ms as f64,
        [("account_id", account_id)],
    );

    Ok(withdrawal)
}

// ── Workflow: sweep ───────────────────────────────────────────────────────────

/// Sweep all confirmed unspent notes belonging to `from_account` into
/// `hot_wallet_address` via a single `z_send_many` call.
///
/// The sent amount is the sum of all discovered notes minus `SWEEP_FEE_BUFFER_ZAT`
/// to leave headroom for the ZIP-317 fee. Returns an error if no confirmed notes
/// are found for the account. Emits `sweep_time_ms` to the metrics recorder.
pub async fn run_sweep(
    rpc: &RpcClient,
    from_account: &str,
    hot_wallet_address: &str,
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
    polling: &PollingConfig,
) -> Result<Sweep, ExchangeError> {
    let start = Instant::now();

    // Collect all confirmed unspent notes, filter to this account.
    let all_notes = rpc
        .z_list_unspent(1, None)
        .await
        .map_err(ExchangeError::Rpc)?;

    let account_notes: Vec<_> = all_notes
        .into_iter()
        .filter(|n| n.account == from_account)
        .collect();

    if account_notes.is_empty() {
        return Err(ExchangeError::EmptyResult {
            context: format!("no confirmed unspent notes found for account {from_account}"),
        });
    }

    let total_zat: u64 = account_notes.iter().map(|n| n.value_zat).sum();
    let sweep_amount_zat = total_zat.saturating_sub(SWEEP_FEE_BUFFER_ZAT);

    let source_addresses: Vec<String> = account_notes
        .iter()
        .filter_map(|n| n.address.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut sweep = Sweep {
        sweep_id: Uuid::new_v4().to_string(),
        source_addresses,
        destination_address: hot_wallet_address.to_string(),
        total_amount_zatoshis: total_zat,
        fee_zatoshis: SWEEP_FEE_BUFFER_ZAT,
        status: SweepStatus::Processing,
        txid: None,
        intent_ids: vec![],
        created_at: Utc::now(),
    };

    let recipients = [Recipient {
        address: hot_wallet_address.to_string(),
        amount: sweep_amount_zat as f64 / 1e8,
        memo: None,
    }];

    let op_id = rpc
        .z_send_many(from_account, &recipients)
        .await
        .map_err(ExchangeError::Rpc)?;

    let op = poll_operation_until_complete(rpc, &op_id, polling).await?;
    let txid = op
        .txid()
        .ok_or_else(|| ExchangeError::EmptyResult {
            context: format!("sweep operation {op_id} succeeded but carried no txid"),
        })?
        .to_string();

    sweep.txid = Some(txid.clone());
    sweep.status = SweepStatus::Broadcast;

    rpc.generate(1).await.map_err(ExchangeError::Rpc)?;
    wait_for_tx_confirmations(rpc, &txid, 1, polling).await?;

    sweep.status = SweepStatus::Confirmed;

    emit(
        &metrics,
        run_id,
        "sweep_time_ms",
        start.elapsed().as_millis() as f64,
        [("from_account", from_account)],
    );

    Ok(sweep)
}

// ── Workflow: balance check ───────────────────────────────────────────────────

/// Snapshot the current wallet balance at the current block height.
///
/// Calls both `z_get_balances` (per-pool zatoshi amounts) and `z_get_total_balance`
/// (aggregate ZEC strings) as required by the spec, then records the snapshot
/// as a `Balance`. `get_block_count` provides the height at which the snapshot
/// was taken. The `*_unconfirmed` fields are always zero because Zallet does
/// not expose unconfirmed pool breakdowns.
///
/// Note: the confirmation tracker in this module uses `get_raw_transaction.confirmations`
/// rather than polling `get_block_count` directly; both are equivalent but the
/// former avoids needing to track the tx inclusion height separately.
pub async fn run_balance_check(
    rpc: &RpcClient,
    wallet_id: &str,
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
) -> Result<Balance, ExchangeError> {
    // Both methods called per spec: z_get_balances for per-pool zatoshi amounts,
    // z_get_total_balance for the aggregate ZEC-string view.
    let balances = rpc.z_get_balances().await.map_err(ExchangeError::Rpc)?;
    let _total = rpc.z_get_total_balance().await.map_err(ExchangeError::Rpc)?;
    let height = rpc.get_block_count().await.map_err(ExchangeError::Rpc)?;

    let total_confirmed = balances.transparent + balances.sapling + balances.orchard;
    let snapshot = Balance {
        wallet_id: wallet_id.to_string(),
        transparent_confirmed: balances.transparent,
        transparent_unconfirmed: 0,
        shielded_confirmed: balances.sapling + balances.orchard,
        shielded_unconfirmed: 0,
        total_confirmed,
        at_block_height: height,
        recorded_at: Utc::now(),
    };

    emit(
        &metrics,
        run_id,
        "balance_total_confirmed_zat",
        total_confirmed as f64,
        [("wallet_id", wallet_id)],
    );

    Ok(snapshot)
}

// ── Mempool watcher ───────────────────────────────────────────────────────────

/// Background task that periodically polls mempool state and records metrics.
///
/// Polls both `get_raw_mempool` (txid list) and `get_mempool_info` (aggregate
/// stats) on each tick, as required by the spec. Emits `mempool_tx_count` and
/// `mempool_bytes` metric samples. Emits an additional `mempool_saturated` sample
/// (value 1.0) when tx count exceeds `saturation_threshold`. Exits cleanly when
/// the `shutdown` channel fires.
pub async fn run_mempool_watcher(
    rpc: Arc<RpcClient>,
    run_id: String,
    saturation_threshold: u64,
    interval: Duration,
    metrics: Arc<dyn MetricsRecorder>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        // Poll raw mempool (txid list) — exercises the method and can be used
        // to detect specific transactions in the mempool if needed.
        let raw = rpc.get_raw_mempool().await.ok();

        if let Ok(info) = rpc.get_mempool_info().await {
            // Use the txid-list count as the authoritative size when available;
            // fall back to the info.size field.
            let tx_count = raw.as_ref().map(|v| v.len() as u64).unwrap_or(info.size);

            metrics.record_metric(MetricSample {
                run_id: run_id.clone(),
                timestamp: Utc::now(),
                metric_name: "mempool_tx_count".to_string(),
                value: tx_count as f64,
                labels: Default::default(),
            });
            metrics.record_metric(MetricSample {
                run_id: run_id.clone(),
                timestamp: Utc::now(),
                metric_name: "mempool_bytes".to_string(),
                value: info.bytes as f64,
                labels: Default::default(),
            });
            if tx_count >= saturation_threshold {
                metrics.record_metric(MetricSample {
                    run_id: run_id.clone(),
                    timestamp: Utc::now(),
                    metric_name: "mempool_saturated".to_string(),
                    value: 1.0,
                    labels: Default::default(),
                });
            }
        }

        tokio::select! {
            _ = &mut shutdown => break,
            _ = sleep(interval) => {}
        }
    }
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

    fn instant_polling() -> PollingConfig {
        PollingConfig {
            operation_poll_interval: Duration::ZERO,
            max_operation_wait: Duration::from_secs(5),
            confirmation_poll_interval: Duration::ZERO,
            max_confirmation_wait: Duration::from_secs(5),
        }
    }

    fn rpc(url: &str) -> RpcClient {
        RpcClient::new(url, "test-run", None, None)
    }

    // ── poll_operation_until_complete ─────────────────────────────────────────

    #[tokio::test]
    async fn poll_operation_returns_ok_on_success() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "op-1", "status": "success",
                    "result": { "txid": "abc123" }, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let op =
            poll_operation_until_complete(&rpc(&server.uri()), "op-1", &instant_polling())
                .await
                .unwrap();
        assert_eq!(op.status, "success");
        assert_eq!(op.txid(), Some("abc123"));
    }

    #[tokio::test]
    async fn poll_operation_returns_err_on_failed_status() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "op-1", "status": "failed", "result": null,
                    "error": { "code": -6, "message": "insufficient funds" }
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let err =
            poll_operation_until_complete(&rpc(&server.uri()), "op-1", &instant_polling())
                .await
                .unwrap_err();
        assert!(matches!(err, ExchangeError::OperationFailed { .. }));
        assert!(err.to_string().contains("insufficient funds"));
    }

    #[tokio::test]
    async fn poll_operation_times_out_when_always_executing() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "op-1", "status": "executing", "result": null, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let timeout_cfg = PollingConfig {
            operation_poll_interval: Duration::ZERO,
            max_operation_wait: Duration::ZERO,
            ..instant_polling()
        };
        let err =
            poll_operation_until_complete(&rpc(&server.uri()), "op-1", &timeout_cfg)
                .await
                .unwrap_err();
        assert!(matches!(err, ExchangeError::Timeout { .. }));
    }

    // ── wait_for_tx_confirmations ─────────────────────────────────────────────

    #[tokio::test]
    async fn wait_for_confirmations_resolves_when_depth_met() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawtransaction" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "tx1", "hex": "dead", "confirmations": 10 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let confs =
            wait_for_tx_confirmations(&rpc(&server.uri()), "tx1", 5, &instant_polling())
                .await
                .unwrap();
        assert!(confs >= 5);
    }

    #[tokio::test]
    async fn wait_for_confirmations_times_out_when_depth_not_met() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawtransaction" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "tx1", "hex": "dead", "confirmations": 1 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let timeout_cfg = PollingConfig {
            confirmation_poll_interval: Duration::ZERO,
            max_confirmation_wait: Duration::ZERO,
            ..instant_polling()
        };
        let err = wait_for_tx_confirmations(&rpc(&server.uri()), "tx1", 10, &timeout_cfg)
            .await
            .unwrap_err();
        assert!(matches!(err, ExchangeError::Timeout { .. }));
    }

    // ── run_balance_check ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_balance_check_sums_all_pools() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getbalances" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "transparent": 100_000_000u64,
                    "sapling": 50_000_000u64,
                    "orchard": 25_000_000u64
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_gettotalbalance" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "transparent": "1.00000000",
                    "private": "0.75000000",
                    "total": "1.75000000"
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getblockcount" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": 42, "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let bal = run_balance_check(
            &rpc(&server.uri()),
            "wallet-1",
            "run-1",
            Some(rec.clone() as Arc<dyn MetricsRecorder>),
        )
        .await
        .unwrap();

        assert_eq!(bal.transparent_confirmed, 100_000_000);
        assert_eq!(bal.shielded_confirmed, 75_000_000);
        assert_eq!(bal.total_confirmed, 175_000_000);
        assert_eq!(bal.at_block_height, 42);
        assert_eq!(bal.transparent_unconfirmed, 0);
        assert_eq!(bal.shielded_unconfirmed, 0);

        let samples = rec.samples();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].metric_name, "balance_total_confirmed_zat");
        assert_eq!(samples[0].value, 175_000_000.0);
        assert_eq!(samples[0].labels["wallet_id"], "wallet-1");
    }

    #[tokio::test]
    async fn run_balance_check_emits_no_metric_when_recorder_absent() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getbalances" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "transparent": 0u64, "sapling": 0u64, "orchard": 0u64 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_gettotalbalance" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "transparent": "0.00000000", "private": "0.00000000", "total": "0.00000000" },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getblockcount" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": 1, "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        // Should not panic with None recorder.
        run_balance_check(&rpc(&server.uri()), "wallet-1", "run-1", None)
            .await
            .unwrap();
    }

    // ── run_deposit ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_deposit_credits_account_on_success() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getaddressforaccount" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "account": "uuid-1",
                    "address": "u1depositaddr",
                    "receiver_types": ["orchard"]
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_sendmany" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-dep-1", "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-dep-1", "status": "success",
                    "result": { "txid": "deptxid" }, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "generate" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["h1", "h2", "h3"], "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        // Detection step: get_address_txids confirms the deposit tx appears on-chain.
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getaddresstxids" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["deptxid"],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawtransaction" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "deptxid", "hex": "dead", "confirmations": 3 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let deposit = run_deposit(
            &rpc(&server.uri()),
            "acc-1",
            "uuid-1",
            "hot-wallet-uuid",
            1_000_000,
            3,
            "run-1",
            Some(rec.clone() as Arc<dyn MetricsRecorder>),
            &instant_polling(),
        )
        .await
        .unwrap();

        assert_eq!(deposit.status, DepositStatus::Credited);
        assert_eq!(deposit.txid.as_deref(), Some("deptxid"));
        assert_eq!(deposit.deposit_address, "u1depositaddr");
        assert_eq!(deposit.amount_zatoshis, 1_000_000);
        assert_eq!(deposit.required_confirmations, 3);
        assert!(deposit.credited_at.is_some());

        let samples = rec.samples();
        assert!(samples
            .iter()
            .any(|s| s.metric_name == "deposit_confirmation_time_ms"));
    }

    #[tokio::test]
    async fn run_deposit_returns_err_on_operation_failure() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getaddressforaccount" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "account": "uuid-1", "address": "u1addr", "receiver_types": [] },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_sendmany" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-fail", "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-fail", "status": "failed", "result": null,
                    "error": { "code": -6, "message": "insufficient funds" }
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let err = run_deposit(
            &rpc(&server.uri()),
            "acc-1",
            "uuid-1",
            "hot-wallet-uuid",
            999_999_999_999,
            3,
            "run-1",
            None,
            &instant_polling(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ExchangeError::OperationFailed { .. }));
    }

    // ── run_withdrawal ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_withdrawal_confirms_and_records_timing_metrics() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_sendmany" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-wdl-1", "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-wdl-1", "status": "success",
                    "result": { "txid": "wdltxid" }, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        // z_get_operation_result retrieves the authoritative final txid per spec.
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationresult" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-wdl-1", "status": "success",
                    "result": { "txid": "wdltxid" }, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "generate" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["h1"], "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawtransaction" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "wdltxid", "hex": "dead", "confirmations": 1 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let wdl = run_withdrawal(
            &rpc(&server.uri()),
            "acc-1",
            "from-uuid",
            "t1destaddr",
            500_000,
            Some("intent-1"),
            "run-1",
            Some(rec.clone() as Arc<dyn MetricsRecorder>),
            &instant_polling(),
        )
        .await
        .unwrap();

        assert_eq!(wdl.status, WithdrawalStatus::Confirmed);
        assert_eq!(wdl.txid.as_deref(), Some("wdltxid"));
        assert_eq!(wdl.intent_id.as_deref(), Some("intent-1"));
        assert_eq!(wdl.amount_zatoshis, 500_000);
        assert!(wdl.broadcast_at.is_some());

        let samples = rec.samples();
        assert!(samples
            .iter()
            .any(|s| s.metric_name == "withdrawal_proving_time_ms"));
        assert!(samples
            .iter()
            .any(|s| s.metric_name == "withdrawal_broadcast_latency_ms"));
    }

    #[tokio::test]
    async fn run_withdrawal_returns_err_on_operation_failure() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_sendmany" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-fail", "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-fail", "status": "failed", "result": null,
                    "error": { "code": -6, "message": "wallet error" }
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let err = run_withdrawal(
            &rpc(&server.uri()),
            "acc-1",
            "from-uuid",
            "t1dest",
            100,
            None,
            "run-1",
            None,
            &instant_polling(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ExchangeError::OperationFailed { .. }));
    }

    #[tokio::test]
    async fn run_withdrawal_with_no_intent_id() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_sendmany" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-1", "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-1", "status": "success",
                    "result": { "txid": "txid1" }, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationresult" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-1", "status": "success",
                    "result": { "txid": "txid1" }, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "generate" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["h1"], "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawtransaction" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "txid1", "hex": "dead", "confirmations": 1 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let wdl = run_withdrawal(
            &rpc(&server.uri()),
            "acc-1",
            "from-uuid",
            "t1dest",
            100,
            None,
            "run-1",
            None,
            &instant_polling(),
        )
        .await
        .unwrap();

        assert!(wdl.intent_id.is_none());
        assert_eq!(wdl.status, WithdrawalStatus::Confirmed);
    }

    // ── run_sweep ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_sweep_consolidates_notes_to_hot_wallet() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_listunspent" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [
                    {
                        "txid": "note1", "confirmations": 5,
                        "account_uuid": "sweep-account",
                        "address": "u1addr1", "value": 0.5, "valueZat": 50_000_000u64
                    },
                    {
                        "txid": "note2", "confirmations": 3,
                        "account_uuid": "sweep-account",
                        "address": "u1addr2", "value": 0.3, "valueZat": 30_000_000u64
                    }
                ],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_sendmany" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-sweep-1", "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-sweep-1", "status": "success",
                    "result": { "txid": "sweeptxid" }, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "generate" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["h1"], "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawtransaction" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "sweeptxid", "hex": "dead", "confirmations": 1 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let sweep = run_sweep(
            &rpc(&server.uri()),
            "sweep-account",
            "u1hotwallet",
            "run-1",
            Some(rec.clone() as Arc<dyn MetricsRecorder>),
            &instant_polling(),
        )
        .await
        .unwrap();

        assert_eq!(sweep.status, SweepStatus::Confirmed);
        assert_eq!(sweep.total_amount_zatoshis, 80_000_000);
        assert_eq!(sweep.fee_zatoshis, SWEEP_FEE_BUFFER_ZAT);
        assert_eq!(sweep.txid.as_deref(), Some("sweeptxid"));
        assert_eq!(sweep.destination_address, "u1hotwallet");
        assert_eq!(sweep.source_addresses.len(), 2);

        let samples = rec.samples();
        assert!(samples.iter().any(|s| s.metric_name == "sweep_time_ms"));
    }

    #[tokio::test]
    async fn run_sweep_returns_err_when_no_notes_found() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_listunspent" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [], "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let err = run_sweep(
            &rpc(&server.uri()),
            "empty-account",
            "u1hotwallet",
            "run-1",
            None,
            &instant_polling(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ExchangeError::EmptyResult { .. }));
    }

    #[tokio::test]
    async fn run_sweep_ignores_notes_from_other_accounts() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_listunspent" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "txid": "note-other", "confirmations": 5,
                    "account_uuid": "other-account",
                    "address": "u1other", "value": 1.0, "valueZat": 100_000_000u64
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let err = run_sweep(
            &rpc(&server.uri()),
            "my-account",
            "u1hotwallet",
            "run-1",
            None,
            &instant_polling(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ExchangeError::EmptyResult { .. }));
    }

    #[tokio::test]
    async fn run_sweep_handles_notes_with_null_address() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_listunspent" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [
                    {
                        "txid": "note1", "confirmations": 2,
                        "account_uuid": "my-account",
                        "address": null, "value": 0.1, "valueZat": 10_000_000u64
                    }
                ],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_sendmany" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-1", "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "z_getoperationstatus" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-1", "status": "success",
                    "result": { "txid": "txid1" }, "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "generate" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["h1"], "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawtransaction" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "txid1", "hex": "dead", "confirmations": 1 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let sweep = run_sweep(
            &rpc(&server.uri()),
            "my-account",
            "u1hotwallet",
            "run-1",
            None,
            &instant_polling(),
        )
        .await
        .unwrap();

        // Note with null address still contributes to total; source_addresses is empty.
        assert_eq!(sweep.total_amount_zatoshis, 10_000_000);
        assert!(sweep.source_addresses.is_empty());
        assert_eq!(sweep.status, SweepStatus::Confirmed);
    }

    // ── run_mempool_watcher ───────────────────────────────────────────────────

    #[tokio::test]
    async fn run_mempool_watcher_records_count_and_bytes_then_exits() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        // get_raw_mempool — 3 txids; this becomes the authoritative count.
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawmempool" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["tx1", "tx2", "tx3"],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getmempoolinfo" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "size": 42, "bytes": 98_000 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let rpc_arc = Arc::new(RpcClient::new(&server.uri(), "run-1", None, None));

        let handle = tokio::spawn(run_mempool_watcher(
            rpc_arc,
            "run-1".to_string(),
            1_000,
            Duration::ZERO,
            rec.clone() as Arc<dyn MetricsRecorder>,
            rx,
        ));

        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = tx.send(());
        handle.await.unwrap();

        let samples = rec.samples();
        // raw mempool has 3 txids → tx_count=3 (takes priority over info.size=42).
        assert!(samples
            .iter()
            .any(|s| s.metric_name == "mempool_tx_count" && s.value == 3.0));
        assert!(samples
            .iter()
            .any(|s| s.metric_name == "mempool_bytes" && s.value == 98_000.0));
        // 3 < threshold 1000 — no saturation event.
        assert!(!samples.iter().any(|s| s.metric_name == "mempool_saturated"));
    }

    #[tokio::test]
    async fn run_mempool_watcher_emits_saturation_event_when_threshold_exceeded() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;

        // get_raw_mempool — 5 txids; raw count (5) > threshold (3) → saturation.
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getrawmempool" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["tx1", "tx2", "tx3", "tx4", "tx5"],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({ "method": "getmempoolinfo" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "size": 5, "bytes": 1_000_000 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let rpc_arc = Arc::new(RpcClient::new(&server.uri(), "run-1", None, None));

        let handle = tokio::spawn(run_mempool_watcher(
            rpc_arc,
            "run-1".to_string(),
            3, // saturation_threshold: 5 txids >= 3 → saturated
            Duration::ZERO,
            rec.clone() as Arc<dyn MetricsRecorder>,
            rx,
        ));

        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = tx.send(());
        handle.await.unwrap();

        let samples = rec.samples();
        assert!(samples
            .iter()
            .any(|s| s.metric_name == "mempool_saturated" && s.value == 1.0));
    }
}
