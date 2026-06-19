# Task 8 — CLI: Implementation Plan

> **Status:** Draft — Review Needed
> **Date:** 2026-06-19
> **Implementation branch:** `t8-cli`
> **T7 baseline:** branch `t7-scenario-runner`, commit `18081f2` — all T7 API references in this plan are verified against that code
> **Output modules:** `src/main.rs`, `src/cli/mod.rs`
> **Implementation prerequisite:** T7 merged into `main`; plan accuracy must be re-verified against the merged code before implementation begins. The plan file itself lives on `t8-cli` and does not depend on T7 merging first.

---

## 1. Executive Summary

Task 8 is the entry-point layer of the Z3 Exchange Simulator. It adds:

- A usable binary (`z3sim`) driven by `clap` derive-style argument parsing
- Four subcommands: `run`, `run --dry-run`, `generate-fixtures`, and `validate-scenario`
- Logging setup via `tracing` with operator-controlled verbosity
- Graceful SIGINT handling that propagates cancellation to the T7 runner through
  the already-wired `RunOptions::cancel` field

**What T8 must not do:**

- Contain any scenario execution logic, RPC calls, metric recording, or fixture generation
  algorithms — those live in T4–T7
- Duplicate YAML loading or validation — T7's `load_scenario` / `validate_scenario` are
  the canonical implementations
- Issue network calls or start the Z3 stack in `validate-scenario` or `run --dry-run`
- Introduce `anyhow`, `thiserror`, or any error handling pattern that diverges from the
  project's existing typed-enum convention

**How T8 fits in:** `main.rs` initialises the Tokio runtime and tracing subscriber, reads
argv, and dispatches to a single `cli::dispatch()` function. `cli/mod.rs` contains the
`clap` types and the per-command handlers. Each handler delegates immediately to existing
library code and prints structured, human-readable output. The CLI layer is intentionally
transparent — it is glue, not logic.

---

## 2. Repository Findings

### 2.1 Relevant existing files

| Path | State | Notes |
|---|---|---|
| `src/main.rs` | Stub — 6 lines, `println!` only | No Tokio, no CLI, no tracing |
| `src/cli/mod.rs` | Stub — doc comment only | Module declared in `lib.rs`; no code |
| `src/lib.rs` | Complete | `pub mod cli;` already declared |
| `src/scenarios/runner/mod.rs` | Complete (T7) | Public API: `run()`, `RunOptions`, `RunnerError` |
| `src/scenarios/runner/config.rs` | Complete (T7) | `load_scenario()`, `validate_scenario()`, `ConfigError` |
| `src/scenarios/runner/scheduler.rs` | Complete (T7) | `LoadShape` enum |
| `src/scenarios/runner/result.rs` | Complete (T7) | `RunResult`, `RunStats`, `IntentOutcome` |
| `src/synthetic/mod.rs` | Complete (T4) | `SyntheticPopulation`, `AccountGenerator` |
| `src/synthetic/fixtures.rs` | Complete (T4) | `write_fixtures()`, `FixtureError` |
| `src/synthetic/generators.rs` | Complete (T4) | `AccountGenerator::new()`, `generate_population()` |
| `Cargo.toml` | Missing CLI deps | No `clap`, no `tracing`, no `tracing-subscriber` |
| `Makefile` | TODO stubs | `generate-fixtures` and `scenario-dry-run` targets have `echo TODO` bodies |

### 2.2 T7 public API confirmed on this branch

```rust
// src/scenarios/runner/mod.rs — re-exported at crate root via src/scenarios/runner/

pub use config::{load_scenario, validate_scenario, ConfigError};
pub use provisioner::PopulationPlan;
pub use result::{IntentOutcome, RunResult, RunStats};
pub use scheduler::LoadShape;

pub struct RunOptions {
    pub output_base: PathBuf,           // default: "experiments/runs"
    pub load_shape: LoadShape,          // default: SteadyState
    pub max_in_flight: usize,           // default: 64
    pub dry_run: bool,                  // default: false
    pub polling: Option<PollingConfig>, // default: None (uses PollingConfig::default())
    pub hot_wallet_uuid: Option<String>,// default: None (creates a new hot wallet)
    pub cancel: Option<tokio_util::sync::CancellationToken>, // SIGINT propagation
}

pub async fn run(
    scenario: ScenarioConfig,
    opts: RunOptions,
) -> Result<RunResult, RunnerError>

pub enum RunnerError {
    Config(ConfigError),
    Setup(String),
    Provision(ProvisionerError),
    Warmup(String),
    Load(String),
    Teardown(String),
    Metrics(String),
}
```

`RunOptions::cancel` accepts a `tokio_util::sync::CancellationToken`. The load phase loop
already checks `cancel.is_cancelled()` on every tick and exits cleanly if set. This is
the hook T8 needs for graceful shutdown.

**Required T7 change (prerequisite for T8):** `RunResult` in `src/scenarios/runner/result.rs`
must gain one field before T8 can be implemented:

```rust
pub struct RunResult {
    pub run_id: String,
    pub output_dir: std::path::PathBuf, // ← add this; populated from run_dir.path()
    pub dry_run: bool,
    pub stats: RunStats,
    pub outcomes: Vec<IntentOutcome>,
}
```

This is a three-line change to T7 (`result.rs` + the two call sites in `runner/mod.rs`
where `RunResult` is constructed). It lets the CLI print the exact output directory without
reconstructing it from `run_id` and `output_base`, which would create hidden coupling to
T7's internal directory naming logic. This change should be included in the T7 PR or applied
as the first commit on the T8 implementation branch after T7 merges.

### 2.3 Fixture generation API

```rust
// src/synthetic/ — two-step call sequence

// Step 1: build a population from the scenario config
let mut gen = AccountGenerator::new(config)?;   // takes ScenarioConfig by value
let population = gen.generate_population()?;    // produces SyntheticPopulation

// Step 2: write accounts.json and wallets.json to disk
write_fixtures(&population, out_dir)?;          // creates out_dir, overwrites existing
```

`write_fixtures` is deterministic: same scenario seed → same output files. The generated
population uses the same `accounts_count`, `accounts_active_fraction`, and
`activity_profiles` fields from `ScenarioConfig` that the live runner uses.

### 2.4 Scenario loading and validation

```rust
// src/scenarios/runner/config.rs — re-exported as:
use z3_exchange_simulator::scenarios::runner::{load_scenario, validate_scenario, ConfigError};

pub fn load_scenario(path: &Path) -> Result<ScenarioConfig, ConfigError>
// Reads the YAML file, computes SHA-256, fills config_hash and source_path

pub fn validate_scenario(config: &ScenarioConfig) -> Result<(), ConfigError>
// Collects ALL violations before returning — never fails on just the first
```

`ConfigError` has three variants: `Io`, `Parse`, `ValidationErrors(Vec<(String, String)>)`.
T8 must print each `(field, message)` pair on a separate line for operator clarity.

### 2.5 LoadShape and CLI representation

`LoadShape` carries inline parameters:

```rust
pub enum LoadShape {
    SteadyState,
    Ramp { ramp_secs: u64 },
    Burst { pre_burst_secs: u64, burst_secs: u64, spike_multiplier: f64 },
    Mixed,
}
```

