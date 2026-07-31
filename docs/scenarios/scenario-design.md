# Scenario Library Design

## Purpose

A scenario defines a complete, reproducible workload for the simulator: account count,
load shape, transaction type mix, confirmation requirements, and observability settings.
Scenarios are YAML files in `configs/scenarios/` and are the primary CLI input.

## Scenario file structure

| Field | Type | Purpose |
|---|---|---|
| `name` | string | Short identifier used in output filenames and logs |
| `description` | string | Human-readable description of what this scenario tests |
| `seed` | integer | RNG seed for deterministic account and transaction generation |
| `accounts_count` | integer | Number of synthetic exchange accounts to provision |
| `accounts_active_fraction` | float 0–1 | Fraction of accounts that generate activity during the run |
| `load_duration_seconds` | integer | Duration of the load phase |
| `load_target_tps` | float | Target transactions per second |
| `flows.transparent_to_transparent` | float 0–1 | Fraction of transactions that are T→T |
| `flows.transparent_to_shielded` | float 0–1 | Fraction of transactions that are T→Z |
| `flows.shielded_to_transparent` | float 0–1 | Fraction of transactions that are Z→T |
| `flows.shielded_to_shielded` | float 0–1 | Fraction of transactions that are Z→Z |
| `confirmations_deposit_required` | integer | Block depth required to credit a deposit |
| `observability.record_rpc_calls` | bool | Whether to write a per-call RPC log |
| `observability.record_component_logs` | bool | Whether to capture Z3 component process logs |
| `observability.metric_sampling_interval_secs` | integer | Metric snapshot interval in seconds |
| `observability.mempool_saturation_threshold` | integer | Pending tx count that triggers a saturation event in the log |

None of the fields above have a parser-level default (no `#[serde(default)]` in
`ScenarioConfig`) — every scenario YAML must set them explicitly. Flow fractions must
sum to 1.0.

## Scenarios

| Name | Load shape | Accounts | Transaction mix | Notes |
|---|---|---|---|---|
| `smoke` | Minimal (1 TPS, 60 s) | 10 | 100% T→T | CI sanity check |
| `steady_state` | Constant TPS (5.0, 300 s) | 100 (50 active) | 100% T→T | Baseline exchange behavior |
| `ramp` | Linearly increasing TPS (0→10, 300 s) | 100 (50 active) | 100% T→T | Find the inflection point |
| `burst` | Spike then recovery (base 3.0 TPS, 300 s) | 50 (40 active) | 100% T→T | Model sudden volume events; burst multiplier is a CLI flag |
| `mixed` | Steady (2.0 TPS, 300 s) | 50 (all active) | 50% T→Z, 50% Z→Z | Exercises the full shielded RPC surface (ZK proving) |
| `flow_check_ttoz` | Minimal (1 TPS, 60 s) | 10 (5 active) | 100% T→Z | Isolates the T→Z deposit path for a live pass/fail signal |
| `flow_check_ztot` | Minimal (1 TPS, 60 s) | 10 (5 active) | 100% Z→T | Isolates the Z→T sweep-then-withdrawal path |
| `flow_check_ztoz` | Minimal (1 TPS, 60 s) | 10 (5 active) | 100% Z→Z | Isolates the Z→Z deposit path |
| `reorg` (planned) | Regtest-control | N/A | N/A | Mine a branch, `invalidateblock`, `reconsiderblock`; verify rollback/restore (`src/scenarios/regtest_control.rs` has the logic; no `configs/scenarios/reorg.yaml` is shipped yet) |

The account counts and TPS targets above for `steady_state`/`ramp`/`burst`/`mixed` are
initial values, not yet calibrated against a load-curve run at exchange scale — none of
the four exceed 100 accounts, well short of the "thousands" design ceiling in
`docs/scope.md`. The `flow_check_*` scenarios exist purely to give each of the four flow
types (T→T, T→Z, Z→T, Z→Z) independent live pass/fail coverage at minimal scale, since
`smoke`/`steady_state`/`ramp`/`burst` are all 100% T→T and only `mixed` touches any
shielded flow.

The `reorg` scenario uses the regtest-control methods (`generate`, `invalidateblock`,
`reconsiderblock`); it exercises chain manipulation rather than transaction load, so it
carries no TPS/account targets and is excluded from the stress latency histograms.

## Zaino coverage

Scenarios that exercise indexer reads additionally drive Zaino's zcashd-style JSON-RPC
mirror (regtest `:28237`) through a dedicated client, recording latency against the
`Zaino` backend. See [`docs/integration/zaino.md`](../integration/zaino.md).

## Reproducibility

Each run records the scenario config hash in its manifest. A run is reproducible if:
- the scenario YAML is unchanged,
- the simulator commit is unchanged,
- the Z3 component commits in `z3-commits.lock` are unchanged.

