# Observability Plan

## What is measured

Every simulator run produces six categories of observation:

| Category | Purpose | Output file |
|---|---|---|
| **RPC call log** | Per-call latency, success/failure, method, component | `rpc_calls.jsonl` |
| **Intent outcome log** | Per-intent confirmed/failed/timed-out result, by flow type | `intents.jsonl` |
| **Metric samples** | Time-series: throughput, mempool size, confirmation times | `metrics.jsonl` |
| **Component logs** | Stdout/stderr from Zebra, Zaino, and Zallet processes | `component_logs/` |
| **Run manifest** | Exact commits, scenario, timeouts, and timestamps for reproducibility | `manifest.json` |
| **Run summary** | Human-readable narrative of results and key findings | `summary.md` |

---

## Run manifest (`manifest.json`)

Written at the start of every run. Links results to the exact software versions and
scenario configuration used. Required for any finding to be reportable.

```json
{
  "run_id": "20260602T140000Z-smoke",
  "run_started_at": "2026-06-02T14:00:00Z",
  "run_completed_at": "2026-06-02T14:01:03Z",
  "simulator_commit": "<git SHA the running binary was compiled from, embedded at build time>",
  "zebra_commit": "<SHA from z3-commits.lock>",
  "zaino_commit": "<SHA from z3-commits.lock>",
  "zallet_commit": "<SHA from z3-commits.lock>",
  "scenario_name": "smoke",
  "scenario_config_hash": "sha256:<hash of scenario YAML content>",
  "timeouts": {
    "rpc_timeout_ms": 30000,
    "operation_poll_interval_ms": 2000,
    "max_operation_wait_ms": 120000,
    "confirmation_poll_interval_ms": 1000,
    "max_confirmation_wait_ms": 60000
  },
  "phase_boundaries": [
    { "phase": "bootstrap", "started_at": "2026-06-02T14:00:00Z" },
    { "phase": "readiness", "started_at": "2026-06-02T14:00:04Z" },
    { "phase": "warmup", "started_at": "2026-06-02T14:00:06Z" },
    { "phase": "funding", "started_at": "2026-06-02T14:00:40Z" },
    { "phase": "load", "started_at": "2026-06-02T14:00:55Z" },
    { "phase": "drain", "started_at": "2026-06-02T14:01:00Z" }
  ],
  "load_and_drain_completed_at": "2026-06-02T14:01:02Z",
  "compose_config_hash": "sha256:<hash of the effective, merged docker compose config>",
  "image_digests": [
    { "service": "zebra", "image": "zfnd/zebra:6.0.0", "id": "sha256:ab12cd..." },
    { "service": "zaino", "image": "zingodevops/zainod:0.6.0", "id": "sha256:34ef56..." },
    { "service": "zallet", "image": "z3sim/zallet:beta.2-local", "id": "sha256:78gh90..." },
    { "service": "rpc-router", "image": "z3sim/rpc-router:local", "id": "sha256:12ij34..." }
  ],
  "host_cpu_count": 8,
  "host_memory_limit_bytes": null,
  "state": {
    "reset_epoch": 3,
    "chain_height_at_start": 1245,
    "hot_wallet_balance_at_start_zat": 5000000000,
    "freshness": "reused"
  }
}
```

**Run ID format:** `<YYYYMMDDTHHMMSSZ>-<scenario-name>`. Sortable, human-readable, and
unique per run (assuming no two runs of the same scenario start in the same second).

**`phase_boundaries`** records the wall-clock start time of each lifecycle phase this run
passed through, in order: `bootstrap` (stack start, before it reports ready) →
`readiness` (hot-wallet resolution) → `warmup` (mining warmup blocks) → `funding`
(the hot-wallet-to-synthetic-account funding fan-out) → `load` (the measured workload) →
`drain` (draining in-flight intents after dispatch stops). Every `RpcCall` in
`rpc_calls.jsonl` carries the same `phase` it was issued during, so workload-scoped
report views (the RPC compatibility matrix, load curve, degradation detection) can
exclude setup-phase activity by construction rather than by convention. A run
directory written before phase tagging existed has no `phase_boundaries` (an old
manifest still deserializes — the field defaults to empty) and every `RpcCall` in it
deserializes with `phase: "unknown"`, which every phase-scoped view excludes rather
than mislabeling as `load`.

