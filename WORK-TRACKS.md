# Work Tracks — Z3 Exchange Simulator

Planning document for parallel development between two engineers. Identifies the
independent work streams, their dependencies, the interface contracts that must be
agreed before splitting work, and the integration milestones where the streams converge.

---

## Quick reference

| Track | Module | Description | Depends on | Can start |
|---|---|---|---|---|
| T1 — Data Model | `src/data_model/` | Core Rust types used across all other modules | Nothing | Immediately |
| T2 — Z3 Process Harness | `src/z3/` | Start, stop, and monitor Zebra / Zaino / Zallet processes | Nothing | Immediately |
| T3 — RPC Client | `src/rpc/` | Typed Rust API wrapping every Z3 JSON-RPC method | T1 (types) | After T1 interface agreed |
| T4 — Synthetic Generators | `src/synthetic/` | Deterministic fake accounts, addresses, and transaction intents | T1 | After T1 |
| T5 — Exchange Emulation | `src/scenarios/exchange/` | Deposit, withdrawal, sweep, and balance-tracking workflows | T1, T3 | After T3 interface agreed |
| T6 — Observability & Metrics | `src/metrics/` | JSONL output, latency histograms, manifest, run summary | T1 | After T1 |
| T7 — Scenario Runner | `src/scenarios/runner/` | YAML parsing, TPS scheduling, run orchestration | T4, T5, T6 | After T5 is functional |
| T8 — CLI | `src/cli/`, `src/main.rs` | Argument parsing and subcommand dispatch | T7 | After T7 |
| T9 — Test Infrastructure | `tests/` | Unit tests, integration tests, CI config, fixtures | All | Unit tests per track as built; integration tests after T2 + T3 |

---

## Interface contracts

These are the APIs that must be agreed between the two engineers **before** work
diverges. Each is a narrow, stable boundary. Both engineers write against the agreed
interface — one implements it, the other consumes it.

### Contract A — Data model types
**Owner:** Engineer B defines, Engineer A consumes.

All core structs — `Account`, `Wallet`, `Address`, `TransactionIntent`,
`TransactionResult`, `Deposit`, `Withdrawal`, `Sweep`, `RpcCall`, `MetricSample`,
`ScenarioConfig` — must be defined (at minimum as stubs) before either engineer writes
anything that depends on them. The full spec is in
[`docs/architecture/data-model.md`](docs/architecture/data-model.md).

Key decision to make upfront: which fields are `Option<T>` vs required, and what the
primary identifiers look like (`String` UUIDs vs typed ID newtypes).

### Contract B — RPC client API surface
**Owner:** Engineer A defines, Engineer B consumes.

The RPC client exposes one async Rust function per method. Before Engineer B starts
implementing exchange emulation against it, the function signatures, error type, and
`RpcCall` recording behaviour must be agreed. A stub returning `todo!()` is sufficient
to unblock B.

Minimum to agree:
- The `RpcError` type (wraps JSON-RPC error code + message + component)
- Whether calls return `Result<T, RpcError>` or `Result<T, anyhow::Error>`
- How `RpcCall` is recorded — does the client write it directly to metrics, or return it
  to the caller?
- The async runtime (Tokio is the standard choice for this stack)

### Contract C — Metrics recorder interface
**Owner:** Engineer B defines, Engineer A consumes.

The RPC client needs to record a `RpcCall` entry for every call it makes. Engineer B
owns the metrics module; Engineer A calls into it. A trait with a single method is
sufficient:

```rust
pub trait MetricsRecorder: Send + Sync {
    fn record_rpc_call(&self, call: RpcCall);
    fn record_metric(&self, sample: MetricSample);
}
```

This interface must be agreed before Engineer A starts wiring the RPC client.

---

## Track 1 — Data Model

**Module:** `src/data_model/`

**What it is:** Pure Rust type definitions — no I/O, no network calls, no business
logic. The shared vocabulary that every other module speaks. Must be the first thing
built because everything else imports it.

**Inputs:** The data model specification at
[`docs/architecture/data-model.md`](docs/architecture/data-model.md).

