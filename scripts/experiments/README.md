# Experiment Scripts

Scripts for running, archiving, and analyzing simulator experiments.

> Placeholder — experiment automation scripts will be added during Phase 2 (Weeks 5–8).

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