**`load_and_drain_completed_at`** records the wall-clock instant the Drain phase's own
work (draining in-flight intents) finished — the same instant `confirmed_tx_throughput`'s
elapsed-time window stops. This is deliberately **not** the same as `run_completed_at`:
the gap between the two is Z3 stack teardown (`docker compose down`), which is
environment/host overhead, not part of the measured workload. The findings report's
"Setup phase timing" section uses this field, rather than `run_completed_at`, as the
Drain row's own end boundary, and shows any residual gap as a separate Teardown row —
so the Drain duration shown there always reconciles with `confirmed_tx_throughput`.
`None`/absent for a manifest predating this field, or a run whose load phase never
completed (setup failed before Drain was reached).

**`simulator_commit` is embedded at compile time** (`build.rs` shells out to
`git rev-parse HEAD` at build time and bakes the result into the binary via
`SIMULATOR_GIT_COMMIT`), not read from the working tree at run time — the binary that
actually executed and the commit recorded in the manifest must never be able to
diverge. Likewise, `zebra_commit`/`zaino_commit`/`zallet_commit` are read from
`z3-commits.lock` at a path anchored to the crate root at compile time
(`CARGO_MANIFEST_DIR`), not resolved relative to the process's working directory at
run time.

**`timeouts`** records the RPC transport timeout and the confirmation/operation
polling patience actually in effect for the run, so a low confirmation rate can be
distinguished from an impatient client.

**`compose_config_hash`** is the SHA-256 hex digest of this run's effective, merged
`docker compose config` (images, env vars, ports, network layout, container-side
paths), computed by `z3::Z3Stack::compose_config_hash`. Checkout-location-dependent
bind-mount source paths (`${Z3_CONFIG_DIR}`-rooted config file mounts) are stripped
before hashing, so two checkouts of identical logical configuration cloned to
different filesystem paths produce the same hash — a real configuration change (a
different image tag, a different port) still changes it. Empty on a manifest
predating this field, or when the hash could not be computed (this is evidence, not
a correctness dependency, so a failure here degrades to a warning rather than
failing the run).

**`image_digests`** is the image (`repository:tag`) and local content-addressed
image ID Docker actually ran for each stack component, from `docker compose images
--format json` (`z3::Z3Stack::image_digests`, the same helper `z3sim print-versions`
uses — see Track 1). Labeled `id`, not `digest`, deliberately: it is not guaranteed
to equal a pullable registry manifest digest (storage-driver dependent), and the
locally-built Zallet/RPC Router images have no registry digest to compare against
at all — it remains a unique, reproducible content hash of the image bytes that ran.

**`host_cpu_count`** / **`host_memory_limit_bytes`** record the number of logical
CPUs available to the `z3sim` process (`std::thread::available_parallelism()`) and,
when running under a constrained Linux cgroup (a containerized CI runner, for
instance), its memory limit in bytes — `null` on an unconstrained bare-metal or VM
host, which is the common case for where `z3sim` itself runs (only the Z3 stack's
own containers are resource-constrained by design).

**`state`** records this run's starting chain-state provenance: `reset_epoch`
(incremented once per `scripts/dev/regtest-reset.sh` execution against this run's
specific environment, persisted at `configs/local/reset-epoch-<env_id>` — scoped per
environment id, like `run-<env_id>.lock`, so a `--fresh-env` run never reads another
environment's reset provenance; `0` if this specific environment has never been
reset), `chain_height_at_start` (observed immediately after the RPC client is constructed,
before any warmup mining), `hot_wallet_balance_at_start_zat` (the hot wallet's total
balance once `warmup()` confirms it funded), and `freshness` — `"fresh"` when this
run is the first to observe chain state since the last reset, `"reused"` when a
prior run already advanced the chain since then. This is deliberately cheap rather
than exact (no Docker-volume content hash): it distinguishes fresh/reused/which-reset-
generation without needing to reconstruct that judgment from raw numbers. Defaulted
(`reset_epoch: 0`, `freshness: "fresh"`) on a manifest predating this field.

---

## Console progress