The CLI must accept `--load-shape steady|ramp|burst|mixed` and expose companion flags for
the parametric shapes. See Section 6 for the proposed flag design.

### 2.6 Dependencies not yet in Cargo.toml

| Crate | Version | Purpose |
|---|---|---|
| `clap` | `4` | CLI parsing (derive feature) |
| `tracing` | `0.1` | Instrumentation macros |
| `tracing-subscriber` | `0.3` | Subscriber setup, env-filter |

`tokio-util` (for `CancellationToken`) is already in `Cargo.toml` as a dependency from T7.

### 2.7 Makefile targets requiring updates

`generate-fixtures` and `scenario-dry-run` targets currently print `echo TODO`. T8 must
replace them with actual binary invocations. `validate-scenario` has no Makefile target
yet; T8 should add one for completeness.

### 2.8 Current cancellation coverage

`RunOptions::cancel` is checked only in the load phase scheduler loop. During `setup`,
`warmup`, and `teardown`, cancellation is **not** currently checked. An operator who
presses Ctrl-C during setup or warmup will have to wait for those phases to complete or
time out before the signal takes effect. This is acceptable behaviour; the plan
documents it, and a runtime note to the operator should explain it.

---

## 3. Task 8 Requirements

### 3.1 Hard requirements

| ID | Requirement |
|---|---|
| R1 | Binary name is `z3sim` (already declared in Cargo.toml) |
| R2 | `z3sim run --scenario <path>` executes a full scenario end-to-end |
| R3 | `z3sim run --scenario <path> --dry-run` validates and summarises without starting Z3 |
| R4 | `z3sim generate-fixtures --scenario <path> --out <dir>` writes `accounts.json` and `wallets.json` to `<dir>` |
| R5 | `z3sim validate-scenario <path>` exits 0 on valid scenario, 1 with detailed errors on invalid |
| R6 | SIGINT (Ctrl-C) triggers graceful shutdown of an active run |
| R7 | `--verbose` / `--quiet` flags adjust log verbosity |
| R8 | Run ID and output directory are printed to stdout when a run starts |
| R9 | All errors print to stderr; normal output prints to stdout |
| R10 | Exit code 0 on success, 1 on any error |
| R11 | CLI layer contains no business logic — all substantive work delegates to T4–T7 |
| R12 | Logging uses `tracing` with `tracing-subscriber` |
| R13 | No `anyhow`, `thiserror`, or divergent error handling |
| R14 | Dry-run and validate-scenario must never start Z3 or issue RPC calls |

### 3.2 Nice-to-have (do not block T8)

- `--output-base <dir>` override on `run`
- `--max-in-flight <n>` override on `run`
- `--hot-wallet-uuid <uuid>` for re-using an existing Zallet hot wallet
- Colour output on interactive terminals (not required for CI)
- Makefile help string for new `validate-scenario` target

### 3.3 Explicitly out of scope

- Additional subcommands beyond the four listed
- Scenario selection from a directory (e.g. `z3sim run --scenario-dir configs/`)
- Batch / multi-scenario orchestration
- Progress bars or TUI output
- Structured JSON output mode
- Anything touching the Z3 binary directly from the CLI layer

---

## 4. Proposed CLI Architecture

### 4.1 Module layout

```
src/
  main.rs          Tokio runtime entry point. Calls cli::dispatch().
  cli/
    mod.rs         Clap types (Cli, Commands, subcommand structs).
                   Logging initialisation (init_tracing).
                   pub fn dispatch() — top-level command dispatch + Ctrl-C handling.
```

Two files. No further splitting is warranted for a thin CLI layer.

If the dispatch logic grows substantially (more than ~200 lines), individual per-command
handler functions should be moved to separate `cli/run.rs`, `cli/fixtures.rs`, etc.
The plan does not require that split now — do it when it helps readability.

### 4.2 Proposed files to create or modify

| File | Action | Reason |
|---|---|---|
| `src/main.rs` | **Modify** (replace stub) | Tokio entry point, tracing init, `cli::dispatch()` call |
| `src/cli/mod.rs` | **Modify** (replace stub) | All CLI code: types, logging, dispatch |
| `Cargo.toml` | **Modify** | Add `clap`, `tracing`, `tracing-subscriber` |
| `Makefile` | **Modify** | Wire `generate-fixtures`, `scenario-dry-run`; add `validate-scenario` |

`src/lib.rs` — no modification needed. `pub mod cli;` is already declared.

### 4.3 Responsibilities

**`src/main.rs`:**

- `#[tokio::main]` entry point with `full` feature (already declared in Cargo.toml)
- Call `cli::init_tracing(verbosity)` immediately after parsing argv
- Call `cli::dispatch(cli_args).await`
- Print errors to stderr and call `std::process::exit(1)` on failure

**`src/cli/mod.rs`:**

- Define `Cli` struct and `Commands` enum using `#[derive(Parser)]`
- Define per-subcommand structs (`RunArgs`, `GenerateFixturesArgs`, `ValidateScenarioArgs`)
- Define `LoadShapeArg` enum for `--load-shape` parsing + conversion to `LoadShape`
- Define `CliError` (typed enum, no `anyhow`)
- Implement `init_tracing(verbose: bool, quiet: bool)` — builds the tracing subscriber
- Implement `pub async fn dispatch(cli: Cli) -> Result<(), CliError>` — dispatches to handlers
- Implement private per-command handler functions: `run_command`, `generate_fixtures_command`,
  `validate_scenario_command`

**No business logic in either file.** Every handler's body is: load → validate → call library → print result.

---

## 5. Command Interface Design

### 5.1 Full proposed command syntax

```text
z3sim [OPTIONS] <COMMAND>

COMMANDS:
  run                    Execute a scenario (or dry-run it)
  generate-fixtures      Write synthetic fixture data to disk
  validate-scenario      Parse and validate a scenario YAML

OPTIONS:
  -v, --verbose          Enable debug-level logging
  -q, --quiet            Suppress all output except errors
  -h, --help             Print help
  -V, --version          Print version
```

```text
z3sim run [OPTIONS] --scenario <PATH>

OPTIONS:
      --scenario <PATH>         Scenario YAML file to execute [required]
      --dry-run                 Validate and summarise without starting Z3
      --load-shape <SHAPE>      Load profile [default: steady]
                                  steady   — constant TPS for full duration
                                  ramp     — TPS ramps from 0 to target
                                  burst    — TPS spikes for a window
                                  mixed    — 50/50 shielded/transparent mix
      --ramp-secs <N>           Ramp duration in seconds [default: 60]
                                  (ignored unless --load-shape ramp)
      --burst-pre-secs <N>      Steady phase before burst spike [default: 60]
                                  (ignored unless --load-shape burst)
      --burst-secs <N>          Duration of burst spike [default: 30]
                                  (ignored unless --load-shape burst)
      --burst-multiplier <X>    TPS spike multiplier [default: 3.0]
                                  (ignored unless --load-shape burst)
      --max-in-flight <N>       Max concurrent in-flight transactions [default: 64]
      --output-base <DIR>       Base directory for run output [default: experiments/runs]
      --hot-wallet-uuid <UUID>  Reuse an existing Zallet hot wallet from a prior run
                                  (optional; creates a new wallet when omitted)
  -h, --help                    Print help
```