**Output:** A compiled Rust module with `pub struct` definitions for all 12 entities,
with `serde` derive macros for serialisation, and `derive(Debug, Clone)` throughout.

**Key tasks:**
- Implement all 12 types: `Account`, `Wallet`, `Address`, `Balance`,
  `TransactionIntent`, `TransactionResult`, `Deposit`, `Withdrawal`, `Sweep`,
  `RpcCall`, `ScenarioConfig`, `MetricSample`
- All monetary amounts as `u64` zatoshis — no `f64` anywhere
- All timestamps as RFC 3339 strings or `chrono::DateTime<Utc>` — decide one and
  be consistent
- Implement all `enum` types: `AccountStatus`, `ActivityProfile`, `AddressType`,
  `AddressPurpose`, `FlowType`, `TransactionStatus`, `DepositStatus`,
  `WithdrawalStatus`, `SweepStatus`, `Z3Component`
- Add `impl` helpers only where genuinely needed (e.g. `FlowType::is_shielded()`)
- Unit tests: serialise/deserialise roundtrip for every type

**Can start:** Immediately. This is the first task on the project.

---

## Track 2 — Z3 Process Harness

**Module:** `src/z3/`

**What it is:** The layer that treats Zebra, Zaino, and Zallet as processes rather than
RPC endpoints. It spawns them, waits until they are ready, captures their logs, monitors
their resource usage, and tears them down cleanly. Completely independent of the rest
of the simulator — it knows nothing about transactions, accounts, or metrics.

**Inputs:** The integration notes for each component:
[`docs/integration/zebra.md`](docs/integration/zebra.md),
[`docs/integration/zaino.md`](docs/integration/zaino.md),
[`docs/integration/zallet.md`](docs/integration/zallet.md).

**Output:** A `Z3Stack` type that can be created (starts all three processes in order),
polled for health, and dropped (shuts everything down). Integration tests spin up a
`Z3Stack` as a test fixture.

**Key tasks:**
- `zebra::Process` — spawn `zebrad` with a regtest TOML config; wait for `getblockchaininfo`
  to return `"chain": "regtest"` before declaring it ready
- `zaino::Process` — spawn Zaino binary; wait for its RPC/gRPC port to accept connections
- `zallet::Process` — spawn `zallet`; initialise the wallet (sequence TBD, see
  [`docs/integration/zallet.md`](docs/integration/zallet.md)); wait for readiness
- Startup sequencing: Zebra must be healthy before Zaino starts; Zaino before Zallet
- Log capture: pipe each process's stdout/stderr to a `component_logs/<name>.log` file
  in the run output directory
- Health checks: implement a periodic `is_alive()` check for each process that fails
  fast if any component crashes unexpectedly
- Resource sampling: poll CPU % and memory (MB) for each PID on a configurable interval;
  write samples to metrics via the metrics recorder
- Graceful shutdown: SIGTERM then wait, SIGKILL on timeout
- Regtest config generation: produce the Zebra TOML config programmatically from a
  struct, so the test environment is fully reproducible

**Can start:** Immediately. No dependency on the data model or any other track.
Early work (process spawning, log capture) can proceed even before the exact
Zebra/Zaino/Zallet config formats are confirmed.

---

## Track 3 — RPC Client

**Module:** `src/rpc/`

**What it is:** A typed Rust API wrapping every Z3 JSON-RPC method the simulator calls.
Each method is one async function. The client automatically records an `RpcCall` entry
per call (latency, success/failure, method name, component). No business logic here —
this module only knows how to send a request and parse the response.

**Inputs:** The RPC coverage matrix at
[`docs/rpc/rpc-coverage-matrix.md`](docs/rpc/rpc-coverage-matrix.md) and the confirmed
method list in
[`docs/rpc/proposed-method-scope.md`](docs/rpc/proposed-method-scope.md).

**Output:** An `RpcClient` struct that can be configured with an endpoint URL (separate
instances for Zebra, Zaino, and Zallet) and exposes one method per RPC call. Every call
records its own `RpcCall` via the metrics recorder.

**Key tasks:**

