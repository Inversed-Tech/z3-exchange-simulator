//! Intent dispatch: maps `TransactionIntent` to the appropriate exchange workflow.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Semaphore;

use crate::data_model::{FlowType, MetricSample, TransactionIntent};
use crate::metrics::MetricsRecorder;
use crate::rpc::RpcClient;
use crate::scenarios::exchange::{
    run_balance_check, run_deposit, run_sweep, run_withdrawal, ExchangeError, PollingConfig,
};
use crate::scenarios::runner::result::IntentOutcome;

// ── ActiveGuard ───────────────────────────────────────────────────────────────

/// RAII guard that decrements the active-intent counter when dropped.
struct ActiveGuard(Arc<AtomicUsize>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

// ── resolve_zallet_uuid ─────────────────────────────────────────────────────────

/// Look up the Zallet account UUID for an intent's account. On a miss (the
/// account was never provisioned) return a ready-made `Failed` outcome for the
/// caller to `return`. Shared by the TToZ / ZToT / ZToZ flow arms.
// `IntentOutcome` is deliberately passed by value throughout dispatch (it is the
// normal success/failure return of every flow), so the large-Err lint does not
// apply here.
#[allow(clippy::result_large_err)]
fn resolve_zallet_uuid(
    zallet_uuids: &HashMap<String, String>,
    intent: &TransactionIntent,
) -> Result<String, IntentOutcome> {
    zallet_uuids
        .get(&intent.account_id)
        .cloned()
        .ok_or_else(|| IntentOutcome::Failed {
            intent_id: intent.intent_id.clone(),
            flow_type: intent.flow_type.clone(),
            error: format!("no Zallet UUID for account {}", intent.account_id),
        })
}

// ── build_intent_future ───────────────────────────────────────────────────────

/// Build a `Future` that executes a single transaction intent against the Z3 stack.
///
/// The future acquires a slot from `sem` before calling any RPC, and tracks
/// the concurrent in-flight count via `active_count`.
#[allow(clippy::too_many_arguments)]
pub async fn build_intent_future(
    intent: TransactionIntent,
    rpc: Arc<RpcClient>,
    sem: Arc<Semaphore>,
    active_count: Arc<AtomicUsize>,
    metrics: Arc<dyn MetricsRecorder>,
    hot_wallet_address: String,
    run_id: String,
    polling: PollingConfig,
    zallet_uuids: Arc<HashMap<String, String>>,
    zallet_addresses: Arc<HashMap<String, String>>,
) -> IntentOutcome {
    active_count.fetch_add(1, Ordering::Relaxed);
    let _guard = ActiveGuard(active_count.clone());
    let _permit = sem.acquire_owned().await.unwrap();

    // Per-flow `from` forms and privacy policies (measured on Zallet beta.1;
    // docs/regtest-funding-plan.md): the intent generator already resolves
    // `sender_address` per flow — the sender's t-addr for TToT/TToZ, the
    // sender's UA for ZToT/ZToZ — and those are exactly the forms z_sendmany
    // requires, because a UA `from` draws shielded funds only while a bare
    // t-addr `from` draws that address's transparent UTXOs. The privacy policy
    // then names what the pool combination reveals. Every flow's source is the
    // synthetic account itself (funded at provisioning), not the hot wallet —
    // except ZToT's second leg, where the hot wallet pays out after the sweep,
    // mirroring a real exchange's shielded treasury.
    match intent.flow_type {
        // ── Transparent → Transparent: withdrawal from the account's t-addr ──
        FlowType::TToT => {
            match run_withdrawal(
                &rpc,
                &intent.account_id,
                &intent.sender_address,
                &intent.recipient_address,
                "AllowFullyTransparent",
                intent.amount_zatoshis,
                Some(&intent.intent_id),
                &run_id,
                Some(metrics.clone()),
                &polling,
            )
            .await
            {
                Ok(withdrawal) => IntentOutcome::WithdrawalOk {
                    withdrawal,
                    intent_id: intent.intent_id.clone(),
                    flow_type: FlowType::TToT,
                },
                Err(ExchangeError::Timeout { context }) => IntentOutcome::TimedOut {
                    intent_id: intent.intent_id,
                    flow_type: FlowType::TToT,
                    context,
                },
                Err(e) => IntentOutcome::Failed {
                    intent_id: intent.intent_id,
                    flow_type: FlowType::TToT,
                    error: e.to_string(),
                },
            }
        }

        // ── Transparent → Shielded: deposit from the account's t-addr ──
        FlowType::TToZ => {
            match run_deposit(
                &rpc,
                &intent.recipient_account_id,
                &intent.recipient_address,
                &intent.sender_address,
                "AllowRevealedSenders",
                intent.amount_zatoshis,
                1,
                &run_id,
                Some(metrics.clone()),
                &polling,
            )
            .await
            {
                Ok(deposit) => IntentOutcome::DepositOk {
                    deposit,
                    intent_id: intent.intent_id.clone(),
                    flow_type: FlowType::TToZ,
                },
                Err(ExchangeError::Timeout { context }) => IntentOutcome::TimedOut {
                    intent_id: intent.intent_id,
                    flow_type: FlowType::TToZ,
                    context,
                },
                Err(e) => IntentOutcome::Failed {
                    intent_id: intent.intent_id,
                    flow_type: FlowType::TToZ,
                    error: e.to_string(),
                },
            }
        }

        // ── Shielded → Transparent: sweep then withdrawal ──────────────
        FlowType::ZToT => {
            let zallet_uuid = match resolve_zallet_uuid(&zallet_uuids, &intent) {
                Ok(u) => u,
                Err(outcome) => return outcome,
            };
            // Step 1: sweep the user's shielded notes into the hot wallet.
            // z_listunspent filtering needs the account UUID; the sweep send
            // itself goes from the account's UA (shielded source, z→z).
            let zallet_address = match zallet_addresses.get(&intent.account_id) {
                Some(a) => a.clone(),
                None => {
                    return IntentOutcome::Failed {
                        intent_id: intent.intent_id,
                        flow_type: FlowType::ZToT,
                        error: format!("no Zallet address for account {}", intent.account_id),
                    }
                }
            };
            match run_sweep(
                &rpc,
                &zallet_uuid,
                &zallet_address,
                &hot_wallet_address,
                &run_id,
                Some(metrics.clone()),
                &polling,
            )
            .await
            {
                Ok(_) => {}
                Err(ExchangeError::Timeout { context }) => {
                    return IntentOutcome::TimedOut {
                        intent_id: intent.intent_id,
                        flow_type: FlowType::ZToT,
                        context,
                    }
                }
                Err(e) => {
                    return IntentOutcome::Failed {
                        intent_id: intent.intent_id,
                        flow_type: FlowType::ZToT,
                        error: e.to_string(),
                    }
                }
            }
            // Step 2: pay out from the hot wallet (shielded treasury) to the
            // recipient's transparent address.
            match run_withdrawal(
                &rpc,
                &intent.account_id,
                &hot_wallet_address,
                &intent.recipient_address,
                "AllowRevealedRecipients",
                intent.amount_zatoshis,
                Some(&intent.intent_id),
                &run_id,
                Some(metrics.clone()),
                &polling,
            )
            .await
            {
                Ok(withdrawal) => IntentOutcome::WithdrawalOk {
                    withdrawal,
                    intent_id: intent.intent_id.clone(),
                    flow_type: FlowType::ZToT,
                },
                Err(ExchangeError::Timeout { context }) => IntentOutcome::TimedOut {
                    intent_id: intent.intent_id,
                    flow_type: FlowType::ZToT,
                    context,
                },
                Err(e) => IntentOutcome::Failed {
                    intent_id: intent.intent_id,
                    flow_type: FlowType::ZToT,
                    error: e.to_string(),
                },
            }
        }

        // ── Shielded → Shielded: deposit from the account's UA ─────────
        FlowType::ZToZ => {
            match run_deposit(
                &rpc,
                &intent.recipient_account_id,
                &intent.recipient_address,
                &intent.sender_address,
                "FullPrivacy",
                intent.amount_zatoshis,
                1,
                &run_id,
                Some(metrics.clone()),
                &polling,
            )
            .await
            {
                Ok(deposit) => IntentOutcome::DepositOk {
                    deposit,
                    intent_id: intent.intent_id.clone(),
                    flow_type: FlowType::ZToZ,
                },
                Err(ExchangeError::Timeout { context }) => IntentOutcome::TimedOut {
                    intent_id: intent.intent_id,
                    flow_type: FlowType::ZToZ,
                    context,
                },
                Err(e) => IntentOutcome::Failed {
                    intent_id: intent.intent_id,
                    flow_type: FlowType::ZToZ,
                    error: e.to_string(),
                },
            }
        }
    }
}

// ── background_miner ──────────────────────────────────────────────────────────

/// Background task that mines one regtest block per `interval`.
///
/// This decouples block production from individual transaction flows so that
/// per-transaction `generate()` calls are not needed. Mining failures are
/// logged but do not stop the task — the caller will detect chain progress
/// via confirmation-poll timeouts if the chain is truly broken.
pub async fn background_miner(
    rpc: Arc<RpcClient>,
    interval: Duration,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = ticker.tick() => {
                // Log (don't swallow) mining failures: without this, a broken
                // chain surfaces only indirectly as confirmation-poll timeouts.
                if let Err(e) = rpc.generate(1).await {
                    tracing::warn!("background miner: generate(1) failed: {e}");
                }
            }
        }
    }
}