Every multi-minute phase with a known total (warmup block-mining, the hot-wallet
balance wait, funding rounds, the load phase's dispatch loop) reports visible
progress via `scenarios::runner::progress::ProgressLine`: current phase, a short
detail string ("N/total blocks mined", "funding round N/total", ...), elapsed time,
and — where a ceiling is known — the timeout budget. On a real terminal this
redraws one line in place (`\r`); when stderr is not a TTY (piped, redirected, CI)
it falls back to one `tracing::info!` line per update instead. This supersedes the
warmup balance-check loop's old every-30-second `eprintln!` — the mechanism is now
one line, updated continuously, rather than a periodic diagnostic print.

---

## Intent outcome log (`intents.jsonl`)

One JSON object per line, one line per dispatched transaction intent, written once
the load phase completes. This is what lets a failure be attributed to a specific
flow type, and lets an async-operation (ZK proving) timeout be told apart from a
confirmation-depth timeout.

```json
{"run_id":"20260602T140000Z-smoke","intent_id":"a1b2c3","flow_type":"t_to_z","outcome":"confirmed","error":null,"timeout_context":null,"recorded_at":"2026-06-02T14:00:45Z"}
{"run_id":"20260602T140000Z-smoke","intent_id":"d4e5f6","flow_type":"z_to_t","outcome":"failed","error":"RPC error: ...","timeout_context":null,"recorded_at":"2026-06-02T14:00:47Z"}
{"run_id":"20260602T140000Z-smoke","intent_id":"g7h8i9","flow_type":"t_to_z","outcome":"timed_out","error":null,"timeout_context":"operation op-1 did not complete within the deadline","recorded_at":"2026-06-02T14:00:52Z"}
```

`outcome` is one of `confirmed` / `failed` / `timed_out`. `timeout_context` is set only
when `outcome == "timed_out"` and distinguishes an async-operation wait ("operation
`<id>` did not complete within the deadline") from a confirmation-depth wait ("tx
`<id>` did not reach N confirmations within the deadline") — the two `ExchangeError::Timeout`
call sites in `src/scenarios/exchange.rs`. `generate_summary` reads this file (if
present — older runs predating this feature simply produce no "Outcomes by flow type"
section) to break down outcomes and timeout stages per flow type in `summary.md`.

---

## RPC call log (`rpc_calls.jsonl`)

One JSON object per line, one line per RPC call. Written incrementally during the run
so partial data is preserved if a run crashes.

```json
{"call_id":"a1b2c3","run_id":"20260602T140000Z-smoke","method":"getblockchaininfo","component":"Zebra","request_at":"2026-06-02T14:00:01.000Z","response_at":"2026-06-02T14:00:01.012Z","latency_ms":12,"success":true,"error_code":null,"error_message":null,"phase":"warmup"}
{"call_id":"d4e5f6","run_id":"20260602T140000Z-smoke","method":"z_sendmany","component":"Zallet","request_at":"2026-06-02T14:00:02.000Z","response_at":"2026-06-02T14:00:02.008Z","latency_ms":8,"success":true,"error_code":null,"error_message":null,"phase":"load"}
{"call_id":"g7h8i9","run_id":"20260602T140000Z-smoke","method":"getnewaddress","component":"Zallet","request_at":"2026-06-02T14:00:03.000Z","response_at":null,"latency_ms":null,"success":false,"error_code":-32601,"error_message":"Method not found","phase":"load"}
```

`phase` names the lifecycle stage the call was issued during — one of `bootstrap`,
`readiness`, `warmup`, `funding`, `load`, `drain`, or `unknown` (a run predating phase
tagging) — see `phase_boundaries` above. Report views scoped to the measured workload
(the RPC compatibility matrix, load curve, degradation detection) include only `load`
and `drain` calls; setup-phase calls (`bootstrap`/`readiness`/`warmup`/`funding`) are
shown separately, in their own unscored appendix.

This file is the primary input for:
- latency histograms (P50/P95/P99 per method),
- success/failure rates per method and component,
- the RPC compatibility matrix (which methods worked, which errored, and how).

---

## Metric samples (`metrics.jsonl`)

One JSON object per line, written at regular intervals during the run. Captures the
state of the simulator and the Z3 stack over time.

```json
{"run_id":"20260602T140000Z-smoke","timestamp":"2026-06-02T14:00:05Z","metric_name":"mempool_tx_count","value":3.0,"labels":{}}
{"run_id":"20260602T140000Z-smoke","timestamp":"2026-06-02T14:00:05Z","metric_name":"confirmed_txs_total","value":1.0,"labels":{"flow_type":"t_to_t"}}
{"run_id":"20260602T140000Z-smoke","timestamp":"2026-06-02T14:00:05Z","metric_name":"rpc_latency_ms","value":12.0,"labels":{"method":"getblockchaininfo","percentile":"p99"}}
{"run_id":"20260602T140000Z-smoke","timestamp":"2026-06-02T14:00:05Z","metric_name":"active_accounts","value":5.0,"labels":{}}
```

Latency is measured **per method**, not per component. All calls go through the Z3 RPC
Router, so there is no per-component latency attribution for JSON-RPC calls. Zaino's
latency is folded into Zallet method response times implicitly.

The `backend` label (Zebra or Zallet) is available for grouping if needed, derived from
the method's routing table.

### Metric names

| Metric | Labels | Description |
|---|---|---|
| `rpc_call_total` | `method`, `backend`, `success` | Running count of RPC calls |
| `rpc_latency_ms` | `method`, `backend`, `percentile` | Latency percentiles (p50, p95, p99) |
| `mempool_tx_count` | — | Number of transactions in the mempool |
| `confirmed_txs_total` | `flow_type` | Cumulative confirmed transactions |
| `failed_txs_total` | `flow_type`, `reason` | Cumulative failed transactions |
| `deposit_confirmation_time_ms` | — | Time from deposit detection to credit |
| `proving_time_ms` | — | ZK proof generation time for shielded txs |
| `active_accounts` | — | Accounts actively transacting at sample time |
| `block_height` | — | Current chain height |
| `scheduled_dispatch_rate` | — | Intents dispatched per second of actual load-phase elapsed time (not the configured duration) — a scheduler-behavior figure, not a confirmed-transaction rate |
| `confirmed_tx_throughput` | — | Confirmed transactions per second of Load+Drain wall-clock time — the only metric this report labels "TPS" |
| `mempool_saturation_event` | `threshold` | Recorded once when mempool depth crosses the saturation threshold; value is the observed depth at crossing |
| `process_cpu_percent` | `process` | CPU usage of each Docker container (Zebra, Zaino, Zallet) |
| `process_memory_mb` | `process` | Memory usage of each Docker container |

Metric sampling interval: **5 seconds** by default, configurable per-scenario via
`observability.metric_sampling_interval_secs` in the scenario YAML. Use 1s for burst
scenarios where fine-grained mempool data is needed; 15s for long steady-state runs.

---

## Component logs (`component_logs/`)

Captured stdout and stderr from each Z3 process during the run. Preserved verbatim for
post-run debugging.

```
component_logs/
  zebra.log
  zaino.log
  zallet.log
```

**Capture mechanism: pipe.** The simulator starts each Z3 process with stdout and stderr
piped into the simulator, reads them in background tasks, and writes the bytes to the
log files above. This gives the simulator full ownership of the output stream, which
enables health-check detection (e.g. watching for "RPC server ready" before marking a
component healthy) without relying on platform-specific tools like `tee`.

Log verbosity should be tuned per run: higher for debugging, lower for large-scale
runs where log volume becomes a bottleneck.

---

## Run summary (`summary.md`)

A human-readable narrative generated at the end of each run. Intended for sharing with
the Foundation and component teams without requiring them to parse JSONL files.

### Contents

```markdown
# Run Summary: <run-id>

## Run metadata
- Scenario: <name>
- Started: <timestamp>
- Duration: <seconds>
- Simulator commit: <sha>
- Z3 commits: Zebra <sha>, Zaino <sha>, Zallet <sha>

## Load results
- Target dispatch rate: <n> intents/s
- Scheduled dispatch rate: <n> intents/s (actual load-phase elapsed time, not the configured duration)
- Confirmed tx throughput (TPS, from `confirmed_tx_throughput`): <n>
- Total transactions attempted: <n>
- Confirmed: <n> (<pct>%)
- Failed: <n> (<pct>%)

## RPC latency (P50 / P95 / P99)
Scoped to Load/Drain-phase calls only — the measured workload. Setup-phase retries
(e.g. the funding fan-out's own anchor-confirmation retries) never inflate this table's
Calls/Errors counts.
| Method | Component | P50 ms | P95 ms | P99 ms | Errors |
|---|---|---|---|---|---|
| ...     | ...       | ...    | ...    | ...    | ...    |

## Setup-phase RPC activity
Present only when the run recorded setup-phase (`Bootstrap`/`Readiness`/`Warmup`/`Funding`)
calls. Same table shape as above, scoped to those phases instead — informational, not
mixed into the workload table.
| Method | Component | P50 ms | P95 ms | P99 ms | Errors |
|---|---|---|---|---|---|
| ...     | ...       | ...    | ...    | ...    | ...    |

## Mempool
- Peak mempool size: <n> transactions
- Mempool saturation events: <n>

## Outcomes by flow type
| Flow type | Confirmed | Failed | Timed out |
|---|---|---|---|
| ...       | ...       | ...    | ...       |

### Timeouts by stage
- async operation (ZK proving) wait: <n>
- on-chain confirmation wait: <n>

(Omitted entirely for runs with no `intents.jsonl`.)

## Notable errors and findings
- ...

## Shielded transaction proving times
- P50: <ms>, P95: <ms>, P99: <ms>
```

---

## Experiment output directory structure

```
experiments/runs/
  <run-id>/
    manifest.json          Run metadata, commit pins, and effective timeouts
    scenario.yaml          Exact scenario config snapshot
    rpc_calls.jsonl        Per-call log (one JSON object per line)
    intents.jsonl          Per-intent outcome log, by flow type
    metrics.jsonl          Time-series metric samples
    component_logs/
      zebra.log
      zaino.log
      zallet.log
    summary.md             Human-readable run summary
```

Run directories are gitignored and are not tracked by version control.

---

## Component metrics endpoints

| Component | Metrics endpoint | Details |
|---|---|---|
| **Zebra** | Yes — Prometheus | In-network scrape endpoint on `:9999` (`ZEBRA_METRICS__ENDPOINT_ADDR`, default `0.0.0.0:9999`). Exposes chain sync, block/tx verification times, peer connections, and more. Scraped by the monitoring profile's Prometheus. |
| **Zaino** | No HTTP metrics | Structured `tracing` logs only; resource usage captured via `docker stats`. |
| **Zallet** | No HTTP metrics | Structured `tracing` logs only; resource usage captured via `docker stats`. |

---

## Z3 monitoring profile (cross-validation)

Z3 ships an opt-in `monitoring` Compose profile — **Prometheus, Grafana, Jaeger (with
spanmetrics), and AlertManager** — declared in `z3-contract.yaml` under
`profiles: monitoring`. It scrapes Zebra's Prometheus endpoint and collects OpenTelemetry
traces (via `ZEBRA_TRACING__OPENTELEMETRY_ENDPOINT`), giving **server-side per-RPC latency
and resource profiles** out of the box.

The simulator's own client-side measurements (`rpc_calls.jsonl`, `metrics.jsonl`) remain
the authoritative findings source — they are transport-accurate and reproducible. The Z3
monitoring profile is used to **cross-validate** those numbers: Jaeger spanmetrics give a
server-side latency view per RPC that should track the client-side histograms, and the
Grafana dashboards are useful for the live demonstration.

Bring it up alongside the stack with the profile enabled (regtest host ports from the
contract: Grafana `23000`, Prometheus `29094`, Jaeger UI `36686`):

```sh
docker compose --env-file .env.regtest --profile monitoring up -d
```

Enabling the profile is **optional** and additive — the simulator runs and produces all
findings without it.

---

## Resource profiling (CPU and memory)

CPU and memory usage of the Z3 containers are sampled during runs to identify resource
bottlenecks at scale.

**Always-on.** Resource profiling runs on every scenario — the polling overhead is
negligible and resource behavior under load is a core project deliverable.

Approach: the harness polls `docker stats --no-stream`, scoped to the active network's
Compose project (containers named `z3-regtest-*`), and records CPU % and memory for each.
Zebra's Prometheus endpoint (and the monitoring profile, when enabled) supplements this
with richer chain-level metrics.

Resource samples are written to `metrics.jsonl` using metric names like
`process_cpu_percent` and `process_memory_mb` with a `process` label.

---

## Mempool monitoring

The simulator monitors the mempool by **polling** `getrawmempool` / `getmempoolinfo`
through the RPC Router (see the mempool watcher in `src/scenarios/exchange.rs`), recording
`mempool_tx_count`, `mempool_bytes`, and saturation events.

ZMQ is not used anywhere in Z3; the push-based replacements below are **documented but out
of scope for this engagement** (no gRPC client is built):

| Component | Mechanism | Details |
|---|---|---|
| **Zebra** | `Indexer.mempool_change()` gRPC stream | Pushes `MempoolChangeMessage` with `change_type` (ADDED / INVALIDATED / MINED) and `tx_hash`. Requires `--features indexer` build flag and an `indexer_listen_addr` in config. |
| **Zaino** | `GetMempoolTx` / `GetMempoolStream` gRPC streams | `GetMempoolStream` streams all mempool transactions until the next block is mined. `GetMempoolTx` streams compact transactions with optional txid filtering. |