```text
z3sim generate-fixtures [OPTIONS] --scenario <PATH> --out <DIR>

OPTIONS:
      --scenario <PATH>         Scenario YAML to derive seed and account params from [required]
      --out <DIR>               Directory to write accounts.json and wallets.json [required]
  -h, --help                    Print help
```

```text
z3sim validate-scenario <PATH>

ARGUMENTS:
      <PATH>    Path to the scenario YAML file

OPTIONS:
  -h, --help    Print help
```

### 5.2 Flags and defaults summary

| Flag | Command | Default | Notes |
|---|---|---|---|
| `--scenario` | run, generate-fixtures | required | Path to YAML; validated for existence |
| `--dry-run` | run | false | Short-circuits before Z3 starts |
| `--load-shape` | run | `steady` | One of: steady, ramp, burst, mixed |
| `--ramp-secs` | run | 60 | Only meaningful with `--load-shape ramp` |
| `--burst-pre-secs` | run | 60 | Only meaningful with `--load-shape burst` |
| `--burst-secs` | run | 30 | Only meaningful with `--load-shape burst` |
| `--burst-multiplier` | run | 3.0 | Only meaningful with `--load-shape burst` |
| `--max-in-flight` | run | 64 | Matches `RunOptions::default()` |
| `--output-base` | run | `experiments/runs` | Matches `RunOptions::default()` |
| `--hot-wallet-uuid` | run | none | Skips hot wallet creation; reuses existing wallet |
| `--out` | generate-fixtures | required | Created if it does not exist |
| `<PATH>` | validate-scenario | required | Positional argument |
| `-v, --verbose` | global | off | Sets log level to DEBUG |
| `-q, --quiet` | global | off | Sets log level to ERROR |

`--verbose` and `--quiet` are mutually exclusive. If both are passed, `clap` should reject
the combination at parse time (use `conflicts_with`).

### 5.3 Help text conventions

- Short help via `-h`; full help via `--help` (clap default)
- All paths described as `<PATH>` (uppercase, angle brackets)
- All directories described as `<DIR>`
- All numeric arguments described as `<N>` or `<X>`
- Defaults shown in `[default: ...]` in help text via `#[arg(default_value_t = ...)]`
- Required args described without a default

### 5.4 Verbosity behaviour

| Flag | Effective log level | RUST_LOG override |
|---|---|---|
| neither | `WARN` | Yes, `RUST_LOG` can override |
| `--verbose` | `DEBUG` | Yes, `RUST_LOG` can override |
| `--quiet` | `ERROR` | Yes, `RUST_LOG` can override |

Use `tracing_subscriber::EnvFilter` so that `RUST_LOG=trace` works for fine-grained
debugging regardless of the flag.

---

## 6. Dispatch and Data Flow

### 6.1 `z3sim run --scenario <path>`

```
argv
 └─ Cli::parse()
     └─ Commands::Run { args }
         └─ cli::run_command(args).await
             1. load_scenario(&args.scenario)           → ScenarioConfig | ConfigError
             2. validate_scenario(&config)              → () | ConfigError
             3. Build LoadShape from args.load_shape + companion flags
             4. Build RunOptions { load_shape, dry_run: false, cancel: Some(token), ... }
             5. Create CancellationToken
             6. Spawn Ctrl-C task: ctrl_c().await → token.cancel()
             7. Print: "Starting run — press Ctrl-C to interrupt"
             8. scenarios::runner::run(config, opts).await → RunResult | RunnerError
             9. On Ok(result):
                  if token.is_cancelled(): return Err(CliError::Interrupted)
                  println!("Run ID   : {}", result.run_id)
                  println!("Output   : {}", result.output_dir.display())
                  println!("Attempted: {}", result.stats.total_attempted)
                  println!("Confirmed: {}", result.stats.confirmed)
                  println!("Failed   : {}", result.stats.failed)
                  println!("Timed out: {}", result.stats.timed_out)
                  exit 0
            10. On Err(CliError::Interrupted): exit 130
            11. On Err(e): eprintln!("error: {e}") → exit 1
```

**Note on output directory path:** The CLI prints `result.output_dir` directly — this field
is populated by T7 from `run_dir.path()` and requires the prerequisite T7 change described
in Section 2.2. The CLI does not reconstruct the path itself.

**Note on exit code 130:** After `runner::run()` returns, the CLI checks
`token.is_cancelled()`. If true, the run was interrupted by SIGINT and the process exits
with code 130 (Unix convention: `128 + SIGINT signal number 2`). This allows shell scripts
and CI pipelines to distinguish "the run failed" from "someone pressed Ctrl-C." The check
happens after `runner::run()` returns because teardown must complete regardless — the exit
code reflects what happened during the run, not during teardown.

**Ctrl-C during setup/warmup:** The cancellation token is checked only in the load phase
loop. If the operator presses Ctrl-C during setup or warmup, those phases complete
(or fail on their own timeout) first. Teardown then runs. The CLI prints:
`"Interrupted — waiting for current phase to complete..."` when the cancel signal fires
before the load phase begins.

### 6.2 `z3sim run --scenario <path> --dry-run`

```
argv
 └─ Commands::Run { args, dry_run: true }
     └─ cli::run_command(args).await
         1. load_scenario(&args.scenario)
         2. validate_scenario(&config)
         3. Build RunOptions { dry_run: true, cancel: None, ... }
            No CancellationToken needed — dry-run is synchronous from the CLI perspective
         4. scenarios::runner::run(config, opts).await
            (T7 short-circuits at step 2: calls print_dry_run_summary, returns Ok)
         5. exit 0
```

T7's `run()` already handles the dry-run case internally via `print_dry_run_summary`.
The CLI does not need to duplicate that output.

### 6.3 `z3sim generate-fixtures --scenario <path> --out <dir>`

```
argv
 └─ Commands::GenerateFixtures { args }
     └─ cli::generate_fixtures_command(args)   (sync — no await needed)
         1. load_scenario(&args.scenario)       → ScenarioConfig | ConfigError
         2. validate_scenario(&config)          → () | ConfigError
         3. AccountGenerator::new(config)       → AccountGenerator | GeneratorError
         4. gen.generate_population()           → SyntheticPopulation | GeneratorError
         5. write_fixtures(&population, &args.out) → () | FixtureError
         6. Print:
              println!("Accounts : {}", population.accounts.len())
              println!("Active   : {}", population.active_count())
              println!("Written  : {}", args.out.display())
              println!("  accounts.json")
              println!("  wallets.json")
         7. exit 0
```

**Note:** `generate_fixtures_command` does not need `async` — all operations are
synchronous (filesystem + pure CPU). But `dispatch()` is `async`, so this handler can be
a synchronous function called with no `.await`.

**Validation note:** The full scenario validation is run before fixture generation, even
though TPS and duration fields are not used by the fixture generator. This is intentional:
it ensures that fixture data generated here will also be valid for a subsequent `run`
command using the same scenario file.

