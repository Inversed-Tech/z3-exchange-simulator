//! Scenario runner: load, validate, provision, and drive load against the Z3 stack.
//!
//! The primary entry point is [`run`]. Use [`RunOptions`] to customize the
//! load shape, concurrency, and output path; use [`load_scenario`] +
//! [`validate_scenario`] to load and validate a YAML config before calling [`run`].

use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;

use crate::data_model::{MetricSample, ScenarioConfig};
use crate::metrics::{
    read_simulator_commit, read_z3_commits, write_manifest, JsonlRecorder, MetricsRecorder, RunDir,
    RunManifest,
};
use crate::scenarios::exchange::{run_mempool_watcher, PollingConfig};
use crate::synthetic::generators::TransactionIntentGenerator;

pub mod config;
pub mod dispatch;
pub mod funding;
pub mod lifecycle;
pub mod provisioner;
pub mod result;
pub mod scheduler;

pub use config::{load_scenario, validate_scenario, ConfigError};
pub use provisioner::PopulationPlan;
pub use result::{IntentOutcome, RunResult, RunStats};
pub use scheduler::LoadShape;

use config::print_dry_run_summary;
use dispatch::{background_miner, build_intent_future, periodic_balance_check};
use lifecycle::{finalize_run_artifacts, setup, teardown, SetupState};
use provisioner::{ProvisionedPopulation, ProvisionerError};
use scheduler::{mixed_flow_config, Scheduler};

// ── RunnerError ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RunnerError {
    Config(ConfigError),
    Setup(String),
    Provision(ProvisionerError),
    Warmup(String),
    Load(String),
    Teardown(String),
    Metrics(String),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerError::Config(e) => write!(f, "Config error: {e}"),
            RunnerError::Setup(e) => write!(f, "Setup error: {e}"),
            RunnerError::Provision(e) => write!(f, "Provision error: {e}"),
            RunnerError::Warmup(e) => write!(f, "Warmup error: {e}"),
            RunnerError::Load(e) => write!(f, "Load error: {e}"),
            RunnerError::Teardown(e) => write!(f, "Teardown error: {e}"),
            RunnerError::Metrics(e) => write!(f, "Metrics error: {e}"),
        }
    }
}

impl std::error::Error for RunnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunnerError::Config(e) => Some(e),
            RunnerError::Provision(e) => Some(e),
            _ => None,
        }
    }
}

// ── RunOptions ────────────────────────────────────────────────────────────────

