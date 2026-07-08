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
use crate::scenarios::runner::provisioner::{provision, ProvisionedPopulation};
use crate::scenarios::runner::RunOptions;
use crate::scenarios::runner::RunnerError;
use crate::z3::{Z3Config, Z3Stack};

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

    // 4. Ensure the hot wallet account exists in Zallet BEFORE mining warmup
    //    blocks. Zallet tracks coinbase payments by scanning blocks for known
    //    account addresses. If the account is only created after mining, Zallet
    //    sets a birthday at the current tip and misses all prior coinbase outputs,
    //    causing the warmup balance check to see 0 ZEC.
    let hot_wallet_uuid = match opts.hot_wallet_uuid.clone() {
        Some(uuid) => uuid,
        None => {
            // Find the account named "hot_wallet" created by init-regtest-fresh.sh.
            // Do NOT take the first account from z_listaccounts: Zallet returns
            // accounts sorted by UUID (not creation order), so synthetic accounts
            // from earlier test runs appear first and have 0 balance.
            let existing = rpc
                .z_list_accounts()
                .await
                .map_err(|e| RunnerError::Setup(format!("z_list_accounts failed: {e}")))?;
            if let Some(hw) = existing.into_iter().find(|a| a.name.as_deref() == Some("hot_wallet")) {
                hw.account
            } else {
                // Fresh volumes with no named account: create one now.
                // warmup blocks will fund it via the ZEBRA_MINING__MINER_ADDRESS.
                rpc.z_get_new_account("hot_wallet")
                    .await
                    .map_err(|e| RunnerError::Setup(format!("z_get_new_account failed: {e}")))?
                    .account
            }
        }
    };

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

    // 7. Retrieve the hot wallet UA at diversifier_index=0. That address was
    //    created by init-regtest-fresh.sh with receivers [orchard, p2pkh], and
    //    its transparent receiver is ZEBRA_MINING__MINER_ADDRESS. Passing it as
    //    the `from` parameter in z_sendmany lets Zallet spend the transparent
    //    coinbase inputs without triggering the "legacy account" restriction.
    //    diversifier_index=0 is explicit so we always get the funded address,
    //    not a newly-created one at the next available index.
    let hot_wallet_address = match rpc
        .z_get_address_for_account(&hot_wallet_uuid, &["orchard", "p2pkh"], Some(0))
        .await
    {
        Ok(ua) => ua.address,
        Err(e) => {
            let _ = stack.stop().await;
            return Err(RunnerError::Setup(format!(
                "failed to get hot wallet address: {e}"
            )));
        }
    };

    Ok(SetupState {
        stack,
        rpc,
        provisioned,
        hot_wallet_uuid,
        hot_wallet_address,
    })
}

// ── warmup ────────────────────────────────────────────────────────────────────

/// Mine warmup blocks and verify that the stack is responsive.
pub async fn warmup(
    rpc: &RpcClient,
    scenario: &ScenarioConfig,
    run_id: &str,
    metrics: Arc<dyn MetricsRecorder>,
) -> Result<(), RunnerError> {
    // Mine warmup blocks. Retry on transport errors: rpc-router can still be
    // restarting (waiting on Zallet's rpc.discover) in the seconds after
    // wait_until_ready() returns. generate() routes to Zebra and will succeed
    // as soon as rpc-router stabilizes.
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        match rpc.generate(scenario.warmup_blocks as u32).await {
            Ok(_) => break,
            Err(RpcError::Transport(_)) if attempts < 12 => {
                sleep(Duration::from_secs(5)).await;
            }
            Err(e) => return Err(RunnerError::Warmup(format!("generate failed: {e}"))),
        }
    }

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
    // Persistent 0 balance means either Zebra's miner_address is not pointing
    // at a Zallet account (check Z3's regtest-init.sh), or warmup_blocks is
    // below the regtest coinbase maturity window.
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
            "hot wallet has 0 balance after warmup mining — verify that Z3's \
             regtest-init.sh configured Zebra's miner_address to a Zallet-managed \
             account, and that warmup_blocks exceeds the regtest coinbase maturity \
             window"
                .into(),
        ));
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
