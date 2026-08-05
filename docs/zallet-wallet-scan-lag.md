# Funding Fails With a Transaction-Expiry Consensus Rejection After Chain History Accumulates

This document records a specific, reproduced problem: the account-funding step of
`ramp-mini` (and, by mechanism, any scenario) fails during setup with a
consensus rejection on the funding transaction's expiry height, never reaching
the load phase. It includes the root-cause analysis and the fix applied,
distinguishing directly-observed facts from inference.

---

## Environment

- Deployment: the Z3 Docker Compose stack (`external/z3`), regtest network.
- Override stack per `z3-commits.lock`: Zebra `v6.0.0`, Zaino `0.6.0`, Zallet
  `v0.1.0-beta.2`.
- Both observed runs below were against the same, long-lived regtest Docker
  volumes — accumulated from every scenario attempt run against this
  environment since 2026-08-04, never wiped (`docker compose down`, no `-v`,
  between runs).

## Summary of the observed behavior

`configs/local/ramp-mini.yaml` (6 accounts, 100% T→T, `warmup_blocks: 20`),
run via `z3sim run --scenario configs/local/ramp-mini.yaml --load-shape ramp
--ramp-secs 25 --max-in-flight 2`, fails during account funding with:

```
error: Setup error: failed to fund synthetic accounts: Setup error: funding
step `wait_operation`: code -4: SendTransaction: Transaction commit failed::
chain backend error: unexpected error response from server: RPC Error (code:
-25): failed to validate tx: WtxId("private"), error: transaction did not
pass consensus validation: transaction must not be mined at a block
Height(N) greater than its expiry Height(M), failing transaction
transaction::Hash(...)
```

Observed twice, ~3 hours apart, against the same environment:

| Run | `run_started_at` | Chain height at rejection (`N`) | Tx expiry (`M`) |
|---|---|---|---|
| `20260805T160530Z-ramp-mini` | 16:05:30Z | 1697 | 1199 |
| `20260805T190012Z-ramp-mini` | 19:00:12Z | 1787 | 1199 |

`attempted=0 confirmed=0`, exit code 1, in both cases — the load phase is
never reached.

## Root cause

Every `z3sim run` restarts the Zallet container (`Z3Stack::start()`/`stop()`,
`src/z3/mod.rs`, `docker compose down`/`up -d` with no `-v` — the Docker
volumes, including the wallet database and chain state, persist across
restarts by design). On each restart, Zallet's `steady_state` sync task must
rescan from its last on-disk checkpoint up to the live chain tip, one block at
a time, at a fixed ~1.6–2.2s/block trial-decryption cost (independent of how
fast those blocks were originally mined). Zallet computes a new transaction's
safety parameters — including its expiry height — from this wallet-scan
position, not from the live chain tip reported by Zebra.

Directly confirmed from `component_logs/zallet.log` in both run directories:
at the moment each run's fan-out `z_sendmany` was built and rejected, the
wallet's scan was stalled at **block 892**, with the **identical block hash**
in both runs (`5e92336354dd61b2c50d9ae238cde5583aa0d2ae1ed67cb90fbe115d30c6fd50`)
despite the runs being three hours apart, against different real chain
heights, and each starting from a genuinely fresh Zallet process (`Latest
block height is 735` logged at container start in the second run). This is
not a coincidence or a stuck/frozen value — it is the same slow linear rescan
hitting the same wall: both runs reach the fan-out step at a similar
wall-clock offset from stack start, and the wallet's from-scratch rescan is
at a similar position at that offset in both cases, because the underlying
on-disk checkpoint (735 in the second run) was already ~1000 blocks behind
the live tip (1736) *before either run mined anything itself* — that
pre-existing gap, not either run's own mining, dominates.

The chain's live tip has grown large enough (from every scenario attempt
since 2026-08-04, none of which reset the volumes) that a fresh container's
catch-up scan can no longer complete inside any single scenario's time
budget. Every transaction built while the scan is still hundreds of blocks
behind is doomed at construction time, regardless of retries: retrying only
buys the scan a few more seconds (a handful of blocks) against a gap in the
hundreds that itself grows on every subsequent attempt.

This is a distinct symptom of the same class of problem flagged as an open
question in
[`zallet-restart-sync-failure.md`](zallet-restart-sync-failure.md) ("Whether
the crash-loop occurs ... given the same or a similar prior transaction
history") — an ever-growing, never-reset regtest environment eventually
breaking a fresh container's ability to catch up, manifesting here as an
expiry rejection rather than a crash-loop.

## What did not fix it

An initial fix (commit `2cb3501`) added a retry that resubmitted the funding
operation from scratch up to 3 times on this exact error, on the theory that
resubmitting would pick up a caught-up wallet-view height. Re-run against the
same environment (`20260805T190012Z-ramp-mini`), the retry visibly engaged —
3 resubmissions logged — but every attempt carried the *identical* expiry
height 1199, and the run failed identically, only ~50s slower. The retry
interval (~8–12s apart) is far too short relative to both the scan's
throughput and the gap's size for resubmission to ever help; it was reverted.

## Resolution

1. **`src/scenarios/runner/funding.rs`**: reverted the ineffective retry.
   `wait_operation` now annotates this specific error (matched on the stable
   "greater than its expiry Height" clause) with the likely cause and the
   fix, instead of retrying — a diagnostic, not a workaround.
2. **`scripts/dev/regtest-reset.sh`** (new): packages the already-validated
   recovery sequence from `zallet-restart-sync-failure.md`'s "Recovery method
   used" section (`down -v`, `regtest-init.sh`, `regtest-miner-setup.sh`,
   `up -d`) into one command, so resetting the environment doesn't depend on
   remembering the right order of three separate scripts.
3. **Guidance**: the README and the message to scenario operators now call
   out that this environment must be periodically reset (or reset
   immediately after this specific error) — the accumulation is monotonic
   and does not resolve itself by running more scenarios.

## What has not been determined

- The precise Zallet-internal computation that turns "wallet-scan position"
  into a specific expiry height (i.e. whether it is exactly `scan_height +
  expiryDelta` or draws from a coarser checkpoint) was not pinned down —
  Zallet's source was not available for inspection in this investigation
  (only the pinned Docker image). This does not affect the fix: either way,
  the wallet-scan/chain-tip gap is the actual mechanism, and closing it
  requires a reset, not a code-level workaround on the client side.
- Whether a *code-level* mitigation — throttling the funding pipeline's own
  `generate()` bursts (`ensure_funded`, `warmup`) to Zallet's own reported
  sync progress — would meaningfully help was not implemented: the dominant
  contributor to the gap in both observed runs was pre-existing history from
  before the run started, not that run's own mining (~51 of ~1050 blocks),
  so this would only be a partial mitigation. No RPC exposing Zallet's scan
  progress was confirmed to exist (`getwalletinfo`'s response is only
  partially parsed by this client, `src/rpc/mod.rs::WalletInfo`) — would need
  live investigation against a running stack to pursue further.
