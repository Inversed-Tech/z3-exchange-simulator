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

use crate::data_model::{MetricSample, Phase, ScenarioConfig};
use crate::metrics::{
    read_reset_state, read_simulator_commit, read_z3_commits, write_manifest, JsonlRecorder,
    MetricsRecorder, RunDir, RunManifest, RunTimeouts, StateFreshness, StateIdentifier,
};
use crate::scenarios::exchange::{run_mempool_watcher, PollingConfig};
use crate::synthetic::generators::TransactionIntentGenerator;
use crate::z3::env_id;

pub mod config;
pub mod dispatch;
pub mod funding;
pub mod lifecycle;
pub mod phase;
pub mod progress;
pub mod provisioner;
pub mod result;
pub mod scheduler;

pub use config::{load_scenario, validate_scenario, ConfigError};
pub use phase::PhaseTracker;
pub use progress::ProgressLine;
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
    /// Maximum number of concurrent `z_getnewaccount` / `z_listunifiedreceivers`
    /// calls during provisioning. See `docs/z3-concurrent-request-ceiling.md` —
    /// concurrent requests above ~11 fail outright on the override stack, so
    /// this should stay at or below that margin regardless of how many
    /// accounts the scenario provisions.
    pub provision_concurrency: usize,
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
    /// Discard this checkout's cached environment id and mint a fresh,
    /// disposable one instead of reusing it (see `z3::env_id`). Use to run a
    /// second, independent environment concurrently with one already in
    /// progress against the same checkout.
    pub fresh_env: bool,
    /// Path to this checkout's cached environment id (see
    /// `z3::env_id::resolve_env_id`). Defaults to `configs/local/env-id`, the
    /// existing convention for gitignored, per-checkout machine state.
    /// Overridable (e.g. to a tempdir) so tests exercising `setup()` don't
    /// mutate the real checkout.
    pub env_id_cache_path: PathBuf,
    /// Directory holding the per-`env_id` concurrency lock files (see
    /// `z3::run_lock::acquire`). Defaults to `configs/local`.
    pub run_lock_dir: PathBuf,
    /// Path to the cloned Z3 Docker Compose repository. Defaults to
    /// `external/z3`. Overridable so tests exercising `setup()`/the CLI
    /// dispatch layer can point it at a path guaranteed not to exist and get
    /// a fast, side-effect-free failure — see
    /// `z3::Z3Config::check_preconditions` — instead of reaching real
    /// bootstrap scripts and Docker state on a machine that happens to have
    /// a real checkout already configured.
    pub compose_dir: PathBuf,
    /// Directory holding the per-`env_id` reset-epoch markers written by
    /// `scripts/dev/regtest-reset.sh` (see `z3::env_id::reset_epoch_path`)
    /// and read into `RunManifest::state.reset_epoch` (see
    /// `metrics::manifest::read_reset_state`). Defaults to `configs/local`,
    /// alongside `env-id` and the per-`env_id` lock files. The actual
    /// filename read is `reset-epoch-<this run's resolved env_id>` — scoped
    /// per environment so a `--fresh-env` run never reads the stable
    /// environment's reset provenance, or vice versa. A missing file reads
    /// as "no reset has run against this specific environment yet" rather
    /// than an error, so this needs no test-only override.
    pub reset_epoch_dir: PathBuf,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            output_base: PathBuf::from("experiments/runs"),
            load_shape: LoadShape::SteadyState,
            max_in_flight: 64,
            provision_concurrency: 8,
            dry_run: false,
            polling: None,
            hot_wallet_uuid: None,
            cancel: None,
            block_interval: Duration::from_secs(2),
            fresh_env: false,
            env_id_cache_path: PathBuf::from("configs/local/env-id"),
            run_lock_dir: PathBuf::from("configs/local"),
            compose_dir: PathBuf::from("external/z3"),
            reset_epoch_dir: PathBuf::from("configs/local"),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn generate_run_id(scenario_name: &str) -> String {
    format!("{}-{}", Utc::now().format("%Y%m%dT%H%M%SZ"), scenario_name)
}

