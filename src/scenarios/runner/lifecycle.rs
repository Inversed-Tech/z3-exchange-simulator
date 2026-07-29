//! Lifecycle management: setup, warmup, and teardown of the Z3 stack.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use tokio::time::{sleep, Duration};

use crate::data_model::{MetricSample, ScenarioConfig};
use crate::metrics::{
    generate_summary, write_manifest, JsonlRecorder, MetricsRecorder, RunDir, RunManifest,
};
use crate::rpc::{RpcClient, RpcError};
use crate::scenarios::runner::funding::{self, FundedAccount, ANCHOR_CONFIRMATIONS};
use crate::scenarios::runner::provisioner::{provision, ProvisionedPopulation};
use crate::scenarios::runner::RunOptions;
use crate::scenarios::runner::RunnerError;
use crate::z3::{Z3Config, Z3Stack};

/// Confirmations a coinbase output needs before it may be spent. Consensus, and
/// identical on regtest — regtest waives the rule that transparent coinbase must
/// be spent to shielded outputs, but not the maturity window. Mirrors
/// `zcash_protocol`'s `COINBASE_MATURITY_BLOCKS`.
const COINBASE_MATURITY_BLOCKS: u64 = 100;

// ── SetupState ────────────────────────────────────────────────────────────────

/// Everything needed to run the load phase, produced by [`setup`].
pub struct SetupState {
    pub stack: Z3Stack,
    pub rpc: Arc<RpcClient>,
    pub provisioned: ProvisionedPopulation,
    pub hot_wallet_uuid: String,
    pub hot_wallet_address: String,
}

// ── setup ─────────────────────────────────────────────────────────────────────

