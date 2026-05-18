# Development Scripts

Shell scripts for setting up and managing the local Z3 development environment.

| Script | Purpose |
|---|---|
| `clone-z3.sh` | Clone Zebra, Zaino, and Zallet at their pinned commits |

## Prerequisites

- `bash` 4.0+
- `git`

## Usage

Prefer running scripts through Makefile targets where possible:

```sh
make clone-z3
```

Running scripts directly is also supported:

```sh
bash scripts/dev/clone-z3.sh
```

## Cloned repositories

Z3 components are cloned into `external/` at the repository root. This directory is
gitignored. See [`z3-commits.lock`](../../z3-commits.lock) for the pinned commit hashes.
