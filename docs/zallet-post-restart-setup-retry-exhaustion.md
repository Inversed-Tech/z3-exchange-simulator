# Setup Fails With Exhausted `z_listaccounts` Retries After a Stack Restart With Existing Wallet History

This document is a factual record of a specific, repeatedly observed problem. It
contains direct observations only: commands run, timestamps, log excerpts, and
recorded data files. Where a hypothesis is included, it is explicitly marked as
such and distinguished from what was actually observed. It is written for
handoff to an independent reviewer or bug-finding agent.

---

## Environment

- Deployment: the Z3 Docker Compose stack (`external/z3`), regtest network,
  brought up via `docker compose --env-file .env.regtest up -d`.
- Component versions in use (the "override" set recorded in `z3-commits.lock`):
  - Zebra `v6.0.0` (image `zfnd/zebra:6.0.0`)
  - Zaino `0.6.0` (image `zingodevops/zainod:0.6.0-no-tls`)
  - Zallet `v0.1.0-beta.2` (image `z3sim/zallet:v0.1.0-beta.2`, built locally
    from the official release tarball) — this is the version that fixed the
    permanent crash-loop documented in
    [`zallet-restart-sync-failure.md`](zallet-restart-sync-failure.md).
- Simulator commit: `5d6340a2bbcf938a80fe2db661dd70f93bd6c9ea`.
- Host: macOS, arm64 (Apple Silicon); the above images run under x86_64
  emulation (`DOCKER_PLATFORM=linux/amd64`), same as the environment described
  in `zallet-restart-sync-failure.md`.
- The regtest environment had been freshly reinitialized shortly before these
  observations: Docker volumes for chain/cookie/zaino/zallet state were
  removed (`docker compose --env-file .env.regtest down -v`) and recreated via
  `external/z3/scripts/regtest-init.sh` followed by
  `scripts/dev/regtest-miner-setup.sh`.

### A separate, already-resolved issue encountered while setting up this environment

Not the subject of this document, but recorded for completeness since it
occurred in the same session, immediately before the observations below.

The first `regtest-init.sh` attempt failed because `external/z3/.env.regtest`'s
`ZEBRA_MINING__MINER_ADDRESS` still held an Orchard-receiver Unified Address
left over from a prior session's `regtest-miner-setup.sh` run, pointing at a
`hot_wallet` account that no longer existed after the volume wipe. Mining to
that address at chain height 0 (before any Orchard anchor exists) caused Zebra
to panic rather than return a JSON-RPC error:

```
ERROR rpc_request{...rpc.method=generate...}: zebra_rpc::methods::types::transaction: Failed to add Orchard output: Cannot create Orchard transactions without an Orchard anchor, or before NU5 activation

thread 'tokio-rt-worker' (28) panicked at zebra-rpc/src/methods/types/get_block_template/zip317.rs:73:18:
valid coinbase transaction template: CoinbaseConstruction("Could not construct output with miner reward")
```

This was resolved by resetting `ZEBRA_MINING__MINER_ADDRESS` to the
placeholder value hardcoded in `scripts/dev/regtest-miner-setup.sh`
(`tmSRd1r8gs77Ja67Fw1JcdoXytxsyrLTPJm`) before re-running `regtest-init.sh`,
then running `scripts/dev/regtest-miner-setup.sh` to re-derive and set the
actual `hot_wallet` Orchard receiver, per that script's documented run order.
This resolved cleanly and is not connected to the issue below except by
occurring in the same session.

---

## Summary of the observed behavior

After the Z3 environment has completed at least one scenario run (creating
Zallet accounts and confirming some transactions), every subsequent attempt to
start a new scenario run against the same, restarted environment has failed
during setup with:

```
error: Setup error: failed to resolve hot wallet: funding step `z_listaccounts`: RPC response parse error: error decoding response body
```

This is a **different symptom** from the crash-loop documented in
[`zallet-restart-sync-failure.md`](zallet-restart-sync-failure.md). In every
instance observed here:

- The Zallet container remained up throughout — no crash, no restart, no
  container-status flapping.
- Zallet's own log shows its sync task progressing cleanly through every
  block and reaching the chain tip, with no error or warning logged at any
  point.
- The failure is a **parse** failure (`RpcError::Parse`, which the client
  displays as `RPC response parse error: {msg}` —
  `src/rpc/mod.rs:47,55`), not a JSON-RPC error response and not a transport
  failure (connection refused/timeout).

The setup code that failed already contains a retry loop written for a
related, previously-observed failure mode. The comment directly above it
(`src/scenarios/runner/lifecycle.rs:83-87`) reads:

> This is the FIRST Zallet call after stack start, so it retries transient
> failures: rpc-router can still be restarting (waiting on Zallet's
> `rpc.discover`) in the seconds after `wait_until_ready()` returns, which
> surfaces as transport errors or as unparseable (router-error) response
> bodies.