/// Start the Z3 stack, build an RPC client, and provision the synthetic population.
pub async fn setup(
    scenario: &ScenarioConfig,
    opts: &RunOptions,
    run_id: &str,
    run_dir: &RunDir,
    metrics: Arc<dyn MetricsRecorder>,
) -> Result<SetupState, RunnerError> {
    // 1. Build Z3 config using the run_dir-managed component log directory.
    //    RunDir::create() already created this directory.
    let z3_config = Z3Config::for_run(run_id, run_dir.component_logs_dir());
    let rpc_url = z3_config.rpc_url.clone();
    let basic_auth = z3_config.basic_auth.clone();

    // 2. Start the Z3 stack. On failure, stop it before returning: `start()` may
    //    have brought some containers up before erroring, and dropping the stack
    //    does not tear them down — leaving them running would leak the stack.
    let mut stack = Z3Stack::new(z3_config, Some(metrics.clone()));
    if let Err(e) = stack.start().await {
        let _ = stack.stop().await;
        return Err(RunnerError::Setup(e.to_string()));
    }

    // 3. Build the RPC client.
    let rpc = {
        let client = RpcClient::new(&rpc_url, run_id, Some(metrics.clone()), None);
        if let Some((user, pass)) = basic_auth {
            client.with_basic_auth(user, pass)
        } else {
            client
        }
    };
    let rpc = Arc::new(rpc);

    // 4. Resolve the hot wallet account BEFORE mining warmup blocks. Zallet
    //    tracks coinbase payments by scanning blocks for known account
    //    addresses; if the account were only created after mining, Zallet would
    //    set its birthday at the current tip and miss all prior coinbase.
    //
    //    funding::resolve_account reads the account's creation-time address
    //    instead of deriving a new one — deriving here would both hand out an
    //    address that is NOT the one regtest-miner-setup.sh pointed Zebra's
    //    miner_address at, and walk the transparent gap window (see
    //    docs/zallet-transparent-gap-limit.md).
    //    This is the FIRST Zallet call after stack start, so it retries
    //    transient failures: rpc-router can still be restarting (waiting on
    //    Zallet's rpc.discover) in the seconds after wait_until_ready()
    //    returns, which surfaces as transport errors or as unparseable
    //    (router-error) response bodies.
    let hot_wallet = {
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let resolved = match opts.hot_wallet_uuid.as_deref() {
                Some(uuid) => funding::resolve_account_by_uuid(&rpc, uuid).await,
                None => funding::resolve_account(&rpc, "hot_wallet").await,
            };
            match resolved {
                Ok(hw) => break hw,
                Err(funding::FundingError::Rpc {
                    source: RpcError::Transport(_) | RpcError::Parse(_),
                    ..
                }) if attempts < 12 => {
                    sleep(Duration::from_secs(5)).await;
                }
                Err(e) => {
                    let _ = stack.stop().await;
                    return Err(RunnerError::Setup(format!(
                        "failed to resolve hot wallet: {e}"
                    )));
                }
            }
        }
    };
    let hot_wallet_uuid = hot_wallet.uuid.clone();

    // 5. Warmup: mine blocks before provisioning. The hot wallet account was
    //    created above so Zallet will credit coinbase outputs as blocks arrive.
    if let Err(e) = warmup(&rpc, scenario, run_id, metrics.clone()).await {
        let _ = stack.stop().await;
        return Err(e);
    }

    // 6. Provision the synthetic population (pass the already-resolved hot
    //    wallet UUID so provisioner skips its own z_list_accounts call).
    let provisioned = match provision(
        rpc.clone(),
        scenario,
        run_id,
        metrics.clone(),
        Some(hot_wallet_uuid.clone()),
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            let _ = stack.stop().await;
            return Err(RunnerError::Provision(e));
        }
    };

    // 7. Fund the active accounts from the hot wallet, in both pools, with one
    //    fan-out transaction. Without this, every intent whose SOURCE is a
    //    synthetic account (all four flow types after the per-flow rework in
    //    dispatch.rs) fails with "Insufficient balance" — the exact 0%-confirmed
    //    failure mode the measured funding pipeline exists to prevent. This is
    //    also the real spendability proof for the warmup coinbase: the send
    //    retries while the wallet catches up, and fails loudly if the hot
    //    wallet's funds cannot actually be spent.
    if let Err(e) = fund_active_accounts(&rpc, scenario, &hot_wallet, &provisioned).await {
        let _ = stack.stop().await;
        return Err(RunnerError::Setup(format!(
            "failed to fund synthetic accounts: {e}"
        )));
    }

    // 8. The `from` for hot-wallet-sourced z_sendmany calls is the account's
    //    creation-time UA, resolved (not derived) in step 4. A UA source draws
    //    the account's shielded funds, which is what the hot wallet holds after
    //    warmup (orchard coinbase) or shielding (transparent coinbase).
    let hot_wallet_address = hot_wallet.address.clone();

    Ok(SetupState {
        stack,
        rpc,
        provisioned,
        hot_wallet_uuid,
        hot_wallet_address,
    })
}

// ── mining ────────────────────────────────────────────────────────────────────

/// Blocks per `generate` call in [`mine_blocks`]. With an Orchard (shielded)
/// `miner_address`, every block carries a halo2 coinbase proof — measured at
/// ~2.2 s per block on an emulated (linux/amd64-on-aarch64) Zebra — so a chunk
/// must fit comfortably inside the RPC client's 30 s HTTP timeout. 5 blocks
/// ≈ 11 s worst-case. Transparent-coinbase chunks are near-instant either way.
const MINE_CHUNK_BLOCKS: u32 = 5;

