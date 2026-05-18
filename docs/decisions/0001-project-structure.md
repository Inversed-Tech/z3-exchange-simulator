# ADR 0001: Project Structure and Tooling

Date: 2026-05-18
Status: Accepted

## Context

Starting a new Rust-based simulator project for the Zcash Foundation. Initial decisions
about repository structure, build tooling, and source layout are needed before development
begins.

Key constraints:
- The simulator must be open-source and reusable by the Foundation and the Zebra, Zaino,
  and Zallet teams.
- Implementation language is Rust — matching the Z3 stack, which makes future interop
  and type-sharing between the simulator and Z3 components more tractable.
- The project involves a CLI tool, an RPC client, scenario configuration, synthetic data
  generation, and metrics collection.
- A two-engineer team will collaborate via a shared public GitHub repository.

## Decisions

### 1. Repository structure

A conventional flat layout with documentation and scripts alongside source code:

- `src/` — Rust source (library + binary crate)
- `docs/` — documentation, organized by concern
- `configs/` — scenario YAML files and local environment overrides
- `scripts/` — development and experiment automation shell scripts
- `tests/` — tests kept separate from inline Rust unit tests for clarity
- `experiments/runs/` — gitignored run output directories

Architecture decisions are recorded as numbered ADRs in `docs/decisions/`.

### 2. Rust crate structure

A single Cargo package containing both a library crate (`src/lib.rs`) and a binary crate
(`src/main.rs`). This keeps the CLI thin and makes the simulator logic independently
testable. If the codebase grows to require distinct versioning or separate CI targets per
component, it can be refactored into a Cargo workspace.

### 3. Module layout

Modules map to the five architectural components described in the proposal:

| Module | Responsibility |
|---|---|
| `cli` | Argument parsing and subcommand dispatch |
| `rpc` | RPC client: sends calls to Z3, records latency and errors |
| `z3` | Z3 component lifecycle: spawn, configure, health-check |
| `data_model` | Core types shared across modules |
| `synthetic` | Synthetic account and transaction generators |
| `scenarios` | Scenario config parsing and workload execution |
| `metrics` | Metric collection, histograms, and run output |

### 4. Build tooling

`make` is the top-level command surface. `cargo` handles all Rust-specific operations.
The Makefile wraps common cargo commands and adds project-specific targets. The `help`
target is self-documenting via `##` comments.

### 5. Cargo.lock

`Cargo.lock` is committed. This is a Rust binary project; committing the lock file
ensures reproducible builds for all team members and in CI.

## Consequences

- A future refactor to a Cargo workspace is possible without structural disruption.
- `experiments/runs/` is gitignored; significant results must be archived externally.
- `configs/local/` is gitignored; local environment overrides are never committed.
- The `z3` module name refers to the Zebra/Zaino/Zallet stack, not the Z3 theorem prover.
  Module doc comments make this explicit.