### 6.4 `z3sim validate-scenario <path>`

```
argv
 └─ Commands::ValidateScenario { path }
     └─ cli::validate_scenario_command(path)  (sync)
         1. load_scenario(&path)
              On Io error:    eprintln!("error: cannot read {path}: {e}") → exit 1
              On Parse error: eprintln!("error: invalid YAML in {path}: {e}") → exit 1
         2. validate_scenario(&config)
              On ValidationErrors(errs):
                eprintln!("error: scenario validation failed ({n} error(s)):")
                for (field, msg) in &errs:
                  eprintln!("  {field}: {msg}")
                exit 1
         3. println!("OK: {path} is valid")
            println!("  name : {}", config.name)
            println!("  seed : {}", config.seed)
            println!("  hash : {}", config.config_hash)
         4. exit 0
```

`validate-scenario` must never call any Z3, RPC, or synthetic generator code.
Its sole inputs are a file path and its YAML content.

---

## 7. Error Handling and Exit Codes

### 7.1 Error type strategy

Introduce `CliError` in `src/cli/mod.rs`. Follow the project's existing typed-enum pattern
(no `anyhow`, no `thiserror` macro, no `Box<dyn Error>`):

```rust
#[derive(Debug)]
pub enum CliError {
    Scenario(ConfigError),       // load / validate failure
    Run(RunnerError),            // runner failure
    Fixture(FixtureError),       // fixture write failure
    Generator(GeneratorError),   // synthetic generator failure
    InvalidArgs(String),         // bad flag combination (e.g. --burst-multiplier <= 0)
    Io(std::io::Error),          // catch-all for filesystem errors not covered above
    Interrupted,                 // SIGINT received; teardown completed; exit 130
}

impl std::fmt::Display for CliError { ... }
impl std::error::Error for CliError { ... }
```

`dispatch()` returns `Result<(), CliError>`. `main()` pattern-matches on the error variant
to select the correct exit code: `Interrupted` → 130, all others → 1.

`InvalidArgs` is used for pre-flight validation of flag combinations that `clap` cannot
enforce statically (e.g. `--burst-multiplier` must be > 0). It produces a clear
`"error: invalid arguments: ..."` message rather than the misleading `"I/O error: ..."`
that `CliError::Io` would produce. `Io` is reserved for actual filesystem errors.

### 7.2 User-facing error formatting

All errors go to stderr. All success output goes to stdout. This allows CI scripts to
capture stdout cleanly and detect failure by exit code without parsing mixed output.

Error messages follow this format:

```
error: <human-readable context>: <cause>
```

For validation errors, each field violation is indented:

```
error: scenario validation failed (3 error(s)):
  load_target_tps: must be > 0.0
  flows: flow fractions must sum to 1.0, got 0.500000
  accounts_active_fraction: must be in range (0.0, 1.0]
```

### 7.3 Exit code policy

| Situation | Exit code |
|---|---|
| All subcommands succeed | 0 |
| Scenario file not found / unreadable | 1 |
| YAML parse error | 1 |
| Scenario validation failure | 1 |
| Invalid flag combination | 1 |
| Output directory creation failure | 1 |
| Synthetic generator error | 1 |
| Runner error (setup, warmup, load, teardown) | 1 |
| SIGINT received (teardown completes, then exit) | 130 |

Exit code 130 follows the Unix convention (`128 + 2` where 2 is `SIGINT`). It is emitted
only after teardown has completed — the run output files and manifest are preserved before
the process exits. CI pipelines and shell scripts can reliably distinguish an interrupted
run from a failed one by testing `$? -eq 130`.

### 7.4 Specific failure paths

| Path | What happens |
|---|---|
| Missing `--scenario` file | `load_scenario` returns `ConfigError::Io`; CLI prints path + OS error; exit 1 |
| Malformed YAML | `load_scenario` returns `ConfigError::Parse`; CLI prints YAML parse error; exit 1 |
| Validation failure | `validate_scenario` returns `ConfigError::ValidationErrors`; CLI lists each violation; exit 1 |
| `--out` dir creation failure | `write_fixtures` returns `FixtureError::Io`; CLI prints path + OS error; exit 1 |
| Bad flag combination | `CliError::InvalidArgs(msg)`; CLI prints message; exit 1 |
| Runner setup failure | `RunnerError::Setup(msg)`; CLI prints message; exit 1 |
| Interrupted during load | cancellation token fires; load phase exits; teardown completes; `token.is_cancelled()` → exit 130 |
| Teardown failure after successful load | `RunnerError::Teardown`; CLI prints message; exit 1 |

---

## 8. Logging and Tracing Plan

### 8.1 Setup

Add to `Cargo.toml`:

```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

### 8.2 Subscriber initialisation

Call `cli::init_tracing(verbose, quiet)` in `main()` immediately after parsing argv,
before any other library code runs:

```rust
pub fn init_tracing(verbose: bool, quiet: bool) {
    use std::io::IsTerminal as _;

    let level = if verbose {
        tracing::Level::DEBUG
    } else if quiet {
        tracing::Level::ERROR
    } else {
        tracing::Level::WARN
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(level.into()),
        )
        .with_target(true)
        .with_level(true)
        .with_ansi(std::io::stderr().is_terminal()) // colour in interactive shells; plain in CI/pipes
        .with_writer(std::io::stderr)               // logs to stderr; stdout stays clean for CLI output
        .init();
}
```

`RUST_LOG` takes precedence (via `from_default_env()`), so operators can override the
level at any granularity without recompiling. The ANSI colour flag is derived from whether
stderr is a real terminal — this means coloured output in local shells and clean plain text
in CI logs and piped output, with zero configuration required.

The `Cli` struct should declare a `long_about` to surface the `RUST_LOG` override:

```rust
#[derive(Parser)]
#[command(
    name = "z3sim",
    version,
    about = "Z3 Exchange Simulator — exchange-scale load testing for the Zcash Z3 stack",
    long_about = "Z3 Exchange Simulator — exchange-scale load testing for the Zcash Z3 stack.\n\n\
                  Verbosity is controlled by --verbose / --quiet. \
                  For fine-grained log filtering, set RUST_LOG before invoking:\n\n  \
                  RUST_LOG=z3_exchange_simulator=debug z3sim run --scenario configs/scenarios/smoke.yaml"
)]
pub struct Cli { ... }
```

### 8.3 What should be logged (not printed)

| Event | Level | Location |
|---|---|---|
| Scenario loaded | DEBUG | CLI handler |
| Validation passed | DEBUG | CLI handler |
| Ctrl-C received | WARN | Ctrl-C task |
| Cancellation propagated | WARN | Ctrl-C task |
| Z3 stack started (if T2 emits tracing) | INFO/DEBUG | T2 |
| RPC calls (if T3 emits tracing) | DEBUG/TRACE | T3 |

### 8.4 What should be printed (not logged)

| Event | Stream | Format |
|---|---|---|
| Run start message | stdout | `"Starting run — press Ctrl-C to interrupt"` |
| Run ID + output dir | stdout | `"Run ID: <id>"` / `"Output: <path>"` |
| Dry-run summary | stdout | Delegated to T7's `print_dry_run_summary` |
| Run statistics | stdout | Tabular key/value |
| validate-scenario pass | stdout | `"OK: <path> is valid"` |
| validate-scenario details | stdout | Indented name/seed/hash |
| generate-fixtures result | stdout | Account count + file paths |
| All errors | stderr | `"error: ..."` |

The split between `println!` (stdout) and `eprintln!` (stderr) must be consistent. Tests
can capture each stream independently to verify correctness.

### 8.5 Avoiding noisy logs

Default log level is WARN, not INFO. Most operators running `z3sim run` should see no
log lines in normal operation — only the structured stdout output printed by the CLI.
Log lines are for diagnostic use (`--verbose`) and failure investigation.

The T7 runner and T5 exchange workflows should already emit tracing spans/events at
appropriate levels. T8 does not add new spans beyond CLI lifecycle events.

---

## 9. Graceful Shutdown Plan

### 9.1 SIGINT handling approach

Use `tokio::signal::ctrl_c()` (cross-platform; maps to SIGINT on Unix, Ctrl-C on
Windows). Spawn a dedicated Tokio task before calling `runner::run`:

```rust
let token = tokio_util::sync::CancellationToken::new();
let cancel_task_token = token.clone();