/// Mine `total` blocks in chunks of [`MINE_CHUNK_BLOCKS`].
///
/// A single `generate(total)` call cannot work for a large `total` with a
/// shielded miner address: Zebra keeps proving and submitting blocks
/// server-side while the HTTP request times out client-side, the retry
/// re-issues the full request, and the chain over-mines while the caller only
/// ever sees transport errors (measured: 188 blocks accepted server-side while
/// warmup "failed"). Chunking keeps every call well inside the timeout.
///
/// Transport errors are retried a few times per chunk: right after stack
/// startup the rpc-router can still be restarting, and an emulated prover can
/// occasionally push a chunk past the deadline.
async fn mine_blocks(rpc: &RpcClient, total: u32) -> Result<(), RunnerError> {
    let mut remaining = total;
    while remaining > 0 {
        let chunk = remaining.min(MINE_CHUNK_BLOCKS);
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            match rpc.generate(chunk).await {
                Ok(_) => break,
                Err(RpcError::Transport(_)) if attempts < 12 => {
                    sleep(Duration::from_secs(5)).await;
                }
                Err(e) => {
                    return Err(RunnerError::Setup(format!(
                        "generate({chunk}) failed with {} of {} blocks left: {e}",
                        remaining, total
                    )))
                }
            }
        }
        remaining -= chunk;
    }
    Ok(())
}

// ── funding ───────────────────────────────────────────────────────────────────

/// Fund every ACTIVE synthetic account from the hot wallet, sized to the
/// scenario's expected spend, then mine the confirmations that make the funds
/// spendable before the load phase begins.
async fn fund_active_accounts(
    rpc: &Arc<RpcClient>,
    scenario: &ScenarioConfig,
    hot_wallet: &FundedAccount,
    provisioned: &ProvisionedPopulation,
) -> Result<(), RunnerError> {
    let active_ids = &provisioned.population.active_account_ids;
    if active_ids.is_empty() {
        return Ok(());
    }

    // Per-account budget: the run issues ~tps × duration intents spread over
    // the active accounts, each at most `max_zatoshis`, plus ZIP-317 fees.
    // Double the per-intent expectation for headroom (an account can be picked
    // more often than the mean).
    //
    // The transparent side is COUNT-based, not just value-based: a transparent
    // spend consumes its whole UTXO and its change returns to the account's
    // shielded pool, so each expected transparent intent needs its own UTXO
    // (see FundingPlan). The shielded side is one output; shielded change
    // stays in the account's own pool.
    // All amount arithmetic is in ZATOSHIS: float multiplication produces
    // values with more than 8 decimals (0.1 × 1.5 = 0.15000000000000002),
    // which Zallet rejects with `-3 Invalid amount`.
    let expected_intents =
        (scenario.load_target_tps * scenario.load_duration_seconds as f64).ceil();
    let per_account_intents = ((expected_intents / active_ids.len() as f64).ceil() as u64).max(1);
    let max_zat = scenario.amounts.max_zatoshis;
    // Each UTXO covers one intent at the maximum amount plus fee headroom.
    let transparent_zat_each = (max_zat + max_zat / 2).max(1_000_000);
    let shielded_zat = (per_account_intents * max_zat * 2).max(100_000_000);
    let plan = funding::FundingPlan {
        transparent_outputs: (per_account_intents * 2) as u32,
        transparent_zec_each: funding::zat_to_zec(transparent_zat_each),
        shielded_zec: funding::zat_to_zec(shielded_zat),
    };

    let sinks: Vec<FundedAccount> = active_ids
        .iter()
        .map(|account_id| {
            let uuid = provisioned
                .zallet_uuids
                .get(account_id)
                .cloned()
                .ok_or_else(|| {
                    RunnerError::Setup(format!("no Zallet UUID for active account {account_id}"))
                })?;
            let address = provisioned
                .zallet_addresses
                .get(account_id)
                .cloned()
                .ok_or_else(|| {
                    RunnerError::Setup(format!("no address for active account {account_id}"))
                })?;
            let transparent_receiver = provisioned
                .zallet_transparent_receivers
                .get(account_id)
                .cloned();
            Ok(FundedAccount {
                uuid,
                address,
                transparent_receiver,
                orchard_receiver: None,
            })
        })
        .collect::<Result<_, RunnerError>>()?;

    funding::fund_accounts(rpc, hot_wallet, &sinks, plan)
        .await
        .map_err(|e| RunnerError::Setup(e.to_string()))?;

    // The fan-out outputs need anchor confirmations before the accounts can
    // spend them; mine those now so the first intents of the load phase do not
    // all stall (the background miner only advances one block per tick).
    mine_blocks(rpc, ANCHOR_CONFIRMATIONS).await?;

    Ok(())
}