The retry loop attempts up to 12 times, 5 seconds apart
(`src/scenarios/runner/lifecycle.rs:88-112`, `attempts < 12`,
`sleep(Duration::from_secs(5))`). In every instance recorded below, **all 12
retries were exhausted** — every attempt failed with the identical parse
error, at fast, consistent latency, and the setup step then gave up and tore
down the stack.

---

## Sequence of actions and observations

All four instances below occurred in the same session, against the same
regtest environment (volumes preserved between them; only containers were
recreated by each restart), each following the pattern: stack already running
→ start a `z3sim run --scenario configs/scenarios/smoke.yaml` invocation →
setup fails as above → `setup()`'s own failure path stops the stack
(`docker compose down`, no `-v`) before the process exits.

A first, separate invocation (`experiments/runs/20260803T084825Z-smoke`,
against a genuinely fresh/empty wallet with no prior accounts) completed
successfully — 53 of 60 attempted transactions confirmed. All four instances
below occurred on subsequent invocations, once at least one account and some
transaction history already existed in the wallet.

| Instance | Run ID | `run_started_at` | `run_completed_at` | Duration |
|---|---|---|---|---|
| 1 | `20260803T090139Z-smoke` | 09:01:39.924661Z | 09:03:01.557051Z | 81.6 s |
| 2 | `20260803T090434Z-smoke` | 09:04:34.168944Z | 09:05:59.921549Z | 85.8 s |
| 3 | `20260803T090851Z-smoke` | 09:08:51.602824Z | 09:10:17.087937Z | 85.5 s |
| 4 | `20260803T091309Z-smoke` | 09:13:09.475537Z | 09:14:35.227974Z | 85.7 s |

All four durations cluster tightly around 85 seconds — consistent with the
12-attempt, 5-second-interval retry loop plus fixed setup overhead before the
first attempt, and largely **independent of how long the environment had
already been running** before the invocation (see Instance 4 below).

Before instances 2, 3, and 4, a manual readiness check was performed first
(`curl -u zebra:zebra -X POST ... z_listaccounts` against the router at
`http://127.0.0.1:8181`, polled every 2 seconds), and in every case this
manual check **succeeded** — returning a complete, valid JSON response
listing all existing accounts — before the `z3sim run` invocation was
started. The manual check succeeding did not prevent the subsequent failure.

### Instance 4 — the fully-logged instance

Zallet's container log was captured live (`docker logs -f z3-regtest-zallet-1`)
starting before the stack was brought up for this instance, so its full
timeline is available:

- Zallet's sync task logged continuous `Scanning block N` lines from block 6
  through block 133.
- `2026-08-03T09:12:29.747365Z` — Zallet logged `Reached chain tip, streaming
  mempool`.
- `2026-08-03T09:13:09.475537Z` — this instance's `z3sim run` invocation
  started (**44 seconds after** Zallet had already reached the chain tip).
- The run's `rpc_calls.jsonl` records 12 `z_listaccounts` attempts, each
  identical:

```
{"call_id":"z_listaccounts-0", ..., "request_at":"2026-08-03T09:13:13.810120Z","response_at":"2026-08-03T09:13:13.860955Z","latency_ms":50,"success":false,"error_code":null,"error_message":"RPC response parse error: error decoding response body"}
{"call_id":"z_listaccounts-1", ..., "request_at":"2026-08-03T09:13:18.862268Z", ..., "latency_ms":31, ...}
{"call_id":"z_listaccounts-2", ..., "request_at":"2026-08-03T09:13:23.896018Z", ..., "latency_ms":27, ...}
   ... (attempts 3 through 11, each ~5.03 seconds after the previous, latency 27-50 ms, identical error) ...
{"call_id":"z_listaccounts-11", ..., "request_at":"2026-08-03T09:14:09.264205Z", ..., "latency_ms":37, "error_message":"RPC response parse error: error decoding response body"}
```

  (Full file: `experiments/runs/20260803T091309Z-smoke/rpc_calls.jsonl`.)