*Infrastructure:*
- HTTP client setup using `reqwest` (async, with configurable timeout and retry)
- JSON-RPC request/response envelope types (`JsonRpcRequest`, `JsonRpcResponse<T>`,
  `JsonRpcError`)
- `RpcClient` struct with fields: base URL, component label, metrics recorder handle
- Automatic `RpcCall` construction and recording on every call (start timestamp,
  end timestamp, derived `latency_ms`, `success`, `error_code`, `error_message`)

*Method implementations — Zebra / Zaino:*
- `get_blockchain_info() -> BlockchainInfo`
- `get_block_count() -> u64`
- `get_best_block_hash() -> String`
- `get_block(hash_or_height: BlockRef) -> Block`
- `get_block_hash(height: u64) -> String`
- `get_block_header(hash_or_height: BlockRef) -> BlockHeader`
- `get_raw_transaction(txid: &str, verbose: bool) -> RawTransaction`
- `get_tx_out(txid: &str, index: u32) -> Option<TxOut>`
- `get_address_balance(addresses: &[&str]) -> AddressBalance`
- `get_address_txids(addresses: &[&str], range: Option<HeightRange>) -> Vec<String>`
- `get_address_utxos(addresses: &[&str]) -> Vec<Utxo>`
- `get_raw_mempool() -> Vec<String>`
- `get_mempool_info() -> MempoolInfo`
- `send_raw_transaction(tx_hex: &str) -> String`
- `validate_address(address: &str) -> AddressValidation`
- `z_validate_address(address: &str) -> AddressValidation`
- `generate(num_blocks: u32) -> Vec<String>` *(regtest only)*

*Method implementations — Zallet:*
- `z_get_new_account(name: &str) -> AccountInfo`
- `z_get_address_for_account(account_uuid: &str) -> UnifiedAddress`
- `z_list_accounts() -> Vec<AccountInfo>`
- `z_get_account(account_uuid: &str) -> AccountInfo`
- `list_addresses() -> Vec<AddressEntry>`
- `z_get_balances() -> Balances`
- `z_get_total_balance() -> TotalBalance`
- `z_send_many(from: &str, recipients: &[Recipient]) -> String` *(returns operation ID)*
- `z_get_operation_status(op_ids: &[&str]) -> Vec<OperationStatus>`
- `z_get_operation_result(op_ids: &[&str]) -> Vec<OperationResult>`
- `z_list_operation_ids(status: Option<&str>) -> Vec<String>`
- `z_list_unspent(min_conf: u32, max_conf: u32) -> Vec<UnspentNote>`
- `get_raw_transaction_zallet(txid: &str, verbose: bool) -> RawTransaction` *(Zallet copy)*
- `validate_address_zallet(address: &str) -> AddressValidation`

*Error handling:*
- Distinguish JSON-RPC errors (method not found, invalid params, etc.) from transport
  errors (timeout, connection refused)
- All errors bubble up as `RpcError` with the original `error_code` preserved for the
  compatibility matrix

**Can start:** After Contract A (data model types) and Contract B (API surface) are
agreed. The infrastructure layer can start before the data model is complete if the
engineer stubs out the types.

---

## Track 4 — Synthetic Data Generators

**Module:** `src/synthetic/`

**What it is:** Deterministic, seeded generators for all synthetic data the simulator
produces. Given the same seed from a scenario YAML, the same population of accounts,
wallets, and transaction intents is always generated. This makes any run exactly
reproducible from the scenario config alone.

**Inputs:** Data model types from T1; scenario config parameters (account count, active
fraction, flow type ratios, seed).

**Output:** A `SyntheticPopulation` that contains pre-generated `Account`, `Wallet`, and
`Address` collections; a `TransactionIntentGenerator` that produces `TransactionIntent`
values on demand at the right flow-type distribution.

**Key tasks:**
- RNG setup: wrap a `rand::SeedableRng` (e.g. `StdRng`) seeded from the scenario seed
- `AccountGenerator`: produce N `Account` records with randomised `ActivityProfile`
  distribution; assign `status` according to `active_fraction`
- `WalletGenerator`: for each account, produce a `Wallet` placeholder (real addresses
  come from Zallet via the RPC client once the stack is running)