tokio::spawn(async move {
    tokio::signal::ctrl_c().await.ok();
    tracing::warn!("interrupt signal received — cancelling load phase");
    cancel_task_token.cancel();
});

let opts = RunOptions {
    cancel: Some(token),
    ..
};
runner::run(config, opts).await
```

### 9.2 Cancellation propagation

The cancellation token is already checked inside the T7 load phase scheduler loop:

```rust
if let Some(cancel) = &opts.cancel {
    if cancel.is_cancelled() {
        break;  // exits load loop; teardown always runs after
    }
}
```

This is checked on every TPS tick. Worst-case latency before the check: one tick period
(`1 / tps` seconds — at 1 TPS, up to 1 second). This is acceptable.

### 9.3 Phases that do not check cancellation

`setup` and `warmup` do not check the cancellation token (confirmed by reading
`lifecycle.rs`). If the operator presses Ctrl-C during these phases:

1. The `cancel_task_token.cancel()` call fires immediately.
2. The token is cancelled in memory.
3. `setup` / `warmup` continue to their natural completion or timeout.
4. When the load phase begins, the first tick check will detect the token and exit
   immediately.
5. Teardown runs normally.

This means Ctrl-C during setup/warmup delays exit by up to the remaining setup/warmup
duration. Operators should be informed: emit `tracing::warn!("interrupt signal received — \
waiting for current phase to complete before shutting down")` from the Ctrl-C task
immediately when the signal fires. This is accepted behaviour for the current scope.

**Known limitation — documented, no code change required for T8:** Making setup and warmup
cancellation-aware would require adding the token to `lifecycle.rs` and checking it between
each RPC call. That is a T7 internal change; T8 cannot and should not reach into T7's
lifecycle internals to implement it. A comment in the CLI source noting this limitation is
sufficient:

```rust
// NOTE: cancellation is only checked in the load phase scheduler loop.
// Ctrl-C during setup or warmup will be registered but will not take
// effect until the load phase begins. See lifecycle.rs for the phases
// that would need token checks to improve responsiveness here.
```

### 9.4 Teardown on interruption

`teardown` always runs after the load phase, whether it exited normally or due to
cancellation:

```rust
// From runner/mod.rs — unchanged in T8
let teardown_result = teardown(
    stack,
    &run_dir,
    &mut manifest,
    &recorder,
    &scenario.source_path,
    load_succeeded,  // false when cancelled
)
.await;
```

Teardown stops the Z3 stack, flushes latency samples, finalises the manifest, copies the
scenario YAML, and (only if `load_succeeded`) generates the summary report. On
interruption, the summary is skipped; the JSONL files and manifest are preserved.

### 9.5 Exit code on interruption

After `runner::run()` returns (teardown has completed), the CLI checks the token:

```rust
let result = runner::run(config, opts).await;
if token.is_cancelled() {
    // Interrupted by SIGINT — teardown has already completed
    return Err(CliError::Interrupted);
}
match result {
    Ok(r) => { /* print stats */ Ok(()) }
    Err(e) => Err(CliError::Run(e)),
}
```

`main()` then translates `CliError::Interrupted` to `std::process::exit(130)`. All other
errors print to stderr and exit 1.

### 9.6 API changes needed in T7

`RunOptions::cancel` is already defined and wired. T8 only needs to populate it.

The one required T7 change is adding `output_dir: PathBuf` to `RunResult` (see Section
2.2). This is the only modification to T7 code that T8 requires.

---

## 10. Testing Plan

### 10.1 Unit tests for argument parsing

These tests live in `src/cli/mod.rs` under `#[cfg(test)]` and use `clap`'s built-in
`try_parse_from` API. No async runtime needed.

```rust
// Test: run subcommand parses --scenario and --dry-run correctly
// Test: run subcommand parses --hot-wallet-uuid when provided
// Test: run subcommand leaves hot_wallet_uuid as None when omitted
// Test: generate-fixtures requires both --scenario and --out
// Test: validate-scenario accepts a positional argument
// Test: --verbose and --quiet are mutually exclusive (clap rejects both)
// Test: unknown subcommand returns a clap error
// Test: missing required --scenario argument returns a clap error
// Test: --load-shape accepts all four variants
// Test: invalid --load-shape value returns a clap error
// Test: LoadShapeArg::Ramp converts to LoadShape::Ramp with default ramp_secs
// Test: LoadShapeArg::Burst converts to LoadShape::Burst with all three parameters
// Test: --burst-multiplier <= 0.0 produces CliError::InvalidArgs (not CliError::Io)
```

All these tests call `Cli::try_parse_from(["z3sim", ...])` and assert on the parsed
struct or the error kind. They are fast, synchronous, and require no external state.

### 10.2 Tests for validate-scenario behaviour

Test the `validate_scenario_command` function in isolation by calling it directly with
paths to temp files. These tests exercise the error formatting without running the CLI
binary.

```rust
// Test: valid smoke.yaml file returns Ok(())
// Test: non-existent path produces CliError::Scenario(ConfigError::Io)
// Test: malformed YAML produces CliError::Scenario(ConfigError::Parse)
// Test: invalid flows (sum != 1.0) produces CliError with all violations listed
```

Use `tempfile::NamedTempFile` for invalid YAML paths (consistent with existing T7 tests).

### 10.3 Tests for generate-fixtures behaviour

```rust
// Test: valid scenario generates accounts.json and wallets.json in the given dir
// Test: output dir is created if it does not exist
// Test: same scenario + seed produces identical files (determinism)
// Test: account count in accounts.json matches scenario's accounts_count
// Test: non-existent scenario file returns CliError
```

### 10.4 Tests for dry-run behaviour

```rust
// Test: dry-run with valid scenario returns Ok without creating run directory
// Test: dry-run with invalid scenario returns CliError (no run dir created)
```

These tests call `run_command` directly with `dry_run: true` and assert on the `Result`.
They do not require a live Z3 stack.

