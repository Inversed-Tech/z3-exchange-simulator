# GCP test machine — spec for reproducible scenario runs

Hand this file to the infra session managing Terraform. It specifies the VM the
simulator's live scenario runs should execute on, why each choice matters, and
what access the driving session needs back.

> **Provisioned 2026-08-13**: `c3d-standard-16` (AMD EPYC 9B14 / Genoa),
> zone `europe-west1-b`, Ubuntu 24.04 LTS amd64, 200 GB pd-ssd, on-demand,
> reserved static IP `34.156.25.159`, user `z3sim`, direct SSH. The startup
> script is idempotent and re-runs on every boot. Setup on the machine follows
> [`scripts/dev/regtest-overrides/apply.sh`](../../scripts/dev/regtest-overrides/apply.sh)'s
> run-order header.

## Why a standard machine

Development so far ran the whole stack under qemu (amd64 images on an aarch64
host, Docker Desktop). Emulation costs ~2.2 s of halo2 proving per Orchard
block and distorts every latency we record (`docs/regtest-funding-plan.md` §6).
The Z3 compose stack pins `DOCKER_PLATFORM=linux/amd64` and Zaino publishes
amd64-only images, so the reproducible target is a **native x86_64 VM** — no
emulation, one fixed CPU platform, comparable numbers across runs.

## Machine

| Item | Value | Rationale |
|---|---|---|
| Machine type | **`c3d-standard-16`** (16 vCPU AMD Genoa, 64 GB) | Halo2/Orchard proving is the bottleneck: CPU-bound, multi-core (rayon). 16 vCPUs cover concurrent `z_sendmany` proving during fan-out + the 3 stack containers + the simulator. Compute-optimized ⇒ fixed CPU platform ⇒ reproducible latency. |
| Economical fallback | `c3d-standard-8` (8 vCPU, 32 GB) | Fine for smoke-scale (60 intents); larger scenarios will queue on proving. |
| If c3d unavailable in region | `c3-standard-*` or `n2d-standard-*` with `min_cpu_platform` pinned | Any fixed modern x86 platform works; do **not** use e2 (unpinned CPU platform → run-to-run variance). |
| Architecture | **x86_64 — mandatory** | Zaino images are amd64-only; stack compose pins linux/amd64. No ARM (t2a) option. |
| Provisioning | **On-demand, not Spot** | A preemption mid-run wastes the whole benchmark. Runs are 10–30 min; stop the instance between sessions instead. |
| Region | Any; pick cheapest/closest | Workload is self-contained on the VM. |
| GPU / nested virt | None needed | Plain Docker only. |

Rough cost: c3d-standard-16 ≈ $0.7/h on-demand (us-central1); c3d-standard-8 ≈ half that.

## Boot disk

- **Ubuntu 24.04 LTS (x86_64)**, 200 GB `pd-ssd` (or `hyperdisk-balanced` on c3d).
- Contents: Docker images (~10 GB), Rust target dir (~10 GB), per-run fresh
  regtest datadirs (small but we recreate them every validated run), vendored
  `external/` clones, `experiments/runs/` outputs.

## Software (startup script or first-login install — either is fine)

```sh
# Docker Engine + compose plugin (get.docker.com or apt docker-ce), run user in `docker` group
# Base tooling
apt-get install -y git curl build-essential pkg-config jq libfontconfig1-dev
# libfontconfig1-dev: required by the simulator's metrics-chart dependency
# (yeslogic-fontconfig-sys) at build time
# rage (str4d/rage release .deb, amd64; needs libfuse2) — the stack's
# setup-network.sh needs rage-keygen to generate the Zallet identity file
# Rust (as the run user, not root): stable via rustup — repo builds on stable, edition 2021
# gh CLI (https://cli.github.com) — used by scripts/dev/zallet-release-image/build.sh
#   to fetch the Zallet release tarball, and for repo access
```

No services need to be pre-arranged beyond this; the driving session does the
rest (clone, `make clone-z3`, build the local `z3sim/zallet:v0.1.0-beta.1`
image via `scripts/dev/zallet-release-image/build.sh`, apply the
`.env.regtest` overrides from `z3-commits.lock`, run scenarios).

## Network / access

- **Inbound: SSH (22) only**, restricted source ranges or IAP as you prefer.
  All stack RPC ports stay on the VM (localhost/docker network); anything the
  driving session needs it reaches over the SSH connection.
- **Outbound: unrestricted egress** — pulls from Docker Hub (`zfnd/zebra`,
  `zingodevops/zainod`), GitHub (repo, ZcashFoundation/z3, Zallet release
  tarball), crates.io.
- **SSH key for the driving Claude session** — the driving session generates an
  ed25519 keypair and hands over the public key out-of-band; add it to instance
  metadata (or OS Login) for the run user. Don't commit the key here.
- **Repo access from the VM**: `Inversed-Tech/z3-exchange-simulator` — either a
  read-only deploy key, a fine-grained PAT for `gh auth login`, or the user
  authenticates `gh` once interactively. (Everything else the setup fetches is
  public.)

## What to hand back to the driving session

1. `ssh <user>@<ip-or-hostname>` that works with the key above (mention if it's
   via IAP: `gcloud compute ssh` invocation instead).
2. The machine type + region actually provisioned (recorded alongside run
   artifacts for attribution).
3. How the repo credential was provided (deploy key path / `gh` pre-authed).

## Reproducibility notes for run attribution

- Record in each run's artifacts: machine type, CPU platform
  (`cat /proc/cpuinfo | grep "model name" | head -1`), image/OS version,
  and the `z3-commits.lock` override set — native timings are **not
  comparable** to the emulated dev-machine timings already in
  `docs/regtest-funding-plan.md`; expect proving to be several times faster.
- Default host-maintenance live migration is acceptable; if a latency blip
  appears in exactly one run, check the instance's migration events before
  suspecting the stack.
