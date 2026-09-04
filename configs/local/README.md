# Local Configuration Overrides

This directory holds local environment-specific configuration files.

**These files are gitignored and must never be committed.**

## Purpose

Use this directory for:

- local port assignments for Z3 component processes,
- local paths to Zebra, Zaino, and Zallet binaries,
- local regtest chain data directories,
- any developer-specific settings that should not apply to other team members.

## Usage

Besides the developer-reference values above, the simulator itself now reads
and writes two kinds of files here — the first machine-written content in
this directory:

- `env-id` — this checkout's stable environment identifier (8-character
  lowercase hex, see `src/z3/env_id.rs`). Generated once per checkout and
  reused on every `z3sim run` so the same Compose project, ports, and subnet
  are reused across runs; pass `--fresh-env` to mint a new one instead of
  reusing this file. Deleting it has the same effect as `--fresh-env` on the
  next run.
- `run-<env_id>.lock` — an advisory lock held for the duration of a
  `z3sim run` against `<env_id>` (see `src/z3/run_lock.rs`), preventing a
  second concurrent invocation against the same environment from colliding
  with the first. Empty; safe to delete when no run is in progress.

Everything else here remains developer-specific values (local paths, ports,
chain data locations) to reference manually or copy into a scenario override
when needed — the simulator does not read those automatically.
