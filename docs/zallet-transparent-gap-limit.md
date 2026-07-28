# Zallet's Transparent Address Gap Limit at Exchange Scale

**Status:** Analysis. Corrects an earlier, inaccurate claim in this repo's source comments.
**Applies to:** Zallet `v0.1.0-alpha.3` (our pin) through `v0.1.0-beta.1`; `librustzcash` `main`
**Relevant to:** per-account transparent deposit addresses, TToT/ZToT flow fidelity

---

## Summary

| Question | Answer |
|---|---|
| What is the limit? | `GapLimits { external: 10, internal: 5, ephemeral: 10 }` |
| Scope of the limit | Per **(account, key scope)** upstream — but a run of ours contradicts this on `alpha.3`; [unresolved](#the-discrepancy) |
| What advances the gap | Consecutive derived addresses with no output in a **mined** transaction |
| What resets it | A mined UTXO to a derived address slides the window forward |
| Configurable? | **No.** Zallet never calls `WalletDb::with_gap_limits`; no `zallet.toml` key exists |
| Is it *the* blocker for per-account transparent addresses? | No — a consensus rule blocks them regardless; see [The real blocker](#the-real-blocker-is-consensus-not-the-gap-limit) |

---

## Correction of a previous claim — and an unresolved discrepancy

Three sites in this repo asserted a wallet-global gap counter:

- `src/rpc/mod.rs` — "across the wallet" (introduced in `6e1770e`)
- `src/scenarios/runner/mod.rs` — "resets Zallet's transparent gap counter and allows all N
  synthetic accounts to receive transparent receivers" (`6e1770e`)
- `src/scenarios/runner/provisioner.rs` — "across the whole wallet" (expanded in `86ebe33`)

**Per current librustzcash source, "across the wallet" is wrong**: the window is maintained
per `(account_id, key_scope)`. All three comments have been corrected.

### The discrepancy

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

**This cannot be resolved from the evidence available.** `rpc_calls.jsonl` records
`params: null`, so we cannot tell whether those eight derivations hit eight distinct
account UUIDs or one. Candidate explanations, none verified:

1. Zallet `alpha.3` pins an older `zcash_client_backend` whose gap accounting was
   effectively wallet-wide (a single global `GAP_LIMIT` existed until 0.22.0).
2. The derivations shared one account or one transparent key scope for a reason not visible
   in the artifact.
3. Pre-generated address rows from prior runs on shared volumes interacted with the window
   in a way the per-account model doesn't capture.

**A decisive experiment** would be: on a fresh datadir, create two accounts, derive
`["p2pkh"]` addresses on account A until it errors, then attempt one derivation on account
B. If B succeeds, the window is per-account; if it fails, it is shared. This has not been
run.

### What this means for the claims in this document

The upstream-source claims below (values, per-scope filtering, reset semantics) are
confirmed against librustzcash `main`. But **the prediction that "one `p2pkh` receiver per
synthetic account sits comfortably inside the limit" is inference from current upstream
source, not a measured property of our pinned `alpha.3`** — and the artifact above is
evidence against it. Treat it as a hypothesis to test, not a settled fact, and run the
experiment above before relying on it.

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

## The real blocker is consensus, not the gap limit

Even setting the gap limit aside — and per the discrepancy above we cannot yet claim it is
harmless on our pin — there is a second, independent blocker that no gap-limit change would
lift.

It is a **consensus rule**. Transparent coinbase outputs may only be
spent by a transaction with **no transparent outputs**. Zebra v5.0.0 enforces this
(`zebra-state/src/service/check/utxo.rs`), quoting the protocol spec:

> A transaction with one or more transparent inputs from coinbase transactions MUST have
> no transparent outputs (i.e. `tx_out_count` MUST be 0).

[ZIP-213](https://zips.z.cash/zip-0213) narrowed this rule to apply only to *transparent*
coinbase outputs — it did not remove it. A 100-block maturity requirement also applies.

In regtest our entire balance originates as mining coinbase. So **no version of Zallet can
distribute coinbase funds to transparent addresses directly.** The supported pipeline is:

```
mine → wait 100 blocks (maturity) → z_shieldcoinbase → z_sendmany from the shielded balance
```

`z_shieldcoinbase` first appears in Zallet `v0.1.0-alpha.4`; our pin is `alpha.3`, which
does not expose it at all (and, per alpha.4's changelog, did not even record `tx_index`
for coinbase transactions, so coinbase outputs could not be identified).

Separately, `z_sendmany` on our pin could **never** select a transparent input: per the
`v0.1.0-beta.1` changelog it "passed a shielded-only spend policy to the proposal builder."
That is the true root cause of the failure recorded in
[`zallet-transparent-spending-bug.md`](zallet-transparent-spending-bug.md), and it is fixed
in beta.1.

Also relevant before designing t→t payouts:
[zallet#644](https://github.com/zcash/zallet/issues/644) (open) — even on beta.1, gathering
a wallet's own transparent funds across multiple addresses in a single transaction requires
the `legacy_pool_seed_fingerprint` legacy-pool workaround.

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

---

## Implications for this simulator

1. The orchard-only derivation in `provisioner.rs` is retained because on our `alpha.3` pin
   the funding path to a transparent recipient does not exist at all (no
   `z_shieldcoinbase`, and `z_sendmany` cannot select transparent inputs). Whether the gap
   limit would *additionally* obstruct per-account `p2pkh` receivers is unresolved — see
   [the discrepancy](#the-discrepancy).
2. TToT/ZToT "transparent" recipients currently settle in the **shielded** pool, so pool
   attribution for those flows is optimistic. `provisioner.rs` stores the *same*
   orchard-only UA string for both the `Transparent` and `Orchard` address entries; the
   `KNOWN LIMITATION` annotation is on the `Transparent` entry, with a cross-reference from
   the derivation site.
3. True TToT/ZToT fidelity requires a Zallet bump to **`v0.1.0-beta.1`** plus a
   `z_shieldcoinbase` step in `setup()` after warmup, and sending to the UA's *transparent
   receiver* rather than the UA itself (a `z_sendmany` to a UA with both receivers will
   prefer the shielded one). Neither `z_shieldcoinbase` nor `z_listunifiedreceivers` has an
   RPC wrapper in `src/rpc/mod.rs` yet — `z_listunifiedreceivers` appears only in the
   backend-routing table. `alpha.4` broke the wallet database format, so this needs a fresh
   datadir. Tracked separately from this document.
4. **Latent address accumulation.** Two call sites still omit `diversifier_index`, so they
   mint a new address on every call: `provisioner.rs` (once per account, at provisioning)
   and `src/scenarios/exchange.rs` in `run_deposit` (once per **deposit intent**, i.e.
   repeatedly on the same account during a run). Both request `["orchard"]` only, so
   neither should consume a transparent index today — but if `p2pkh` is added to either
   without also pinning a diversifier index, they will exhaust the transparent window
   quickly. Fix the `exchange.rs` call site as part of any such change.

---

## Sources

- [`zcash_keys/src/keys/transparent/gap_limits.rs`](https://github.com/zcash/librustzcash/blob/main/zcash_keys/src/keys/transparent/gap_limits.rs) — `GapLimits`, defaults, rationale
- [`zcash_client_sqlite/src/wallet/transparent.rs`](https://github.com/zcash/librustzcash/blob/main/zcash_client_sqlite/src/wallet/transparent.rs) — `find_gap_start`, `generate_gap_addresses`, `ReachedGapLimit`
- [`zcash_client_sqlite::WalletDb`](https://docs.rs/zcash_client_sqlite/latest/zcash_client_sqlite/struct.WalletDb.html) — `with_gap_limits`
- [Zallet CHANGELOG @ v0.1.0-beta.1](https://raw.githubusercontent.com/zcash/zallet/v0.1.0-beta.1/CHANGELOG.md) — transparent spend fix; coinbase exclusion
- [ZIP-213: Shielded Coinbase](https://zips.z.cash/zip-0213)
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
