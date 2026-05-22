# Observability Plan

Defines what the simulator measures, how it records results, what each output file
contains, and how a contributor can interpret a completed run.

---

## What we measure

Every simulator run produces five categories of observation:

| Category | Purpose | Output file |
|---|---|---|
| **RPC call log** | Per-call latency, success/failure, method, component | `rpc_calls.jsonl` |
| **Metric samples** | Time-series: throughput, mempool size, confirmation times | `metrics.jsonl` |
| **Component logs** | Stdout/stderr from Zebra, Zaino, and Zallet processes | `component_logs/` |
| **Run manifest** | Exact commits, scenario, and timestamps for reproducibility | `manifest.json` |
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
  "simulator_commit": "<git SHA of this repository>",
  "zebra_commit": "<SHA from z3-commits.lock>",
  "zaino_commit": "<SHA from z3-commits.lock>",
  "zallet_commit": "<SHA from z3-commits.lock>",
  "scenario_name": "smoke",
  "scenario_config_hash": "sha256:<hash of scenario YAML content>"
}
```

**Run ID format:** `<YYYYMMDDTHHMMSSZ>-<scenario-name>`. Sortable, human-readable, and
unique per run (assuming no two runs of the same scenario start in the same second).

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

### Planned metric names

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

Log verbosity level should be set high enough to capture RPC request/response traces
during development, and reduced for large-scale runs where log volume becomes a bottleneck.

---

## Run summary (`summary.md`)

A human-readable narrative generated at the end of each run. Intended for sharing with
the Foundation and component teams without requiring them to parse JSONL files.

### Planned contents

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
    manifest.json          Run metadata and commit pins
    scenario.yaml          Exact scenario config snapshot
    rpc_calls.jsonl        Per-call log (one JSON object per line)
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
| **Zebra** | Yes — Prometheus | `http://localhost:9999/metrics`, disabled by default. Enable via `[metrics] endpoint_addr = "0.0.0.0:9999"` in `zebrad.toml`. Exposes chain sync, block/tx verification times, peer connections, and more. |
| **Zaino** | No | Uses `tracing` crate for structured logging only. No HTTP metrics endpoint at pinned commit. |
| **Zallet** | No | Uses `tracing` crate for structured logging only. No HTTP metrics endpoint at pinned commit. |

The simulator should enable Zebra's Prometheus endpoint in the regtest config and
optionally scrape it during runs for richer chain-level metrics. Zaino and Zallet
resource usage is measured via OS-level polling.

---

## Resource profiling (CPU and memory)

CPU and memory usage of all three Z3 processes are sampled during runs to identify
resource bottlenecks at scale.

**Always-on.** Resource profiling runs on every scenario — the polling overhead is
negligible and resource behavior under load is a core project deliverable.

Approach:
- **Zebra**: OS-level polling supplemented by Prometheus endpoint scraping where enabled
- **Zaino / Zallet**: OS-level polling via `/proc/<pid>/stat` (Linux) or `ps` (macOS)

Resource samples are written to `metrics.jsonl` using metric names like
`process_cpu_percent` and `process_memory_mb` with a `process` label.

---

## Mempool notification mechanism

ZMQ is not used anywhere in Z3. Both Zebra and Zaino provide gRPC-based streaming instead.

| Component | Mechanism | Details |
|---|---|---|
| **Zebra** | `Indexer.mempool_change()` gRPC stream | Pushes `MempoolChangeMessage` with `change_type` (ADDED / INVALIDATED / MINED) and `tx_hash`. Requires `--features indexer` build flag and `[rpc] indexer_listen_addr = "127.0.0.1:8230"` in config. |
| **Zaino** | `GetMempoolTx` / `GetMempoolStream` gRPC streams | `GetMempoolStream` streams all mempool transactions until the next block is mined. `GetMempoolTx` streams compact transactions with optional txid filtering. |

**For the simulator:** Use Zebra's `Indexer.mempool_change()` for event-driven deposit detection — it tells you exactly when a transaction is added, invalidated, or mined. Use Zaino's `GetMempoolStream` as a secondary signal for mempool saturation measurement. Polling `getrawmempool` remains a fallback if gRPC is not configured.
