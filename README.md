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

The simulator drives the Z3 Docker Compose stack — a unified deployment of three
components plus an RPC Router:

| Component | Role | Repository |
|---|---|---|
| **Z3 stack** | Docker Compose orchestration — the primary testing target | https://github.com/ZcashFoundation/z3 |
| **Zebra** | Zcash full node — validates and maintains the local chain state | https://github.com/ZcashFoundation/zebra |
| **Zaino** | Blockchain indexing — covered via its zcashd-style JSON-RPC mirror (`:28237`); lightwalletd gRPC (`:28137`) documented, out of scope | https://github.com/zingolabs/zaino |
| **Zallet** | Wallet — address generation, signing, transaction broadcasting | https://github.com/zcash/wallet |
| **RPC Router** | Single JSON-RPC endpoint (`:8181`) routing calls to Zebra or Zallet | Part of Z3 repo |

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
      z3.md                          Z3 Docker Compose stack — primary integration reference
      zebra.md                       Zebra standalone build and config notes
      zaino.md                       Zaino role and standalone config notes
      zallet.md                      Zallet standalone build and config notes
      pinned-commits.md              Commit pinning rationale
    rpc/rpc-coverage-matrix.md       RPC method coverage and zcashd parity matrix
    rpc/method-scope.md              Confirmed method list: stress vs smoke split
    scenarios/scenario-design.md     Scenario library design

  configs/
    scenarios/                       Scenario YAML files
    local/                           Local environment overrides (gitignored)

  scripts/
    dev/clone-z3.sh                  Clone the pinned Z3 Docker Compose repository
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

```sh
# Clone the Z3 Docker Compose stack at the pinned commit
make clone-z3

# Start the Z3 regtest stack (one-time init, then up)
cd external/z3 && ./scripts/regtest-init.sh
docker compose --env-file .env.regtest up -d

# Build the simulator binary
make build

# Run the smoke scenario
./target/debug/z3sim run --scenario configs/scenarios/smoke.yaml
```

## Pinned commits

All benchmark runs reference the commits in [`z3-commits.lock`](z3-commits.lock).
The primary pin is the Z3 meta-repository; component versions are determined by the
images that Z3 commit pins. All pins are confirmed and frozen for the engagement —
see [`docs/integration/pinned-commits.md`](docs/integration/pinned-commits.md).

| Entity | Version / image | Pinned commit |
|---|---|---|
| Z3 stack | `main` | `dfb9d0eae6d3f67a3c184e0d8fcb1166e7740724` |
| Zebra | `v5.0.0` (`zfnd/zebra:5.0.0`) | `1e6519ea91e2d3035c20aadd4d9a40dcac2eed3a` |
| Zaino | `0.4.0-rc.2` (`zingodevops/zainod:0.4.0-rc.2`) | `0cf4fd5008a7536e3495e3e377073faac1cb28f3` |
| Zallet | `v0.1.0-alpha.3` (`electriccoinco/zallet:v0.1.0-alpha.3`) | `6fc85f68cf5ebe456160c6518255a83129e7d21c` |

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
  rpc_calls.jsonl      Per-call log: method, backend, latency_ms, status
  metrics.jsonl        Time-series metric samples
  component_logs/      Captured Zebra, Zaino, and Zallet process logs
  summary.md           Human-readable run summary
```

Run directories are gitignored and are not tracked by version control.

---

## Documentation index

| Document | Purpose |
|---|---|
| [`docs/scope.md`](docs/scope.md) | Project objectives, scope, deliverables, timeline, and constraints |
| [`docs/architecture/z3-overview.md`](docs/architecture/z3-overview.md) | Plain-English Z3 stack overview for new contributors |
| [`docs/architecture/data-model.md`](docs/architecture/data-model.md) | Core data model: accounts, transactions, metrics |
| [`docs/architecture/observability.md`](docs/architecture/observability.md) | Observability plan: metrics, latency, output format |
| [`docs/integration/z3.md`](docs/integration/z3.md) | Z3 Docker Compose stack — primary integration reference |
| [`docs/integration/zebra.md`](docs/integration/zebra.md) | Zebra standalone build and config notes |
| [`docs/integration/zaino.md`](docs/integration/zaino.md) | Zaino role and standalone config notes |
| [`docs/integration/zallet.md`](docs/integration/zallet.md) | Zallet standalone build and config notes |
| [`docs/integration/pinned-commits.md`](docs/integration/pinned-commits.md) | Commit pinning rationale |
| [`docs/rpc/rpc-coverage-matrix.md`](docs/rpc/rpc-coverage-matrix.md) | RPC coverage and zcashd parity matrix |
| [`docs/rpc/method-scope.md`](docs/rpc/method-scope.md) | Confirmed method list: stress vs smoke split |
| [`docs/rpc/proposed-method-scope.md`](docs/rpc/proposed-method-scope.md) | Proposed method list for Foundation confirmation |
| [`docs/scenarios/scenario-design.md`](docs/scenarios/scenario-design.md) | Scenario library design |
| [`docs/regtest-funding-plan.md`](docs/regtest-funding-plan.md) | How regtest funding works, the Zallet beta.1 bump, and the shielding step |

---

*Built by [Inversed](https://inversed.tech) for the [Zcash Foundation](https://zfnd.org).*
