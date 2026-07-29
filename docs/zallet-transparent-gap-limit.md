# Zallet's Transparent Address Gap Limit at Exchange Scale

**Status:** Analysis, updated with measurements. Corrects an earlier, inaccurate claim in
this repo's source comments; the discrepancy that the first revision left unresolved has
since been **resolved by measurement on `v0.1.0-beta.1`** (2026-07-29; see
[`regtest-funding-plan.md`](regtest-funding-plan.md) for the probe evidence).
**Applies to:** Zallet `v0.1.0-alpha.3` (our pin) through `v0.1.0-beta.1`; `librustzcash` `main`
**Relevant to:** per-account transparent deposit addresses, TToT/ZToT flow fidelity

---

## Summary

| Question | Answer |
|---|---|
| What is the limit? | `GapLimits { external: 10, internal: 5, ephemeral: 10 }` |
| Scope of the limit | Per **(account, key scope)** — measured on beta.1: failures interleave per account; [resolved](#the-discrepancy--resolved) |
| What advances the gap | Consecutive derived addresses with no output in a **mined** transaction |
| What resets it | A mined UTXO to a derived address slides the window forward |
| Configurable? | **No.** Zallet never calls `WalletDb::with_gap_limits`; no `zallet.toml` key exists |
| Is it *the* blocker for per-account transparent addresses? | No — Zallet's client-side coinbase policy and alpha.3's spend policy block them regardless; see [The real blocker](#the-real-blocker-is-wallet-policy-not-the-gap-limit) |
| Why did fresh accounts hit "index 10"? | Derivation-index jumps: each derive lands on the next *Sapling-valid* index, so an unfunded account's 0–9 window is gone in 2–3 calls; [resolved](#the-discrepancy--resolved) |

---

## Correction of a previous claim — and an unresolved discrepancy

Three sites in this repo asserted a wallet-global gap counter:

- `src/rpc/mod.rs` — "across the wallet" (introduced in `6e1770e`)
- `src/scenarios/runner/mod.rs` — "resets Zallet's transparent gap counter and allows all N
  synthetic accounts to receive transparent receivers" (`6e1770e`)
- `src/scenarios/runner/provisioner.rs` — "across the whole wallet" (expanded in `86ebe33`)

**Per current librustzcash source, "across the wallet" is wrong**: the window is maintained
per `(account_id, key_scope)`. All three comments have been corrected.

### The discrepancy — resolved

Current upstream source says per-account. But this repo holds a run artifact that a naive
per-account reading cannot explain —
`experiments/runs/20260630T131145Z-smoke/rpc_calls.jsonl` (simulator commit `4774d684`):

```
z_listaccounts                       ok      (hot_wallet found — volumes were not fresh)
z_getnewaccount        × 9           ok
z_getaddressforaccount × 7           ok
z_getaddressforaccount               ERR: reached the transparent gap limit while
                                          attempting to generate a new address at index 10.
```

At that commit `z_get_address_for_account` took no `receiver_types` (so Zallet's default
set, which includes `p2pkh`, was used) and no diversifier index, and `provision()` ran
*before* the hot-wallet derivation — so every one of those calls targeted a **freshly
created** account. Under a strict per-account window, each fresh account would derive at
index 0, and "index 10" should be unreachable.

**Resolution (measured on `v0.1.0-beta.1`, 2026-07-29).** The per-account model is
correct; what the naive reading missed is the *derivation step size*. A
`z_getaddressforaccount` call without an explicit diversifier index derives at the next
**Sapling-valid** diversifier index, and only about half of all indices have a valid
Sapling diversifier, so the index advances in jumps (observed: 0 → 1, 2, 4, 6, 9, 14 …
varying per account key). On an account with **no funded address**, `gap_start` stays at 0
and the transparent window is indices 0–9 only — so **two or three unfunded derivations
put the next candidate index past 9**, and the call fails with exactly the recorded error:
"reached the transparent gap limit … at index 10".

Observed live during the fan-out probe runs: after one explicit derivation per account in
one run, the *next* run's single derivation failed at index 10 on two of five fresh
accounts, while derivations on other accounts (and on the funded source account, whose
window had slid forward) kept succeeding. Failures interleaving with successes across
accounts is per-account scoping, measured — the decisive experiment sketched in the first
revision of this document effectively ran itself.

For the June artifact specifically, the derivation-jump mechanism (possibly compounded by
address rows left by earlier runs on the reused volumes) accounts for "index 10" on fresh
accounts. No wallet-global counter is needed to explain it, and the librustzcash history
below shows the global constant was removed well before our pin.

### What this means for the claims in this document

The upstream-source claims below (values, per-scope filtering, reset semantics) are
confirmed against librustzcash `main` **and now against measured behaviour**. The practical
rule for any client: **derive an address once and reuse it** (read existing addresses back
from `z_listaccounts`; account creation already generates the diversifier-0 address with
every receiver type). One `p2pkh` receiver per account is safe under that rule — it is
*repeated* derivation on unfunded accounts that is unsafe, and it fails within a handful of
calls, not at the 10th account.

> The `docker compose down -v` reset that recovered this state is recorded in the
> maintainer's working notes, not in any repo script or doc. The subsequent run
> (`experiments/runs/20260630T131407Z-smoke/`) shows 11 successful derivations, consistent
> with a volume reset.

---

## What the limit actually is

From [`zcash_keys/src/keys/transparent/gap_limits.rs`](https://github.com/zcash/librustzcash/blob/main/zcash_keys/src/keys/transparent/gap_limits.rs):

```rust
impl Default for GapLimits {
    fn default() -> Self {
        Self { external: 10, internal: 5, ephemeral: 10 }
    }
}
```

- **external = 10** — normal receiving (deposit) addresses
- **internal = 5** — change addresses
- **ephemeral = 10** — ZIP-320 / TEX addresses

BIP-44's classic gap limit is 20. librustzcash deliberately uses less, and says why:

> we don't want to use the full 20-address gap limit space because it's possible that in
> the future, changes to the light wallet protocol will obviate the need to query for
> UTXOs in a fashion that links those addresses to one another.

So the limit is a **privacy and light-client-scanning tradeoff**, not a consensus rule and
not a storage constraint.

### Scope: per account, per key scope

In [`zcash_client_sqlite/src/wallet/transparent.rs`](https://github.com/zcash/librustzcash/blob/main/zcash_client_sqlite/src/wallet/transparent.rs), the gap queries
(`find_gap_start`, the `v_address_first_use` view) filter on **both** `account_id` and
`key_scope`, and `GapLimits::limit_for(scope)` returns a per-scope value. Each
(account, scope) pair carries an independent window.

History in `zcash_client_backend/CHANGELOG.md` confirms the direction of travel: a single
global `GAP_LIMIT` constant was added in 0.15.0, moved to `zcash_keys` in 0.21.0, and
**removed** in 0.22.0 — "gap limits are now configured based upon the key scope that
they're associated with; there is no longer a globally applicable gap limit."

### What advances and what resets the window

The window is anchored to the last address index that received an output in a **mined**
transaction. From `utxo_query_height`:

> We must start looking for UTXOs for addresses within the current gap limit as of the
> block height at which they might have first been revealed. This would have occurred
> when the gap advanced as a consequence of a transaction being mined.

- **Advances:** each consecutive derived-but-never-funded index.
- **Resets:** a mined UTXO to a derived address. `update_gap_limits` →
  `generate_gap_addresses` marks the index used and pre-generates another gap-limit worth
  of addresses beyond it.
- **Does not reset:** merely handing out or reserving an address; a mempool-only receipt.
  Confirmation is required.
- Exhaustion surfaces as `SqliteClientError::ReachedGapLimit`, which Zallet maps to
  "reached the transparent gap limit while attempting to generate a new address at index {index}."

**Consequence: a fund-as-you-derive strategy is not gap-limited.** Only *consecutive
unfunded* indices are bounded. What you cannot do is pre-generate a long run of addresses
and leave them all unfunded.

One caveat, verified in [zallet#637](https://github.com/zcash/zallet/issues/637): a given
(account, scope)'s address rows are pre-generated **at account-creation time** using
whatever gap limit was active then, and later reservations only select among existing
rows. A raised limit therefore cannot retroactively unstick an already-exhausted account.

---

## The real blocker is wallet policy, not the gap limit

Even setting the gap limit aside, there is a second, independent blocker that no gap-limit
change would lift. The first revision of this document attributed it to consensus; the
measurements corrected that in two steps (full trail in
[`regtest-funding-plan.md`](regtest-funding-plan.md)):

**On mainnet it is a consensus rule.** Transparent coinbase outputs may only be spent by a
transaction with **no transparent outputs** (protocol spec: "A transaction with one or more
transparent inputs from coinbase transactions MUST have no transparent outputs");
[ZIP-213](https://zips.z.cash/zip-0213) narrowed it to *transparent* coinbase but did not
remove it, and a 100-block maturity requirement applies separately.

**On regtest that consensus rule is off** — Zebra hardcodes the exemption in
`Parameters::new_regtest()` (`.with_unshielded_coinbase_spends(true)`, not configurable),
so mature transparent coinbase is consensus-legal to spend to transparent outputs on our
chain.

**But Zallet enforces the rule client-side anyway, on every version.** Measured on beta.1:
209 mature coinbase UTXOs on the sending account, and every `from` form is refused with
"Insufficient balance (have 0)" — the proposal engine excludes transparent coinbase from
transparent-output proposals regardless of network. For an exchange simulation this is
arguably the *right* behaviour to exercise (mainnet has the consensus rule), but it means:

```
mine → wait 100 blocks (maturity) → z_shieldcoinbase → z_sendmany from the shielded balance
```

or, cheaper on regtest — skip transparent coinbase entirely (measured 12/12 in
`scripts/experiments/fanout-probe.sh`, and directly spendable with no maturity and no
shielding step when NU6.2 is active):

```
mine to the hot wallet's ORCHARD receiver → z_sendmany fan-out to per-account t-receivers
```

`z_shieldcoinbase` first appears in Zallet `v0.1.0-alpha.4`; our pin is `alpha.3`, which
does not expose it at all (and, per alpha.4's changelog, did not even record `tx_index`
for coinbase transactions, so coinbase outputs could not be identified).

Separately, `z_sendmany` on our pin could **never** select a transparent input: per the
`v0.1.0-beta.1` changelog it "passed a shielded-only spend policy to the proposal builder."
That is the true root cause of the failure recorded in
[`zallet-transparent-spending-bug.md`](zallet-transparent-spending-bug.md), and it is fixed
in beta.1 — confirmed by measurement: on beta.1 a sink account spent its (non-coinbase)
transparent UTXO with a bare t-addr `from` (fan-out probe step 12).

Also relevant for t→t payouts, now measured: a **UA `from` draws shielded funds only**
([zallet#644](https://github.com/zcash/zallet/issues/644), by design), and a bare t-addr
`from` draws that address's own UTXOs — sufficient for per-account spending, so the
`legacy_pool_seed_fingerprint` workaround is only needed to *gather across* multiple
t-addrs in one transaction.

---

## Is the limit configurable?

**No, not from outside Zallet.**

- `zcash_client_sqlite` exposes `WalletDb::with_gap_limits(GapLimits)`, but Zallet always
  builds via `WalletDb::from_connection` (`zallet-core/src/components/database/connection.rs`),
  so it is stuck on `GapLimits::default()`.
- `GapLimits::new(external, internal, ephemeral)` is gated behind librustzcash's
  `unstable` / `test-dependencies` features, with a note that "the gap limits recommended
  for use with this crate are supplied by the `Default` implementation."
- No `gap`-related key exists in `zallet.toml`. The `[note_management]` section offers only
  `min_note_value` and `target_note_count`.
- [zallet#638](https://github.com/zcash/zallet/pull/638) proposed
  `note_management.transparent_*_gap_limit` threaded through to `with_gap_limits`. It was
  **closed unmerged** (2026-07-20) in favour of an upstream root-cause fix,
  [librustzcash#2666](https://github.com/zcash/librustzcash/pull/2666), which stopped
  marking never-exposed ZeWIF addresses as exposed.

The maintainer question on [zallet#637](https://github.com/zcash/zallet/issues/637) —
"Do we still want configurable gap limit or does it classify as footgun material?" — has
**no recorded answer**. Configurability is unresolved, not rejected on the merits.

---

## Is this a fundamental limitation?

No. It is a conservative default protecting a light-client privacy property, applied
uniformly to a workload it does not fit.

The scanning concern is real: for each address inside the gap window a light client issues
UTXO queries, and querying a batch of addresses together lets the server link them as
belonging to one wallet. Keeping the window small keeps that linkage set small. That
reasoning is sound for an interactive personal wallet.

It does not transfer to an exchange. An exchange:

- already publishes the association between its deposit addresses and itself, so the
  linkage the gap limit protects is not a property it holds;
- runs its own indexer (or a trusted one), so the untrusted-server threat model does not apply;
- needs address counts in the 10⁵–10⁶ range, and needs to issue deposit addresses **before**
  they are funded — precisely the pattern the limit forbids, since the window only advances
  on confirmed receipt.

The last point is the sharp edge: `external = 10` means an exchange can have at most 10
outstanding un-funded deposit addresses per account. Deposit addresses are by nature
issued in advance and may never be used. Per-account partitioning mitigates this
(one account per customer), but at the cost of one account per customer in a wallet whose
transparent performance at scale is itself an open concern
([librustzcash#2470](https://github.com/zcash/librustzcash/issues/2470)).

That upstream issues #2470 (transparent wallet performance at hundreds of thousands of
addresses) and #2473 (`find_gap_start` recomputing a window aggregation per UTXO on
ingest) are open and recent suggests the exchange-scale transparent path is known to be
immature rather than deliberately closed.

---

## Recommendation to the Zallet team

Draft — for the Foundation to forward or file upstream if it agrees.

1. **Expose the gap limits as configuration.** Revive the shape of
   [PR #638](https://github.com/zcash/zallet/pull/638):
   `note_management.transparent_external_gap_limit` and siblings, wired to
   `WalletDb::with_gap_limits`. #638 was closed because the *migration* bug it worked
   around had an upstream root-cause fix — that does not address the independent
   exchange-scale need. Two things would help this land: guard it behind an
   explicitly-named opt-in so it reads as a deliberate choice rather than a footgun, and
   document that it must be set before account creation (per #637, rows are pre-generated
   at account-creation time). This also requires promoting `GapLimits::new` out of
   librustzcash's `unstable` feature gate.

2. **Let a caller declare which indices to watch.** More precise than raising a limit: an
   API to register explicit transparent address indices (or ranges) for tracking,
   independent of the consecutive-unfunded window. An operator who issues deposit
   addresses knows exactly which indices it handed out. This removes the guesswork the gap
   heuristic exists to perform, rather than widening it. The privacy tradeoff becomes an
   explicit, caller-owned decision.

3. **Decide the "footgun" question explicitly.** The unanswered maintainer question on #637
   leaves integrators with no guidance and no supported path. Either answer is actionable;
   silence is not. If the answer is no, the recommended pattern for high-volume transparent
   deposit addresses should be documented instead.

4. **Distinguish the errors.** `ReachedGapLimit` currently surfaces to operators as an
   opaque address-generation failure. An exchange hitting it needs to know whether the
   cause is an un-funded backlog, an out-of-order scan, or an exhausted pre-generated row
   set — the three have different remedies. See
   [librustzcash#2594](https://github.com/zcash/librustzcash/issues/2594).

5. **Give `z_getaddressforaccount` a read-only sibling.** The RPC always *derives*, at the
   next Sapling-valid diversifier index — so a client that calls it to "get the address"
   walks the transparent gap window in jumps and exhausts an unfunded account within a few
   calls (measured; this was our June failure). The workaround is scraping
   `z_listaccounts`' `addresses` array. An explicit "return the existing address at the
   lowest (or a given) diversifier index" API — or documenting the derive-on-every-call
   semantics prominently — would remove the footgun.

---

## Implications for this simulator

1. The orchard-only derivation in `provisioner.rs` is retained for now: on the `alpha.3`
   pin the funding path to a transparent recipient does not exist (no `z_shieldcoinbase`,
   and `z_sendmany` cannot select transparent inputs). The gap-limit question is resolved:
   per-account `p2pkh` receivers are safe **if derived once and reused** — the planned
   replacement is `funding::resolve_account`, which reads the account's creation-time
   address instead of deriving.
2. TToT/ZToT "transparent" recipients currently settle in the **shielded** pool, so pool
   attribution for those flows is optimistic. `provisioner.rs` stores the *same*
   orchard-only UA string for both the `Transparent` and `Orchard` address entries; the
   `KNOWN LIMITATION` annotation is on the `Transparent` entry, with a cross-reference from
   the derivation site.
3. True TToT/ZToT fidelity requires the beta.1 override stack (see
   [`regtest-funding-plan.md`](regtest-funding-plan.md) — Zebra ≥ 6.0.0 is required by
   beta.1, and a fresh datadir by alpha.4's DB format break), and sending to the UA's
   *transparent receiver* rather than the UA itself (paying a UA settles shielded). The
   `z_shieldcoinbase` / `z_listunifiedreceivers` / policy-aware `z_sendmany` wrappers now
   exist in `src/rpc/mod.rs`, and `src/scenarios/runner/funding.rs` implements the fan-out;
   wiring it into the runner is the next PR.
4. **Latent address accumulation.** Two call sites still omit `diversifier_index`, so they
   mint a new address on every call: `provisioner.rs` (once per account, at provisioning)
   and `src/scenarios/exchange.rs` in `run_deposit` (once per **deposit intent**, i.e.
   repeatedly on the same account during a run). Both request `["orchard"]` only, so
   neither consumes a transparent index today — but the measured derivation-jump behaviour
   makes the failure mode concrete: adding `p2pkh` to either call site would exhaust the
   account's transparent window within a few calls, not after ten. Both sites should move
   to read-and-reuse (`funding::resolve_account`) rather than derive.

---

## Sources

- [`zcash_keys/src/keys/transparent/gap_limits.rs`](https://github.com/zcash/librustzcash/blob/main/zcash_keys/src/keys/transparent/gap_limits.rs) — `GapLimits`, defaults, rationale
- [`zcash_client_sqlite/src/wallet/transparent.rs`](https://github.com/zcash/librustzcash/blob/main/zcash_client_sqlite/src/wallet/transparent.rs) — `find_gap_start`, `generate_gap_addresses`, `ReachedGapLimit`
- [`zcash_client_sqlite::WalletDb`](https://docs.rs/zcash_client_sqlite/latest/zcash_client_sqlite/struct.WalletDb.html) — `with_gap_limits`
- [Zallet CHANGELOG @ v0.1.0-beta.1](https://raw.githubusercontent.com/zcash/zallet/v0.1.0-beta.1/CHANGELOG.md) — transparent spend fix; coinbase exclusion
- [ZIP-213: Shielded Coinbase](https://zips.z.cash/zip-0213)
- [`scripts/experiments/fanout-probe.sh`](../scripts/experiments/fanout-probe.sh) — the measurements resolving the discrepancy (12/12 on the beta.1 override stack)
- [`docs/regtest-funding-plan.md`](regtest-funding-plan.md) — full measured funding pipeline and override-stack rationale
- [Zcash Protocol Specification — Transaction Consensus Rules](https://zips.z.cash/protocol/protocol.pdf#txnconsensus)
- [zallet#637](https://github.com/zcash/zallet/issues/637) — gap limit exhausted on zcashd migration
- [zallet#638](https://github.com/zcash/zallet/pull/638) — configurable gap limits (closed unmerged)
- [zallet#644](https://github.com/zcash/zallet/issues/644) — cannot spend own transparent funds in one tx
- [librustzcash#2470](https://github.com/zcash/librustzcash/issues/2470) — transparent wallet performance at scale
- [librustzcash#2594](https://github.com/zcash/librustzcash/issues/2594) — promoting the gap-limit error
- [librustzcash#2666](https://github.com/zcash/librustzcash/pull/2666) — never-exposed ZeWIF addresses (merged)

> Note: the `zcash/wallet` repository has been renamed to
> [`zcash/zallet`](https://github.com/zcash/zallet); the book moved to
> https://zcash.github.io/zallet/. Our `z3-commits.lock` still cites the old URL.
