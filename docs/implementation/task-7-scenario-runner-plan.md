# Task 7 — Scenario Runner: Implementation Plan

> **Status:** Planning — not yet implemented  
> **Branch target:** `main` (merge after T6 is reviewed and merged)  
> **Prerequisites:** T1–T5 on `main`; T6 on `plan/t6-observability-metrics` (unmerged)  
> **Output module:** `src/scenarios/runner/`

---

## 1. Executive Summary

Task 7 is the convergence point for the entire Z3 Exchange Simulator. It wires together the Z3 harness (T2), RPC client (T3), synthetic generators (T4), exchange emulation workflows (T5), and the observability layer (T6) into a single orchestrated run loop. The public surface is intentionally minimal:

```rust
pub async fn run(scenario: ScenarioConfig, opts: RunOptions) -> Result<RunResult, RunnerError>
```

Internally, T7 executes four sequential phases — **Setup → Warmup → Load → Teardown** — with a Tokio-native TPS scheduler and a semaphore-bounded concurrency model. Dry-run mode short-circuits at provisioning without touching the Z3 stack.

Because T6 is not yet merged, the runner is built in two tiers:

- **Tier 1 (current `main`):** compiles against the minimal `MetricsRecorder` trait already on `main`, using a `NullRecorder` defined in T7 itself. All metrics hook-points are wired; only the JSONL writers are absent.
- **Tier 2 (post-T6 merge):** swap `NullRecorder` for `JsonlRecorder` + `RunDir`, enable manifest writing, and activate summary generation.

---

## 2. Repository Findings

### 2.1 What exists and is ready

| Track | Module | State |
|-------|--------|-------|
| T1 — Data model | `src/data_model/mod.rs` | Complete. All 12 core types defined. |
| T2 — Z3 harness | `src/z3/mod.rs` | Complete. `Z3Stack::start/stop`, health check, Docker Compose lifecycle. |
| T3 — RPC client | `src/rpc/mod.rs` | Complete. All JSON-RPC methods, routing table, timing, `MetricsRecorder` integration. |
| T4 — Synthetic generators | `src/synthetic/` | Complete. `AccountGenerator`, `SyntheticPopulation`, `TransactionIntentGenerator` with ChaCha8Rng determinism. |
| T5 — Exchange workflows | `src/scenarios/exchange.rs` | Complete. `run_deposit`, `run_withdrawal`, `run_sweep`, `run_balance_check`, `run_mempool_watcher`. |
| Regtest control | `src/scenarios/regtest_control.rs` | Complete. `run_reorg`. |
| T6 — Observability | `src/metrics/` (branch `plan/t6-observability-metrics`) | Complete but **unmerged**. `JsonlRecorder`, `RunDir`, `RunManifest`, latency accumulator, JSONL writers, `generate_summary`. |

### 2.2 What does not yet exist (T7 must create)

- `src/scenarios/runner/` — module does not exist
- `RunResult` type — not defined anywhere
- `RunOptions` struct — not defined
- `LoadShape` enum — not in `ScenarioConfig` or elsewhere
- `NullRecorder` — not on `main`; T7 Phase 1 must define it
- `ProvisionedPopulation` — not defined; provisioner needs this wrapper (see Section 8)
- `sha2` crate — not in `Cargo.toml` (needed for config hash computation)
- `docs/implementation/` directory — created for this document

### 2.3 Gaps in `ScenarioConfig` (T1 type, `src/data_model/mod.rs`)

The current `ScenarioConfig` has no fields for:

- Load shape variant (steady / ramp / burst / mixed)
- Warmup duration (blocks to mine before load begins)
- Ramp duration (seconds over which TPS scales from 0 to target)
- Burst parameters (spike multiplier, burst window length)
- Maximum in-flight concurrency (backpressure limit)
- Hot-wallet account identifier

**Decision:** These are run-time operational parameters, not scenario identity fields. They belong in a separate `RunOptions` struct passed to `run()`, not in `ScenarioConfig`. This keeps `ScenarioConfig` as the reproducibility-determining artifact (hashed into the manifest) and `RunOptions` as the operator's control knobs.

Exception: `warmup_blocks` is scenario-specific (smoke needs fewer than steady-state) and should be added to `ScenarioConfig` with a sensible default via `#[serde(default = "default_warmup_blocks")]`.

### 2.4 Key type signatures (from existing code)

```rust
// T2
pub struct Z3Stack { ... }
impl Z3Stack {
    pub fn new(config: Z3Config, metrics: Option<Arc<dyn MetricsRecorder>>) -> Self
    pub async fn start(&mut self) -> Result<(), Z3Error>
    pub async fn stop(&mut self) -> Result<(), Z3Error>
}

// T3
pub struct RpcClient { ... }
impl RpcClient {
    pub fn new(base_url: impl Into<String>, run_id: impl Into<String>, metrics: Option<Arc<dyn MetricsRecorder>>, timeout: Option<Duration>) -> Self
    //   ^ `&str` is accepted at call sites because `&str: Into<String>`
    pub fn with_basic_auth(self, username: impl Into<String>, password: impl Into<String>) -> Self
    //   ^ Required by the Z3 regtest RPC Router (default "zebra" / "zebra"). Must be called
    //   ^ after new(). Clone rpc_url and basic_auth from Z3Config BEFORE moving it into Z3Stack.
    pub async fn z_get_new_account(&self, name: &str) -> Result<AccountInfo, RpcError>
    //   ^ AccountInfo { account: String, name: Option<String> }
    pub async fn z_get_address_for_account(&self, account: &str) -> Result<UnifiedAddress, RpcError>
    //   ^ UnifiedAddress { account: String, address: String, receiver_types: Vec<String> }
    //   ^ one argument only; always returns a Unified Address (no pool-type selection)
    pub async fn get_wallet_info(&self) -> Result<WalletInfo, RpcError>
}

// T4
pub struct AccountGenerator { ... }
impl AccountGenerator {
    pub fn new(config: ScenarioConfig) -> Result<Self, GeneratorError>
    //   ^ takes ScenarioConfig by value (owned)
    pub fn generate_population(&mut self) -> Result<SyntheticPopulation, GeneratorError>
    //   ^ &mut self
}
pub struct TransactionIntentGenerator { ... }
impl TransactionIntentGenerator {
    pub fn new(population: &SyntheticPopulation, config: &ScenarioConfig) -> Result<Self, GeneratorError>
    //   ^ both borrowed; population is NOT Clone
    pub fn next_intent(&mut self, run_id: &str, population: &SyntheticPopulation) -> Option<TransactionIntent>
}
// SyntheticPopulation does NOT implement Clone — must wrap in Arc for shared ownership

// T5 (exchange.rs)
pub async fn run_deposit(
    rpc: &RpcClient,
    account_id: &str,
    zallet_uuid: &str,       // Zallet UUID of account_id's Zallet account (the exchange acts on behalf of account_id)
    from_account: &str,      // hot wallet Zallet account UUID (must be Zallet-managed; see §8.4)
    amount_zatoshis: u64,
    required_confirmations: u64,
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
    polling: &PollingConfig,
) -> Result<Deposit, ExchangeError>

pub async fn run_withdrawal(
    rpc: &RpcClient,
    account_id: &str,
    from_account: &str,      // Zallet account UUID (AccountInfo.account)
    destination_address: &str,
    amount_zatoshis: u64,
    intent_id: Option<&str>,
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
    polling: &PollingConfig,
) -> Result<Withdrawal, ExchangeError>

pub async fn run_sweep(
    rpc: &RpcClient,
    from_account: &str,      // Zallet account UUID to sweep FROM
    hot_wallet_address: &str, // transparent address to sweep TO
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
    polling: &PollingConfig,
) -> Result<Sweep, ExchangeError>

pub async fn run_balance_check(
    rpc: &RpcClient,
    wallet_id: &str,
    run_id: &str,
    metrics: Option<Arc<dyn MetricsRecorder>>,
) -> Result<Balance, ExchangeError>

pub async fn run_mempool_watcher(
    rpc: Arc<RpcClient>,
    run_id: String,
    saturation_threshold: u64,
    interval: Duration,
    metrics: Arc<dyn MetricsRecorder>,  // non-Optional; use Arc::new(NullRecorder) in Phase 1-3
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
)

// T6 MetricsRecorder trait (on main — minimal stub)
pub trait MetricsRecorder: Send + Sync {
    fn record_rpc_call(&self, call: RpcCall);
    fn record_metric(&self, sample: MetricSample);
}
// NullRecorder is NOT on main. T7 Phase 1 must define it (see Section 3.2).
```

### 2.5 Cargo.toml — required additions

```toml
sha2 = "0.10"                                              # config hashing; no other SHA crate is present
tokio-util = { version = "0.7", features = ["sync"] }     # CancellationToken for RunOptions (Phase 1)
```

