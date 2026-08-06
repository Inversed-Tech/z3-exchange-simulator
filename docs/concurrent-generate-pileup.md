# Concurrent `generate` Calls From Retry Loops Stall Zebra (and Everything Routed Through It)

This document records a specific, reproduced problem: during the load phase, latency
for `generate` and every other Zebra-routed RPC method (`getblockcount`,
`getrawtransaction`, `getrawmempool`) periodically spikes into the multi-second range,
degrading throughput without causing outright failures. It includes the root-cause
analysis and the fix applied, distinguishing directly observed facts from inference.

---

## Environment

- Deployment: the Z3 Docker Compose stack (`external/z3`), regtest network, override
  stack per `z3-commits.lock` (Zebra `v6.0.0`, Zaino `0.6.0`, Zallet `v0.1.0-beta.2`).
- Host: macOS, arm64 (Apple Silicon); these images run under x86_64 emulation.
- Run: `20260806T070613Z-smoke` (`configs/scenarios/smoke.yaml`, 10 accounts, 5 active,
  100% T→T, default `--max-in-flight 64`), simulator commit `25e10a0`.

## Summary of the observed behavior

`attempted=60 confirmed=47 (78.3%)`, exit code 0 — the confirm-rate floor is met and the
run exits cleanly, but the mechanical report flagged 8 High-severity latency/degradation
findings. Two match already-documented, expected behavior
(`docs/regtest-funding-plan.md`'s coinbase-maturity/anchor-wait window and the
`WalletDb::get_memo` defect). The other six do not match any existing doc:

- `getblockcount` P99 3833ms (958.2× its own P50 of 4ms)
- `getrawtransaction` P99 8583ms (4291.5× its own P50 of 2ms)
- `getrawmempool` P99 3757ms (626.2× its own P50 of 6ms)
- `z_gettotalbalance` P99 188ms (11.8× its own P50 of 16ms)
- `z_sendmany` P99 251ms (10.5× its own P50 of 24ms)
- A load-degradation window at +410s: 1.6 TPS, P99 18655ms, 0% error rate

## Root cause

Directly measured from `rpc_calls.jsonl`: every one of the run's 147 `generate` calls
took over 1 second, with a **median of 11.2s and a P99 of 25.7s** — nowhere near the
~2.2s/block baseline this codebase's own comments document for Orchard-coinbase mining
(`src/scenarios/runner/lifecycle.rs`). Cross-referencing timestamps shows exactly why:

- Every "Insufficient balance" `z_sendmany` failure (85 of them, the expected
  anchor-maturity condition above) is immediately followed, within milliseconds, by a
  `generate` call — `send_many_with_anchor_retries` in `src/scenarios/exchange.rs` called
  `rpc.generate(1)` on every retry, once per retrying intent.
- With `--max-in-flight 64`, many withdrawal intents hit "Insufficient balance"
  concurrently (a normal condition right after funding, not an edge case), and their
  independent retry loops each fired their own `generate(1)` call at overlapping times.
  Example: at 07:17:38.174Z a `generate` call starts (finishes ~17.9s later); at
  07:17:38.233Z a second one starts (finishes ~21.3s later); at 07:17:40.159Z a third
  (finishes ~23.0s later) — three genuinely concurrent `generate` calls, each taking far
  longer than a solo call would.
- While these piled up, every other Zebra-routed call stalled with them: at 07:17:47.243Z
  a `generate` call began (25.7s duration), and in the same instant 10 concurrent
  `getrawtransaction` calls all completed at near-identical latencies of 8549–8637ms —
  not a coincidence of independently slow calls, but all of them queued behind the same
  contended resource and released together. `getblockcount` (3833ms) and `getrawmempool`
  (3757ms) spiked in the same narrow window for the same reason.
- `process_cpu_percent` for `z3-regtest-zebra-1` (sampled via `docker stats` throughout
  the run) averaged 347% and peaked at 791% — multiple cores pegged, consistent with
  several concurrent halo2 proving operations competing for the same constrained,
  emulated CPU rather than one proceeding at a time.

