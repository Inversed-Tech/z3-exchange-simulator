# Funding the Simulator in Regtest: Zallet Upgrade and Funding Strategy

**Status:** Plan. No code changes yet — this document is the design record for the
work that follows.
**Depends on:** [`zallet-transparent-gap-limit.md`](zallet-transparent-gap-limit.md),
[`zallet-transparent-spending-bug.md`](zallet-transparent-spending-bug.md)

---

## Why this document exists

Every transaction the simulator issues must be funded, and in regtest the only source of
value is mining coinbase. Two facts make the path from "coinbase" to "N funded accounts
that can transact in both pools" non-obvious:

1. **Transparent coinbase cannot be spent to a transparent output.** This is consensus, not
   a wallet limitation, and no Zallet version changes it.
2. **Our pinned Zallet (`v0.1.0-alpha.3`) cannot spend transparent funds at all**, and has
   no `z_shieldcoinbase` to escape fact 1.

Together these mean the current runs confirm 0 transactions. Fixing it requires a version
bump *and* a funding pipeline designed around the consensus rule.

---

## 1. The pin bump

| Entity | Current | Target |
|---|---|---|
| Zallet image | `electriccoinco/zallet:v0.1.0-alpha.3` | `electriccoinco/zallet:v0.1.0-beta.1` |
| Zallet commit | `6fc85f68cf5ebe456160c6518255a83129e7d21c` | `5be0f4861feedc47978102c627c6293dea2d7838` |
| Released | 2025-12-15 | 2026-07-12 |

Zebra (`v5.0.0`) and Zaino (`0.4.0-rc.2`) stay as pinned.

### What the bump buys

From the `v0.1.0-beta.1` changelog:

- **`z_sendmany` can spend transparent funds.** Previously it "passed a shielded-only spend
  policy to the proposal builder, so no transparent input could ever be selected, and the
  `AllowFullyTransparent` privacy policy was unreachable." This is the direct cause of our
  100%-failure runs.
- `InputSource::select_spendable_transparent_outputs` and
  `WalletWrite::reserve_next_n_internal_addresses` are implemented; both previously
  defaulted to `unimplemented!()`.
- **The `zebra` and `zaino` backends support regtest.** Earlier releases rejected regtest at
  startup. Worth noting our stack works today only because of the backend mode it happens to
  use; beta.1 removes that constraint.

From `v0.1.0-alpha.4` (included transitively):

- **`z_shieldcoinbase` exists.** Required to escape the consensus rule below.
- Coinbase `tx_index` is now recorded, "enabling `z_shieldcoinbase` (and any other consumer
  of `TransparentOutputFilter::CoinbaseOnly`) to correctly identify coinbase outputs."
  Without this the shielding step could not find the coinbase UTXOs.

### Cost and mechanics

- **`alpha.4` broke the wallet database format — a fresh datadir is required.** Accepted.
  In practice this means the full reset: `docker compose down -v`, reset the `pwhash`
  placeholder in `config/regtest/zallet.toml`, reset the miner address in `.env.regtest`,
  re-run `regtest-init.sh`, then bring the stack up.
- `external/` is **gitignored**, so the image tag cannot be committed from this repo. The
  compose file already reads `${Z3_ZALLET_IMAGE:-electriccoinco/zallet:v0.1.0-alpha.3}`, so
  the bump is expressed by writing `Z3_ZALLET_IMAGE` into `external/z3/.env.regtest`.
  `scripts/dev/regtest-miner-setup.sh` already sets a precedent for our repo writing into
  that file.
- `z3-commits.lock` records the pin for run attribution and must be updated in step with it,
  along with the version table in `README.md`.
- `z3-commits.lock` says pins are "frozen for the duration of the engagement". This bump is
  a deliberate, recorded exception — the frozen pin cannot produce a non-zero
  confirmation rate, so it cannot produce the findings the engagement is for.