### 10.5 Golden / help output tests

Consider a snapshot test that captures `z3sim --help` output and asserts it matches a
reference string. This prevents accidental regression of the help interface. Use
`assert!(output.contains("run"))` rather than full-string comparison to avoid brittleness
from clap formatting changes across versions.

### 10.6 Ctrl-C / cancellation unit test

A unit test for cancellation dispatch:

```rust
// Create a CancellationToken
// Call token.cancel() immediately
// Verify that run_command with the token returns Ok (graceful exit, not panic)
// Verify that no run directory was created (teardown was never invoked, or dry-run was used)
```

This can be done with a dry-run invocation plus a pre-cancelled token, without starting Z3.

For a non-dry-run cancellation test: this requires the Z3 stack and belongs in integration
tests (T9). Mark explicitly as `#[ignore]` until a live stack is available.

### 10.7 What should be mocked

- No mocking needed for CLI unit tests — clap parsing is pure and synchronous
- `validate-scenario` and `generate-fixtures` tests use real temp files (consistent with
  existing project practice — see T7's config tests)
- `run` command tests that don't start Z3 use `--dry-run` as the isolation mechanism

### 10.8 What must not require a live Z3 stack

Everything except the full `run` integration test. All the tests in sections 10.1–10.6
can run with `cargo test --lib` under CI.

Mark any test that requires Z3 with `#[ignore]`. The CI workflow already runs only
`--lib` tests on PR; tagged tests run in the separate manual/nightly job.

---

## 11. Implementation Steps

Ordered from prerequisites to completion. Each step should compile and pass `cargo
test --lib` before the next step begins.

### Step 1: Add dependencies to Cargo.toml

```toml
[dependencies]
# existing entries unchanged
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

**Checkpoint:** `cargo build` succeeds with new deps.

### Step 2: Write clap types in `src/cli/mod.rs`

Implement `Cli`, `Commands`, `RunArgs`, `GenerateFixturesArgs`, `LoadShapeArg`.
Do not implement dispatch logic yet — leave `dispatch()` as a `todo!()`.

```rust
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "z3sim",
    version,
    about = "Z3 Exchange Simulator — exchange-scale load testing for the Zcash Z3 stack",
    long_about = "Z3 Exchange Simulator — exchange-scale load testing for the Zcash Z3 stack.\n\n\
                  Verbosity is controlled by --verbose / --quiet. \
                  For fine-grained log filtering, set RUST_LOG before invoking:\n\n  \
                  RUST_LOG=z3_exchange_simulator=debug z3sim run --scenario configs/scenarios/smoke.yaml"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    #[arg(short = 'v', long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,
    #[arg(short = 'q', long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    Run(RunArgs),
    GenerateFixtures(GenerateFixturesArgs),
    ValidateScenario { path: std::path::PathBuf },
}

#[derive(clap::Args)]
pub struct RunArgs {
    #[arg(long)]
    pub scenario: std::path::PathBuf,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, value_enum, default_value_t = LoadShapeArg::Steady)]
    pub load_shape: LoadShapeArg,
    #[arg(long, default_value_t = 60)]
    pub ramp_secs: u64,
    #[arg(long, default_value_t = 60)]
    pub burst_pre_secs: u64,
    #[arg(long, default_value_t = 30)]
    pub burst_secs: u64,
    #[arg(long, default_value_t = 3.0)]
    pub burst_multiplier: f64,
    #[arg(long, default_value_t = 64)]
    pub max_in_flight: usize,
    #[arg(long, default_value = "experiments/runs")]
    pub output_base: std::path::PathBuf,
    #[arg(long)]
    pub hot_wallet_uuid: Option<String>,
}

#[derive(clap::Args)]
pub struct GenerateFixturesArgs {
    #[arg(long)]
    pub scenario: std::path::PathBuf,
    #[arg(long)]
    pub out: std::path::PathBuf,
}

#[derive(ValueEnum, Clone, Default, Debug)]
pub enum LoadShapeArg {
    #[default]
    Steady,
    Ramp,
    Burst,
    Mixed,
}
```

**Checkpoint:** `cargo build` succeeds. `cargo test --lib` passes.

### Step 3: Add clap parsing unit tests

Write tests in `src/cli/mod.rs` for all argument parsing cases listed in Section 10.1.

**Checkpoint:** `cargo test --lib cli` passes.

### Step 4: Implement `init_tracing` and `CliError`

```rust
pub fn init_tracing(verbose: bool, quiet: bool) {
    // See Section 8.2 for full implementation.
    // Key points: compact formatter, stderr writer, ANSI auto-detection, EnvFilter.
}

