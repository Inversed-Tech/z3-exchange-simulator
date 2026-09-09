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
apt-get install -y git curl build-essential pkg-config jq libfontconfig1-dev libssl-dev
# libfontconfig1-dev + libssl-dev: required at build time by the simulator's
# metrics-chart dependency (yeslogic-fontconfig-sys) and openssl-sys
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
- **Repo access from the VM**: none to provision —
  `Inversed-Tech/z3-exchange-simulator` is public, as is everything else the
  setup fetches; plain `https://` clones work unauthenticated.

## What to hand back to the driving session

1. `ssh <user>@<ip-or-hostname>` that works with the key above (mention if it's
   via IAP: `gcloud compute ssh` invocation instead).
2. The machine type + region actually provisioned (recorded alongside run
   artifacts for attribution).
3. Any deviation from this spec (machine type, OS image, package set).

## Operating the machine (driving session workflow)

### SSH alias

The driving session's `~/.ssh/config` entry (keypair at `~/.ssh/z3sim_gcp`,
public key in instance metadata for `z3sim`):

```ssh-config
Host z3sim-gcp
    HostName 34.156.25.159
    User z3sim
    IdentityFile ~/.ssh/z3sim_gcp
    ServerAliveInterval 30
```

After that, everything is `ssh z3sim-gcp '<command>'` or an interactive login.
Note the run user `z3sim` is **uid 1001** (GCP's default `ubuntu` account
claimed 1000) — the uid-sensitive fixes in
`scripts/dev/regtest-overrides/` exist for exactly this.

### Setup + run commands

The repo lives at `~/z3-exchange-simulator` on the VM (public repo, plain
https clone). Fresh setup and a scenario run:

```sh
cd ~/z3-exchange-simulator
git fetch && git checkout <branch> && git pull --ff-only
export PATH="$HOME/.cargo/bin:$PATH"   # rustup installs per-user; non-login
                                       # shells (tmux, ssh command mode) miss it
make clone-z3 && make bootstrap        # idempotent; safe to re-run
./target/debug/z3sim run --scenario configs/scenarios/smoke.yaml
make regtest-reset                     # between runs that reuse the wallet
```

Run outputs land in `experiments/runs/<run-id>/`; pull them back with
`scp -r z3sim-gcp:z3-exchange-simulator/experiments/runs/<run-id> …`.

### Long runs: use tmux

Multi-scenario sequences take hours — never run them on a bare SSH
connection. Launch inside tmux (installed) so the run survives disconnects
and stays attachable:

```sh
tmux new-session -d -s z3runs 'bash ~/run-scenarios.sh'  # detached launch
tmux attach -t z3runs                                    # watch live (Ctrl-b d to detach)
tmux capture-pane -t z3runs -p | tail -20                # peek without attaching
```

Two gotchas, both hit in practice: tmux starts a **non-login shell**, so
export `~/.cargo/bin` onto PATH inside the script itself; and have the script
tee everything to log files (e.g. `~/scenario-runs-<stamp>/`) so tmux is only
the supervisor, never the record. Plain `nohup` also survives disconnects but
gives up interactive attach; containerizing the driver adds nothing since it
orchestrates `docker compose` itself.

### Stale state warning

`.env.regtest` inside `external/z3` persists values across environments —
including `ZEBRA_MINING__MINER_ADDRESS`, which a previous environment's
miner-setup pointed at *its* wallet's Orchard UA. A brand-new Compose
project reusing that file panics Zebra's `generate` at heights < NU5
("Cannot create Orchard transactions … before NU5 activation") and, past
init, would mine rewards to an address the new wallet does not control.
When in doubt, wipe `external/z3` (a throwaway pinned clone) and re-run
`make clone-z3 && make bootstrap`.

### Cost

The instance is on-demand (~$0.7/h): **stop it when idle**. The static IP
and all disk state survive stop/start.

## Baseline benchmarks (2026-09-09, machine idle)

Standard, quick synthetic benchmarks recorded once so a future rerun can
detect a changed machine type, CPU platform, noisy neighbour, or disk
regression before blaming the stack. Re-run the exact commands below on an
idle VM (no containers, load ≈ 0) and compare; deviations of more than ~10 %
on CPU/RAM or a different plateau on disk mean the numbers are not comparable.

Environment at the time of the run: `c3d-standard-16` in `europe-west1-b`,
AMD EPYC 9B14, Ubuntu 24.04.4 LTS, kernel `6.17.0-1022-gcp`, 200 GB pd-ssd
(`PERSISTENT-SSD`, NVMe interface, `mq-deadline` scheduler), sysbench 1.0.20,
fio 3.36. Install with `sudo apt-get install -y sysbench fio`.

### CPU — `sysbench cpu --cpu-max-prime=20000 --time=20`

| Threads | events/s | avg latency (ms) | 95th (ms) |
|---|---|---|---|
| 1 | 1522 | 0.66 | 0.67 |
| 16 | 13456 | 1.19 | 1.21 |

16-thread scaling is 8.8× single-thread, consistent with 8 physical cores ×
2 SMT threads: halo2 proving should not expect a 16× speed-up from rayon.

### RAM — `sysbench memory --memory-block-size=1M`

| Test | Threads | Throughput |
|---|---|---|
| write (`--memory-total-size=100G`) | 1 | 29.3 GiB/s |
| write (`--memory-total-size=400G`) | 16 | 68.1 GiB/s |
| read (`--memory-total-size=400G`) | 16 | 445 GiB/s (cache-resident; sanity check only) |

### Disk — `fio --direct=1 --ioengine=libaio --size=4G --runtime=20 --time_based` on `/`

| Test | bs | jobs × iodepth | IOPS | Bandwidth | avg lat | p99 lat |
|---|---|---|---|---|---|---|
| seq write | 1M | 1 × 16 | 336 | 336 MiB/s | 47 ms | 50 ms |
| seq read | 1M | 1 × 16 | 336 | 336 MiB/s | 47 ms | 48 ms |
| rand write | 4k | 4 × 32 | 13 044 | 50 MiB/s | 9.8 ms | 11.1 ms |
| rand read | 4k | 4 × 32 | 12 190 | 47 MiB/s | 10.5 ms | 10.9 ms |

The identical read/write figures and flat, high latencies show the disk is
sitting at its **provisioned pd-ssd throttle**, not a media limit. Expect
~340 MiB/s sequential and ~12–13k random 4k IOPS regardless of queue depth;
a larger disk or hyperdisk-balanced would move these, a CPU change would not.
Zebra/Zallet state on regtest is small, so this is unlikely to be a
bottleneck, but it explains any fsync-heavy stalls if they appear.

Exact fio invocation used (one line per test, `RW/BS/JOBS/QD` from the table):

```sh
fio --name=t --filename=~/fio-test.bin --size=4G --direct=1 --ioengine=libaio \
    --runtime=20 --time_based --group_reporting --rw=RW --bs=BS --numjobs=JOBS --iodepth=QD
rm ~/fio-test.bin
```

## Reproducibility notes for run attribution

- Record in each run's artifacts: machine type, CPU platform
  (`cat /proc/cpuinfo | grep "model name" | head -1`), image/OS version,
  and the `z3-commits.lock` override set — native timings are **not
  comparable** to the emulated dev-machine timings already in
  `docs/regtest-funding-plan.md`; expect proving to be several times faster.
- Default host-maintenance live migration is acceptable; if a latency blip
  appears in exactly one run, check the instance's migration events before
  suspecting the stack.
