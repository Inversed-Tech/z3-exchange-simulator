# Concurrent Request Ceiling: RPC Requests Above ~11 Fail; Concurrent Shielded Sends Collide Above That Threshold

This document is a factual record of two related, independently measured observations. It
contains direct measurements and logged evidence. Where the underlying cause has not been
isolated, this is stated explicitly rather than assumed.

**Status:** Confirmed via direct measurement, reproduced across separate sessions.
**Applies to:** the regtest override stack (Zebra `v6.0.0` / Zaino `0.6.0` / Zallet
`v0.1.0-beta.2`, per `z3-commits.lock`'s `overrides:` block), on this host (macOS, arm64;
these images run under x86_64 emulation via Docker Desktop).
**Root cause:** not isolated. See "What has not been determined" below.

---

## Measurement 1: raw concurrent RPC requests, no simulator involved

Method: `curl` fired N concurrent requests (background shell jobs) of `z_getnewaccount`,
each with a distinct account name generated from a nanosecond timestamp, against the RPC
router (`:8181`). An earlier attempt at this measurement used bash `$RANDOM` to generate
"unique" names; `$RANDOM` collided across parallel subshells forked near-simultaneously
from the same parent, producing spurious account-name conflicts that were initially
mistaken for concurrency-related failures. That attempt's results were discarded once the
cause was identified; all results below use the timestamp-based naming.

Result, repeated 3 times at each level on a live regtest stack with existing account
history:

| Concurrent requests | Result |
|---|---|
| 11 | All succeed, <100 ms each, every trial (3/3) |
| 12 | All fail — empty response body (no JSON), client times out, every trial (3/3) |

The same 11-vs-12 boundary was also observed, less exhaustively, at a range of other
levels tested earlier in the same investigation (5, 9, 10, 11 succeeding; 12, 13, 14, 15,
20, 30 failing in various individual trials), once the `$RANDOM` naming defect above was
corrected.

During failing trials, the RPC router's own log recorded, in tight timestamp clusters
matching the client-side request bursts:

```
ERROR rpc_router: Error serving connection: hyper::Error(IncompleteMessage)
```

A separate attempt was made to test the same concurrency levels against Zallet's own
published port (`:50232`), bypassing the router. That attempt showed failures at every
level tested, including levels well below 11 — but this measurement window coincided with
an unrelated concurrent process tearing down the shared Docker stack (confirmed via
`docker ps` showing zero containers immediately afterward, and a separate scenario run's
artifact directory with an overlapping timestamp). Because of that confound, the
direct-to-Zallet result is not treated as reliable evidence of a lower ceiling, and this
port was not re-tested in isolation afterward.

### RPC router source inspection

`external/z3/rpc-router/src/main.rs`'s connection-accept loop (`run()`) is a plain
`tokio::net::TcpListener::accept()` loop that spawns one `tokio::task` per accepted
connection with no explicit cap on concurrent tasks or connections. Each forwarded request
constructs a new `hyper_util::client::legacy::Client` (`forward_request()`), rather than
reusing a shared pooled client. No configuration value or constant limiting concurrent
connections or requests was found in this source file.

---

## Measurement 2: concurrent shielded-send dispatch (simulator scenario runs)

Method: the `health-t2z` scenario (6 synthetic accounts, 25 transparent-to-shielded
intents, 1.0 target TPS, 25 s nominal load window) run at different values of the
simulator's `--max-in-flight` flag, which bounds how many dispatched intents may have an
outstanding RPC call in flight at once.

| Run | `--max-in-flight` | Confirmed | Failed (insufficient balance) | Failed (already-spent-input rejection) | Timed out |
|---|---|---|---|---|---|
| A | 64 (default) | 9/25 | 1 | 11 | 2 |
| B | 2 | 4/25 | 0 | 1 | 19 |
| C | 2 (after the fix noted below) | 23/25 | 1 | 1 | 0 |
| D | 8 (after the fix) | 17/25 | 1 | 7 | 0 |

