//! Lifecycle management: setup, warmup, and teardown of the Z3 stack.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;

use crate::data_model::{MetricSample, ScenarioConfig};
use crate::metrics::{
    generate_summary, write_manifest, JsonlRecorder, MetricsRecorder, RunDir, RunManifest,
};
use crate::rpc::RpcClient;
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

    // 2. Start the Z3 stack.
    let mut stack = Z3Stack::new(z3_config, Some(metrics.clone()));
    stack
        .start()
        .await
        .map_err(|e| RunnerError::Setup(e.to_string()))?;

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

    // 4. Provision the synthetic population.
    let provisioned = match provision(
        rpc.clone(),
        scenario,
        run_id,
        metrics.clone(),
        opts.hot_wallet_uuid.clone(),
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            let _ = stack.stop().await;
            return Err(RunnerError::Provision(e));
        }
    };

    // 5. Derive the hot wallet's Unified Address.
    let hot_wallet_address = match rpc
        .z_get_address_for_account(&provisioned.hot_wallet_uuid)
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

    let hot_wallet_uuid = provisioned.hot_wallet_uuid.clone();

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
    // Mine warmup blocks.
    rpc.generate(scenario.warmup_blocks as u32)
        .await
        .map_err(|e| RunnerError::Warmup(format!("generate failed: {e}")))?;

    // Confirm chain is advancing.
    rpc.get_blockchain_info()
        .await
        .map_err(|e| RunnerError::Warmup(format!("get_blockchain_info failed: {e}")))?;

    // Confirm Zallet is responding.
    rpc.get_wallet_info()
        .await
        .map_err(|e| RunnerError::Warmup(format!("get_wallet_info failed: {e}")))?;

    // Verify that warmup mining funded the hot wallet. A zero balance means
    // Zebra's miner_address is not pointing at a Zallet account (check Z3's
    // regtest-init.sh), or warmup_blocks is below the regtest coinbase maturity
    // window (increase warmup_blocks in the scenario config).
    let balance = rpc
        .z_get_total_balance()
        .await
        .map_err(|e| RunnerError::Warmup(format!("balance check failed: {e}")))?;
    let total_zec: f64 = balance.total.parse().unwrap_or(0.0);
    if total_zec == 0.0 {
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

    // 2. Flush accumulated latency samples to metrics.jsonl.
    recorder.flush_latency_samples(&manifest.run_id);

    // 3. Mark the run complete and persist the final manifest.
    manifest.run_completed_at = Some(Utc::now());
    write_manifest(&run_dir.manifest_path(), manifest)
        .map_err(|e| RunnerError::Metrics(e.to_string()))?;

    // 4. Copy the scenario YAML into the run directory for reproducibility.
    //    Skipped silently when source_path is empty (e.g. in unit tests).
    run_dir
        .copy_scenario_yaml(Path::new(scenario_source_path))
        .map_err(|e| RunnerError::Metrics(e.to_string()))?;

    // 5. Generate the Markdown summary report (successful runs only).
    if load_succeeded {
        generate_summary(run_dir, manifest).map_err(|e| RunnerError::Metrics(e.to_string()))?;
    }

    Ok(())
}
