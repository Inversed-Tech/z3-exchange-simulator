# Funding Fan-Out Round Count Scales With Per-Account Intent Count — Pending Fix

**Status:** Root cause confirmed by direct arithmetic reconstruction against a live
incident. A partial, low-risk mitigation has been applied (see "What has been done"
below). The structural fix has not been implemented — this document exists to record
what was learned while evaluating it, including why two candidate approaches were
rejected, so that work is not repeated or redone carelessly.
**Applies to:** `src/scenarios/runner/lifecycle.rs`'s `compute_funding_plan` and
`src/scenarios/runner/funding.rs`'s `fund_accounts`. Not a Z3-stack issue — this is
entirely simulator-side design.

---

## The incident

Run `20260804T180454Z-ramp-fast` (15 accounts, `load_target_tps: 8.0`,
`load_duration_seconds: 100`, `--max-in-flight 4`), on simulator commit `cc0b949`
(after the funding-budget-mean and mining-top-up fixes were applied):

- Total run wall time: 31m 4s.
- The funding fan-out's 109 `z_sendmany` calls spanned 18:08:14–18:35:19 — **27
  minutes 5 seconds** — against a scenario configured for 100 seconds of load.
- Only 1 load-phase intent was ever dispatched, confirmed at the very end of the run,
  immediately after funding finished.
- Every `z_sendmany` call succeeded on its first attempt (109/109) — this was not a
  retry storm or a stuck operation. Funding worked correctly; it was simply slow.

## Root cause

`compute_funding_plan` (`lifecycle.rs`) sets:

```rust
transparent_outputs: per_account_intents as u32,   // as of this fix; was × 2 before it
```

where `per_account_intents = ceil(load_target_tps × load_duration_seconds / active_count)`.
For this scenario: `ceil(8.0 × 100 / 15) = 54` (the reported run predates this fix and
saw `54 × 2 = 108`, plus one; the discrepancy from the reported 109 was not chased
further since the order of magnitude is what matters here).

`fund_accounts` (`funding.rs`) can only mint **one transparent UTXO per sink per
round**: each round is a single `z_sendmany` transaction paying every sink's t-address
once, and `z_sendmany` rejects a transaction with a duplicate recipient address. To
give a sink N separate UTXOs, the fan-out needs N rounds — so `transparent_outputs`
is not just a value, it is directly the fan-out's **round count**. Rounds run
strictly sequentially: each waits for its own operation to reach `success` via
`z_getoperationstatus` before the next round starts, because concurrent sends *from
the same source address* risk racing Zallet's own note selection (the same failure
class as the "already-spent-input" rejections documented in
`z3-concurrent-request-ceiling.md`).

Per-round latency is dominated by the ~2s Orchard coinbase-proof cost per mined block
on this emulated host (`docs/regtest-funding-plan.md`) plus operation-poll overhead —
roughly 15s/round measured (27m5s ÷ 109 ≈ 14.9s). 108 rounds × ~15s lands almost
exactly on the observed 27 minutes.

**Why this was never seen before:** every scenario run prior to commits `02aef78` /
`cc0b949` failed the funding fan-out outright on insufficient balance (the bugs those
commits fixed), so the fan-out never ran long enough, on few-enough accounts, to
expose that round count itself — not aggregate value — was the next bottleneck.

## What has been done

Commit (this session): removed the unvalidated `× 2` headroom multiplier on
`transparent_outputs` in `compute_funding_plan`. This halves round count for every
scenario (108 → 54 for the incident above) with no change in *what* gets funded and
no new failure mode — it was pure, never-measured safety margin. See the
`ramp_fast_incident_round_count_is_halved` test in `lifecycle.rs` for the specific
regression check against this incident's numbers.

This is a partial mitigation. It does not change the fact that round count scales
linearly with `per_account_intents`, i.e. with the scenario's own
`load_target_tps × load_duration_seconds / active_count`. A scenario with fewer
accounts and/or a higher TPS-to-account ratio than this one will still spend a
correspondingly large amount of wall-clock time in setup before the load phase can
start.

## Structural approaches considered, and why they were not implemented

### Option A: derive several transparent addresses per account, batch multiple UTXOs per sink per round

The idea: give each sink N distinct transparent addresses instead of one, so a single
round can pay each sink N UTXOs (one per distinct address) instead of 1, cutting round
count by roughly N.

