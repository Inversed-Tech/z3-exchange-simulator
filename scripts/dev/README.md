# Development Scripts

Shell scripts for setting up and managing the local Z3 development environment.

| Script | Purpose |
|---|---|
| `clone-z3.sh` | Clone Zebra, Zaino, and Zallet at their pinned commits |
| `regtest-miner-setup.sh` | Fund coinbase into Zallet: point `ZEBRA_MINING__MINER_ADDRESS` at the hot wallet's receiver |
| `zallet-release-image/build.sh` | Build a local Zallet Docker image from the official release tarball (used for pins with no published image, e.g. beta.2) |

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