- `AddressGenerator`: manage the mapping of simulator accounts to real Zallet-derived
  addresses; populate after Zallet provisioning
- `TransactionIntentGenerator`: draw `(sender, recipient, amount, flow_type)` tuples
  according to the scenario's flow-type weights; respects which accounts are active
- Amount generation: draw zatoshi amounts within a configurable range; ensure no
  attempt to send more than the account's known balance
- `generate_fixtures` subcommand output: write a complete synthetic population to JSON
  for development and testing without a live Z3 stack

**Can start:** After T1 (data model types). Can proceed in full parallel with T2 and T3.

---

## Track 5 — Exchange Emulation

**Module:** `src/scenarios/exchange/` (or `src/exchange/` — decide at implementation time)

**What it is:** The business logic of the simulator. Implements the four core exchange
workflows using the RPC client and the data model. Each workflow is a self-contained
async function that takes intents from the synthetic generators and drives them to
completion via Z3, updating the data model state along the way.

**Inputs:** RPC client (T3), data model types (T1), metrics recorder (T6 interface).

**Output:** Four independently callable workflow functions:
`run_deposit`, `run_withdrawal`, `run_sweep`, `run_balance_check`.

**Key tasks:**

*Deposit flow:*
1. Assign a Zallet address to an account (`z_get_address_for_account`)
2. Fund the address in regtest (call `generate` to mine blocks as needed)
3. Poll block data (`get_address_txids`, `get_block`) to detect the incoming transaction
4. Track confirmations: poll `get_block_count` until `required_confirmations` is reached
5. Credit the exchange balance; update `Deposit.status` → `credited`
6. Record `deposit_confirmation_time_ms` to metrics

*Withdrawal flow:*
1. Construct a `z_sendmany` call with the recipient and amount; `fee` = `null`
2. Capture the returned operation ID
3. Poll `z_get_operation_status` until the operation is complete (async ZK proving)
4. On success, retrieve txid via `z_get_operation_result`
5. Poll `get_block_count` to confirm on-chain
6. Record `proving_time_ms` and `broadcast_latency_ms` to metrics

*Sweep flow:*
1. Collect all UTXOs held in deposit addresses (`z_list_unspent` or `get_address_utxos`)
2. Construct a single `z_sendmany` call with all inputs and one output (the hot-wallet)
3. Same async tracking as withdrawal
4. Note: multi-input sweep transactions may have higher fees — record the actual fee from
   the operation result

*Balance check:*
- Call `z_get_total_balance` and `z_get_balances` for a wallet; store result as a
  `Balance` snapshot; record to metrics

*Mempool watcher:*
- Periodic `get_raw_mempool` + `get_mempool_info` polling; emit `mempool_tx_count` and
  `mempool_bytes` metric samples; detect and log saturation events

*Confirmation tracker:*
- Shared utility used by both deposit and withdrawal flows; polls `get_block_count`
  and resolves a future when the required depth is reached

**Can start:** After Contract B (RPC client API surface) is agreed. Exchange emulation
can be written against RPC client stubs from day one — the stubs can return canned
responses for unit tests until the real client is ready.

---

## Track 6 — Observability & Metrics

**Module:** `src/metrics/`

**What it is:** Everything related to recording, computing, and writing the simulator's
output. Completely independent of business logic — it only consumes the data types
defined in T1 and writes them to disk. Other modules call into this module; this module
calls nothing outside itself except the filesystem.

**Inputs:** Data model types from T1 (`RpcCall`, `MetricSample`); run manifest
parameters; file output path.

**Output:** Per-run output files written to `experiments/runs/<run-id>/`:
`rpc_calls.jsonl`, `metrics.jsonl`, `manifest.json`, `summary.md`.
Plus the `MetricsRecorder` trait (Contract C) consumed by T3 and T5.

**Key tasks:**

*JSONL writers:*
- `RpcCallWriter`: append-only, opens `rpc_calls.jsonl` at run start, flushes each
  `RpcCall` immediately (so partial data is preserved if the run crashes)
- `MetricSampleWriter`: append-only, opens `metrics.jsonl` at run start