- `2026-08-03T09:14:19Z` — Zallet's log records `Received SIGTERM, starting
  shutdown` (from the simulator's teardown after setup gave up), followed by
  `Shutting down Zallet`. No error, warning, or unusual log line appears in
  Zallet's own log at any point between reaching the chain tip and receiving
  SIGTERM.
- The RPC Router container's log (`docker logs -f z3-regtest-rpc-router-1`,
  captured over the same window) remained **completely empty (0 bytes)**
  throughout. Whether this means the router emits no application-level logs
  in this configuration, or logs were not captured for some other reason, was
  not determined.

Every attempt's latency (27-50 ms) is consistent with a normal, fast
round-trip through the router to Zallet and back — not a hang, not a timeout,
and not consistent with Zallet being mid-scan and slow to respond. Zallet had
already been idle at the chain tip for 44+ seconds before the first attempt in
this instance.

---

## Effect observed on the client side

Every occurrence produces the identical top-level error:

```
error: Setup error: failed to resolve hot wallet: funding step `z_listaccounts`: RPC response parse error: error decoding response body
```

This originates from `RpcError::Parse` (`src/rpc/mod.rs:46-47`), which is
returned when the HTTP response body could not be deserialized into the
expected JSON-RPC response type — not a JSON-RPC error object returned by the
server (`RpcError::JsonRpc`), and not a connection-level failure
(`RpcError::Transport`). The raw response body itself was not captured in any
instance; only the fact that deserialization failed is recorded.

A manual `curl` request for the identical method (`z_listaccounts`, empty
params array) against the identical endpoint, with the identical Basic Auth
credentials, made from the same host in the same time window as a failing
`z3sim run` attempt, **succeeded** and returned a complete, valid,
well-formed JSON response (one observed response was several kilobytes,
listing 11 accounts with nested address arrays — one account had 16 addresses
across transparent, Orchard, and unified-address entries).

---

## Recovery method used

No manual recovery of the *environment* was needed or attempted — in every
instance, `setup()` itself stopped the Docker Compose stack (`docker compose
down`, without `-v`) on the funding-resolution failure, before the process
exited (confirmed via `docker ps -a` showing no containers immediately after
each failure). The next `z3sim run` invocation was simply started again
against the same (down) stack, which brought the stack back up itself
(`Z3Stack::start()` calls `docker compose up -d`).

No instance in this investigation found a way to make a scenario run succeed
against an environment with pre-existing wallet/account history. All four
timed attempts, regardless of prior manual readiness checks or elapsed wait
time, failed identically.

---

## What has not been determined or tested

- **Why the manual `curl` request succeeds while the simulator's own
  `reqwest`-based client fails**, for what appears to be the identical method,
  params, endpoint, and credentials. This was observed directly (a successful
  manual curl call made in the same window as failing simulator attempts) but
  the reason for the difference — headers, HTTP version, keep-alive/connection
  reuse behavior, chunked transfer handling, timeout configuration, or
  something else — was not investigated.
- **The raw bytes of the response body that failed to parse.** Only the fact
  of a parse failure is logged (`error decoding response body`); no instance
  captured the actual bytes returned.
- **Whether the RPC Router is a contributing factor.** Its log was captured
  live in Instance 4 and was empty throughout; whether this reflects the
  router genuinely doing nothing notable, or a logging configuration gap, was
  not determined. The router was not bypassed (i.e., pointing the simulator
  directly at Zallet's own published port) to isolate whether the router
  specifically is implicated.
- **Whether Zallet's sync state is a factor at all.** The original code
  comment that added the 12×5s retry loop attributes this failure mode to the
  router "still restarting ... in the seconds after `wait_until_ready()`
  returns" — implying a short-lived (few-second) transient condition. Instance
  4 directly contradicts a *sync-lag* explanation specifically: Zallet had
  been idle at the chain tip for 44 seconds before the first of 12 failed
  attempts, each 5 seconds apart, spanning nearly a further minute — i.e., the
  condition (whatever it is) persisted for at least that combined ~104-second
  window in this instance, well beyond what the retry loop's own comment
  anticipates.
- **Whether the failure is deterministic given any pre-existing account, or
  depends on some threshold** (number of accounts, number of blocks, size of
  the `z_listaccounts` response, or elapsed wallet age). All four observed
  attempts occurred against a wallet holding at least 11 accounts (1 hot
  wallet + 10 from the first successful run) and a chain of 100+ blocks; no
  attempt was made against a restart with, say, only 1-2 pre-existing
  accounts.
- **Whether retrying beyond 12 attempts (or waiting longer than 60 seconds
  total) would eventually succeed.** Not tested — the current retry budget is
  fixed at 12 attempts and was not temporarily extended for investigation.
- **Whether the problem is specific to `z_listaccounts`**, or would also
  affect other Zallet RPC methods called at the same point in setup. Only
  `z_listaccounts` is called before this failure surfaces, so no other method
  was exercised in this state.
- **Whether restarting only the `zallet` container (or only the
  `rpc-router` container) rather than the full Compose stack changes the
  outcome.** Not tested.

---

## Related documented issues

- [`zallet-restart-sync-failure.md`](zallet-restart-sync-failure.md) — a
  related but distinct symptom: a genuine, permanent Zallet crash-loop after a
  restart, believed resolved by the `v0.1.0-beta.2` upgrade (commit
  `aa6ae42`). The issue in this document was observed entirely on
  `v0.1.0-beta.2`, using the same override stack, and was **not** accompanied
  by any crash-loop, container restart, or error in Zallet's own log — Zallet
  stayed up and synced cleanly in every instance observed here. Whether the
  two issues share a root cause upstream, or are unrelated, was not
  determined.
- `src/scenarios/runner/lifecycle.rs:83-112` — the existing retry loop and its
  comment, which already anticipated a related (but, per Instance 4, evidently
  not identical) failure mode and attempted to mitigate it.