- The repo also still cites `https://github.com/zcash/wallet`; that repository is now
  [`zcash/zallet`](https://github.com/zcash/zallet), and the book moved to
  https://zcash.github.io/zallet/.

---

## 2. The `z_shieldcoinbase` step

### Why it is mandatory

Transparent coinbase outputs may only be spent by a transaction with **no transparent
outputs** ([ZIP-213](https://zips.z.cash/zip-0213) narrowed the rule to transparent
coinbase; it did not remove it). Zebra v5.0.0 enforces it as
`UnshieldedTransparentCoinbaseSpend`. A 100-block maturity requirement applies separately.

beta.1's changelog states the consequence plainly: "Coinbase outputs are not spendable this
way: consensus requires them to be spent to a single shielded output, which remains
`z_shieldcoinbase`'s job. A transparent spend therefore requires a non-coinbase UTXO."

So **every** funding strategy below must pass through a shielding step. There is no
configuration or version that avoids it.

### Where it goes

In `lifecycle.rs::setup()`, between warmup and provisioning:

```
start stack
  → resolve hot wallet account          (must exist before mining, or Zallet's account
                                         birthday is set at the tip and misses coinbase)
  → warmup: mine warmup_blocks          (110 = 100 maturity + 10 buffer)
  → z_shieldcoinbase                    NEW — converts mature transparent coinbase to Orchard
  → poll operation to completion        (z_sendmany-style async op: z_getoperationstatus,
                                         then z_getoperationresult)
  → mine a few blocks to confirm the shielding tx
  → verify shielded balance > 0         (replaces today's z_gettotalbalance check, which is
                                         a false positive: it counts unspendable coinbase)
  → provision population
```

### Maturity arithmetic — a real constraint

`warmup_blocks: 110` is documented in every scenario as "100 for regtest coinbase maturity
+ 10 buffer". At height 110 only the coinbase of blocks 1–10 is mature, so **roughly 10
coinbase outputs are shieldable at handoff**, not all 110. The remainder mature during the
load phase as the background miner advances the chain.

Consequences to design for:

- The shielded balance available at provisioning time is bounded by ~10 coinbase outputs.
  Scenario amount ranges and account counts must fit inside that, or `warmup_blocks` must
  rise (mining is cheap in regtest — raising it is the simpler lever).
- `z_shieldcoinbase` may need to be **re-run during the load phase** as more coinbase
  matures, if a scenario's total value exceeds the initial mature set.
- Today's warmup check (`z_gettotalbalance > 0`) must be replaced. It passes on coinbase
  that cannot be spent — the exact false positive that let 100%-failure runs look healthy.

### RPC surface to add

Neither method has a wrapper in `src/rpc/mod.rs` today:

- **`z_shieldcoinbase`** — absent entirely (it appears only in comments).
- **`z_listunifiedreceivers`** — present only in the backend-routing table, and routed to
  `Backend::Zebra`, which is questionable for a wallet method. Needed to extract a UA's
  transparent receiver.

Both return async operation IDs or plain results consistent with existing wrappers;
`z_get_operation_status` / `z_get_operation_result` already exist for the polling half.

---

## 3. Funding strategies

Four candidates, all of which must respect the consensus rule.

### A. Single hot wallet, shielded (minimum viable)

Shield the hot wallet's coinbase; every flow spends from the hot wallet's shielded balance.

- **Pros:** smallest change; only needs the shielding step. Confirmation rate goes from 0%
  to meaningful immediately.
- **Cons:** every transaction's *source* is one shielded account. TToT and TToZ flows have
  no genuine transparent input, so "transparent sender" is fiction and per-pool latency
  attribution stays optimistic on the sending side. Also serialises all spends through one
  account's note set, which is itself a bottleneck that could dominate the measurement.

### B. Rotate `ZEBRA_MINING__MINER_ADDRESS` per account

Mine coinbase directly to each account's transparent address.

- **Pros:** funds N transparent addresses with no spend at all.
- **Cons:** Zebra has **no `generatetoaddress`** (confirmed: the v5.0.0 RPC surface has 39
  methods; `generate` takes only a block count and always pays `mining.miner_address`), and
  `miner_address` is read at startup with no RPC to change it — so this costs **one Zebra
  restart per account**. Unusable beyond trivial N. And the funds arrive as *coinbase*, so
  they still cannot be spent transparently — it does not even solve the problem.

### C. `getblocktemplate` + custom coinbase + `submitblock`

Build our own coinbase paying an arbitrary address and submit the block.

- **Pros:** no restarts; Zebra explicitly sanctions it ("Miners can make arbitrary changes
  to blocks, as long as the data sent to `submitblock` is a valid Zcash block"), and regtest
  disables PoW so no Equihash solution is needed.
- **Cons:** significant implementation cost (coinbase construction, merkle root, block
  serialisation) and, again, **the funds are coinbase** and cannot be spent transparently.
  Solves address targeting, not spendability.

### D. Shield once, then fan out to per-account transparent receivers (recommended)

```
mine 110+ blocks                     coinbase → hot wallet transparent
z_shieldcoinbase                     coinbase → hot wallet Orchard (consensus-legal)
z_sendmany (shielded → per-account
  transparent receivers)             creates NON-COINBASE transparent UTXOs
                                     ↑ this is the step that unlocks everything
now: TToT / TToZ can genuinely spend transparent inputs from those accounts
     ZToT / ZToZ spend from shielded as before
```

- **Pros:** the only option that produces **genuinely spendable transparent funds**, because
  the resulting UTXOs are not coinbase — exactly what beta.1's "a transparent spend
  therefore requires a non-coinbase UTXO" calls for. Fixes TToT/ZToT recipient fidelity
  *and* TToT/TToZ sender fidelity, so per-pool attribution becomes real. Costs one extra
  fan-out transaction per account at setup, which is a one-time provisioning cost rather
  than a per-intent cost.
- **Cons / open risks:**
  - Requires per-account `p2pkh` receivers, which reopens the **unresolved gap-limit
    question** — hence step 3 of the plan (the experiment) runs before this is implemented.
  - [zallet#644](https://github.com/zcash/zallet/issues/644) (open): a UA source is
    shielded-only by design, and a bare t-addr source draws only *that address's* UTXOs;
    gathering across multiple addresses in one transaction needs
    `features.legacy_pool_seed_fingerprint`. **Per-account funding sidesteps this** — each
    account has one transparent address holding its own UTXO, so `from` = that t-addr is
    sufficient. Worth confirming empirically.
  - Sending to a UA that has both receivers will prefer the **shielded** one. The fan-out
    and any TToT/ZToT recipient must therefore target the **extracted transparent
    receiver**, not the UA — hence the `z_listunifiedreceivers` wrapper.
  - Change outputs: `reserve_next_n_internal_addresses` consumes the **internal** scope,
    whose gap limit is **5** — tighter than external's 10. Worth watching.

### Recommendation

**D**, reached in two stages so value lands early and risk stays isolated:

1. Bump to beta.1 and add shielding → strategy **A** works, confirmation rate becomes
   non-zero, and the upgrade is validated independently of any address-derivation change.
2. Run the gap-limit experiment, then add the per-account transparent fan-out → **D**.

B and C are rejected: both target *where coinbase lands*, which is not the constraint. The
constraint is that coinbase must be shielded before it can fund anything transparent.

---

## 4. Open questions to settle by measurement

1. **Gap limit scope on the pinned release.** Upstream source says per-`(account, key
   scope)`; our own artifact
   (`experiments/runs/20260630T131145Z-smoke/rpc_calls.jsonl`) hit "index 10" while deriving
   on fresh accounts. Unresolved — see
   [`zallet-transparent-gap-limit.md`](zallet-transparent-gap-limit.md). **Decisive
   experiment:** on a fresh datadir, create accounts A and B; derive `["p2pkh"]` on A until
   it errors; then derive once on B. B succeeds ⇒ per-account. B fails ⇒ shared. This gates
   strategy D and is step 3 of the plan.
2. **How many accounts can hold transparent receivers**, given the answer to (1) and the
   internal-scope limit of 5 for change.
3. **Whether a bare t-addr `from` works per-account** on beta.1, or whether zallet#644's
   legacy-pool workaround is unavoidable.
4. **Whether `z_shieldcoinbase` needs re-running mid-load** as coinbase matures, or whether
   a higher `warmup_blocks` covers a full scenario's value.
5. **Whether the load phase should assert a minimum confirmation rate.** Today the
   integration test asserts only that the runner returns `Ok`, which is why 0/60 confirmed
   passed CI-visible checks. This should become a real assertion once funding works —
   otherwise the next regression of this class is equally invisible.

---

## 5. Sequencing

| Step | Work | Validation |
|---|---|---|
| 1 | This document | — |
| 2 | Bump Zallet to beta.1; fresh datadir; `z_shieldcoinbase` + `z_listunifiedreceivers` wrappers; shielding step in `setup()`; replace the warmup balance check | Live smoke run; confirmation rate > 0 |
| 3 | Gap-limit experiment against beta.1 | Answers open question 1 |
| 4 | Reevaluate, then implement the chosen funding strategy (D unless the experiment says otherwise); fix `exchange.rs::run_deposit`'s per-intent address derivation | Live run; per-pool attribution checks |

Steps 2–4 are separate PRs. Step 2 cannot be validated without a running stack and a fresh
datadir.