*Latency computation:*
- Accumulate `latency_ms` values per method+component pair during the run
- Compute P50, P95, P99 percentiles on demand (use a T-digest or a sorted vec)
- Emit periodic `rpc_latency_ms` metric samples with `percentile` label

*Metric sampler:*
- A background task that fires on a configurable interval (e.g. every 5 seconds)
- On each tick, samples: `mempool_tx_count`, `block_height`, `confirmed_txs_total`,
  `failed_txs_total`, `active_accounts`, `tps_achieved`
- Writes each sample as a `MetricSample` to `MetricSampleWriter`

*Manifest generator:*
- Reads `z3-commits.lock` at run start
- Writes `manifest.json` with `run_id`, timestamps, simulator commit (via `git rev-parse
  HEAD`), Z3 commits, scenario name, and `scenario_config_hash`

*Run summary generator:*
- At run end, reads the completed `rpc_calls.jsonl` and `metrics.jsonl`
- Computes aggregate statistics: total txs, confirmed %, failed %, peak mempool, P50/P95/P99
  per method
- Writes `summary.md` using the template in
  [`docs/architecture/observability.md`](docs/architecture/observability.md)

*Run directory management:*
- Derive `run_id` from timestamp + scenario name
- Create `experiments/runs/<run-id>/` at the start of each run
- Copy the scenario YAML into `scenario.yaml`

**Can start:** After T1 (data model types). Can proceed in full parallel with T2 and T3.

---

## Track 7 — Scenario Runner

**Module:** `src/scenarios/runner/` (within `src/scenarios/`)

**What it is:** The orchestration layer that brings everything together. Reads a scenario
YAML config, provisions the synthetic population, schedules transaction intents at the
target TPS, dispatches them to the exchange emulation layer, and collects results. This
is the convergence point of all other tracks.

**Inputs:** T2 (Z3 harness), T3 (RPC client), T4 (synthetic generators), T5 (exchange
emulation), T6 (metrics recorder). Reads scenario configs from `configs/scenarios/`.

**Output:** A `run(scenario: ScenarioConfig) -> RunResult` function that fully executes
one scenario end-to-end.

**Key tasks:**

*Scenario config:*
- YAML deserialization into `ScenarioConfig` (defined in T1)
- Config validation: flow fractions must sum to 1.0; `active_fraction` ∈ (0, 1]; TPS
  target > 0; etc.
- Compute and record `config_hash` for the manifest

*Account provisioning:*
- Call `z_get_new_account` for each synthetic account via the RPC client
- Call `z_get_address_for_account` to derive at least one deposit address per account
- Populate the in-memory `SyntheticPopulation` with real Zallet-derived addresses

*TPS scheduler:*
- A rate limiter that controls how many transaction intents are dispatched per second
- For steady-state: constant rate
- For ramp: linearly increasing rate from 0 to `target_tps` over `ramp_duration`
- For burst: spike to N× `target_tps` for a short window, then return to baseline
- For mixed: steady rate with transaction type distribution biased towards shielded

*Run phases:*
1. **Setup** — start Z3 stack (T2), initialise Zallet wallet, provision accounts
2. **Warmup** — mine a few regtest blocks to fund hot-wallet; verify RPC connectivity
3. **Load** — run the TPS scheduler for `load_duration_seconds`; dispatch intents to
   exchange emulation (T5) as concurrent async tasks; collect results
4. **Teardown** — wait for all in-flight operations to complete or time out; shut down
   Z3 stack; flush all output files; generate manifest and summary

*Concurrency model:*
- Each dispatched transaction intent becomes an async task on the Tokio runtime
- Bound the number of concurrent in-flight transactions (configurable backpressure
  limit) to prevent the simulator itself from becoming the bottleneck
- Collect task results into an aggregate `RunResult`

*Dry-run mode:*
- Parse and validate the scenario config; provision synthetic population; print a
  summary of what would be executed; do not start Z3 or issue any RPC calls

**Can start:** After T5 (exchange emulation) is functional enough to run one transaction
end-to-end. Scaffold and config parsing can start earlier.

---

## Track 8 — CLI

**Module:** `src/cli/`, `src/main.rs`