The load phase already runs a dedicated `background_miner` task
(`src/scenarios/runner/dispatch.rs`) whose entire purpose, per its own doc comment, is
that "per-transaction `generate()` calls are not needed" — it mines one block every
`block_interval` (2s, fixed, not currently exposed via any CLI flag) for the whole load
phase, independently of any intent's outcome. `send_many_with_anchor_retries`'s own
`generate(1)` call was calling this again anyway on top of that, for every retrying
intent — not a rare correctness fallback, but a call that fires on the single most common
failure mode of the load phase, at whatever concurrency the scenario is running.
`run_sweep` (used by the ZToT flow, not exercised by this particular scenario but the
same defect) had the identical unconditional `generate(1)` call after every sweep
broadcast.

This is a **simulator bug**, not a Z3-stack defect: the redundant calls were entirely
within our control, the dedicated mechanism to avoid them already existed and is used
correctly elsewhere in the same file (`run_deposit`/`run_withdrawal`'s confirmation wait
calls neither of them), and removing them requires no coordination with, or change to,
any pinned component.

## Fix

`src/scenarios/exchange.rs`:
- `send_many_with_anchor_retries`: removed the `rpc.generate(1)` call between retries.
  The function now only sleeps between attempts, relying on `background_miner`'s already-
  running, independent 2s cadence to advance the chain — exactly the pattern
  `wait_for_tx_confirmations` already uses.
- `run_sweep`: removed the identical unconditional `generate(1)` call after broadcast, for
  the same reason.
- `run_withdrawal`'s doc comment, which claimed the function itself mines a block, was
  stale (the code never did this) — corrected while in the area.
- Four now-dead `generate`-method mocks in the test suite (three that exercised the
  removed calls, one pre-existing and already unused before this change) were removed.

This does not change how quickly funded accounts reach spendable anchor depth —
`background_miner` was already advancing the chain at the same fixed rate regardless of
whether the retry loop also called `generate()` — it only removes the redundant,
uncoordinated extra calls that were piling up against Zebra under concurrency.

## Z3-stack observation (flagged, not fixed)

Independent of the pileup above: even accounting for concurrency, the measured `generate`
costs are higher than this codebase's existing documentation assumes. The warmup phase's
`generate_in_chunks` calls (5 blocks each, no concurrency — warmup runs before
`background_miner` starts and before any load-phase intents exist) were measured at
~18–20s per 5-block chunk, versus the ~11s "worst case" already documented in
`lifecycle.rs`'s own comment (~2.2s/block × 5). This suggests the per-block Orchard-
coinbase proving cost on this host/emulation combination runs somewhat higher than
previously measured, independent of the concurrency bug above. This is a Z3-stack /
environment characteristic (Zebra's mining RPC under x86_64 emulation on this host), not
something addressable from the simulator side — flagged here for awareness, at Likely
(not Confirmed) confidence, since it wasn't isolated from ordinary run-to-run host
variance (thermal state, other load on the machine) in this investigation.

Separately: the mechanical report's severity criteria (P99/P50 ratio per method) did not
flag `generate` itself as anomalous, because `generate`'s P50 (11.2s) is itself already
enormous — the ratio (P99/P50 ≈ 2.3×) looks unremarkable next to `getrawtransaction`'s
4291×, even though `generate`'s absolute latency is the actual root cause. A ratio-only
threshold is blind to "uniformly slow," only to "usually fast, occasionally slow."

**Update:** fixed in `src/report/findings.rs`. `uniformly_slow_candidates` adds a second
check alongside the existing ratio-based one: any method whose median (P50) alone clears
a fixed 1000ms floor is flagged High, regardless of its P99/P50 ratio, and — unlike the
ratio check — regtest-control methods (`generate` and friends) are deliberately *not*
exempt from it, since a pathologically slow control-plane call is exactly the signal that
predicts (and, per this document, can directly cause) degraded latency for every other
RPC method sharing its backend. A method already flagged by the ratio check is not
flagged again by this one.

## Sources

- `experiments/runs/20260806T070613Z-smoke/{rpc_calls.jsonl,metrics.jsonl,component_logs/*.log}`
- `src/scenarios/exchange.rs`, `src/scenarios/runner/dispatch.rs`,
  `src/scenarios/runner/lifecycle.rs`