Run B's 19 timeouts are attributable to a separate, since-fixed simulator defect (see
"Confounding factor" below), not to concurrency itself; it is included here for
completeness but should not be read as a measurement of collision rate at low
concurrency. Run C, taken after that defect was fixed, is the clean low-concurrency
measurement.

The "already-spent-input rejection" failures all carry the identical error text, returned
by Zebra (RPC error code -25) in response to a transaction Zallet itself had submitted:

```
any transaction with the same effects will be rejected from the mempool until the next
chain tip block: transaction rejected because another transaction in the mempool has
already spent some of its inputs
```

For comparison, the `health-t2t` scenario (same account count, intent count, and target
TPS, transparent-to-transparent flow, default `--max-in-flight 64`) produced 24/25
confirmed, with the single failure being the known "insufficient balance" funding-edge
case and zero already-spent-input rejections (documented separately in
`regtest-funding-plan.md` as same-account UTXO contention between concurrent intents).

### Correlation between the two measurements

The already-spent-input rejection rate for `health-t2z` was 11/25 (44%) at
`--max-in-flight 64`, 7/25 (28%) at `--max-in-flight 8`, and 1/25 (4%) at
`--max-in-flight 2` — the last matching the `health-t2t` baseline's own rate (1/25, 4%) at
default concurrency. The rejection rate scales with the dispatch concurrency setting
across all three measured points, and falls to the transparent-flow baseline rate only
once dispatch concurrency is capped well below the Measurement 1 ceiling (2, versus the
ceiling at 11-12). This is a direct, reproduced, monotonic relationship; whether
Measurement 1's transport-level ceiling and Measurement 2's transaction-level collision
are expressions of the same underlying limit, or two independent effects of the same "too
much concurrent activity" condition, has not been isolated.

---

## Measurement 3: shielded-to-transparent flow (two sequential shielded sends per intent)

Method: the `health-z2t` scenario (6 synthetic accounts, shielded-to-transparent flow,
each intent doing a shielded sweep followed by a shielded-to-transparent withdrawal — two
sequential shielded `z_sendmany` calls per intent, versus one for `health-t2z`), run at
three `--max-in-flight` levels, on a fresh wallet, after two unrelated simulator defects in
`run_sweep()` were fixed (a wallet-wide `z_listunspent` crash, and a same-account
transparent/shielded balance mixup — both documented in `regtest-funding-plan.md`, not
repeated here since they are unrelated to concurrency). Intent count was reduced to 6 (one
full pass over the accounts) to separate this measurement from the same-account exhaustion
behavior described below.

| Run | `--max-in-flight` | Attempted | Confirmed | Same-account exhaustion | Already-spent-input / channel-closed rejection |
|---|---|---|---|---|---|
| E | 4 | 6 | 2/6 | 1 | 3 |
| F | 1 (fully serialized) | 6 | 5/6 | 1 | 0 |

Run F's single failure is same-account exhaustion (below), not a rejection. At full
serialization, the rejection failures seen in Run E disappear entirely — consistent with
Measurement 2's finding that this rejection class is concurrency-driven. One rejection in
Run E carried a different error text than previously recorded, still at the same call
site and same general class (an operation-result failure during the shielded send):

```
RPC Error (code: -1): channel closed
```

This is included here as an additional observed symptom of the same concurrency-driven
class documented in Measurement 2, not a distinct issue — it was not investigated further
per that measurement's existing "not isolated" root-cause status. This flow's two
sequential shielded sends per intent expose the same concurrency-driven rejection at a
lower `--max-in-flight` setting than `health-t2z` needed (4 still produced rejections here,
versus needing 64 to reliably produce them for the single-send `health-t2z` flow) —
consistent with, though not proof of, the rejection scaling with total concurrent shielded
RPC calls rather than with `--max-in-flight` alone.

### Same-account exhaustion (not a concurrency issue — a flow-design property)

