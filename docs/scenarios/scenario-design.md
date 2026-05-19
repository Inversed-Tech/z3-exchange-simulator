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
| `accounts.count` | integer | Number of synthetic exchange accounts to provision |
| `accounts.active_fraction` | float 0–1 | Fraction of accounts that generate activity during the run |
| `load.duration_seconds` | integer | Duration of the load phase |
| `load.target_tps` | float | Target transactions per second |
| `flows.transparent_to_transparent` | float 0–1 | Fraction of transactions that are T→T |
| `flows.transparent_to_shielded` | float 0–1 | Fraction of transactions that are T→Z |
| `flows.shielded_to_transparent` | float 0–1 | Fraction of transactions that are Z→T |
| `flows.shielded_to_shielded` | float 0–1 | Fraction of transactions that are Z→Z |
| `confirmations.deposit_required` | integer | Block depth required to credit a deposit (default: 10 if omitted) |
| `observability.record_rpc_calls` | bool | Whether to write a per-call RPC log |
| `observability.record_component_logs` | bool | Whether to capture Z3 component process logs |
| `observability.metric_sampling_interval_secs` | integer | Metric snapshot interval in seconds (default: 5) |
| `observability.mempool_saturation_threshold` | integer | Pending tx count that triggers a saturation event in the log (default: 500) |

Flow fractions must sum to 1.0.

## Planned scenarios

| Name | Load shape | Accounts | Transaction mix | Notes |
|---|---|---|---|---|
| `smoke` | Minimal (1 TPS, 60 s) | 10 | 100% transparent | CI sanity check |
| `steady-state` | Constant TPS | TBD | TBD | Baseline exchange behavior |
| `ramp` | Linearly increasing TPS | TBD | TBD | Find inflection point |
| `burst` | Spike then recovery | TBD | TBD | Model sudden volume events |
| `mixed` | Steady with shielded mix | TBD | TBD | Exercise full shielded RPC surface |

Exact account counts and TPS targets will be calibrated based on Phase 1 findings.

## Reproducibility

Each run records the scenario config hash in its manifest. A run is reproducible if:
- the scenario YAML is unchanged,
- the simulator commit is unchanged,
- the Z3 component commits in `z3-commits.lock` are unchanged.

## Open questions

- What is the minimum target TPS for the steady-state scenario?
- At what account count does the simulator itself become the bottleneck rather than Z3?