// ── warmup ────────────────────────────────────────────────────────────────────

/// Mine warmup blocks and verify that the stack is responsive.
pub async fn warmup(
    rpc: &RpcClient,
    scenario: &ScenarioConfig,
    run_id: &str,
    metrics: Arc<dyn MetricsRecorder>,
) -> Result<(), RunnerError> {
    // Mine warmup blocks in chunks — see mine_blocks for why a single
    // generate(warmup_blocks) call cannot work with shielded coinbase.
    mine_blocks(rpc, scenario.warmup_blocks as u32)
        .await
        .map_err(|e| RunnerError::Warmup(format!("warmup mining failed: {e}")))?;

    // Confirm chain is advancing.
    rpc.get_blockchain_info()
        .await
        .map_err(|e| RunnerError::Warmup(format!("get_blockchain_info failed: {e}")))?;

    // Confirm Zallet is responding.
    rpc.get_wallet_info()
        .await
        .map_err(|e| RunnerError::Warmup(format!("get_wallet_info failed: {e}")))?;

    // Verify that warmup mining funded the hot wallet. generate() returns once
    // Zebra has mined the blocks, but Zallet's sync is asynchronous: it pulls
    // from Zaino's block cache in a background loop. Poll for up to 60 seconds
    // so the check isn't racy against Zallet's sync lag.
    //
    // Persistent 0 balance means either Zebra's miner_address is not pointing at
    // the hot_wallet account (run scripts/dev/regtest-miner-setup.sh), or
    // warmup_blocks is below the regtest coinbase maturity window.
    let mut funded = false;
    for _ in 0..30 {
        let balance = rpc
            .z_get_total_balance()
            .await
            .map_err(|e| RunnerError::Warmup(format!("balance check failed: {e}")))?;
        let total_zec: f64 = balance.total.parse().unwrap_or(0.0);
        if total_zec > 0.0 {
            funded = true;
            break;
        }
        sleep(Duration::from_secs(2)).await;
    }
    if !funded {
        return Err(RunnerError::Warmup(
            "hot wallet has 0 balance after warmup mining — verify that Zebra's \
             miner_address is the hot_wallet account's transparent receiver (run \
             scripts/dev/regtest-miner-setup.sh), and that warmup_blocks exceeds \
             the regtest coinbase maturity window"
                .into(),
        ));
    }

    // A non-zero balance is necessary but NOT sufficient: it counts outputs the
    // wallet has merely *received*. A measured run on Zallet v0.1.0-alpha.3 held
    // 662.50 ZEC in 105-confirmation coinbase UTXOs, visible to z_listunspent and
    // owned by the sending account, while every z_sendmany still answered
    // "Insufficient balance (have 0)". Treating receipt as spendability is how a
    // 0%-confirmation run came to look healthy, so also require an output old
    // enough to be selectable. The threshold is pool-aware: the 100-block
    // coinbase maturity applies to TRANSPARENT coinbase only (ZIP 213); shielded
    // coinbase — the preferred warmup route, mined to the hot wallet's Orchard
    // receiver — only needs the ~10-confirmation anchor policy.
    //
    // The definitive spendability proof is the account fan-out that follows in
    // setup(): it retries while the wallet catches up and fails loudly if the
    // funds cannot actually be spent. That is also why a z_listunspent failure
    // here degrades to a log instead of failing setup — one shielded coinbase
    // note breaks z_listunspent wallet-wide on beta.1 (`get_memo` UTF-8, an
    // upstream defect recorded in docs/regtest-funding-plan.md), which would
    // otherwise make the healthiest warmup configuration fail this check.
    match rpc.z_list_unspent(1, None).await {
        Ok(utxos) => {
            let spendable_age = |u: &crate::rpc::UnspentNote| match u.pool.as_deref() {
                // Missing pool defaults to the stricter transparent reading.
                Some("transparent") | None => u.confirmations >= COINBASE_MATURITY_BLOCKS,
                Some(_) => u.confirmations >= ANCHOR_CONFIRMATIONS as u64,
            };
            if !utxos.iter().any(spendable_age) {
                return Err(RunnerError::Warmup(format!(
                    "hot wallet holds a balance but no output is old enough to spend \
                     (transparent coinbase needs {COINBASE_MATURITY_BLOCKS} confirmations, \
                     shielded needs {ANCHOR_CONFIRMATIONS}; {} unspent output(s), highest \
                     confirmations {}) — raise warmup_blocks",
                    utxos.len(),
                    utxos.iter().map(|u| u.confirmations).max().unwrap_or(0),
                )));
            }
        }
        Err(e) => {
            eprintln!(
                "warmup: z_listunspent failed ({e}); skipping the output-age check — \
                 known Zallet beta.1 defect when shielded coinbase notes exist \
                 (WalletDb::get_memo UTF-8). The account fan-out will prove spendability."
            );
        }
    }

    // Record warmup metric.
    metrics.record_metric(MetricSample {
        run_id: run_id.to_string(),
        timestamp: Utc::now(),
        metric_name: "warmup_blocks_mined".to_string(),
        value: scenario.warmup_blocks as f64,
        labels: Default::default(),
    });

    Ok(())
}