/// `count` per second of `elapsed` wall-clock time, floored at 1 second so a
/// sub-second `elapsed` can't produce a division blow-up. Shared by
/// `scheduled_dispatch_rate` (elapsed = actual load-phase duration) and
/// `confirmed_tx_throughput` (elapsed = actual Load+Drain duration) — same
/// formula, different inputs; see their call sites in `load_phase` for which
/// `Duration` each must be given.
fn rate_per_second(count: u64, elapsed: Duration) -> f64 {
    count as f64 / elapsed.as_secs_f64().max(1.0)
}

/// Logical CPUs available to this process, for `RunManifest::host_cpu_count`.
fn host_cpu_count() -> u32 {
    std::thread::available_parallelism().map_or(0, |n| n.get() as u32)
}

/// This host's memory limit in bytes, for `RunManifest::host_memory_limit_bytes` —
/// `None` on an unconstrained bare-metal/VM host (the common case for where
/// `z3sim` itself runs; only the Z3 stack's own containers are ever
/// resource-constrained today). Reads a Linux cgroup memory limit when one is
/// in effect (cgroup v2 first, falling back to v1), since that is the one
/// case a `z3sim` process could itself be running under a real constraint
/// (e.g. a containerized CI runner); no such concept exists on macOS/Windows,
/// so this is `None` there unconditionally.
fn host_memory_limit_bytes() -> Option<u64> {
    for path in [
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ] {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some(limit) = parse_cgroup_memory_limit(&content) {
                return Some(limit);
            }
        }
    }
    None
}