pub enum CliError {
    Scenario(ConfigError),
    Run(RunnerError),
    Fixture(FixtureError),
    Generator(GeneratorError),
    InvalidArgs(String),  // bad flag combination; distinct from Io
    Io(std::io::Error),   // actual filesystem errors only
    Interrupted,          // SIGINT; triggers exit 130
}
impl std::fmt::Display for CliError { ... }
impl std::error::Error for CliError { ... }
```

The `Display` impl for `InvalidArgs` should produce:
`"invalid arguments: <message>"` — not `"I/O error: <message>"`.

**Checkpoint:** `cargo build` succeeds.

### Step 5: Implement `validate_scenario_command` and its tests

The simplest command: no async, no Z3, pure I/O + validation.

```rust
fn validate_scenario_command(path: &Path) -> Result<(), CliError> { ... }
```

Write the tests from Section 10.2.

**Checkpoint:** `cargo test --lib` passes including validate tests.

### Step 6: Implement `generate_fixtures_command` and its tests

```rust
fn generate_fixtures_command(args: &GenerateFixturesArgs) -> Result<(), CliError> { ... }
```

Write tests from Section 10.3.

**Checkpoint:** `cargo test --lib` passes including fixture tests.

### Step 7: Implement `run_command` in dry-run mode

Implement the `run` handler for `dry_run: true` only first. This avoids needing a live Z3
stack to test the command wiring.

Write tests from Section 10.4 (dry-run tests).

**Checkpoint:** `cargo test --lib` passes.

### Step 8: Implement `run_command` for live runs with Ctrl-C

Add the `CancellationToken` setup, `ctrl_c()` task, and full `runner::run()` call.
After `runner::run()` returns, check `token.is_cancelled()` to decide between
`CliError::Interrupted` (→ exit 130) and reporting a genuine runner error (→ exit 1).
Use `result.output_dir` (the new `RunResult` field) to print the output path.
Add the cancellation unit test from Section 10.6.

**Checkpoint:** `cargo test --lib` passes. Binary can be built and executes `--help`.

### Step 9: Implement `dispatch()` and wire up `main.rs`

```rust
// src/cli/mod.rs
pub async fn dispatch(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Commands::Run(args) => run_command(args).await,
        Commands::GenerateFixtures(args) => generate_fixtures_command(&args),
        Commands::ValidateScenario { path } => validate_scenario_command(&path),
    }
}
```

```rust
// src/main.rs
#[tokio::main]
async fn main() {
    use z3_exchange_simulator::cli::{dispatch, init_tracing, Cli, CliError};
    use clap::Parser;

    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.quiet);

    match dispatch(cli).await {
        Ok(()) => {}
        Err(CliError::Interrupted) => {
            // Teardown completed cleanly; exit 130 (128 + SIGINT).
            std::process::exit(130);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
```

**Checkpoint:** `cargo build` succeeds. `./target/debug/z3sim --help` shows correct help
including the `RUST_LOG` note in the long description.
`./target/debug/z3sim validate-scenario configs/scenarios/smoke.yaml` exits 0.

### Step 10: Update Makefile

Replace `echo TODO` bodies in `generate-fixtures` and `scenario-dry-run` targets.
Add a `validate-scenario` target.

```makefile
generate-fixtures: ## Generate synthetic fixture data for tests
	./target/debug/$(BINARY) generate-fixtures \
		--scenario configs/scenarios/smoke.yaml \
		--out experiments/fixtures

scenario-dry-run: ## Validate and summarise smoke scenario without starting Z3
	./target/debug/$(BINARY) run \
		--scenario configs/scenarios/smoke.yaml \
		--dry-run

validate-scenario: ## Validate a scenario YAML file (usage: make validate-scenario SCENARIO=<path>)
	./target/debug/$(BINARY) validate-scenario \
		$(or $(SCENARIO),configs/scenarios/smoke.yaml)
```

**Checkpoint:** `make generate-fixtures` and `make scenario-dry-run` succeed.

### Step 11: Final review pass

- Run `cargo clippy -- -D warnings` and resolve all findings
- Run `cargo fmt -- --check`
- Run `cargo test --lib`
- Manually verify all four subcommands from the shell
- Check that stdout and stderr are correctly separated in each command

---

## 12. Resolved Design Decisions

All design decisions and open questions from initial planning have been resolved. This
section records the final decision for each so implementors have a single authoritative
reference.

### D1 — Branching strategy

**Decision:** T8 lives on the dedicated `t8-cli` branch. The plan is written against the
T7 code visible at `t7-scenario-runner` commit `18081f2` and can be committed without
waiting for T7 to merge. Implementation must not begin until T7 is merged into `main`;
at that point, re-verify this plan against the merged code for any API drift before
writing a single line of T8 Rust.

### D2 — Output directory path

**Decision:** Add `output_dir: PathBuf` to `RunResult` in `src/scenarios/runner/result.rs`.
The CLI prints `result.output_dir` directly. This is a prerequisite T7 change (3 lines:
field declaration + 2 construction sites). It removes hidden coupling between the CLI and
T7's internal directory naming logic.

### D3 — Cancellation during setup/warmup

**Decision:** Accept the limitation. Setup and warmup do not check the cancellation token;
Ctrl-C during those phases takes effect only once the load phase begins. No code change
is required. The CLI source must include an inline comment documenting this (see Section
9.3) so future maintainers understand the constraint without having to read `lifecycle.rs`.

### D4 — Error type for invalid flag combinations

**Decision:** Use `CliError::InvalidArgs(String)`. This produces
`"error: invalid arguments: <message>"`. The `CliError::Io` variant is reserved for actual
filesystem errors. Both `InvalidArgs` and `Interrupted` are new variants added in T8;
all others wrap existing error types from T4–T7.

### D5 — Fixture generation validation scope

**Decision:** Full validation always runs before fixture generation. One validation rule
for all commands — the scenario file must be valid for a live run before any command
accepts it.

### D6 — `--hot-wallet-uuid` flag

**Decision:** Implement it. It is one `#[arg(long)] pub hot_wallet_uuid: Option<String>`
field in `RunArgs`, wires directly into `RunOptions::hot_wallet_uuid` (already exists in
T7), and is genuinely useful for multi-run experiment workflows where re-mining funds
for each run would be wasteful.

### D7 — Exit code for SIGINT interruption

**Decision:** Exit 130. `CliError::Interrupted` is returned when `token.is_cancelled()` is
true after `runner::run()` returns. `main()` maps it to `std::process::exit(130)`. All
other errors exit 1. This follows the Unix convention (`128 + 2`) and allows CI scripts
to distinguish interrupted from failed runs with `$? -eq 130`.

### D8 — `RUST_LOG` in help text

**Decision:** Document it in `long_about` on the `Cli` struct. Shown only for
`z3sim --help` (long form); not shown for `-h` (short form). Format matches the
idiomatic one-liner used by other `tracing-subscriber` tools (see Section 8.2).

### D9 — Log formatter

**Decision:** Compact formatter with automatic ANSI colour detection:
`.with_ansi(std::io::stderr().is_terminal())`. No user configuration needed. Interactive
shells get colour; CI logs and piped output get plain text. The compact format (one line
per event) is appropriate for both contexts.

---

## 13. Acceptance Criteria

T8 is complete when all of the following pass:

### AC1 — Command behaviour

| Check | Pass condition |
|---|---|
| `z3sim --help` | Exits 0, output includes `run`, `generate-fixtures`, `validate-scenario` |
| `z3sim run --help` | Exits 0, shows `--scenario`, `--dry-run`, `--load-shape` |
| `z3sim validate-scenario configs/scenarios/smoke.yaml` | Exits 0, prints `OK:` |
| `z3sim validate-scenario /nonexistent.yaml` | Exits 1, stderr contains `error:` |
| `z3sim validate-scenario` (with a deliberately invalid YAML) | Exits 1, lists all validation errors |
| `z3sim generate-fixtures --scenario configs/scenarios/smoke.yaml --out /tmp/fixtures` | Exits 0, creates `accounts.json` and `wallets.json` |
| `z3sim run --scenario configs/scenarios/smoke.yaml --dry-run` | Exits 0, prints dry-run summary, no run directory created |

### AC2 — Tests

| Check | Pass condition |
|---|---|
| `cargo test --lib` | All unit tests pass |
| CLI parsing tests | All argument variants, conflicts, and error cases covered |
| validate-scenario tests | Valid, Io error, Parse error, ValidationErrors covered |
| generate-fixtures tests | Creates files, correct count, determinism verified |
| Dry-run test | No run directory created |
| Cancellation test | Pre-cancelled token returns Ok gracefully |

### AC3 — Logging

| Check | Pass condition |
|---|---|
| Default invocation | No log lines visible in normal successful run |
| `--verbose` | DEBUG-level lines appear |
| `--quiet` | No log lines unless an error occurs |
| `RUST_LOG=trace` | Overrides `--verbose`/`--quiet` and enables trace-level output |

### AC4 — Graceful shutdown

| Check | Pass condition |
|---|---|
| Ctrl-C during load phase | Load exits, teardown runs, JSONL files preserved, exit **130** |
| Ctrl-C during setup/warmup | Signal registered; `tracing::warn!` emitted; phases run to completion; exit 130 |
| Normal completion after Ctrl-C ignored | Exit 0 |

### AC5 — Architecture boundaries

| Check | Pass condition |
|---|---|
| `src/cli/mod.rs` contains no calls to `reqwest`, `tokio::net`, or Z3 harness types | Confirmed by grep |
| `src/main.rs` contains no business logic beyond `Cli::parse()` + `dispatch().await` | Confirmed by inspection |
| No `anyhow`, no `thiserror` imports in CLI files | `grep -r 'anyhow\|thiserror' src/cli src/main.rs` returns nothing |
| `z3sim --help` long description mentions `RUST_LOG` | Confirmed by running the binary |
| Log output goes to stderr, not stdout | Confirmed by stream-separation test |
| `stderr` colour disabled when piped | Confirmed by `z3sim ... 2>/tmp/stderr.txt; grep -c $'\033' /tmp/stderr.txt` returning 0 |

### AC6 — Makefile

| Check | Pass condition |
|---|---|
| `make generate-fixtures` | Succeeds after `make build` |
| `make scenario-dry-run` | Succeeds after `make build` |
| `make validate-scenario` | Succeeds after `make build` |

---

## 14. Reviewer Checklist

The following checklist is for a human or agent reviewing this plan before implementation begins.

### Architecture

- [ ] The module layout (two files: `main.rs`, `cli/mod.rs`) is appropriate for the scope
- [ ] No business logic is proposed inside `src/cli/` or `src/main.rs`
- [ ] All delegation paths correctly name the T4–T7 API they call
- [ ] The `LoadShapeArg → LoadShape` conversion correctly maps all four variants
- [ ] The only required T7 change (`output_dir` on `RunResult`) is clearly identified as a prerequisite

### Command interface

- [ ] All four subcommands from WORK-TRACKS.md are covered
- [ ] `--hot-wallet-uuid` is included in `RunArgs` and wired to `RunOptions::hot_wallet_uuid`
- [ ] `--dry-run` is correctly scoped to the `run` subcommand (not a global flag)
- [ ] `--verbose` and `--quiet` are declared `conflicts_with` each other
- [ ] `--scenario` is declared required (not optional) for `run` and `generate-fixtures`
- [ ] Burst companion flags have sensible defaults and a `CliError::InvalidArgs` validation step
- [ ] The proposed flag names follow Rust CLI conventions (kebab-case)

### Data flow

- [ ] `run` command: cancellation token is created before `runner::run()` and passed via `RunOptions::cancel`
- [ ] After `runner::run()` returns, `token.is_cancelled()` is checked to select between exit 130 and exit 1
- [ ] `run --dry-run`: no `CancellationToken` needed (synchronous from CLI perspective)
- [ ] `generate-fixtures`: full scenario validation is run before population generation
- [ ] `validate-scenario`: only calls `load_scenario` + `validate_scenario`, nothing else
- [ ] CLI prints `result.output_dir` (not a reconstructed path) to show where output landed

### Error handling

- [ ] `CliError` has six variants: `Scenario`, `Run`, `Fixture`, `Generator`, `InvalidArgs`, `Io`, `Interrupted`
- [ ] `InvalidArgs` is used for bad flag combinations; `Io` is reserved for filesystem errors
- [ ] All errors go to stderr; all success output goes to stdout
- [ ] Exit code policy: 0 success, 1 error, 130 SIGINT — all three cases handled in `main()`
- [ ] The `ValidationErrors` formatting lists each field/message pair on a separate line

### Logging

- [ ] Logs go to stderr (not stdout)
- [ ] Default level is WARN (no output in normal successful run)
- [ ] `RUST_LOG` can override the level set by `--verbose` / `--quiet`
- [ ] `RUST_LOG` usage is documented in `long_about` on the `Cli` struct
- [ ] ANSI colour is auto-detected via `std::io::stderr().is_terminal()`
- [ ] `init_tracing` is called before any library code runs

### Graceful shutdown

- [ ] Ctrl-C is handled via `tokio::signal::ctrl_c()` (not OS signal handler)
- [ ] Cancellation propagates via `RunOptions::cancel` (already wired in T7)
- [ ] Teardown runs after both normal completion and cancellation
- [ ] The limitation (no cancellation in setup/warmup) is documented with a code comment (Section 9.3)
- [ ] SIGINT produces exit code 130, not 1

### Testing

- [ ] Parsing tests use `clap::Parser::try_parse_from` (no subprocess invocation)
- [ ] Tests that require a live Z3 stack are marked `#[ignore]`
- [ ] Determinism is tested for `generate-fixtures` (same seed → same output)
- [ ] Ctrl-C test uses a pre-cancelled token (no signal injection needed)

### Dependencies

- [ ] Only `clap`, `tracing`, `tracing-subscriber` are added — no other new crates
- [ ] `tokio-util` (for `CancellationToken`) is already in Cargo.toml — confirmed
- [ ] No `anyhow`, no `thiserror`

### Makefile

- [ ] `generate-fixtures`, `scenario-dry-run`, and `validate-scenario` targets are updated
- [ ] Makefile targets depend on `build` target (or invoke `cargo build` first) to avoid
  running a stale binary

---

## 15. Recommended Validation Commands

```sh
# After implementing: build the binary and verify help output
cargo build
./target/debug/z3sim --help
./target/debug/z3sim run --help
./target/debug/z3sim generate-fixtures --help
./target/debug/z3sim validate-scenario --help

# Run all library unit tests (no Z3 required)
cargo test --lib

# Lint and format
cargo clippy -- -D warnings
cargo fmt -- --check

# Smoke-test each subcommand (no Z3 required)
./target/debug/z3sim validate-scenario configs/scenarios/smoke.yaml
./target/debug/z3sim validate-scenario /nonexistent.yaml        # should exit 1
./target/debug/z3sim generate-fixtures \
    --scenario configs/scenarios/smoke.yaml \
    --out /tmp/z3sim-fixtures
./target/debug/z3sim run --scenario configs/scenarios/smoke.yaml --dry-run

# Verify stdout/stderr separation
./target/debug/z3sim validate-scenario configs/scenarios/smoke.yaml 1>/tmp/stdout.txt 2>/tmp/stderr.txt
cat /tmp/stdout.txt   # should contain "OK:"
cat /tmp/stderr.txt   # should be empty

./target/debug/z3sim validate-scenario /nonexistent.yaml 1>/tmp/stdout.txt 2>/tmp/stderr.txt
cat /tmp/stdout.txt   # should be empty
cat /tmp/stderr.txt   # should contain "error:"

# Verify ANSI colour is absent when stderr is piped
./target/debug/z3sim --verbose validate-scenario configs/scenarios/smoke.yaml 2>/tmp/stderr.txt
grep -c $'\033' /tmp/stderr.txt   # should be 0 (no ANSI escape codes in piped output)

# Verify exit codes
./target/debug/z3sim validate-scenario configs/scenarios/smoke.yaml; echo "exit: $?"   # should be 0
./target/debug/z3sim validate-scenario /nonexistent.yaml;            echo "exit: $?"   # should be 1

# Makefile targets
make build
make generate-fixtures
make scenario-dry-run
make validate-scenario

# Full integration test (requires live Z3 stack — run manually)
make clone-z3
cd external/z3 && ./scripts/regtest-init.sh && docker compose --env-file .env.regtest up -d && cd ../..
./target/debug/z3sim run --scenario configs/scenarios/smoke.yaml
# Press Ctrl-C during the load phase and verify: echo "exit: $?"  → should be 130
```
