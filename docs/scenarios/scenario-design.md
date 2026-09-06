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
| `activity_profiles.low_fraction` | float 0–1 | Fraction of active accounts assigned a low activity level |
| `activity_profiles.medium_fraction` | float 0–1 | Fraction of active accounts assigned a medium activity level |
| `activity_profiles.high_fraction` | float 0–1 | Fraction of active accounts assigned a high activity level |
| `amounts.min_zatoshis` | integer | Minimum transaction amount, in zatoshis |
| `amounts.max_zatoshis` | integer | Maximum transaction amount, in zatoshis (must be ≥ `min_zatoshis`) |
| `confirmations_deposit_required` | integer | Block depth required to credit a deposit |
| `warmup_blocks` | integer | Blocks mined before the load phase starts (e.g. for coinbase maturity). Default: `10` if omitted — but every shipped scenario sets this explicitly (`20` or `110`) |
| `observability.record_rpc_calls` | bool | Whether to write a per-call RPC log |
| `observability.record_component_logs` | bool | Whether to capture Z3 component process logs |
| `observability.metric_sampling_interval_secs` | integer | Metric snapshot interval in seconds |
| `observability.mempool_saturation_threshold` | integer | Pending tx count that triggers a saturation event in the log |
| `expectations.min_confirmed` | integer | Minimum confirmed transactions required to pass |
| `expectations.max_terminal_failures` | integer | Maximum terminal (non-retry) transaction failures tolerated, per failed intent — never per RPC retry attempt |
| `expectations.max_timeouts` | integer | Maximum timeouts tolerated |
| `expectations.allowed_error_classes` | list of string | Failure classes (`insufficient_balance`, `mempool_conflict`, `timeout`, `other`) pre-approved as not counting toward `max_terminal_failures`. Optional — defaults to an empty list if omitted |

`flows.*` values must sum to 1.0. `activity_profiles.*` values must also sum to 1.0.
Every field above is required (parse error if omitted) except `warmup_blocks`, which
defaults to `10`, and `expectations.allowed_error_classes`, which defaults to an empty
list. The `expectations` block itself is mandatory — a scenario file that omits it fails
to parse, rather than silently running with no pass/fail criterion. The YAML parser does
not reject unrecognized keys — a typo'd field name is silently ignored rather than
raising an error, so always confirm a new or edited scenario with `make validate-scenario
SCENARIO=<path>` (or `make scenario-dry-run`, which also prints the parsed
account/duration/TPS values) rather than eyeballing the file.

A run's pass/fail result (`z3sim run`'s "Result: PASS/FAIL" line, exit code `2` on
failure) is computed by comparing the run's actual `confirmed`/`timed_out` counts and
per-class terminal-failure breakdown against this block — see
`scenarios::runner::result::RunStats::evaluate`.

### Validation rules

Beyond parsing, `validate_scenario` additionally enforces:

- `load_target_tps > 0.0`, `load_duration_seconds > 0`
- `accounts_count >= 1`, `accounts_active_fraction` in `(0.0, 1.0]`
- `floor(accounts_count × accounts_active_fraction) >= 2` (at least 2 active accounts)
- `confirmations_deposit_required >= 1`
- `amounts.min_zatoshis <= amounts.max_zatoshis`

All violations are collected and reported together, not just the first one.

## Planned scenarios

| Name | Load shape | Accounts | Transaction mix | Notes |
|---|---|---|---|---|
| `smoke` | Minimal (1 TPS, 60 s) | 10 | 100% transparent | CI sanity check |
| `steady-state` | Constant TPS (5 TPS, 300 s) | 100 | 100% transparent | Baseline exchange behavior |
| `ramp` | Linearly increasing TPS (ceiling 10 TPS, 300 s) | 100 | 100% transparent | Find inflection point |
| `burst` | Spike then recovery (base 3 TPS, 300 s) | 50 | 100% transparent | Model sudden volume events |
| `mixed` | Steady with shielded mix (2 TPS, 300 s) | 50 | 50% T→Z, 50% Z→Z | Exercise full shielded RPC surface |
| `reorg` | Regtest-control | N/A | N/A | Mine a branch, `invalidateblock`, `reconsiderblock`; verify rollback/restore. `run_reorg` is implemented and unit-tested in `src/scenarios/regtest_control.rs`, but has no scenario YAML or CLI wiring yet — not runnable as a scenario. |

Account counts and TPS targets for `steady-state`, `ramp`, `burst`, and `mixed` have
been calibrated from initial load runs (see `configs/scenarios/*.yaml` for exact
values); further tuning may follow as more runs accumulate.

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