No `hex` crate — use hand-rolled hex formatting per OQ-6 (resolved).
No `anyhow` — T7 must use typed error variants matching the project's existing pattern.

---

## 3. Target Architecture

### 3.1 Module layout

```
src/scenarios/runner/
├── mod.rs          # Public API: RunOptions, RunResult, RunnerError, generate_run_id, pub async fn run()
├── config.rs       # ScenarioConfig loading, validation, config_hash computation
├── lifecycle.rs    # Phase orchestration: setup, warmup, load, teardown
├── scheduler.rs    # TPS scheduler: LoadShape, interval-based rate limiter per shape
├── provisioner.rs  # Account provisioning: z_get_new_account + z_get_address_for_account
├── dispatch.rs     # Intent dispatch: per-intent task spawn, semaphore backpressure, JoinSet
└── result.rs       # RunResult, IntentOutcome, RunStats aggregation
```

### 3.2 Additions to existing modules

**`src/scenarios/mod.rs`** — add `pub mod runner;`

**`src/data_model/mod.rs`** — add `warmup_blocks: u64` field to `ScenarioConfig` with default of 10:

```rust
#[serde(default = "default_warmup_blocks")]
pub warmup_blocks: u64,

fn default_warmup_blocks() -> u64 { 10 }
```

**`src/metrics/mod.rs`** — extend with `NullRecorder` (T7 owns this until T6 is merged):

```rust
pub struct NullRecorder;
impl MetricsRecorder for NullRecorder {
    fn record_rpc_call(&self, _: RpcCall) {}
    fn record_metric(&self, _: MetricSample) {}
}
```

**Important — Phase 4 cleanup:** When T6 merges, the T6 branch defines `NullRecorder` in `src/metrics/recorder.rs` and re-exports it with `pub use recorder::NullRecorder` in `src/metrics/mod.rs`. Phase 4 Step 4 must **remove the inline struct definition and its impl** from `src/metrics/mod.rs`; leaving both causes a duplicate-definition error that Git cannot auto-resolve. T6's re-export provides the type transparently.

**`src/scenarios/exchange.rs`** — add `#[derive(Clone, Copy)]` to `PollingConfig` (all four fields are `Duration`, which is `Copy`; deriving both eliminates `.clone()` calls at dispatch call sites and is idiomatic for small all-copy structs). This is a T5-touching cross-module change; note it explicitly in the Phase 2 PR.

**`Cargo.toml`** — add `sha2 = "0.10"` and `tokio-util = { version = "0.7", features = ["sync"] }` to `[dependencies]` (see Section 2.5).

### 3.3 Public API surface (`src/scenarios/runner/mod.rs`)

```rust
pub use config::{load_scenario, validate_scenario};
pub use result::{RunResult, RunStats, IntentOutcome};
pub use scheduler::LoadShape;

pub struct RunOptions {
    /// Base directory for experiment output (parent of the per-run directory).
    pub output_base: PathBuf,
    /// Load shape to apply during the load phase.
    pub load_shape: LoadShape,
    /// Maximum in-flight concurrent transaction tasks. Default: 64.
    pub max_in_flight: usize,
    /// If true: validate + provision plan, print summary, do not start Z3.
    pub dry_run: bool,
    /// Polling config forwarded to exchange workflows. None uses PollingConfig::default().
    pub polling: Option<PollingConfig>,
    /// Zallet UUID of the hot-wallet account provisioned during setup (see Section 8.4).
    /// None means the runner provisions it automatically during setup; override for
    /// test scenarios that pre-provision a known account.
    pub hot_wallet_uuid: Option<String>,
    /// Optional cancellation token. If set, the load loop checks it each scheduler tick.
    /// Wired by T8 for Ctrl-C. Default: None (not interruptible).
    pub cancel: Option<tokio_util::sync::CancellationToken>,
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
        }
    }
}

/// Produce a run ID of the form `<YYYYMMDDTHHMMSSZ>-<scenario_name>`.
/// Used in Phases 1–3 before T6's RunDir is available.
pub fn generate_run_id(scenario_name: &str) -> String {
    format!("{}-{}", Utc::now().format("%Y%m%dT%H%M%SZ"), scenario_name)
}

/// Execute one scenario end-to-end.
pub async fn run(
    scenario: ScenarioConfig,
    opts: RunOptions,
) -> Result<RunResult, RunnerError>
```

---

## 4. Data Flow