/// Parses a cgroup memory-limit file's content. Cgroup v2 writes the literal
/// `max` for "unconstrained"; cgroup v1 writes a huge sentinel value instead
/// (typically `i64::MAX` rounded down to the page size) — both mean the same
/// thing, so anything at or above 2^62 bytes (4 exbibytes; no real host has
/// this much RAM) is treated as unconstrained rather than as a real limit.
fn parse_cgroup_memory_limit(content: &str) -> Option<u64> {
    let trimmed = content.trim();
    if trimmed == "max" {
        return None;
    }
    let value: u64 = trimmed.parse().ok()?;
    if value >= (1u64 << 62) {
        None
    } else {
        Some(value)
    }
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
            // Dry runs never execute a load phase, so there is nothing to
            // evaluate — trivially "passing" is the only sensible value.
            assertion: result::AssertionOutcome {
                passed: true,
                violations: vec![],
            },
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
    // Anchored to the crate root at compile time, not resolved relative to the
    // process's working directory at run time — a relative path here silently
    // reads whatever file happens to occupy that path from the invoking shell's
    // cwd, which is exactly the kind of build/run environment dependency this
    // manifest exists to rule out.
    let lock_path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/z3-commits.lock"));
    let (zebra_commit, zaino_commit, zallet_commit) = read_z3_commits(lock_path);
    // Same resolution `load_phase` applies later — recorded here so the
    // manifest reflects the polling patience actually in effect for this run,
    // not just the crate's compiled-in default.
    let effective_polling = opts.polling.unwrap_or_default();
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
        timeouts: RunTimeouts {
            rpc_timeout_ms: crate::rpc::DEFAULT_TIMEOUT.as_millis() as u64,
            operation_poll_interval_ms: effective_polling.operation_poll_interval.as_millis()
                as u64,
            max_operation_wait_ms: effective_polling.max_operation_wait.as_millis() as u64,
            confirmation_poll_interval_ms: effective_polling.confirmation_poll_interval.as_millis()
                as u64,
            max_confirmation_wait_ms: effective_polling.max_confirmation_wait.as_millis() as u64,
        },
        phase_boundaries: Vec::new(),
        load_and_drain_completed_at: None,
        compose_config_hash: String::new(),
        image_digests: Vec::new(),
        host_cpu_count: host_cpu_count(),
        host_memory_limit_bytes: host_memory_limit_bytes(),
        state: StateIdentifier::default(),
        assertion: None,
    };
    write_manifest(&run_dir.manifest_path(), &manifest)
        .map_err(|e| RunnerError::Metrics(e.to_string()))?;

    // 5.5. Start tracking lifecycle phases BEFORE setup() runs, so the
    //    Bootstrap boundary genuinely predates Z3Stack::start() rather than
    //    being recorded only once an RpcClient happens to exist partway
    //    through setup.
    let phase_tracker = PhaseTracker::new();
    let progress = ProgressLine::new();

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
    let setup_state = match setup(
        &scenario,
        &opts,
        &run_id,
        &run_dir,
        metrics.clone(),
        &phase_tracker,
        &progress,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            // setup() already stopped the stack on every failure path. Still
            // finalize the run artifacts so an aborted setup doesn't leave a run
            // dir with a null completed_at, unflushed latency samples, and no
            // scenario copy. Best-effort: the setup error is the one we report.
            manifest.phase_boundaries = phase_tracker.boundaries();
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
        // Held for the remainder of this function — including through the
        // load phase and teardown below — so the environment stays reserved
        // for the entire run. Released when it drops at function exit.
        run_lock: _run_lock,
        env_id: resolved_env_id,
        chain_height_at_start,
        hot_wallet_balance_at_start_zat,
    } = setup_state;

    // Read the state/image/config evidence now, while the stack is still up
    // (image_digests requires running containers) and before `stack` moves
    // into `teardown` below. These are evidence fields, not correctness
    // dependencies — a failure degrades to an empty/default value with a
    // warning rather than failing an otherwise-successful run.
    manifest.image_digests = match stack.image_digests().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("failed to read running image digests for manifest: {e}");
            Vec::new()
        }
    };
    manifest.compose_config_hash = match stack.compose_config_hash().await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("failed to compute compose config hash for manifest: {e}");
            String::new()
        }
    };
    // Scoped to THIS run's resolved env_id, not a single checkout-wide file
    // — otherwise a `--fresh-env` run would read (or a stable run would be
    // misattributed) another environment's reset provenance entirely.
    let reset_epoch_path = env_id::reset_epoch_path(&opts.reset_epoch_dir, &resolved_env_id);
    let (reset_epoch, height_at_reset) = read_reset_state(&reset_epoch_path);
    manifest.state = StateIdentifier {
        reset_epoch,
        chain_height_at_start,
        hot_wallet_balance_at_start_zat,
        freshness: StateFreshness::classify(chain_height_at_start, height_at_reset),
    };

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
        &phase_tracker,
        &progress,
    )
    .await;
    // Captured at the exact instant `load_phase()` returns — the same
    // instant its own `confirmed_tx_throughput` metric's elapsed-time
    // denominator stops counting (see `load_phase`'s `start.elapsed()` at
    // its own return). Teardown (stopping the Z3 stack) happens after this
    // point, so this timestamp — not `run_completed_at` — is the correct
    // end boundary for the Drain phase's *measured* duration; the report
    // shows any residual gap to `run_completed_at` as a separate Teardown
    // row instead of silently folding it into Drain.
    let load_and_drain_completed_at = Utc::now();

    // Tally stats and persist per-intent outcomes to intents.jsonl *before*
    // teardown — teardown calls finalize_run_artifacts(), which generates
    // summary.md from whatever intents.jsonl contains at that moment. Doing
    // this after teardown (as an earlier version of this code did) meant
    // summary.md was always generated before any intent was recorded, so its
    // "Outcomes by flow type" section silently never appeared.
    let stats_and_assertion = if let Ok((total_attempted, outcomes)) = &load_result {
        let mut stats = RunStats {
            total_attempted: *total_attempted,
            ..Default::default()
        };
        let mut intent_records = Vec::with_capacity(outcomes.len());
        for o in outcomes {
            match o {
                IntentOutcome::WithdrawalOk { .. } | IntentOutcome::DepositOk { .. } => {
                    stats.confirmed += 1;
                }
                IntentOutcome::Failed { .. } => stats.failed += 1,
                IntentOutcome::TimedOut { .. } => stats.timed_out += 1,
            }
            let record = crate::data_model::IntentRecord::from_outcome(o, &run_id);
            recorder.record_intent(record.clone());
            intent_records.push(record);
        }
        let failures_by_class = result::terminal_failures_by_class(&intent_records);
        let assertion = stats.evaluate(&scenario.expectations, &failures_by_class);
        Some((stats, assertion))
    } else {
        None
    };

    // 8. Teardown always runs; propagate its error only when load succeeded.
    let load_succeeded = load_result.is_ok();
    manifest.phase_boundaries = phase_tracker.boundaries();
    manifest.load_and_drain_completed_at = Some(load_and_drain_completed_at);
    manifest.assertion = stats_and_assertion.as_ref().map(|(_, a)| a.clone());
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
        Ok((_, outcomes)) => {
            teardown_result?;
            let (stats, assertion) = stats_and_assertion
                .expect("stats/assertion computed above whenever load_result is Ok");
            Ok(RunResult {
                run_id,
                output_dir: Some(run_dir.path.clone()),
                dry_run: false,
                stats,
                outcomes,
                assertion,
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
    phase_tracker: &PhaseTracker,
    progress: &ProgressLine,
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

    // Mark the Load phase BEFORE spawning the background miner, mempool
    // watcher, and balance checker below: each can issue its first RPC call
    // as soon as the async runtime schedules it, which may happen before the
    // scheduler loop's own `Instant::now()` a few lines further down —
    // marking Load only there would leave a real window in which those
    // calls are still tagged `Phase::Funding`.
    phase_tracker.mark(Phase::Load);

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

    // Throttles the dispatch loop's own progress line to roughly once per
    // second of real elapsed time, not once per tick — a high-TPS scenario
    // ticks far more often than that and would otherwise flood the line.
    let mut last_progress_update = Instant::now() - Duration::from_secs(1);

    let mut is_first_tick = true;
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

        // Update the tick interval based on current TPS — except on the
        // first tick, which the ticker was already primed for above via
        // `initial_tps` (see `Scheduler::dispatch_tick_period`'s doc comment
        // for why recomputing here on that first tick is wrong).
        if let Some(period) = scheduler.dispatch_tick_period(elapsed, is_first_tick) {
            ticker.reset_after(period);
        }
        is_first_tick = false;
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

        if last_progress_update.elapsed() >= Duration::from_secs(1) {
            progress.update(
                Phase::Load,
                &format!(
                    "{total_attempted} dispatched, {} in flight",
                    active_count.load(std::sync::atomic::Ordering::Relaxed)
                ),
                elapsed,
                Some(load_duration),
            );
            last_progress_update = Instant::now();
        }
    }
    progress.finish();
    // Real elapsed dispatch-loop duration — distinct from `load_duration`
    // (the configured target): scheduler jitter and tick-interval rounding
    // mean the loop can run slightly over or under it.
    let load_elapsed = start.elapsed();
    phase_tracker.mark(Phase::Drain);

    // Drain all in-flight tasks. The background miner, mempool watcher, and
    // balance checker are kept running throughout this drain (signalled to
    // stop only after it completes, below) — otherwise transactions still
    // in flight when the load window ends are waiting on a chain that has
    // already been told to stop producing blocks, guaranteeing every one of
    // them times out regardless of the actual state of the backend.
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

    // Signal background tasks to stop now that every in-flight task has
    // resolved (confirmed, failed, or timed out).
    let _ = mempool_tx.send(());
    let _ = balance_tx.send(());
    let _ = miner_tx.send(());

    // Emit end-of-load summary metrics consumed by generate_summary.
    let confirmed = outcomes
        .iter()
        .filter(|o| {
            matches!(
                o,
                IntentOutcome::WithdrawalOk { .. } | IntentOutcome::DepositOk { .. }
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
    // Scheduled-dispatch rate: how many intents the scheduler dispatched per
    // second of ACTUAL load-phase wall-clock time (`load_elapsed`, captured
    // via `start.elapsed()` right when the dispatch loop broke — see above —
    // NOT the configured `load_duration_seconds`) — a measure of the
    // scheduler's own dispatch behavior, not of confirmed throughput. See
    // `confirmed_tx_throughput` below for the latter; only that metric is
    // "TPS" in the report.
    let scheduled_dispatch_rate = rate_per_second(total_attempted, load_elapsed);
    // Confirmed-transaction throughput across the Load+Drain window (from
    // this function's own entry through the end of the drain, i.e. the same
    // instant these end-of-load metrics are computed) — the only rate this
    // codebase or its reports may label "TPS".
    let confirmed_tx_throughput = rate_per_second(confirmed as u64, start.elapsed());
    for (name, value) in [
        ("scheduled_dispatch_rate", scheduled_dispatch_rate),
        ("confirmed_tx_throughput", confirmed_tx_throughput),
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── host resources ───────────────────────────────────────────────────────

    #[test]
    fn host_cpu_count_matches_available_parallelism() {
        let expected = std::thread::available_parallelism().map_or(0, |n| n.get() as u32);
        assert_eq!(host_cpu_count(), expected);
    }

    #[test]
    fn parse_cgroup_memory_limit_treats_v2_max_as_unconstrained() {
        assert_eq!(parse_cgroup_memory_limit("max\n"), None);
    }

    #[test]
    fn parse_cgroup_memory_limit_treats_v1_huge_sentinel_as_unconstrained() {
        // The classic cgroup v1 "unconstrained" sentinel: i64::MAX rounded
        // down to the host's page size.
        assert_eq!(parse_cgroup_memory_limit("9223372036854771712\n"), None);
    }

    #[test]
    fn parse_cgroup_memory_limit_returns_a_real_limit() {
        assert_eq!(
            parse_cgroup_memory_limit("2147483648\n"),
            Some(2_147_483_648)
        );
    }

    #[test]
    fn parse_cgroup_memory_limit_rejects_garbage() {
        assert_eq!(parse_cgroup_memory_limit("not-a-number"), None);
    }

    // ── rate_per_second ────────────────────────────────────────────────────

    #[test]
    fn rate_per_second_uses_the_given_elapsed_not_a_hardcoded_duration() {
        // The whole point of the scheduled_dispatch_rate rename (from the
        // old tps_achieved, which divided by the *configured*
        // load_duration_seconds) is that the caller controls which elapsed
        // time is used. Feed two different `elapsed` values for the same
        // count and confirm the result actually changes accordingly —
        // a regression guard against this ever being hardcoded again.
        let fast = rate_per_second(100, Duration::from_secs(10));
        let slow = rate_per_second(100, Duration::from_secs(20));
        assert_eq!(fast, 10.0);
        assert_eq!(slow, 5.0);
        assert!(
            fast > slow,
            "a shorter elapsed must yield a higher rate for the same count"
        );
    }

    #[test]
    fn rate_per_second_floors_elapsed_at_one_second() {
        // Guards against a division blow-up for a sub-second window (e.g. a
        // scenario whose load phase dispatches nothing and returns near-
        // instantly).
        assert_eq!(rate_per_second(5, Duration::from_millis(100)), 5.0);
    }

    #[test]
    fn rate_per_second_zero_count_is_zero() {
        assert_eq!(rate_per_second(0, Duration::from_secs(10)), 0.0);
    }

    // ── structural ordering guards ──────────────────────────────────────────
    //
    // These two invariants (PhaseTracker constructed before setup() starts;
    // Phase::Load marked before the background tasks are spawned) are
    // enforced by plain sequential program order in the current source, not
    // by any runtime race — there is no `.await` between the two statements
    // in either pair, so under Rust's synchronous execution model within one
    // task, reordering either pair is a source-code change, not a scheduling
    // outcome. A live-stack test can't exercise this any more directly than
    // reading the source; these regression guards do that mechanically, so a
    // future refactor that silently reorders either pair fails CI instead of
    // only reintroducing the exact mislabeling bug this track fixes.

    fn own_source() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/scenarios/runner/mod.rs"
        ))
        .expect("this file must be readable at test time")
    }

    #[test]
    fn phase_tracker_is_constructed_before_setup_is_called() {
        let src = own_source();
        let tracker_pos = src
            .find("let phase_tracker = PhaseTracker::new();")
            .expect("PhaseTracker::new() call site must exist in run()");
        let setup_pos = src
            .find("let setup_state = match setup(")
            .expect("setup() call site must exist in run()");
        assert!(
            tracker_pos < setup_pos,
            "PhaseTracker::new() must be constructed before setup() is called, so the \
             Bootstrap phase boundary predates Z3Stack::start() — found PhaseTracker::new() \
             at byte {tracker_pos}, setup( at byte {setup_pos}"
        );
    }

    #[test]
    fn load_phase_marks_load_before_spawning_background_tasks() {
        let src = own_source();

        // Scope the search to load_phase's own body, ending at the
        // scheduler's own dispatch-task spawn (`tasks.spawn(fut)` — a
        // different call, `JoinSet::spawn`, not `tokio::spawn`, and the
        // last thing before the dispatch loop's closing brace) so a
        // tokio::spawn appearing in some unrelated later function can never
        // stand in for one of the three background-task spawns this test
        // means to check.
        let fn_start = src
            .find("async fn load_phase(")
            .expect("load_phase must exist in this file");
        let fn_body = &src[fn_start..];
        let dispatch_end = fn_body
            .find("tasks.spawn(fut)")
            .expect("the scheduler's own dispatch-task spawn must exist in load_phase()");
        let scope = &fn_body[..dispatch_end];

        let mark_pos = scope
            .find("phase_tracker.mark(Phase::Load);")
            .expect("phase_tracker.mark(Phase::Load) call site must exist in load_phase()");

        // Every tokio::spawn(...) in scope (the mempool watcher, miner, and
        // balance-checker spawns) must come after the mark — check the
        // EARLIEST one, not merely "the next one found from the mark's
        // position onward" (which would trivially pass even if the mark sat
        // between two of the three spawns).
        let spawn_positions: Vec<usize> = scope
            .match_indices("tokio::spawn(")
            .map(|(pos, _)| pos)
            .collect();
        assert!(
            !spawn_positions.is_empty(),
            "expected at least one tokio::spawn(...) background-task call in load_phase()"
        );
        assert_eq!(
            spawn_positions.len(),
            3,
            "expected exactly 3 background-task tokio::spawn(...) calls (mempool watcher, \
             miner, balance checker) in load_phase(); found {}",
            spawn_positions.len()
        );
        let earliest_spawn_pos = *spawn_positions.iter().min().unwrap();
        assert!(
            mark_pos < earliest_spawn_pos,
            "phase_tracker.mark(Phase::Load) must precede every tokio::spawn(...) of a \
             background task — found the mark at byte {mark_pos}, earliest tokio::spawn( at \
             byte {earliest_spawn_pos}"
        );
    }

    // ── Load-mark-before-spawn: behavioral confirmation ─────────────────────

    #[tokio::test]
    async fn background_task_spawned_after_the_load_mark_is_tagged_load_not_the_prior_phase() {
        // Exercises the real production functions (PhaseTracker,
        // RpcClient::attach_phase_tracker, background_miner) in exactly the
        // order load_phase() itself uses them: mark(Phase::Load), then
        // tokio::spawn the background task, with no `.await` in between —
        // matching the production code's own structure (see the structural
        // test above). Proves the tagging outcome the structural test's
        // ordering exists to guarantee.
        use crate::rpc::RpcClient;
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["blockhash0"],
                "error": null,
                "id": 1
            })))
            .mount(&server)
            .await;

        struct RecordingRecorder {
            calls: std::sync::Mutex<Vec<crate::data_model::RpcCall>>,
        }
        impl MetricsRecorder for RecordingRecorder {
            fn record_rpc_call(&self, call: crate::data_model::RpcCall) {
                self.calls.lock().unwrap().push(call);
            }
            fn record_metric(&self, _: MetricSample) {}
        }

        let recorder = Arc::new(RecordingRecorder {
            calls: std::sync::Mutex::new(Vec::new()),
        });
        let metrics: Arc<dyn MetricsRecorder> = recorder.clone();
        let mut client = RpcClient::new(server.uri(), "test-run", Some(metrics), None);

        let tracker = PhaseTracker::new();
        tracker.mark(Phase::Funding);
        client.attach_phase_tracker(tracker.shared_atomic());
        let rpc = Arc::new(client);

        // Production ordering: mark(Load) immediately before the spawn, no
        // `.await` between them.
        tracker.mark(Phase::Load);
        let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(dispatch::background_miner(rpc, Duration::from_secs(2), rx));

        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.abort();

        let calls = recorder.calls.lock().unwrap();
        assert!(
            !calls.is_empty(),
            "expected background_miner to have issued at least one call"
        );
        for call in calls.iter() {
            assert_eq!(
                call.phase,
                Phase::Load,
                "background_miner's call must be tagged Load, not the phase active before \
                 the mark — got {:?}",
                call.phase
            );
        }
    }
}