/// Knobs that control how the runner behaves without touching the scenario YAML.
pub struct RunOptions {
    /// Base directory for per-run output (logs, metrics, etc.).
    pub output_base: PathBuf,
    /// Load-shape profile used by the scheduler.
    pub load_shape: LoadShape,
    /// Maximum number of concurrently in-flight intents.
    pub max_in_flight: usize,
    /// If true, print a summary and exit without starting the Z3 stack.
    pub dry_run: bool,
    /// Override the default `PollingConfig`.
    pub polling: Option<PollingConfig>,
    /// Reuse an existing Zallet hot wallet UUID instead of creating a new one.
    pub hot_wallet_uuid: Option<String>,
    /// Cancel token — signal this to abort the load phase early.
    pub cancel: Option<tokio_util::sync::CancellationToken>,
    /// Interval between background regtest block-mining ticks during the load
    /// phase. Block cadence bounds confirmation latency, so a value far above
    /// the confirmation-poll interval quantizes (and inflates) measured latency;
    /// tune it to the stack under test rather than measuring the miner's tick.
    pub block_interval: Duration,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            output_base: PathBuf::from("experiments/runs"),
            load_shape: LoadShape::SteadyState,
            max_in_flight: 64,
            dry_run: false,
            polling: None,
            hot_wallet_uuid: None,
            cancel: None,
            block_interval: Duration::from_secs(2),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn generate_run_id(scenario_name: &str) -> String {
    format!("{}-{}", Utc::now().format("%Y%m%dT%H%M%SZ"), scenario_name)
}

// ── run ───────────────────────────────────────────────────────────────────────

/// The primary entry point. Validates the config, starts Z3, provisions the
/// population, runs the load phase, and returns aggregated outcomes.
pub async fn run(scenario: ScenarioConfig, opts: RunOptions) -> Result<RunResult, RunnerError> {
    // 1. Validate.
    validate_scenario(&scenario).map_err(RunnerError::Config)?;

    // 2. Short-circuit for dry runs. No filesystem artifacts are produced.
    if opts.dry_run {
        let run_id = generate_run_id(&scenario.name);
        let plan = PopulationPlan {
            account_count: scenario.accounts_count,
            active_count: (scenario.accounts_count as f64 * scenario.accounts_active_fraction)
                .floor() as u64,
        };
        print_dry_run_summary(&scenario, &opts, &plan);
        return Ok(RunResult {
            run_id,
            output_dir: None,
            dry_run: true,
            stats: RunStats::default(),
            outcomes: vec![],
        });
    }

    // 3. Create the per-run output directory and derive the canonical run ID.
    let run_dir = RunDir::create(&opts.output_base, &scenario.name)
        .map_err(|e| RunnerError::Metrics(e.to_string()))?;
    let run_id = run_dir.run_id.clone();

    // 4. Create the metrics recorder.
    let recorder =
        Arc::new(JsonlRecorder::new(&run_dir).map_err(|e| RunnerError::Metrics(e.to_string()))?);
    let metrics: Arc<dyn MetricsRecorder> = recorder.clone();

    // 5. Write the initial run manifest (completed_at filled in by teardown).
    let (zebra_commit, zaino_commit, zallet_commit) = read_z3_commits(Path::new("z3-commits.lock"));
    let mut manifest = RunManifest {
        run_id: run_id.clone(),
        run_started_at: Utc::now(),
        run_completed_at: None,
        simulator_commit: read_simulator_commit(),
        zebra_commit,
        zaino_commit,
        zallet_commit,
        scenario_name: scenario.name.clone(),
        scenario_config_hash: scenario.config_hash.clone(),
        target_tps: scenario.load_target_tps,
    };
    write_manifest(&run_dir.manifest_path(), &manifest)
        .map_err(|e| RunnerError::Metrics(e.to_string()))?;

    // 6. Setup: start Z3, warmup, provision population.
    //    Warmup (mining) runs inside setup() before provisioning so the hot
    //    wallet account exists when coinbase blocks are mined — otherwise Zallet
    //    sets the account birthday at the current tip and misses every prior
    //    coinbase output, leaving the warmup balance check at 0. (An earlier
    //    comment here credited this ordering with resetting a wallet-global
    //    transparent gap counter and thereby letting all N synthetic accounts
    //    receive transparent receivers. That is not what it does: synthetic
    //    accounts are derived orchard-only and get no transparent receiver at
    //    all. See docs/zallet-transparent-gap-limit.md.)
    let setup_state = match setup(&scenario, &opts, &run_id, &run_dir, metrics.clone()).await {
        Ok(s) => s,
        Err(e) => {
            // setup() already stopped the stack on every failure path. Still
            // finalize the run artifacts so an aborted setup doesn't leave a run
            // dir with a null completed_at, unflushed latency samples, and no
            // scenario copy. Best-effort: the setup error is the one we report.
            let _ = finalize_run_artifacts(
                &run_dir,
                &mut manifest,
                &recorder,
                &scenario.source_path,
                false,
            );
            return Err(e);
        }
    };

    let SetupState {
        stack,
        rpc,
        provisioned,
        // The resolved hot-wallet UUID is not needed by the load phase: every
        // flow spends from `hot_wallet_address`.
        hot_wallet_uuid: _,
        hot_wallet_address,
    } = setup_state;

    let provisioned = Arc::new(provisioned);

    // 7. Load phase.
    let load_result = load_phase(
        rpc.clone(),
        provisioned,
        &scenario,
        &opts,
        &run_id,
        &hot_wallet_address,
        metrics.clone(),
    )
    .await;

    // 8. Teardown always runs; propagate its error only when load succeeded.
    let load_succeeded = load_result.is_ok();
    let teardown_result = teardown(
        stack,
        &run_dir,
        &mut manifest,
        &recorder,
        &scenario.source_path,
        load_succeeded,
    )
    .await;

    match load_result {
        Ok((total_attempted, outcomes)) => {
            teardown_result?;
            let mut stats = RunStats {
                total_attempted,
                ..Default::default()
            };
            for o in &outcomes {
                match o {
                    IntentOutcome::WithdrawalOk(_) | IntentOutcome::DepositOk(_) => {
                        stats.confirmed += 1;
                    }
                    IntentOutcome::Failed { .. } => stats.failed += 1,
                    IntentOutcome::TimedOut { .. } => stats.timed_out += 1,
                }
            }
            Ok(RunResult {
                run_id,
                output_dir: Some(run_dir.path.clone()),
                dry_run: false,
                stats,
                outcomes,
            })
        }
        Err(e) => Err(e),
    }
}

// ── load_phase ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn load_phase(
    rpc: Arc<crate::rpc::RpcClient>,
    provisioned: Arc<ProvisionedPopulation>,
    scenario: &ScenarioConfig,
    opts: &RunOptions,
    run_id: &str,
    hot_wallet_address: &str,
    metrics: Arc<dyn MetricsRecorder>,
) -> Result<(u64, Vec<IntentOutcome>), RunnerError> {
    // For Mixed shape, override the flow config.
    let effective_scenario = if matches!(opts.load_shape, LoadShape::Mixed) {
        let mut s = scenario.clone();
        s.flows = mixed_flow_config();
        s
    } else {
        scenario.clone()
    };

    // Build the transaction intent generator.
    let mut intent_gen =
        TransactionIntentGenerator::new(&provisioned.population, &effective_scenario)
            .map_err(|e| RunnerError::Load(e.to_string()))?;

    // Concurrency controls.
    let sem = Arc::new(Semaphore::new(opts.max_in_flight));
    let active_count = Arc::new(AtomicUsize::new(0));
    let mut tasks: JoinSet<IntentOutcome> = JoinSet::new();

    let polling = opts.polling.unwrap_or_default();

    // Shutdown channels for background tasks.
    let (mempool_tx, mempool_rx) = tokio::sync::oneshot::channel::<()>();
    let (balance_tx, balance_rx) = tokio::sync::oneshot::channel::<()>();
    let (miner_tx, miner_rx) = tokio::sync::oneshot::channel::<()>();

    // Start mempool watcher.
    let mempool_interval =
        Duration::from_secs(scenario.observability.metric_sampling_interval_secs);
    let mempool_threshold = scenario.observability.mempool_saturation_threshold;
    tokio::spawn(run_mempool_watcher(
        rpc.clone(),
        run_id.to_string(),
        mempool_threshold,
        mempool_interval,
        metrics.clone(),
        mempool_rx,
    ));

    // Start background miner (cadence configured via RunOptions::block_interval).
    tokio::spawn(background_miner(rpc.clone(), opts.block_interval, miner_rx));

    // Start periodic balance check.
    tokio::spawn(periodic_balance_check(
        rpc.clone(),
        run_id.to_string(),
        mempool_interval,
        metrics.clone(),
        active_count.clone(),
        balance_rx,
    ));

    // Scheduler loop.
    let scheduler = Scheduler::new(opts.load_shape.clone(), scenario.load_target_tps);
    let load_duration = Duration::from_secs(scenario.load_duration_seconds);
    let start = Instant::now();
    let mut total_attempted: u64 = 0;

    let initial_tps = scheduler.initial_tps();
    let initial_interval = Duration::from_secs_f64(1.0 / initial_tps);
    let mut ticker = tokio::time::interval_at(
        tokio::time::Instant::now() + initial_interval,
        initial_interval,
    );
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        let elapsed = start.elapsed();
        if elapsed >= load_duration {
            break;
        }

        // Check cancellation.
        if let Some(cancel) = &opts.cancel {
            if cancel.is_cancelled() {
                break;
            }
        }

        // Update the tick interval based on current TPS.
        let tps = scheduler.instantaneous_tps(elapsed).max(0.001);
        let period = Duration::from_secs_f64(1.0 / tps);
        ticker.reset_after(period);
        ticker.tick().await;

        // Dispatch next intent.
        if let Some(intent) = intent_gen.next_intent(run_id, &provisioned.population) {
            total_attempted += 1;
            let fut = build_intent_future(
                intent,
                rpc.clone(),
                sem.clone(),
                active_count.clone(),
                metrics.clone(),
                hot_wallet_address.to_string(),
                run_id.to_string(),
                polling,
                provisioned.zallet_uuids.clone(),
                provisioned.zallet_addresses.clone(),
            );
            tasks.spawn(fut);
        }
    }

    // Signal background tasks to stop.
    let _ = mempool_tx.send(());
    let _ = balance_tx.send(());
    let _ = miner_tx.send(());

    // Drain all in-flight tasks.
    let mut outcomes = Vec::with_capacity(tasks.len());
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(outcome) => outcomes.push(outcome),
            Err(join_err) => {
                outcomes.push(IntentOutcome::Failed {
                    intent_id: "unknown".to_string(),
                    flow_type: crate::data_model::FlowType::TToT,
                    error: format!("task panicked: {join_err}"),
                });
            }
        }
    }

    // Emit end-of-load summary metrics consumed by generate_summary.
    let confirmed = outcomes
        .iter()
        .filter(|o| {
            matches!(
                o,
                IntentOutcome::WithdrawalOk(_) | IntentOutcome::DepositOk(_)
            )
        })
        .count();
    let failed = outcomes
        .iter()
        .filter(|o| {
            matches!(
                o,
                IntentOutcome::Failed { .. } | IntentOutcome::TimedOut { .. }
            )
        })
        .count();
    let tps_achieved = total_attempted as f64 / load_duration.as_secs_f64().max(1.0);
    for (name, value) in [
        ("tps_achieved", tps_achieved),
        ("confirmed_txs_total", confirmed as f64),
        ("failed_txs_total", failed as f64),
    ] {
        metrics.record_metric(MetricSample {
            run_id: run_id.to_string(),
            timestamp: Utc::now(),
            metric_name: name.to_string(),
            value,
            labels: Default::default(),
        });
    }

    Ok((total_attempted, outcomes))
}