```
┌─────────────────────────────────────────────────────────────────────────┐
│ run(scenario, opts)                                                      │
│                                                                          │
│  generate_run_id(scenario.name) → run_id                                  │
│  run_started_at = Utc::now()  (captured before Z3Stack::start)           │
│  load_scenario / validate → config_hash (sha256)                         │
│         │                                                                │
│         ▼                                                                │
│  [if dry_run] provision_plan → print → return RunResult{dry_run: true}  │
│         │                                                                │
│         ▼                                                                │
│  NullRecorder (Phase 1-3) / JsonlRecorder (Phase 4, after T6 merge)     │
│  RunDir::create() — Phase 4 only                                         │
│         │                                                                │
│         ▼                                                                │
│  SETUP: create log_dir → Z3Stack::start() → RpcClient::new()            │
│         → provisioner::provision() → ProvisionedPopulation              │
│         ↳ z_get_new_account(hot_wallet) → hot_wallet_uuid (see §8.4)    │
│         ↳ z_get_new_account × N → z_get_address_for_account × N         │
│         ↳ SyntheticPopulation + zallet_uuids Arc<HashMap>               │
│         │                                                                │
│         ▼                                                                │
│  WARMUP: generate(warmup_blocks) → get_blockchain_info + get_wallet_info │
│         (hot wallet = Zallet account provisioned during setup; §8.4)     │
│         │                                                                │
│         ▼                                                                │
│  LOAD: scheduler ticks at shape-controlled rate (interval-based)         │
│         ↳ intent_gen.next_intent() → dispatch::build_intent_future()     │
│              ↳ Future pushed to JoinSet (JoinSet::spawn)                 │
│              ↳ acquires Semaphore permit before T5 call                  │
│              ↳ single T5 call (ZToT: two sequential calls, §9.2)         │
│         ↳ run_mempool_watcher (T5) in background task                   │
│         ↳ periodic_balance_check background task (§9.2)                 │
│         ↳ AtomicUsize tracks active task count for active_accounts metric│
│         │                                                                │
│         ▼                                                                │
│  TEARDOWN: stop scheduler → drain JoinSet → stop watcher                │
│         ↳ Z3Stack::stop()                                                │
│         ↳ [Phase 4] write_manifest(run_completed_at)                    │
│         ↳ [Phase 4] generate_summary() → summary.md                     │
│         ↳ [Phase 4] copy scenario YAML to run dir                       │
│         │                                                                │
│         ▼                                                                │
│  RunResult { run_id, stats, outcomes, dry_run }                          │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 5. Run Lifecycle

### Phase 1: Setup

**Goal:** Start the Z3 stack, create the RPC client and metrics recorder, provision all synthetic accounts.

```rust
async fn setup(
    scenario: &ScenarioConfig,
    opts: &RunOptions,
    run_id: &str,
    metrics: Arc<dyn MetricsRecorder>,
) -> Result<SetupState, RunnerError>
```

Steps:
1. Construct the log directory path and create it:
   ```rust
   let log_dir = opts.output_base.join(run_id).join("component_logs");
   std::fs::create_dir_all(&log_dir).map_err(|e| RunnerError::Setup(e.to_string()))?;
   ```
   In Phase 4, replace with `run_dir.component_logs_dir()`.
2. Construct `Z3Config::for_run(run_id, log_dir)` (or `from_contract` if `z3-contract.yaml` is present).
   **Clone `rpc_url` and `basic_auth` before the next step moves `z3_config`:**
   ```rust
   let rpc_url = z3_config.rpc_url.clone();
   let basic_auth = z3_config.basic_auth.clone();  // Option<(String, String)>
   ```
3. Call `Z3Stack::new(z3_config, Some(metrics.clone()))` and `stack.start().await`.
   (`z3_config` is moved here — `rpc_url` and `basic_auth` must already be cloned.)
4. Construct the `RpcClient` with auth. Auth is required by the Z3 regtest RPC Router
   (default credentials `"zebra"` / `"zebra"`). Without it every call returns HTTP 401:
   ```rust
   let rpc = {
       let client = RpcClient::new(&rpc_url, run_id, Some(metrics.clone()), Some(Duration::from_secs(30)));
       match basic_auth {
           Some((user, pass)) => client.with_basic_auth(user, pass),
           None => client,
       }
   };
   ```
5. Call `provisioner::provision(rpc, scenario, run_id, metrics.clone(), opts.hot_wallet_uuid.clone()).await`
   — this creates the hot wallet Zallet account (or uses the override) and all synthetic accounts
   (see Section 8.4).
6. Extract `hot_wallet_uuid` from `provisioned.hot_wallet_uuid`.
7. Resolve `hot_wallet_address`: call `rpc.z_get_address_for_account(&hot_wallet_uuid).await` and store `ua.address` (the full Unified Address string). `run_sweep` accepts a UA directly as its `hot_wallet_address` parameter — `z_send_many` routes correctly; no transparent-receiver extraction is needed or possible (`UnifiedAddress` exposes no receiver components, only `address: String`).
8. Return `SetupState { stack, rpc: Arc::new(rpc), provisioned, hot_wallet_uuid, hot_wallet_address }`.

**Error handling:** `Z3Error` and `RpcError` from provisioning are wrapped in `RunnerError::Setup`. On failure, attempt `stack.stop()` before propagating the error.

### Phase 2: Warmup

**Goal:** Bring the chain to a usable state; verify all RPC endpoints are responding.

```rust
async fn warmup(
    rpc: &RpcClient,
    scenario: &ScenarioConfig,
    run_id: &str,
    metrics: Arc<dyn MetricsRecorder>,
) -> Result<(), RunnerError>
```

Steps:
1. Call `rpc.generate(scenario.warmup_blocks)` to advance the chain past the genesis coinbase
   maturity window. This funds the Zebra coinbase transparent address; how those funds reach the
   Zallet hot wallet account is resolved by OQ-9 (Phase 2 blocker — see Section 15).
2. Call `rpc.get_blockchain_info()` — asserts Zebra is responding.
3. Call `rpc.get_wallet_info()` — asserts Zallet is responding.
4. Record a `MetricSample { metric_name: "warmup_blocks_mined", value: warmup_blocks as f64 }`.

**Error handling:** Warmup failures are fatal. Wrapped in `RunnerError::Warmup`.

### Phase 3: Load

**Goal:** Dispatch transaction intents at the scheduled rate for `load_duration_seconds`.

```rust
async fn load(
    rpc: Arc<RpcClient>,
    provisioned: Arc<ProvisionedPopulation>,
    scenario: &ScenarioConfig,
    opts: &RunOptions,
    run_id: &str,
    hot_wallet_address: &str,
    metrics: Arc<dyn MetricsRecorder>,
) -> Result<Vec<IntentOutcome>, RunnerError>
```

Steps:
1. Create `TransactionIntentGenerator::new(&provisioned.population, scenario)`.
2. Start `run_mempool_watcher` in a background task with `Arc::new(NullRecorder) as Arc<dyn MetricsRecorder>` (Phase 1-3) or the real recorder (Phase 4). Retain the `shutdown` sender.
3. Create `Arc<Semaphore>::new(opts.max_in_flight)`.
4. Create `Arc<AtomicUsize>::new(0)` for active task count.
5. Create a `JoinSet<IntentOutcome>` to track all spawned tasks.
6. Create a `Scheduler` for the load shape and target TPS.
7. Drive the scheduler loop (see Section 7) for `load_duration_seconds`, checking `opts.cancel` each tick.
8. On each scheduler tick: call `intent_gen.next_intent(run_id, &provisioned.population)`, push the `JoinHandle` from `spawn_intent_task` into the `JoinSet`.
9. When duration elapses: send shutdown to mempool watcher; drain the `JoinSet` with `jset.join_next().await` collecting `IntentOutcome`s; log any `JoinError` as `IntentOutcome::Failed`.
10. Return collected outcomes.

### Phase 4: Teardown

**Goal:** Clean shutdown regardless of load phase outcome.

```rust
async fn teardown(
    stack: Z3Stack,
    // Phase 4 adds: run_dir: &RunDir, manifest: &mut RunManifest, scenario: &ScenarioConfig
) -> Result<(), RunnerError>
```

Steps:
1. Call `stack.stop().await`. Log but do not propagate `Z3Error` — teardown must not abort output writing.
2. **Phase 4 only:**
   - Set `manifest.run_completed_at = Some(Utc::now())` and call
     `write_manifest(&run_dir.manifest_path(), &manifest).map_err(|e| RunnerError::Metrics(e.to_string()))?`.
   - Call `generate_summary` **only if load succeeded** — if load returned `Err`, skip it (partial
     JSONL is preserved by the append-only writers, but a summary derived from incomplete data would
     be misleading). `generate_summary` returns `Result<String, MetricsError>`; write the string
     explicitly:
     ```rust
     let summary = generate_summary(&run_dir, &manifest)
         .map_err(|e| RunnerError::Metrics(e.to_string()))?;
     std::fs::write(run_dir.summary_path(), summary)
         .map_err(|e| RunnerError::Metrics(e.to_string()))?;
     ```
   - `copy_scenario_yaml` takes `&Path`, not `&String`:
     ```rust
     run_dir.copy_scenario_yaml(Path::new(&scenario.source_path))
         .map_err(|e| RunnerError::Metrics(e.to_string()))?;
     ```

Note: `flush_latency_samples` is a method on `JsonlRecorder` (not on the `MetricsRecorder` trait). It is called directly on the concrete recorder in Phase 4, not through the trait object:
```rust
// Phase 4 teardown, before stack.stop():
if let Some(recorder) = json_recorder.as_ref() {
    recorder.flush_latency_samples(run_id);
}
```

**Invariant:** Teardown always runs. Use `match load_result { Ok(o) => o, Err(e) => { teardown(...).await; return Err(e); } }` in `run()`.

---

## 6. Scenario Config Handling

### 6.1 Loading

```rust
// src/scenarios/runner/config.rs
pub fn load_scenario(path: &Path) -> Result<ScenarioConfig, ConfigError>
```

Steps:
1. Read file bytes with `std::fs::read(path)`.
2. Compute SHA-256 of raw bytes: `let hash = sha2::Sha256::digest(&bytes)`. Format into a local variable: `let config_hash = format!("sha256:{}", hash.iter().map(|b| format!("{b:02x}")).collect::<String>())`. Do NOT assign to `config` yet — it does not exist until step 3.
3. Deserialize: `let mut config = serde_yaml::from_slice::<ScenarioConfig>(&bytes)?`.
4. Set `config.config_hash = config_hash`.
5. Set `config.source_path = path.to_string_lossy().into_owned()`.

**Note:** Hash is computed from raw bytes before deserialization (so the hash reflects literal file content, not the deserialized struct), then assigned after `config` is created in step 3.

### 6.2 Validation

```rust
pub fn validate_scenario(config: &ScenarioConfig) -> Result<(), ConfigError>
```

Collects all violations before returning (not fail-fast):

- `load_target_tps > 0.0`
- `load_duration_seconds > 0`
- `accounts_count >= 1`
- `accounts_active_fraction > 0.0 && accounts_active_fraction <= 1.0`
- `floor(accounts_count × accounts_active_fraction) >= 2` — required by `TransactionIntentGenerator` which always picks two distinct active accounts per intent
- `flows.transparent_to_transparent + flows.transparent_to_shielded + flows.shielded_to_transparent + flows.shielded_to_shielded` within `[0.9999, 1.0001]`
- All individual flow fractions ∈ `[0.0, 1.0]`
- `activity_profiles.low_fraction + activity_profiles.medium_fraction + activity_profiles.high_fraction` within `[0.9999, 1.0001]`
- `confirmations_deposit_required >= 1`
- `amounts.min_zatoshis <= amounts.max_zatoshis` — `TransactionIntentGenerator` calls `rng.gen_range(min..=max)`, which panics if `min > max`

Return `ConfigError::ValidationErrors(violations)` where `violations: Vec<(String, String)>` is `(field_name, message)`.

### 6.3 Dry-run output

Before calling `print_dry_run_summary`, construct `PopulationPlan` from the config (no RPC calls needed):
```rust
let population_plan = PopulationPlan {
    account_count: scenario.accounts_count,
    active_count: (scenario.accounts_count as f64 * scenario.accounts_active_fraction).floor() as u64,
};
```

```rust
pub fn print_dry_run_summary(config: &ScenarioConfig, opts: &RunOptions, population_plan: &PopulationPlan)
```

Prints to stdout:
```
Scenario:      smoke (sha256:abc123...)
Accounts:      10 total, 10 active
Load:          1.0 TPS for 60 s (steady-state)
Warmup:        10 blocks
Max in-flight: 64
Output dir:    experiments/runs/<run-id>
Flows:         100% T→T, 0% T→Z, 0% Z→T, 0% Z→Z
Dry run — no Z3 processes will start.
```

---

## 7. TPS Scheduler Design

### 7.1 `LoadShape` enum

```rust
// src/scenarios/runner/scheduler.rs