// ── teardown ──────────────────────────────────────────────────────────────────

/// Stop the Z3 stack, flush latency samples, complete the run manifest, and
/// optionally generate the summary report.
///
/// Always runs even on failed loads. `load_succeeded` gates summary generation
/// only — the stack is stopped and the manifest is finalized regardless.
pub async fn teardown(
    mut stack: Z3Stack,
    run_dir: &RunDir,
    manifest: &mut RunManifest,
    recorder: &JsonlRecorder,
    scenario_source_path: &str,
    load_succeeded: bool,
) -> Result<(), RunnerError> {
    // 1. Stop the Z3 stack.
    stack
        .stop()
        .await
        .map_err(|e| RunnerError::Teardown(e.to_string()))?;

    // 2. Finalize the run's on-disk artifacts.
    finalize_run_artifacts(
        run_dir,
        manifest,
        recorder,
        scenario_source_path,
        load_succeeded,
    )
}

/// Finalize a run's on-disk artifacts: flush latency samples, complete and
/// persist the manifest, copy the scenario YAML, and (on success) generate the
/// summary report.
///
/// Split out of [`teardown`] so it can also run on a *setup* failure — where the
/// stack has already been stopped by [`setup`]. Without this, an aborted setup
/// would leave the run directory with a null `run_completed_at`, unflushed
/// latency samples, and no scenario copy.
pub fn finalize_run_artifacts(
    run_dir: &RunDir,
    manifest: &mut RunManifest,
    recorder: &JsonlRecorder,
    scenario_source_path: &str,
    load_succeeded: bool,
) -> Result<(), RunnerError> {
    // 1. Flush accumulated latency samples to metrics.jsonl.
    recorder.flush_latency_samples(&manifest.run_id);

    // 2. Mark the run complete and persist the final manifest.
    manifest.run_completed_at = Some(Utc::now());
    write_manifest(&run_dir.manifest_path(), manifest)
        .map_err(|e| RunnerError::Metrics(e.to_string()))?;

    // 3. Copy the scenario YAML into the run directory for reproducibility.
    //    Skipped silently when source_path is empty (e.g. in unit tests).
    run_dir
        .copy_scenario_yaml(Path::new(scenario_source_path))
        .map_err(|e| RunnerError::Metrics(e.to_string()))?;

    // 4. Generate the Markdown summary report (successful runs only).
    if load_succeeded {
        generate_summary(run_dir, manifest).map_err(|e| RunnerError::Metrics(e.to_string()))?;
    }

    Ok(())
}
