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
and writes three kinds of files here — the first machine-written content in
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
- `reset-epoch-<env_id>` — written by `scripts/dev/regtest-reset.sh`'s last
  step, two whitespace-separated fields: an incrementing reset-generation
  counter and the chain height observed right after that reset. Read by
  `z3sim run` into a run's manifest (`state.reset_epoch`, see
  `src/z3/env_id.rs::reset_epoch_path` and
  `src/metrics/manifest.rs::read_reset_state`) so a run can be told "freshly
  reset" apart from "reused since the last reset." Scoped per `<env_id>` —
  like `run-<env_id>.lock` — so a `--fresh-env` environment and the stable
  one never read or write each other's reset provenance; a run against an
  environment with no file here reads as "no reset recorded yet" for that
  specific environment (epoch 0), not a leftover value from whichever
  environment was reset most recently. Absent until the first
  `regtest-reset.sh` run against a given environment; deleting it just
  resets that environment's provenance to "no reset recorded," not a
  functional change. (An environment with no cached `env-id` at all — no
  `z3sim run` has happened here yet — falls back to the unscoped
  `reset-epoch`, matching `regtest-reset.sh`'s own `z3-regtest` project-name
  fallback.)

Everything else here remains developer-specific values (local paths, ports,
chain data locations) to reference manually or copy into a scenario override
when needed — the simulator does not read those automatically.