#[derive(Debug, Clone)]
pub enum LoadShape {
    /// Constant TPS for the full load duration.
    SteadyState,
    /// Linearly increase TPS from 0 to `target_tps` over `ramp_secs`, then hold steady.
    Ramp { ramp_secs: u64 },
    /// Hold at `target_tps` for `pre_burst_secs`, spike to `spike_multiplier × target_tps`
    /// for `burst_secs`, then return to `target_tps`.
    Burst { pre_burst_secs: u64, burst_secs: u64, spike_multiplier: f64 },
    /// Steady-state TPS with a shielded-biased FlowConfig (50% TToZ, 50% ZToZ).
    /// The scenario's own `flows` field is replaced for the duration of the load phase.
    Mixed,
}
```

### 7.2 Scheduler loop

The scheduler uses `tokio::time::interval` with `MissedTickBehavior::Skip` for deadline-based timing that self-corrects at high TPS without accumulating drift from loop overhead.

```rust
pub struct Scheduler {
    shape: LoadShape,
    target_tps: f64,
}

impl Scheduler {
    fn instantaneous_tps(&self, elapsed: Duration) -> f64 {
        match &self.shape {
            LoadShape::SteadyState | LoadShape::Mixed => self.target_tps,
            LoadShape::Ramp { ramp_secs } => {
                let ramp = Duration::from_secs(*ramp_secs);
                if elapsed >= ramp {
                    self.target_tps
                } else {
                    self.target_tps * (elapsed.as_secs_f64() / ramp.as_secs_f64())
                }
            }
            LoadShape::Burst { pre_burst_secs, burst_secs, spike_multiplier } => {
                let pre = Duration::from_secs(*pre_burst_secs);
                let post = pre + Duration::from_secs(*burst_secs);
                if elapsed < pre { self.target_tps }
                else if elapsed < post { self.target_tps * spike_multiplier }
                else { self.target_tps }
            }
        }
    }

    /// Rate to use for the very first ticker interval.
    ///
    /// `instantaneous_tps(Duration::ZERO)` returns 0 for `Ramp` (mathematically
    /// correct — no load has elapsed), but 1/0 is unusable as a tick interval.
    /// For Ramp, start at ≥10% of target TPS so the initial interval is bounded.
    /// All other shapes return a non-zero value at elapsed=0 and need no special case.
    fn initial_tps(&self) -> f64 {
        match &self.shape {
            LoadShape::Ramp { .. } => (self.target_tps * 0.1).max(0.001),
            _ => self.instantaneous_tps(Duration::ZERO).max(0.001),
        }
    }
}
```

**Scheduler loop body:**

```rust
let start = Instant::now();
let load_duration = Duration::from_secs(scenario.load_duration_seconds);
let mut run_stats = RunStats::default();

// Compute initial interval and create a deadline-based ticker.
// Use initial_tps() rather than instantaneous_tps(ZERO) — for Ramp shapes,
// ZERO elapsed returns 0 TPS (correct mathematically), yielding a 1 000-second
// first interval that effectively delivers only one intent for the entire run.
let initial_tps = scheduler.initial_tps();
let mut ticker = tokio::time::interval(Duration::from_secs_f64(1.0 / initial_tps));
ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