**Rejected as unsafe to implement without much more validation.**
`docs/zallet-transparent-gap-limit.md` measures that Zallet's transparent gap limit
(10 unfunded addresses per account) is consumed by derivation-index *jumps*, not one
index per call — its own measurement: "on an account with no funded address ...
two or three unfunded derivations put the next candidate index past 9." Deriving even
2-3 extra addresses per account to batch UTXOs risks `ReachedGapLimit` errors — a
failure mode the funding step has never had to handle, because today it only ever
uses the single address each account already has from creation. Implementing this
properly would require validating the actual, current jump behavior across many
accounts before trusting any specific batch size, which was judged too large a body
of new, unvalidated risk to take on as part of this fix.

### Option B: hard-cap `transparent_outputs` at a small constant, independent of scenario scale

The idea: bound round count (and therefore setup time) to a small constant no matter
how large `per_account_intents` gets, accepting that some late transparent-outbound
intents in a long/high-throughput run would fail once an account's pre-funded UTXOs
run out.

**Rejected because it would contaminate exactly the signal some scenarios exist to
measure.** `ramp.yaml` (and `steady_state.yaml`, `burst.yaml`, `mixed.yaml`) are
explicitly designed to find the TPS point where the Z3 stack's own confirmation rate
degrades under load. A hard cap on pre-funded UTXOs would inject an *unrelated*
failure ("ran out of our own pre-funded UTXOs") partway through a long run, which
would look identical in the report to a genuine stack-side degradation — without a
careful way to distinguish the two, this risks corrupting the very measurement these
scenarios exist to produce.

### Option C (sketched, not evaluated in depth): bounded concurrency across rounds

The idea: run a small, bounded number of rounds concurrently (mirroring the existing
`--provision-concurrency` / `--max-in-flight` pattern) instead of strictly
sequentially.

Not pursued: this directly reintroduces the same-source note-selection race
documented in `z3-concurrent-request-ceiling.md` (Measurement 2 — the
"already-spent-input" rejection class), and `send_with_anchor_retries`
(`funding.rs`) does not currently retry on that rejection message, only on
"Insufficient balance." Making this safe would require both careful concurrency
tuning (the ceiling doc's own measurements found even `--max-in-flight 8` produced a
meaningfully higher rejection rate than `--max-in-flight 2`) and extending the retry
logic — enough additional scope that it was left unexplored rather than attempted
partially.

## What a real fix likely needs

Not designed here, but the shape of the problem suggests the fix has to change *how*
transparent value reaches an account, not just *how much* is pre-funded upfront. Two
directions worth investigating before committing to one:

1. Whether Zallet's `z_sendmany` has any policy or configuration that returns
   transparent change to a transparent address instead of always routing it to the
   shielded pool (`docs/regtest-funding-plan.md`'s "a transparent spend consumes its
   whole UTXO and the change returns to the account's shielded pool" — stated there
   as the wallet's fixed change strategy, but not exhaustively checked against every
   available `z_sendmany` privacy policy). If change could stay transparent, an
   account would need far fewer pre-funded UTXOs — closer to O(1) than O(intents) —
   because a spent UTXO's change could fund the next spend.
2. Whether funding can be spread across the run instead of front-loaded entirely
   before the load phase starts — e.g. a low-rate background top-up of transparent
   UTXOs during the load phase itself, sized to keep pace with dispatch rather than
   pre-provisioning the run's entire lifetime demand at time zero. This changes the
   funding phase from a blocking setup step into an ongoing background process,
   which is a bigger architectural change than anything else in this document and
   was not scoped further here.

## Sources

- `experiments/runs/20260804T180454Z-ramp-fast/` — `rpc_calls.jsonl`,
  `intents.jsonl`, and the console summary this document's numbers are drawn from.
- `src/scenarios/runner/lifecycle.rs` — `compute_funding_plan`.
- `src/scenarios/runner/funding.rs` — `fund_accounts`, `send_with_anchor_retries`.
- `docs/zallet-transparent-gap-limit.md` — gap-limit measurements underlying the
  rejection of Option A.
- `docs/z3-concurrent-request-ceiling.md` — concurrency/rejection-rate measurements
  underlying the rejection of Option C.
- `docs/regtest-funding-plan.md` — the transparent-spend change-strategy note
  underlying direction 1 in "What a real fix likely needs."
