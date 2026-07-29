# Experiment Scripts

Scripts for running, archiving, and analyzing simulator experiments.

## Probe scripts

Step-by-step probes against a running regtest stack. Each prints an OK/FAIL verdict per
operation, so one run maps exactly which stack behaviours work on the pins under test.
Evidence and conclusions live in [`docs/regtest-funding-plan.md`](../../docs/regtest-funding-plan.md).

| Script | Purpose |
|---|---|
| `funding-probe.sh <pool>` | Probe one coinbase pool (transparent / sapling / orchard) through three layers: Zebra mines it → Zallet detects it → `z_sendmany` spends it |
| `fanout-probe.sh` | The full exchange-shaped funding flow: mine to one generated address → shield → fan out to N accounts → a sink spends back (12 steps) |

Both mutate `ZEBRA_MINING__MINER_ADDRESS` in `external/z3/.env.regtest` (restarting Zebra)
but never wipe chain or wallet state.

## Planned scripts

| Script | Purpose |
|---|---|
| `run-scenario.sh` | Run a single named scenario and archive its output |
| `run-suite.sh` | Run a full suite of scenarios sequentially |
| `collect-results.sh` | Aggregate metrics across multiple runs |

## Experiment output

Each run produces a timestamped directory under `experiments/runs/`. Run directories are
gitignored. See [`docs/architecture/observability.md`](../../docs/architecture/observability.md)
for the full output structure.