loop {
    ticker.tick().await;
    let elapsed = start.elapsed();
    if elapsed >= load_duration { break; }

    // Recompute interval for Ramp/Burst shapes and reset the ticker if it changed materially.
    let new_tps = scheduler.instantaneous_tps(elapsed).max(0.001);
    let new_interval = Duration::from_secs_f64(1.0 / new_tps);
    if (new_interval.as_secs_f64() - ticker.period().as_secs_f64()).abs() > 0.001 {
        // Use interval_at to avoid an immediate-fire on the first tick after the reset.
        // tokio::time::interval(d) fires immediately at t=0; interval_at delays the first tick.
        let new_start = tokio::time::Instant::now() + new_interval;
        ticker = tokio::time::interval_at(new_start, new_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    }

    // Check cancellation.
    if let Some(ref cancel) = opts.cancel {
        if cancel.is_cancelled() { break; }
    }

    match intent_gen.next_intent(run_id, &provisioned.population) {
        Some(intent) => {
            jset.spawn(dispatch::build_intent_future(
                intent, rpc.clone(), sem.clone(), active_count.clone(),
                metrics.clone(), hot_wallet_uuid.to_string(),
                hot_wallet_address.to_string(),
                run_id.to_string(), polling.clone(),
                Arc::clone(&provisioned.zallet_uuids),
            ));
            run_stats.total_attempted += 1;
        }
        None => break,
    }
}
```

**Backpressure note:** The semaphore is acquired inside each spawned task (before the T5 call), not before spawning. This means the scheduler can queue more tasks than `max_in_flight` allows, which is intentional: it measures _attempted_ TPS rather than _completed_ TPS, providing a realistic backpressure signal. At very high TPS with a small `max_in_flight`, Tokio may accumulate many parked tasks — consider increasing `max_in_flight` before assuming the scheduler is the bottleneck.

### 7.3 Mixed load shape

`LoadShape::Mixed` uses the same tick rate as `SteadyState` but replaces the scenario's `flows` configuration when constructing `TransactionIntentGenerator`. `TransactionIntentGenerator::new` takes `&ScenarioConfig` — the original cannot be mutated. Define a module-level constant for the flow override, then construct a local clone:

```rust
// In scheduler.rs (or dispatcher.rs — wherever Mixed branching lives):
pub(super) const MIXED_FLOW_CONFIG: FlowConfig = FlowConfig {
    transparent_to_transparent: 0.0,
    transparent_to_shielded: 0.5,
    shielded_to_transparent: 0.0,
    shielded_to_shielded: 0.5,
};

let effective_config = if let LoadShape::Mixed = &opts.load_shape {
    let mut c = scenario.clone();
    c.flows = MIXED_FLOW_CONFIG;
    c
} else {
    scenario.clone()
};
let mut intent_gen = TransactionIntentGenerator::new(&provisioned.population, &effective_config)?;
```

This ensures `sender_address` and `recipient_address` inside each generated `TransactionIntent` are consistent with the actual flow type — no post-hoc override.

---

## 8. Account Provisioning Design

### 8.1 Types

```rust
// src/scenarios/runner/provisioner.rs

/// Carries the synthetic population AND the mapping from synthetic account_id
/// to Zallet UUID (AccountInfo.account). run_deposit requires the UUID as a
/// separate parameter; SyntheticPopulation has no field for this.
pub struct ProvisionedPopulation {
    pub population: SyntheticPopulation,
    /// synthetic account_id → Zallet UUID (AccountInfo.account field).
    /// Arc-wrapped so spawned tasks can clone a reference without copying the full map.
    pub zallet_uuids: Arc<HashMap<String, String>>,
    /// UUID of the Zallet hot-wallet account created during provisioning.
    pub hot_wallet_uuid: String,
}

pub struct PopulationPlan {
    pub account_count: u64,
    pub active_count: u64,
}

pub async fn provision(
    rpc: &RpcClient,
    scenario: &ScenarioConfig,
    run_id: &str,
    metrics: Arc<dyn MetricsRecorder>,
    hot_wallet_uuid_override: Option<String>,
) -> Result<ProvisionedPopulation, ProvisionerError>
```

### 8.2 Steps

1. `AccountGenerator::new(scenario.clone()).map_err(ProvisionerError::Generator)?` — takes config by value; clone here so `scenario` is still accessible after.
2. `generator.generate_population().map_err(ProvisionerError::Generator)?` — `generate_population` is `&mut self`.
3. Resolve the hot wallet UUID. If `hot_wallet_uuid_override` is `Some`, use it directly (skip the
   RPC call). Otherwise create a new Zallet account:
   ```rust
   let hot_wallet_uuid = match hot_wallet_uuid_override {
       Some(uuid) => uuid,
       None => rpc.z_get_new_account("hot_wallet").await.map_err(ProvisionerError::Rpc)?.account,
   };
   ```
4. For each account in `population.accounts`: call Zallet to create a real account and address (see parallelism below).
5. Record a `MetricSample { metric_name: "accounts_provisioned", value: count as f64 }`.
6. Return `ProvisionedPopulation { population, zallet_uuids: Arc::new(zallet_uuids), hot_wallet_uuid }`.

### 8.3 Parallelism

```rust
let sem = Arc::new(Semaphore::new(16));
let mut tasks: JoinSet<Result<(String, String, String), RpcError>> = JoinSet::new();

for account in &population.accounts {
    let permit = sem.clone().acquire_owned().await.unwrap();
    let rpc = rpc.clone();
    let account_id = account.account_id.clone();  // field is account_id, not id
    tasks.spawn(async move {
        let _permit = permit;
        let account_info = rpc.z_get_new_account(&account_id).await?;
        let zallet_uuid = account_info.account.clone();
        let ua = rpc.z_get_address_for_account(&zallet_uuid).await?;
        Ok((account_id, zallet_uuid, ua.address))
    });
}

let mut zallet_uuids = HashMap::new();
while let Some(result) = tasks.join_next().await {
    let (account_id, uuid, address) = result.map_err(ProvisionerError::Join)?
        .map_err(ProvisionerError::Rpc)?;
    let wallet_id = population
        .wallet_for_account(&account_id)
        .map(|w| w.wallet_id.clone())
        .unwrap_or_default();

    // Register the UA as both Transparent and Orchard so all four flow types resolve correctly.
    // resolve_address (src/synthetic/generators.rs) picks from transparent_addresses for
    // TToT/ZToT recipients and from shielded_addresses for TToZ/ZToZ recipients. A Unified
    // Address includes all receiver types, so registering it under both is correct.
    population.add_address(&account_id, Address {
        address_id: format!("addr-{account_id}-t"),
        wallet_id: wallet_id.clone(),
        address: address.clone(),
        address_type: AddressType::Transparent,
        purpose: AddressPurpose::Deposit,
        created_at: Utc::now(),
        last_used_at: None,
    }).map_err(ProvisionerError::Population)?;

    population.add_address(&account_id, Address {
        address_id: format!("addr-{account_id}-z"),
        wallet_id,
        address,
        address_type: AddressType::Orchard,
        purpose: AddressPurpose::Deposit,
        created_at: Utc::now(),
        last_used_at: None,
    }).map_err(ProvisionerError::Population)?;

    zallet_uuids.insert(account_id, uuid);
}
```

### 8.4 Hot wallet

The hot wallet must be a **Zallet-managed account** whose private key Zallet controls. `z_send_many` (used inside `run_deposit` and `run_sweep`) is routed to `Backend::Zallet` (`src/rpc/mod.rs:385`), which requires Zallet to own the signing key of `from_account`. A Zebra regtest coinbase transparent address has its key held by Zebra, not Zallet — passing it as `from_account` to `z_send_many` produces a key-not-found error at runtime.

**Architecture:** During `provision()`, create a dedicated Zallet account for the hot wallet (Section 8.2 Step 3):
```rust
let hw_info = rpc.z_get_new_account("hot_wallet").await.map_err(ProvisionerError::Rpc)?;
let hot_wallet_uuid = hw_info.account.clone();  // Zallet UUID, e.g. "abc123-..."
```

This UUID is stored in `ProvisionedPopulation::hot_wallet_uuid` and flows into `SetupState::hot_wallet_uuid: String`. It is passed to:
- `run_deposit` as `from_account` (source of funds sent to each user's Zallet deposit address)
- `run_sweep` as `hot_wallet_address` parameter — `run_sweep` accepts the full Unified Address string (`ua.address`) returned by `z_get_address_for_account(&hot_wallet_uuid)`; `z_send_many` routes correctly to the UA. No transparent-receiver extraction is needed or possible (`UnifiedAddress` exposes no receiver components, only `address: String`).

**Funding:** Mining blocks via `generate()` funds the Zebra regtest coinbase transparent address. The mechanism for transferring those funds to the hot wallet Zallet account is **OQ-9** — a **Phase 2 blocker** (must be resolved before provisioner code is written). The Z3 init script (`regtest-init.sh`) may pre-fund a known Zallet account; if so, `opts.hot_wallet_uuid` can override the auto-provisioned UUID.

---

## 9. Exchange Workflow Dispatch

### 9.1 Intent routing

```rust
// src/scenarios/runner/dispatch.rs

pub fn build_intent_future(
    intent: TransactionIntent,
    rpc: Arc<RpcClient>,
    sem: Arc<Semaphore>,
    active_count: Arc<AtomicUsize>,
    metrics: Arc<dyn MetricsRecorder>,
    hot_wallet_uuid: String,
    hot_wallet_address: String,  // full UA string from ua.address; precomputed at setup; run_sweep accepts a UA directly
    run_id: String,
    polling: PollingConfig,
    zallet_uuids: Arc<HashMap<String, String>>,
) -> impl Future<Output = IntentOutcome> + Send + 'static
```

The function returns an **unspawned future**. The caller pushes it into the `JoinSet` via `jset.spawn(build_intent_future(...))`. This keeps the `JoinSet<IntentOutcome>` type clean — `JoinHandle<T>` implements `Future<Output = Result<T, JoinError>>`, not `Future<Output = T>`, and passing a `JoinHandle` to `JoinSet::spawn` would produce a type mismatch. `Arc<HashMap>` satisfies `'static`; a bare `&HashMap` reference does not.

The returned future:
1. Increments `active_count` on entry via a RAII guard:
   ```rust
   struct ActiveGuard(Arc<AtomicUsize>);
   impl Drop for ActiveGuard { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::Relaxed); } }
   active_count.fetch_add(1, Ordering::Relaxed);
   let _guard = ActiveGuard(active_count.clone());
   ```
2. Acquires one `Semaphore` permit.
3. Routes `intent.flow_type` to the correct T5 workflow.
4. Returns `IntentOutcome` (permit and guard drop automatically on return).

### 9.2 Routing table

One intent = one T5 workflow call (except `ZToT`, which requires two sequential calls — see below). `from_account` in `run_deposit` is always `hot_wallet_uuid` (the Zallet hot wallet account UUID); `from_account` in `run_withdrawal` is `hot_wallet_uuid` for `ZToT` flows (the sweep funds the hot wallet, which then pays out) and the sender's `zallet_uuid` for `TToT` flows.

| `FlowType` | T5 workflow | What it models |
|------------|-------------|----------------|
| `TToT` | `run_withdrawal` | User sends transparent funds to another transparent address |
| `TToZ` | `run_deposit` | Hot wallet funds a user's Zallet UA (transparent → shielded receive) |
| `ZToT` | `run_sweep` **then** `run_withdrawal` | Sweep shielded notes to hot wallet, then pay out transparent |
| `ZToZ` | `run_deposit` | Hot wallet funds a user's Zallet UA (shielded receive, Z→Z routing) |

**Zallet UUID lookup:** For intents that dispatch to `run_deposit` or `run_withdrawal`, the Zallet UUID is retrieved from `zallet_uuids.get(&intent.account_id)`. `account_id` is the **primary exchange user account** in every operation — the one the exchange is acting on behalf of. For `TToZ`/`ZToZ` flows, the hot wallet funds `account_id`'s shielded account; there is no separate recipient account in this exchange-centric model. (Note: the `run_deposit` signature uses the parameter name `zallet_uuid` to mean `account_id`'s Zallet UUID, not a different user's account.)

**`ZToT` two-step dispatch:** This is the only flow requiring two sequential T5 calls. If `run_sweep` fails, return immediately — do not attempt `run_withdrawal` without funds:

```rust
FlowType::ZToT => {
    let zallet_uuid = match zallet_uuids.get(&intent.account_id) {
        Some(u) => u.clone(),
        None => return IntentOutcome::Failed {
            intent_id: intent.intent_id.clone(),
            flow_type: FlowType::ZToT,
            error: format!("no Zallet UUID for account {}", intent.account_id),
        },
    };
    // Step 1: sweep shielded notes from sender's Zallet account to the hot wallet address.
    // run_sweep accepts a full UA string as the destination (z_send_many routes correctly).
    // Match on ExchangeError::Timeout so sweep timeouts produce TimedOut, not Failed.
    match run_sweep(&rpc, &zallet_uuid, &hot_wallet_address, &run_id,
                    Some(metrics.clone()), &polling).await {
        Ok(_) => {}
        Err(ExchangeError::Timeout { .. }) => return IntentOutcome::TimedOut {
            intent_id: intent.intent_id.clone(),
            flow_type: FlowType::ZToT,
        },
        Err(other) => return IntentOutcome::Failed {
            intent_id: intent.intent_id.clone(),
            flow_type: FlowType::ZToT,
            error: other.to_string(),
        },
    }
    // Step 2: withdraw from the hot wallet (which now holds the swept funds) to
    // the recipient's transparent address. Do NOT use zallet_uuid here — that
    // account was just emptied by the sweep.
    match run_withdrawal(&rpc, &intent.account_id, &hot_wallet_uuid,
                         &intent.recipient_address, intent.amount_zatoshis,
                         Some(&intent.intent_id), &run_id, Some(metrics.clone()), &polling)
        .await
    {
        Ok(w) => IntentOutcome::WithdrawalOk(w),
        Err(ExchangeError::Timeout { .. }) => IntentOutcome::TimedOut {
            intent_id: intent.intent_id.clone(),
            flow_type: FlowType::ZToT,
        },
        Err(other) => IntentOutcome::Failed {
            intent_id: intent.intent_id.clone(),
            flow_type: FlowType::ZToT,
            error: other.to_string(),
        },
    }
}
```

Note: `hot_wallet_address` is the full Unified Address string (`ua.address`) from `z_get_address_for_account(&hot_wallet_uuid)`, obtained once during setup and stored in `SetupState`. Passing the UA directly to `run_sweep` is correct — `z_send_many` accepts a UA as destination.

**`run_balance_check` (periodic sampler):** Not dispatched per-intent. It runs as a **background task** spawned before the load loop, following the same shutdown-oneshot pattern as `run_mempool_watcher`:

```rust
let balance_interval = Duration::from_secs(scenario.observability.metric_sampling_interval_secs);
let (balance_tx, balance_rx) = tokio::sync::oneshot::channel::<()>();
let balance_task = tokio::spawn(periodic_balance_check(
    rpc.clone(), run_id.to_string(), balance_interval,
    metrics.clone(), active_count.clone(), balance_rx,
));
// After load loop exits:
let _ = balance_tx.send(());
balance_task.await.ok();
```

`periodic_balance_check` polls `run_balance_check` each interval and emits two metrics so both share one background timer:
- `active_accounts`: `active_count.load(Ordering::Relaxed)` — in-flight task count at sample time.
- `block_height`: `snapshot.at_block_height` from the `run_balance_check` return value — required by `observability.md`.

```rust
metrics.record_metric(MetricSample {
    run_id: run_id.clone(),
    timestamp: Utc::now(),
    metric_name: "block_height".to_string(),
    value: snapshot.at_block_height as f64,
    labels: Default::default(),
});
```

### 9.3 Result types

```rust
// src/scenarios/runner/result.rs

#[derive(Debug)]
pub enum IntentOutcome {
    WithdrawalOk(Withdrawal),
    DepositOk(Deposit),
    // SweepOk is intentionally absent: ZToT returns WithdrawalOk on success (the
    // sweep is an internal step, not a user-visible outcome). No current flow type
    // produces a standalone sweep result.
    Failed {
        intent_id: String,
        flow_type: FlowType,
        error: String,
    },
    TimedOut {
        intent_id: String,
        flow_type: FlowType,
    },
}

#[derive(Debug, Default)]
pub struct RunStats {
    /// Total intents spawned into the JoinSet (attempted, regardless of outcome).
    pub total_attempted: u64,
    /// Intents that returned `DepositOk` or `WithdrawalOk` (no `SweepOk` — sweeps are internal).
    pub confirmed: u64,
    /// Intents that returned `Failed` (including task panics mapped to Failed).
    pub failed: u64,
    /// Intents that returned `TimedOut`.
    pub timed_out: u64,
}

#[derive(Debug)]
pub struct RunResult {
    pub run_id: String,
    pub dry_run: bool,
    pub stats: RunStats,
    /// Full per-intent outcome log, in drain order (not dispatch order).
    pub outcomes: Vec<IntentOutcome>,
}
```

`ExchangeError::Timeout` maps to `IntentOutcome::TimedOut`; all other `ExchangeError` variants map to `IntentOutcome::Failed`. Use a direct `match` on the `Result` — do **not** use `.map_err(...)?`. The future returns `IntentOutcome` (not `Result`), so `?` cannot propagate `IntentOutcome` as an error; `.map_err` + `?` would require `From<IntentOutcome> for IntentOutcome` in a `Result`-returning context, which does not apply here:

```rust
// Correct pattern for all non-ZToT dispatch branches:
match run_withdrawal(/* or run_deposit */).await {
    Ok(result) => IntentOutcome::WithdrawalOk(result),  // or DepositOk
    Err(ExchangeError::Timeout { .. }) => IntentOutcome::TimedOut {
        intent_id: intent.intent_id.clone(),
        flow_type: <FlowType>,
    },
    Err(other) => IntentOutcome::Failed {
        intent_id: intent.intent_id.clone(),
        flow_type: <FlowType>,
        error: other.to_string(),
    },
}
```

Task panics are detected by `JoinSet::join_next()` returning a `JoinError`, which maps to `IntentOutcome::Failed { error: "task panicked".into(), ... }`.

`RunStats` is aggregated in the outcome drain loop: increment `confirmed` for `*Ok` variants, `failed` for `Failed` and `JoinError`, `timed_out` for `TimedOut`.

---

## 10. Metrics and Observability Integration

### 10.1 T6 gating strategy

T6 is on branch `plan/t6-observability-metrics` and must not be assumed merged. The runner uses only the `MetricsRecorder` trait (already on `main`) for all hook-points. `NullRecorder` is defined in `src/metrics/mod.rs` by T7 Phase 1.

**Tier 1 (Phases 1–3, before T6 merge):** All code uses `Arc::new(NullRecorder) as Arc<dyn MetricsRecorder>`. No JSONL files are written. The run produces no output artifacts.

**Tier 2 (Phase 4, after T6 merge):** Replace `NullRecorder` with `JsonlRecorder::new(&run_dir)`. Add `RunDir::create()` and `RunManifest` construction. Uncomment the teardown output-writing steps. Note: `flush_latency_samples` is called on the concrete `JsonlRecorder`, not through the trait.

**Preferred approach over a feature flag:** stub out the three T6 call sites in `lifecycle.rs` with `// TODO(T6): uncomment after merge` comments. Phase 4 upgrade is a three-line change.

### 10.2 Metrics hook-points in the runner

| Measurement point | Method | Notes |
|-------------------|--------|-------|
| Each RPC call | Handled automatically by `RpcClient` (T3) | No action needed in runner |
| `rpc_call_total` | Derived by post-processing `rpc_calls.jsonl` (T6) | Not a runner `record_metric` call — `RpcClient` writes per-call records; `rpc_call_total` is aggregated from that log |
| `accounts_provisioned` | `record_metric` in `provisioner.rs` | After all accounts hydrated |
| `warmup_blocks_mined` | `record_metric` in `lifecycle.rs` | After `generate()` returns |
| `tps_achieved` | `record_metric` in `lifecycle.rs` | End of load: `total_attempted / load_duration_seconds` |
| `active_accounts` | `record_metric` in `periodic_balance_check` background task | Sampled each `metric_sampling_interval_secs`. Note: measures concurrent **in-flight operations**, not distinct account identities (at 10 TPS with a 5 s confirmation wait, value is ~50 while only 10 accounts exist). Diverges from the spec label description but is a useful load-level metric. |
| `block_height` | `record_metric` in `periodic_balance_check` background task | `snapshot.at_block_height` from `run_balance_check` return value; no labels. Required by `observability.md`. |
| `confirmed_txs_total` | `record_metric` per `DepositOk`/`WithdrawalOk` | In outcome drain loop. Label `flow_type` uses snake_case: `"t_to_t"`, `"t_to_z"`, `"z_to_t"`, `"z_to_z"`. |
| `failed_txs_total` | `record_metric` per `Failed`/`TimedOut` | In outcome drain loop. Same `flow_type` label format. |
| Latency flush | `JsonlRecorder::flush_latency_samples()` — concrete type, not trait | Phase 4 teardown only |
| Mempool metrics | Handled by `run_mempool_watcher` (T5) | Background task |

### 10.3 `RunManifest` (T6, Phase 4 only)

Two-phase write:
1. **At start of load phase:** `write_manifest(&run_dir.manifest_path(), &manifest)` with `run_completed_at: None`.
2. **At end of teardown:** set `manifest.run_completed_at = Some(Utc::now())` and overwrite with `write_manifest(&run_dir.manifest_path(), &manifest)`.

Both call sites take `&Path` as the first argument — `&RunDir` is not `&Path`. Use `run_dir.manifest_path()` which returns `PathBuf`.

Fields populated by T7 (Phase 4 construction pattern):
```rust
let (zebra_commit, zaino_commit, zallet_commit) =
    read_z3_commits(Path::new("z3-commits.lock"));  // file exists at repo root
let simulator_commit = read_simulator_commit();
let manifest = RunManifest {
    run_id: run_dir.run_id.clone(),  // canonical ID from RunDir, not from generate_run_id
    run_started_at,                  // captured before Z3Stack::start (see Section 4)
    run_completed_at: None,
    simulator_commit,
    zebra_commit,
    zaino_commit,
    zallet_commit,
    scenario_name: scenario.name.clone(),
    scenario_config_hash: scenario.config_hash.clone(),
    target_tps: scenario.load_target_tps,
};
```

`z3-commits.lock` is present at the repository root. Absent or malformed sections return `"unknown"` (T6 handles this internally).

---

## 11. Error Handling Strategy

### 11.1 `RunnerError`

```rust
#[derive(Debug)]
pub enum RunnerError {
    Config(ConfigError),
    Setup(String),
    Provision(ProvisionerError),
    Warmup(String),
    Load(String),
    Teardown(String),
    Metrics(String),  // wraps MetricsError when T6 is merged (Phase 4)
}
```

### 11.2 `ConfigError`

```rust
#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(serde_yaml::Error),
    // Single-violation cases use a one-element vec — no separate Validation variant.
    ValidationErrors(Vec<(String, String)>),  // (field, message) pairs; no recursive nesting
}
```

### 11.3 `ProvisionerError`

```rust
#[derive(Debug)]
pub enum ProvisionerError {
    Rpc(RpcError),
    Population(PopulationError),
    Generator(GeneratorError),  // from AccountGenerator::new and generate_population
    Join(tokio::task::JoinError),
}
```

### 11.4 Error propagation rules

- **Config errors** — always fatal; bubble up before any Z3 process starts.
- **Setup errors** — attempt graceful `stack.stop()` before returning. Discard `stop()` errors.
- **Provision errors** — fatal. Do not partially provision.
- **Warmup errors** — fatal. A non-responsive stack is not safe to load-test.
- **Load errors** (individual intents) — not fatal. `Failed`/`TimedOut` outcomes are collected. `RunnerError::Load` is only for non-recoverable internal failures (scheduler panic, channel closed).
- **Teardown errors** — logged, never returned from `run()`.

### 11.5 Ctrl-C / SIGINT handling

T8 (CLI) intercepts `tokio::signal::ctrl_c()` and sets a `CancellationToken` that T7 polls each scheduler tick via `opts.cancel`. If `opts.cancel` is `None`, the run is not interruptible at the runner level. `tokio-util = { version = "0.7", features = ["sync"] }` is added to `Cargo.toml` in Phase 1 (the type must be importable when `RunOptions` compiles, even with `cancel: None` as default).

---

## 12. Testing Plan

### 12.1 Unit tests (no Z3 required)

**`config.rs` tests:**
- `load_scenario_computes_config_hash` — write a temp YAML, assert `config_hash` starts with `"sha256:"` and is 71 chars.
- `validate_scenario_rejects_zero_tps`
- `validate_scenario_rejects_flows_not_summing_to_one`
- `validate_scenario_rejects_activity_profiles_not_summing_to_one`
- `validate_scenario_rejects_insufficient_active_accounts` — e.g. `accounts_count=2, active_fraction=0.4` → `floor(0.8)=0` active → error
- `validate_scenario_rejects_min_gt_max_zatoshis` — `amounts.min_zatoshis > amounts.max_zatoshis` must return a validation error (not panic at generation time)
- `validate_scenario_accepts_smoke_yaml`
- `load_scenario_sets_source_path`
- `validate_scenario_rejects_missing_file` — error path for `load_scenario`
- `validate_scenario_rejects_invalid_yaml` — parse error path

**`scheduler.rs` tests:**
- `steady_state_tps_is_constant`
- `ramp_tps_is_zero_at_start` — `instantaneous_tps(ZERO)` returns 0 (mathematically correct; keep this test)
- `ramp_initial_tps_is_nonzero` — `initial_tps()` for a Ramp returns `target_tps * 0.1` (not 0); verifies the scheduler loop never sets a 1 000-second initial interval
- `ramp_tps_reaches_target_after_ramp_secs`
- `burst_tps_spikes_during_window`
- `burst_tps_returns_to_base_after_burst`

**`provisioner.rs` tests (wiremock):**
- `provision_creates_accounts_and_addresses` — assert `zallet_uuids` populated with `AccountInfo.account` values; assert per-account `wallet.transparent_addresses.len() == 1` AND `wallet.shielded_addresses.len() == 1` (both populated by the double-add pattern).
- `provisioner_constructs_address_struct_correctly` — verify `Address` fields: correct `address_type` (`Transparent` vs `Orchard`), `purpose: Deposit`, `wallet_id` sourced from `wallet_for_account`, distinct `address_id` suffixes `-t` and `-z`.
- `provision_records_metric`
- `provision_fails_on_rpc_error`

**`result.rs` tests:**
- `run_stats_counts_outcomes_correctly`

**`dispatch.rs` tests (tokio, no network):**
- `spawn_intent_task_respects_backpressure` — `max_in_flight = 2`, spawn 10 futures via `jset.spawn(build_intent_future(...))`; assert `active_count` never exceeds 2.
- `active_count_decrements_on_task_completion` — assert counter returns to 0 after JoinSet drains.
- `zot_dispatch_calls_sweep_then_withdrawal` — mock the two-step `ZToT` path; assert both T5 calls are made in order; assert early return on sweep failure.
- `periodic_balance_check_emits_active_accounts_and_block_height_metrics` — drive one tick with a mock RPC; assert both `active_accounts` and `block_height` samples are emitted.
- `periodic_balance_check_exits_cleanly_on_shutdown` — send the shutdown signal; assert the background task completes without hanging.

**`mod.rs` tests:**
- `test_dry_run_does_not_start_z3` — call `run(smoke_scenario, RunOptions { dry_run: true, .. })` without a live Z3 stack; assert `result.dry_run == true`. **No `#[ignore]`** — dry-run has no external dependencies.
- `test_validate_scenario_smoke` — load and validate `configs/scenarios/smoke.yaml`.

### 12.2 Integration tests (live Z3 required, `#[ignore]`)

**`test_smoke_scenario_via_runner`:**
```rust
#[tokio::test]
#[ignore]
async fn test_smoke_scenario_via_runner() {
    let scenario = load_scenario(Path::new("configs/scenarios/smoke.yaml")).unwrap();
    validate_scenario(&scenario).unwrap();
    let opts = RunOptions::default();
    let result = run(scenario, opts).await.unwrap();
    assert!(result.stats.total_attempted > 0);
    assert!(result.stats.confirmed > 0);
}
```

Place in `tests/integration/main.rs` (the `tests/integration/` directory already exists; a `main.rs` entry point is required for a multi-file integration test binary).

Run against a locally running Z3 regtest stack with:
```
cargo test --test integration test_smoke_scenario_via_runner -- --include-ignored
```

Add assertion: no outcome contains `error` with the substring `"unprovisioned"` — regression guard for the double-add provisioner fix.

---

## 13. Implementation Steps

### Phase 1: Config, validation, dry-run (no Z3 dependency)

1. Add `sha2 = "0.10"` and `tokio-util = { version = "0.7", features = ["sync"] }` to `Cargo.toml` (both needed when `RunOptions` compiles).
2. Add `warmup_blocks: u64` to `ScenarioConfig` with `#[serde(default = "default_warmup_blocks")]`.
3. Extend `src/metrics/mod.rs` with `NullRecorder`.
4. Create `src/scenarios/runner/mod.rs` with `RunOptions` (including `hot_wallet_uuid: Option<String>` and `cancel: Option<tokio_util::sync::CancellationToken>`), `RunResult` (minimal, with `dry_run: bool`), `RunnerError`, `generate_run_id`, and a `run()` stub that validates + dry-runs only. Capture `run_started_at = Utc::now()` immediately after `generate_run_id`, before any Z3 calls.
5. Implement `config.rs`: `load_scenario`, `validate_scenario` (with all checks from Section 6.2, including `amounts.min_zatoshis <= amounts.max_zatoshis`), `ConfigError`.
6. Implement `result.rs`: `IntentOutcome`, `RunStats`, `RunResult`.
7. Add `pub mod runner;` to `src/scenarios/mod.rs`.
8. Verify `configs/scenarios/smoke.yaml` passes `validate_scenario` after adding `warmup_blocks`.
9. Write all unit tests for `config.rs` and `mod.rs` (dry-run test, including `validate_scenario_rejects_min_gt_max_zatoshis`).
10. `cargo test --lib`, `cargo clippy -- -D warnings`.

**PR title:** `feat(runner): config loading, validation, dry-run skeleton (T7 Phase 1)`

### Phase 2: Scheduler and provisioner (mock-testable)

**Prerequisite:** OQ-9 (hot wallet funding mechanism) must be resolved before this phase ships — the provisioner design depends on it. See Section 15.

1. Add `#[derive(Clone, Copy)]` to `PollingConfig` in `src/scenarios/exchange.rs`. Note in the PR as a T5-touching cross-module change.
2. Implement `LoadShape` and `Scheduler` (interval-based) in `scheduler.rs`. Use `interval_at` for ticker resets to avoid immediate-fire (Section 7.2).
3. Define `ProvisionedPopulation` (with `zallet_uuids: Arc<HashMap<String, String>>` and `hot_wallet_uuid: String`) and implement `provision()` in `provisioner.rs`.
   - Signature: `provision(rpc, scenario, run_id, metrics, hot_wallet_uuid_override: Option<String>)`.
   - Use the override when `Some`; skip `z_get_new_account("hot_wallet")` in that case (Section 8.2).
   - Use `account.account_id` (not `account.id`) when iterating accounts.
   - Call `add_address` **twice** per account — once with `AddressType::Transparent` and once with `AddressType::Orchard` — using the same UA string and distinct `address_id` suffixes (`-t`, `-z`). This is required for `TToT`/`ZToT` flows to resolve non-empty `transparent_addresses` (Section 8.3).
   - Add `Generator(GeneratorError)` variant to `ProvisionerError` and use it for `AccountGenerator::new` and `generate_population` failures.
4. Specify `Mixed` LoadShape as a config-clone at the `TransactionIntentGenerator` construction site (Section 7.3).
5. Write scheduler unit tests.
6. Write provisioner unit tests using wiremock, including `provisioner_constructs_address_struct_correctly` (assert `transparent_addresses.len() == 1 && shielded_addresses.len() == 1`).
7. `cargo test --lib`, `cargo clippy`.

**PR title:** `feat(runner): TPS scheduler and account provisioner (T7 Phase 2)`

### Phase 3: Full run orchestration (requires Z3)

**Prerequisite:** OQ-9 must be resolved before this phase starts (Phase 2 blocker).

1. Implement `SetupState`, `setup()`, `warmup()`, `teardown()` in `lifecycle.rs`.
   - Clone `rpc_url` and `basic_auth` from `Z3Config` **before** moving it into `Z3Stack::new` (Section 5 Steps 2–3).
   - Chain `.with_basic_auth(user, pass)` after `RpcClient::new(...)` (Section 5 Step 4). Without this every RPC call returns HTTP 401.
   - `SetupState` holds `hot_wallet_uuid: String` and `hot_wallet_address: String` (the full UA string from `ua.address`, obtained via `z_get_address_for_account(&hot_wallet_uuid)` during setup; passed to `run_sweep` as the sweep destination — no transparent-receiver extraction is needed).
2. Implement `build_intent_future()` (returns `impl Future`, not `JoinHandle`) with `AtomicUsize` RAII guard in `dispatch.rs`. Include the explicit `ZToT` two-step dispatch with failure handling (Section 9.2).
3. Wire `load()` loop with scheduler, JoinSet, semaphore, mempool watcher, and `periodic_balance_check` background task.
4. Wire all phases into `run()` in `mod.rs` using `NullRecorder`.
5. Write dispatch unit tests (backpressure, active_count, `zot_dispatch_calls_sweep_then_withdrawal`).
6. Write integration test `test_smoke_scenario_via_runner` (marked `#[ignore]`).
7. Run: `cargo test --test integration test_smoke_scenario_via_runner -- --include-ignored` against a live Z3 stack.
8. `cargo test`, `cargo clippy`.

**PR title:** `feat(runner): full run lifecycle with NullRecorder wiring (T7 Phase 3)`

### Phase 4: T6 observability integration (after T6 merge)

1. Call `RunDir::create(&opts.output_base, &scenario.name)?` **after** the dry-run guard and **before** `Z3Stack::start()`. A dry run must produce no filesystem artifacts — guarding here ensures `RunDir::create` is never called for `opts.dry_run == true`. Use `run_dir.run_id.clone()` as the canonical `run_id` everywhere — pass it to `RpcClient::new`, `Z3Config::for_run`, and all T5 workflow calls. Remove the `generate_run_id()` call (or gate it behind `#[cfg(test)]` if any tests depend on it directly).
2. Capture `run_started_at = Utc::now()` immediately after `RunDir::create`, before `Z3Stack::start()`.
3. Construct `RunManifest` using `read_simulator_commit()` and `read_z3_commits(Path::new("z3-commits.lock"))` from the T6 helpers. `z3-commits.lock` is at the repository root (confirmed present). See Section 10.3 for the full construction pattern.
4. Replace `NullRecorder` with `JsonlRecorder::new(&run_dir).map_err(|e| RunnerError::Metrics(e.to_string()))?`. Also **remove the inline `NullRecorder` struct and its `MetricsRecorder` impl** from `src/metrics/mod.rs` — T6's `pub use recorder::NullRecorder` now provides it; leaving both is a duplicate-definition error.
5. Wire two-phase manifest writes: `write_manifest(&run_dir.manifest_path(), &manifest)` at load start (with `run_completed_at: None`) and again at teardown (with `run_completed_at: Some(Utc::now())`).
6. In teardown, call `generate_summary` only if load succeeded. `generate_summary` returns
   `Result<String, MetricsError>` — handle with `?` and write the string explicitly:
   ```rust
   let summary = generate_summary(&run_dir, &manifest)
       .map_err(|e| RunnerError::Metrics(e.to_string()))?;
   std::fs::write(run_dir.summary_path(), summary)
       .map_err(|e| RunnerError::Metrics(e.to_string()))?;
   ```
   If load returned `Err`, skip entirely (partial data).
7. `copy_scenario_yaml` takes `&Path`, not `&String`:
   ```rust
   run_dir.copy_scenario_yaml(Path::new(&scenario.source_path))
       .map_err(|e| RunnerError::Metrics(e.to_string()))?;
   ```
8. Uncomment `flush_latency_samples` call and remove `// TODO(T6)` stubs.
9. Verify: `experiments/runs/<run-id>/` contains all expected artifact files after smoke run.
10. `cargo test`, `cargo clippy`.

**PR title:** `feat(runner): wire T6 observability (JsonlRecorder, RunManifest, summary) (T7 Phase 4)`

---

## 14. Acceptance Criteria

- [ ] `cargo build` and `cargo test --lib` pass with zero warnings (`-D warnings`).
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] `load_scenario("configs/scenarios/smoke.yaml")` succeeds; `validate_scenario` passes.
- [ ] `run(smoke_scenario, RunOptions { dry_run: true, .. })` returns `RunResult { dry_run: true }` without Docker.
- [ ] `run(smoke_scenario, RunOptions::default())` against a live Z3 regtest stack:
  - Completes without panic.
  - Returns `stats.total_attempted > 0` and `stats.confirmed > 0`.
  - (Post-T6) Produces all five output artifacts in `experiments/runs/<run-id>/`.