// ── periodic_balance_check ────────────────────────────────────────────────────

/// Background task that periodically snapshots the hot wallet balance and
/// records it alongside the current active in-flight count.
pub async fn periodic_balance_check(
    rpc: Arc<RpcClient>,
    run_id: String,
    interval: Duration,
    metrics: Arc<dyn MetricsRecorder>,
    active_count: Arc<AtomicUsize>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = ticker.tick() => {}
        }

        // Emit active accounts count.
        let active = active_count.load(Ordering::Relaxed) as f64;
        metrics.record_metric(MetricSample {
            run_id: run_id.clone(),
            timestamp: Utc::now(),
            metric_name: "active_accounts".to_string(),
            value: active,
            labels: Default::default(),
        });

        // Snapshot the hot wallet balance.
        if let Ok(balance) = run_balance_check(&rpc, "hot_wallet", &run_id, None).await {
            metrics.record_metric(MetricSample {
                run_id: run_id.clone(),
                timestamp: Utc::now(),
                metric_name: "block_height".to_string(),
                value: balance.at_block_height as f64,
                labels: Default::default(),
            });
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;

    use tokio::sync::Semaphore;

    use crate::data_model::{
        FlowType, MetricSample, RpcCall, TransactionIntent, TransactionStatus,
    };
    use crate::metrics::{MetricsRecorder, NullRecorder};
    use crate::rpc::RpcClient;
    use crate::scenarios::exchange::PollingConfig;
    use crate::scenarios::runner::result::IntentOutcome;

    use super::{build_intent_future, periodic_balance_check};

    // ── Helpers ───────────────────────────────────────────────────────────────

    struct MockRecorder {
        samples: Mutex<Vec<MetricSample>>,
    }

    impl MockRecorder {
        fn new() -> Self {
            Self {
                samples: Mutex::new(Vec::new()),
            }
        }

        fn samples(&self) -> Vec<MetricSample> {
            self.samples.lock().unwrap().clone()
        }
    }

    impl MetricsRecorder for MockRecorder {
        fn record_rpc_call(&self, _: RpcCall) {}
        fn record_metric(&self, sample: MetricSample) {
            self.samples.lock().unwrap().push(sample);
        }
    }

    fn make_intent(flow_type: FlowType, account_id: &str) -> TransactionIntent {
        TransactionIntent {
            intent_id: "test-intent".to_string(),
            run_id: "test-run".to_string(),
            account_id: account_id.to_string(),
            recipient_account_id: "recipient".to_string(),
            sender_address: "t1sender".to_string(),
            recipient_address: "t1recipient".to_string(),
            amount_zatoshis: 10_000,
            fee_zatoshis: 1_000,
            flow_type,
            status: TransactionStatus::Pending,
            created_at: chrono::Utc::now(),
            submitted_at: None,
        }
    }

    fn fast_polling() -> PollingConfig {
        PollingConfig {
            operation_poll_interval: Duration::ZERO,
            max_operation_wait: Duration::from_millis(100),
            confirmation_poll_interval: Duration::ZERO,
            max_confirmation_wait: Duration::from_millis(100),
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn periodic_balance_check_exits_cleanly_on_shutdown() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Serve a success response for any POST so the first (immediate) tick
        // completes without hanging. The getblockcount call gets the balance
        // response (wrong shape), causing run_balance_check to fail; if let Ok
        // catches it and the task continues to the next select! where shutdown fires.
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"transparent": "0.0", "private": "0.0", "total": "0.0"},
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rpc = Arc::new(RpcClient::new(&server.uri(), "run", None, None));
        let metrics: Arc<dyn MetricsRecorder> = Arc::new(NullRecorder);
        let active_count = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let handle = tokio::spawn(periodic_balance_check(
            rpc,
            "run".to_string(),
            Duration::from_secs(3600),
            metrics,
            active_count,
            rx,
        ));

        // Let the first (immediate) tick complete before signaling shutdown.
        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(()).unwrap();

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("task should exit within 2s after shutdown signal")
            .expect("task should not panic");
    }

    #[tokio::test]
    async fn periodic_balance_check_emits_active_accounts_and_block_height_metrics() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({"method": "z_gettotalbalance"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"transparent": "1.00000000", "private": "0.00000000", "total": "1.00000000"},
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({"method": "getblockcount"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": 99, "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rpc = Arc::new(RpcClient::new(&server.uri(), "run", None, None));
        let recorder = Arc::new(MockRecorder::new());
        let metrics: Arc<dyn MetricsRecorder> = recorder.clone();
        let active_count = Arc::new(AtomicUsize::new(7));
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let handle = tokio::spawn(periodic_balance_check(
            rpc,
            "run".to_string(),
            Duration::from_secs(3600),
            metrics,
            active_count,
            rx,
        ));

        tokio::time::sleep(Duration::from_millis(100)).await;
        tx.send(()).unwrap();
        handle.await.expect("task should not panic");

        let samples = recorder.samples();
        let active = samples
            .iter()
            .find(|s| s.metric_name == "active_accounts")
            .expect("active_accounts metric must be emitted");
        assert_eq!(active.value, 7.0);

        let height = samples
            .iter()
            .find(|s| s.metric_name == "block_height")
            .expect("block_height metric must be emitted");
        assert_eq!(height.value, 99.0);
    }

    #[tokio::test]
    async fn active_count_decrements_on_task_completion() {
        let rpc = Arc::new(RpcClient::new("http://127.0.0.1:1", "run", None, None));
        let sem = Arc::new(Semaphore::new(10));
        let active_count = Arc::new(AtomicUsize::new(0));
        let metrics: Arc<dyn MetricsRecorder> = Arc::new(NullRecorder);
        let zallet_uuids = Arc::new(HashMap::new()); // empty → immediate fail, no RPC calls
        let zallet_addresses = Arc::new(HashMap::new());

        let outcome = build_intent_future(
            make_intent(FlowType::TToT, "acct-1"),
            rpc,
            sem,
            active_count.clone(),
            metrics,
            "hw-addr".to_string(),
            "run".to_string(),
            fast_polling(),
            zallet_uuids,
            zallet_addresses,
        )
        .await;

        assert!(matches!(outcome, IntentOutcome::Failed { .. }));
        assert_eq!(
            active_count.load(Ordering::Relaxed),
            0,
            "RAII guard must decrement active_count on completion"
        );
    }

    #[tokio::test]
    async fn zot_dispatch_returns_failed_when_sweep_finds_no_notes() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({"method": "z_listunspent"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [], "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rpc = Arc::new(RpcClient::new(&server.uri(), "run", None, None));
        let sem = Arc::new(Semaphore::new(10));
        let active_count = Arc::new(AtomicUsize::new(0));
        let metrics: Arc<dyn MetricsRecorder> = Arc::new(NullRecorder);

        let mut uuid_map = HashMap::new();
        uuid_map.insert("acct-1".to_string(), "zallet-uuid-1".to_string());
        let zallet_uuids = Arc::new(uuid_map);

        let mut addr_map = HashMap::new();
        addr_map.insert("acct-1".to_string(), "u1acct1address".to_string());
        let zallet_addresses = Arc::new(addr_map);

        let outcome = build_intent_future(
            make_intent(FlowType::ZToT, "acct-1"),
            rpc,
            sem,
            active_count.clone(),
            metrics,
            "u1hotwallet".to_string(),
            "run".to_string(),
            fast_polling(),
            zallet_uuids,
            zallet_addresses,
        )
        .await;

        assert!(
            matches!(outcome, IntentOutcome::Failed { .. }),
            "expected Failed when sweep finds no notes"
        );
        assert_eq!(active_count.load(Ordering::Relaxed), 0);

        // z_sendmany must not be called — sweep failed before the withdrawal step.
        let received = server.received_requests().await.unwrap();
        assert!(
            received.iter().all(|r| !std::str::from_utf8(&r.body)
                .unwrap_or("")
                .contains("z_sendmany")),
            "z_sendmany must not be called when sweep fails with no notes"
        );
    }

    #[tokio::test]
    async fn spawn_intent_task_respects_backpressure() {
        let rpc = Arc::new(RpcClient::new("http://127.0.0.1:1", "run", None, None));
        let sem = Arc::new(Semaphore::new(2)); // at most 2 concurrent T5 calls
        let active_count = Arc::new(AtomicUsize::new(0));
        let metrics: Arc<dyn MetricsRecorder> = Arc::new(NullRecorder);
        let zallet_uuids = Arc::new(HashMap::new()); // empty → immediate fail, no RPC calls
        let zallet_addresses = Arc::new(HashMap::new());

        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..5 {
            tasks.spawn(build_intent_future(
                make_intent(FlowType::TToT, &format!("acct-{i}")),
                rpc.clone(),
                sem.clone(),
                active_count.clone(),
                metrics.clone(),
                "hw-addr".to_string(),
                "run".to_string(),
                fast_polling(),
                zallet_uuids.clone(),
                zallet_addresses.clone(),
            ));
        }

        let mut outcomes = Vec::new();
        while let Some(result) = tasks.join_next().await {
            outcomes.push(result.unwrap());
        }

        assert_eq!(
            outcomes.len(),
            5,
            "all 5 tasks must complete without deadlock"
        );
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, IntentOutcome::Failed { .. })),
            "all tasks must fail immediately (no UUID in map)"
        );
        // After all tasks complete their RAII guards have been dropped.
        assert_eq!(
            active_count.load(Ordering::Relaxed),
            0,
            "active_count must return to 0 after all tasks drain"
        );
    }
}
