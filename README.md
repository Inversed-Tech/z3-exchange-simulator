# Z3 Exchange Simulator

Open-source simulator for stress-testing the Z3 Zcash stack — **Zebra**, **Zaino**, and
**Zallet** — under realistic exchange-scale load.

Built for the [Zcash Foundation](https://zfnd.org) by [Inversed](https://inversed.tech).
All testing runs in regtest mode with synthetically generated data.

---

## What this project is

The Z3 Exchange Simulator models how a cryptocurrency exchange might operate on the
next-generation Zcash wallet stack:

- Thousands of synthetic exchange accounts with configurable activity profiles
- Transparent and shielded Zcash transaction flows
- Deposits, withdrawals, hot-wallet sweeps, and balance tracking
- Full RPC coverage of the Z3 stack against the RFP method list
- Configurable load scenarios: steady-state, ramp, burst, and mixed transaction types

The simulator measures where the Z3 stack starts to degrade, records the results with
per-RPC latency histograms and failure traces, and produces a findings report tied to
pinned commits of each component.

## What this project is not

| | |
|---|---|
| **Not a security audit** | Security auditing of Z3 is out of scope. A dedicated audit partner handles that separately. |
| **Not a mainnet tool** | All testing runs in regtest (local, isolated chain). No real funds are involved at any point. |
| **Not a real exchange** | All accounts, addresses, and transaction data are synthetically generated. No real user data. |
| **Not a production deployment** | This is a load-testing and measurement tool. |

## Z3 components

The simulator drives three components that together make up the next-generation Zcash wallet
stack:

| Component | Role | Repository |
|---|---|---|
| **Zebra** | Zcash full node — validates and maintains the local chain state | https://github.com/ZcashFoundation/zebra |
| **Zaino** | Blockchain data access, indexing, and RPC passthrough layer | https://github.com/zingolabs/zaino |
| **Zallet** | Wallet — address generation, signing, transaction broadcasting | https://github.com/zcash/wallet |

See [`docs/architecture/z3-overview.md`](docs/architecture/z3-overview.md) for a plain-English
explanation of how these fit together and how the simulator interacts with them.

---

## Repository structure

```
z3-exchange-simulator/
  README.md                          This file
  Cargo.toml                         Rust package manifest
  Makefile                           Development command surface
  z3-commits.lock                    Pinned Z3 component commits
  LICENSE

  docs/
    architecture/
      z3-overview.md                 Plain-English Z3 stack overview
      data-model.md                  Simulator data model
      observability.md               Metrics and experiment output structure
    integration/
      zebra.md                       Zebra build, run, and config notes
      zaino.md                       Zaino build, run, and config notes
      zallet.md                      Zallet build, run, and config notes
      pinned-commits.md              Commit pinning rationale and process
    rpc/rpc-coverage-matrix.md       RPC method coverage and zcashd parity matrix
    scenarios/scenario-design.md     Scenario library design

  configs/
    scenarios/                       Scenario YAML files
    local/                           Local environment overrides (gitignored)

  scripts/
    dev/clone-z3.sh                  Clone and check out pinned Z3 repositories
    experiments/                     Experiment automation scripts

  src/                               Simulator source code (Rust)
    main.rs                          CLI binary entry point
    lib.rs                           Library root and module declarations
    cli/                             Argument parsing and subcommand dispatch
    rpc/                             RPC client — calls Z3, records latency and errors
    z3/                              Z3 component lifecycle and process management
    data_model/                      Core data types
    synthetic/                       Synthetic account and transaction generators
    scenarios/                       Scenario runner and workload shaping
    metrics/                         Metrics collection, histograms, and output

  tests/
    unit/                            Unit tests
    integration/                     Integration tests against a live Z3 regtest stack
    fixtures/                        Static test fixtures and scenario snapshots

  experiments/
    runs/                            Experiment output directories (gitignored)
```

---

## Quickstart

> The simulator is not yet fully implemented. These commands will work once the regtest
> harness and RPC client are in place.

```sh
# Clone pinned Z3 component repositories
make clone-z3

# Build the simulator binary
make build

# Run the smoke scenario (requires a running Z3 regtest stack)
./target/debug/z3sim run --scenario configs/scenarios/smoke.yaml
```

## Pinned component commits

All benchmark runs reference the commits in [`z3-commits.lock`](z3-commits.lock). Findings
in the report are only valid for these specific component versions.

| Component | Pinned commit |
|---|---|
| Zebra | `d4cd662c716382f6397d2a730148025a1ca79fec` |
| Zaino | `4ddbfd29c9f0e74f20b4d5bf81f51042aae4302a` |
| Zallet | `05926f3f3ec1b1d90348ae899628cc0e28547ef3` |

## Development commands

```sh
make help                # List all available commands
make setup               # Check local development dependencies
make clone-z3            # Clone pinned Z3 repositories
make build               # Build the simulator binary (debug)
make build-release       # Build an optimized release binary
make test                # Run all tests
make fmt                 # Format source code
make lint                # Run clippy lints
make generate-fixtures   # Generate synthetic fixture data
make scenario-dry-run    # Validate a scenario config without issuing RPC calls
make clean               # Remove build artifacts
```

## Scenario configuration

Scenarios are YAML files under `configs/scenarios/`. Each specifies account count,
activity distribution, load shape, transaction type ratios, confirmation requirements,
and observability flags.

See [`configs/scenarios/smoke.yaml`](configs/scenarios/smoke.yaml) for a minimal example
and [`docs/scenarios/scenario-design.md`](docs/scenarios/scenario-design.md) for the full
design documentation.

## Synthetic data

All account and transaction data is generated synthetically by the harness. No real user
data, real private keys, or real funds appear anywhere in this repository. Generators are
seeded for deterministic, reproducible runs.

## RPC coverage

See [`docs/rpc/rpc-coverage-matrix.md`](docs/rpc/rpc-coverage-matrix.md) for the method
coverage matrix, including which component serves each method, zcashd behavioral parity
status, and transparent/shielded coverage.

## Experiment outputs

Each simulator run produces a timestamped output directory:

```
experiments/runs/<run-id>/
  manifest.json        Simulator commit, Z3 commits, scenario hash, run timestamp
  scenario.yaml        Exact scenario config used
  rpc_calls.jsonl      Per-call log: method, component, latency_ms, status
  metrics.jsonl        Time-series metric samples
  component_logs/      Captured Zebra, Zaino, and Zallet process logs
  summary.md           Human-readable run summary
```

Run directories are gitignored. Significant results should be published externally or
archived manually.

---

## Documentation index

| Document | Purpose |
|---|---|
| [`docs/architecture/z3-overview.md`](docs/architecture/z3-overview.md) | Plain-English Z3 stack overview for new contributors |
| [`docs/architecture/data-model.md`](docs/architecture/data-model.md) | Core data model: accounts, transactions, metrics |
| [`docs/architecture/observability.md`](docs/architecture/observability.md) | Observability plan: metrics, latency, output format |
| [`docs/integration/zebra.md`](docs/integration/zebra.md) | Zebra integration notes |
| [`docs/integration/zaino.md`](docs/integration/zaino.md) | Zaino integration notes |
| [`docs/integration/zallet.md`](docs/integration/zallet.md) | Zallet integration notes |
| [`docs/integration/pinned-commits.md`](docs/integration/pinned-commits.md) | Commit pinning rationale and update process |
| [`docs/rpc/rpc-coverage-matrix.md`](docs/rpc/rpc-coverage-matrix.md) | RPC coverage and zcashd parity matrix |
| [`docs/rpc/proposed-method-scope.md`](docs/rpc/proposed-method-scope.md) | Proposed method list for Foundation confirmation |
| [`docs/scenarios/scenario-design.md`](docs/scenarios/scenario-design.md) | Scenario library design |

---

*Built by [Inversed](https://inversed.tech) for the [Zcash Foundation](https://zfnd.org).*