- [ ] Steady-state 1 TPS for 60 s dispatches 55–65 intents (±8% tolerance).
- [ ] `max_in_flight = 2` with slow mock workflows: `active_count` never exceeds 2 (backpressure unit test).
- [ ] `test_dry_run_does_not_start_z3` passes without a live Z3 stack (no `#[ignore]`).
- [ ] T8 (CLI) can call `run()` after providing a `ScenarioConfig` loaded from a CLI path argument.

---

## 15. Open Questions

**OQ-1: `warmup_blocks` placement** — Resolved: `ScenarioConfig` (scenario identity; affects config hash). Default: 10. Smoke YAML may use `warmup_blocks: 5`.

**OQ-2: Account selection for dispatch** — Recommendation: round-robin over `provisioned.population.active_account_ids`, seeded from `config.seed ^ DISPATCH_SEED_SALT` for determinism.

**OQ-3: T6 merge timing** — Phase 3 uses `NullRecorder`. If T6 merges first, Phase 4 folds into Phase 3.

**OQ-4: Ramp/Burst parameters in YAML** — Resolved: `RunOptions`, not `ScenarioConfig`. New scenario YAMLs for ramp/burst are created in T9.

**OQ-5: `hex` crate** — Resolved: hand-roll hex encoding. No `hex` dependency needed.

