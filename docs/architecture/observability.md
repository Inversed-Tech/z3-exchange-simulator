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
  }
}
```

**Run ID format:** `<YYYYMMDDTHHMMSSZ>-<scenario-name>`. Sortable, human-readable, and
unique per run (assuming no two runs of the same scenario start in the same second).

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
{"call_id":"a1b2c3","run_id":"20260602T140000Z-smoke","method":"getblockchaininfo","component":"Zebra","request_at":"2026-06-02T14:00:01.000Z","response_at":"2026-06-02T14:00:01.012Z","latency_ms":12,"success":true,"error_code":null,"error_message":null}
{"call_id":"d4e5f6","run_id":"20260602T140000Z-smoke","method":"z_sendmany","component":"Zallet","request_at":"2026-06-02T14:00:02.000Z","response_at":"2026-06-02T14:00:02.008Z","latency_ms":8,"success":true,"error_code":null,"error_message":null}
{"call_id":"g7h8i9","run_id":"20260602T140000Z-smoke","method":"getnewaddress","component":"Zallet","request_at":"2026-06-02T14:00:03.000Z","response_at":null,"latency_ms":null,"success":false,"error_code":-32601,"error_message":"Method not found"}
```

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
| `tps_achieved` | — | Observed transactions per second vs. target |
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
- Target TPS: <n>
- Achieved TPS: <n>
- Total transactions attempted: <n>
- Confirmed: <n> (<pct>%)
- Failed: <n> (<pct>%)

## RPC latency (P50 / P95 / P99)
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