**What it is:** The entry point. Argument parsing and subcommand dispatch. Thin layer —
no business logic lives here.

**Inputs:** All other tracks (indirectly, via the scenario runner).

**Subcommands to implement:**

| Subcommand | Description |
|---|---|
| `z3sim run --scenario <path>` | Execute a scenario end-to-end |
| `z3sim run --scenario <path> --dry-run` | Validate config without starting Z3 |
| `z3sim generate-fixtures --scenario <path> --out <dir>` | Dump synthetic population to JSON |
| `z3sim validate-scenario <path>` | Parse and validate a scenario YAML, exit with error if invalid |

**Key tasks:**
- Argument parsing using `clap` (derive-style, idiomatic Rust CLI)
- Load and validate scenario YAML before dispatching
- Wire up logging (`tracing` crate) with configurable verbosity (`--verbose` / `--quiet`)
- Handle Ctrl-C gracefully (SIGINT): trigger teardown, flush output, exit cleanly
- Print run ID and output directory path on start so the operator knows where to look

**Can start:** Scaffold (`clap` setup, subcommand structure) can start at any time.
Full wiring requires T7 to be functional.

---

## Track 9 — Test Infrastructure & CI

**Module:** `tests/`

**What it is:** Two categories of tests with different requirements:

*Unit tests* live alongside each module (`src/<module>/tests.rs` or inline). They test
individual functions in isolation with no external dependencies. Each engineer writes
unit tests for the tracks they own.

*Integration tests* (`tests/integration/`) require a live Z3 regtest stack. They use T2
(Z3 harness) to spin up the full stack in a `setup` fixture, run a minimal scenario,
and assert on the output files.

**Key tasks:**

*Unit tests (per track):*
- T1: serialise/deserialise roundtrip for all types
- T3: mock HTTP server returning canned JSON-RPC responses; assert each method parses
  correctly and records the right `RpcCall`
- T4: assert determinism (same seed → same output); assert flow-type distribution
  matches configured ratios over N samples
- T5: test each workflow against a mock RPC client
- T6: write a batch of `RpcCall`s; read back `rpc_calls.jsonl`; assert content; test
  P99 latency computation

*Integration tests:*
- `test_smoke_scenario`: run the `smoke.yaml` scenario against a live Z3 stack; assert
  `manifest.json` is written; assert `rpc_calls.jsonl` contains at least one successful
  call per expected method; assert `summary.md` exists
- `test_deposit_flow`: fund a regtest address; run the deposit workflow; assert
  `Deposit.status` reaches `credited`
- `test_withdrawal_flow`: fund an account; execute a withdrawal; assert txid is recorded
  and confirmed on-chain
- `test_rpc_coverage`: call every method in the proposed method list; record pass/fail;
  this is the basis of the compatibility matrix

*CI:*
- GitHub Actions workflow: on every PR, run `cargo test --lib` (unit tests only, no Z3
  required), `cargo clippy -- -D warnings`, `cargo fmt --check`
- Separate CI job (longer, manual or nightly trigger): full integration test suite
  requiring Z3 processes — runs only when explicitly triggered to avoid requiring
  Z3 binaries in every PR

**Can start:** Unit tests are written alongside each track from day one. Integration
test scaffolding (harness setup/teardown fixtures) can start as soon as T2 is
functional.

---

## Dependency map

```
T1 (Data Model) ─────────────────────────────┐
                                              │
T2 (Z3 Harness) ──────────────────┐          │
                                  │          │
T3 (RPC Client) ──────────────────┤          │
        depends on T1             │          │
                                  ▼          ▼
T4 (Synthetic) ──────────► T5 (Exchange Emulation)
       depends on T1              depends on T1, T3
                                  │
T6 (Observability) ───────────────┤
       depends on T1              │
                                  ▼
                         T7 (Scenario Runner)
                         depends on T2, T3, T4, T5, T6
                                  │
                                  ▼
                             T8 (CLI)
                         depends on T7
```

---

## Suggested allocation

### Engineer A — Integration & Infrastructure

**Owns:** T2 (Z3 Process Harness), T3 (RPC Client), T8 (CLI), integration tests (T9).