**OQ-6: `z_get_address_for_account` receiver type** — Resolved: confirmed one-argument signature returning `UnifiedAddress { account, address, receiver_types }`. No pool-type selection. Close this question.

**OQ-7: Ctrl-C / `CancellationToken`** — `opts.cancel: Option<CancellationToken>` is wired by T8 for Ctrl-C. `tokio-util` is added to `Cargo.toml` in Phase 1 (the type must compile in `RunOptions` immediately, even with `cancel: None` as the default). Closed for Phase 1–3 purposes.

**OQ-9: Hot wallet funding mechanism** *(Phase 2 blocker — must resolve before provisioner code is written)*

The hot wallet must be a Zallet-managed account (Section 8.4). The outstanding question is: how do ZEC funds get into that Zallet account?

`generate()` mines blocks to the Zebra regtest coinbase transparent address (Zebra holds that key). The Zallet hot wallet account has its own Unified Address. Funds must travel from the Zebra coinbase address to the Zallet hot wallet UA before any `run_deposit` or `run_sweep` can succeed.

Options:
- **(a) Z3 init script handles it:** `regtest-init.sh` in the Z3 Docker repo pre-mines blocks and sends coinbase ZEC to a known Zallet account UUID. If so, `opts.hot_wallet_uuid` can be set to that UUID by the caller (T8 CLI reads it from environment or config) and the provisioner skips creating a new one.
- **(b) T7 provisioner handles it:** After creating the hot wallet Zallet account and getting its UA, call `z_sendmany` from the Zebra coinbase transparent address to the hot wallet UA. This requires Zebra (not Zallet) to sign the coinbase send — which may require a different RPC path (Zebra `sendtoaddress` or `z_sendmany` with coinbase source).
- **(c) Warmup step handles it:** After `generate(warmup_blocks)`, call a transparent `sendtoaddress` from Zebra coinbase to the hot wallet UA's transparent receiver.

**Project owner must confirm the correct option before Phase 2 provisioner code is written.**