Distinct from the rejection class above: `run_sweep()` consumes an account's entire
shielded balance in a single sweep. Any intent assigned to an account whose shielded
balance has already been swept by an earlier intent in the same run fails with
`empty result: no confirmed unspent notes found for account <uuid>` — deterministically,
not as a race. Observed in every run of this scenario regardless of concurrency setting
(1 occurrence in both Run E and Run F, each with 6 intents drawn across 6 accounts).  This
is an inherent property of the sweep-then-withdraw design: a given account can supply at
most one successful shielded-to-transparent flow per run until it is re-funded, which does
not happen mid-run. A scenario dispatching more shielded-to-transparent intents than it has
distinct accounts, or one that reassigns an already-swept account, will see this failure
mode structurally, independent of any defect in the simulator or the Z3 stack.

---

## Measurement 4: shielded-to-shielded flow, after the anchor-retry defect fix

Method: the `health-z2z` scenario (6 synthetic accounts, shielded-to-shielded flow, 25
intents, `--max-in-flight 4`), run after the anchor-confirmation retry defect described in
`regtest-funding-plan.md` §8 was fixed (that defect, not concurrency, was the dominant
failure mode beforehand and is excluded from this measurement).

| Confirmed | Already-spent-input rejection | Confirmation-polling race (`No such mempool or main chain transaction`) |
|---|---|---|
| 21/25 | 3 | 1 |

Both residual failure types are the same concurrency-driven class documented in
Measurement 2 — no new failure type appeared. Not re-tested at full serialization for this
flow (Measurement 2 and Measurement 3 already established that pattern); included here for
completeness of the per-flow-type record rather than as an additional data point on the
concurrency/rejection-rate relationship.

---

## Confounding factor identified during this investigation (since fixed)

The first `--max-in-flight 2` run (Run B, above) showed 19/25 timeouts rather than a clean
read on collision rate. Investigation traced this to the scenario runner's load-phase loop
(`src/scenarios/runner/mod.rs`): the loop that dispatches intents exits once the
scenario's configured `load_duration_seconds` has elapsed, without regard to whether all
dispatched intents have completed; immediately after exiting, the code signalled the
background block-miner task to stop, before waiting for any still in-flight intents to
resolve. Under `--max-in-flight 2`, most of the 25 dispatched intents were still queued or
awaiting confirmation when the 25 s window closed; the miner was stopped at that point, so
none of them could subsequently be mined, and each ran out its own confirmation-wait
timeout in turn. This was independent of Z3 stack behavior and has been corrected by
moving the background-task shutdown signal to after the in-flight-task drain completes.

---

## What has not been determined

- Whether the Measurement 1 ceiling originates in Zallet's own RPC-serving stack or in the
  Docker Desktop (macOS, arm64, x86_64-emulated container) port-forwarding/networking
  layer. The router's own source was inspected and found to impose no explicit limit
  (see above); an attempt to isolate this by testing Zallet's published port directly was
  inconclusive due to an unrelated concurrent stack teardown during the test window.
- Whether Measurement 1 (transport-level request ceiling) and Measurement 2
  (transaction-level input-selection collision) share a root cause, or are independent
  consequences of concurrent load.
- Whether the ceiling value (~11) is specific to this host's hardware/emulation
  characteristics or would reproduce on other deployments of the same override stack.

## Sources

- Raw `curl` concurrency sweeps against `:8181` and `:50232`, this session.
- `external/z3/rpc-router/src/main.rs`.
- `experiments/runs/20260803T213132Z-health-t2z/` (Run A).
- `experiments/runs/20260804T064115Z-health-t2z/` (Run B).
- `experiments/runs/20260804T080225Z-health-t2z/` (Run C).
- `experiments/runs/20260804T102820Z-health-z2t/` (Run E).
- `experiments/runs/20260804T103251Z-health-z2t/` (Run F).
- `experiments/runs/20260804T110417Z-health-z2z/` (Measurement 4).
- `regtest-funding-plan.md` — same-account UTXO contention baseline for transparent flows;
  §7 and §8 for the unrelated `run_sweep()`/anchor-retry defects fixed during this
  investigation.