**Rationale:** These tracks require the most interaction with external systems (OS
processes, HTTP, Z3 binaries) and benefit from deep familiarity with the actual Z3
RPC surface. Once T2 and T3 are stable, Engineer A moves to T8 and helps with T7.

**Sequence:**
1. T2 (start immediately): Get Zebra running in regtest; verify one `getblockchaininfo`
   call works
2. T3 (after Contract A + B agreed): Implement all Zebra/Zaino methods first (simpler);
   then Zallet methods
3. T9 integration scaffolding: Z3 test fixture once T2 is stable
4. T7 + T8: final assembly once T5 is functional

### Engineer B — Simulation Core

**Owns:** T1 (Data Model), T4 (Synthetic Generators), T5 (Exchange Emulation),
T6 (Observability), unit tests for each (T9).

**Rationale:** These tracks are pure Rust business logic that can be fully built and
unit-tested without any live Z3 processes. Engineer B can build against mock RPC stubs
and verify all logic independently until T3 is ready.

**Sequence:**
1. T1 (start immediately): Define all types; this is the first thing anyone needs
2. T6 (in parallel with T1): Scaffold JSONL writers; implement metrics recorder interface
   (Contract C)
3. T4 (after T1): Synthetic generators; write unit tests proving determinism
4. T5 (after Contract B agreed): Exchange emulation against RPC stubs; integrate with T6
5. T7: Help with scenario runner once both T4 and T5 are functional

---

## Integration milestones

Three points where the two tracks must converge and be tested together:

### Milestone 1 — Smoke test (target: end of Week 2)
**What:** Engineer A's `get_blockchain_info()` call works against a live Zebra instance
managed by T2. Engineer B's `RpcCallWriter` records the call to `rpc_calls.jsonl`.

**Success criteria:**
- Zebra starts in regtest via the T2 harness
- `get_blockchain_info()` returns `chain: "regtest"` and latency_ms is recorded
- `rpc_calls.jsonl` contains one correct entry
- Both engineers can reproduce this locally

### Milestone 2 — First end-to-end transaction (target: end of Week 4)
**What:** A single synthetic deposit → confirm → withdrawal cycle completes using the
full stack, with all four output files written correctly.

**Success criteria:**
- Z3 stack (Zebra + Zaino + Zallet) starts via T2
- T4 generates a synthetic account; T3 provisions it in Zallet via `z_getnewaccount`
- A deposit is funded, detected, confirmed, and credited (T5 deposit flow)
- A withdrawal is constructed, broadcast, proved (async), and confirmed (T5 withdrawal flow)
- `rpc_calls.jsonl`, `metrics.jsonl`, `manifest.json`, `summary.md` all written
- The `smoke` scenario runs without error

### Milestone 3 — Full load run (target: end of Week 6)
**What:** The scenario runner (T7) drives the full scenario library at target TPS with
thousands of accounts.

**Success criteria:**
- All four scenario types run to completion (steady-state, ramp, burst, mixed)
- Thousands of accounts provisioned and active
- Latency histograms and mempool data collected and correct
- No memory leaks or panics under sustained load
- Results are reproducible: two runs with the same seed produce the same `manifest.json`
  content

---

## Notes for implementation planning

**Start with T1 and T2 in parallel — they are completely independent and block
everything else.** The first PR from each engineer should be these two tracks.

**Agree Contracts A, B, and C in a short design session before day two.** Write them
down as Rust traits/signatures in a shared document or as stub code in a `draft/` branch.
Disagreements here are cheap to resolve now and expensive to resolve after both tracks
have diverged.

**Build against mocks early.** Engineer B should not wait for Engineer A's RPC client
before writing exchange emulation. A `MockRpcClient` returning `Ok(canned_response)`
lets T5 be fully unit-tested in isolation. The real client is a drop-in replacement at
Milestone 1.

**The scenario runner (T7) is the hardest track.** It integrates all other tracks and
owns the concurrency model. Treat Milestone 2 as its MVP. Defer multi-scenario
orchestration, ramp/burst shaping, and backpressure logic to Phase 2.
