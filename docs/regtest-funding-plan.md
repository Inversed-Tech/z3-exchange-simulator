# Funding the Simulator in Regtest: Measured Findings and Funding Strategy

**Status:** Measured. Every claim about stack behaviour below was produced by
[`scripts/experiments/funding-probe.sh`](../scripts/experiments/funding-probe.sh) and
[`scripts/experiments/fanout-probe.sh`](../scripts/experiments/fanout-probe.sh) against
freshly initialised regtest stacks: the pinned versions (2026-07-28) and the working
override set Zebra v6.0.0 + Zaino 0.6.0 + Zallet v0.1.0-beta.1 (2026-07-29; see
`z3-commits.lock` overrides).
**Depends on:** [`zallet-transparent-gap-limit.md`](zallet-transparent-gap-limit.md),
[`zallet-transparent-spending-bug.md`](zallet-transparent-spending-bug.md)

---

## Why this document exists

Every transaction the simulator issues must be funded, and on a regtest chain the only
source of value is mining coinbase — there is no premine, no funded genesis, and no
import/faucet RPC. Coinbase can be paid into three different pools, and each has to survive
three independent layers before it is usable:

1. **Zebra** must build the coinbase and accept the resulting block.
2. **Zallet** must detect the output and credit the account.
3. **Zallet's `z_sendmany`** must select it as an input.

The probe script walks one pool through all three layers and prints a verdict per layer.
The answer on the current pins is that **no pool survives all three**, which is why runs
confirm 0 transactions.

## What this document previously got wrong

Two rounds of correction, both load-bearing, recorded here rather than quietly edited away:

1. The first version asserted the binding constraint was consensus: "transparent coinbase
   cannot be spent to a transparent output … no Zallet version changes it," and that a
   `z_shieldcoinbase` step was therefore mandatory. **The consensus rule is off on regtest**
   (hardcoded in Zebra's `Parameters::new_regtest()`; see below), so that reasoning was
   wrong.
2. The second version then concluded shielding was unnecessary. **Also wrong, for a
   different reason:** Zallet's proposal engine enforces the coinbase-to-shielded rule
   *client-side*, on every version, regardless of what regtest consensus allows. Measured
   on beta.1: 209 mature coinbase UTXOs, every `from` form refused with "Insufficient
   balance (have 0)". Transparent coinbase can only leave the wallet via
   `z_shieldcoinbase` — or be avoided entirely by mining shielded coinbase (§4).

---

## 1. Measured results

Probed on a fresh datadir at the pinned versions (Zebra `v5.0.0`, Zaino `0.4.0-rc.2`,
Zallet `v0.1.0-alpha.3`). "Detects" means `z_gettotalbalance` credited the account; "Spends"
means `z_sendmany` built a proposal.

| Coinbase pool | Zebra mines | Zallet detects | `z_sendmany` spends |
|---|---|---|---|
| **Transparent** | yes | yes — 662.50 ZEC, 105+ confirmations | **no** — `Insufficient balance (have 0, need 1010000)` |
| **Sapling** (ZIP-213) | yes | yes — 12.50 ZEC, 4 notes, credited within ~6 blocks | **no** — `Insufficient balance (have 0)` |
| **Orchard** (ZIP-213) | **no** — `could not validate orchard proof` | — | — |

The transparent row is the important one: the funds were **mature** (105 confirmations,
well past the 100-block rule), **owned by the sending account**, and **visible to
`z_listunspent`** — and `z_sendmany` still selected zero inputs. Every `from` form fails the
same way:

| `from` | Result |
|---|---|
| Account UA | `-4 Insufficient balance (have 0, need …)` |
| Bare t-addr that holds the UTXOs | `-5 Invalid from address, no payment source found for address.` |
| `ANY_TADDR` | `-11 The legacy account is currently unsupported for spending from` |

**Conclusion: `v0.1.0-alpha.3` cannot spend from any pool, so the beta.1 bump is not an
optimisation — it is the only route to a non-zero confirmation rate.**

### The consensus rule is off on regtest

Zebra hardcodes the exemption when it constructs Regtest parameters —
`zebra-chain/src/parameters/network/testnet.rs`, `Parameters::new_regtest()`:

```rust
let mut parameters = Self::build()
    .with_disable_pow(true)
    .with_unshielded_coinbase_spends(true)
```

and the enforcement site honours it (`zebra-chain/src/transaction.rs`):

```rust
if self.outputs().is_empty() || network.should_allow_unshielded_coinbase_spends() {
    CheckCoinbaseMaturity { spend_height }
} else {
    DisallowCoinbaseSpend
}
```

It is not a config knob; it is unconditional for Regtest. So on our chain transparent
coinbase spends are subject to **maturity only** (100 blocks). The wallet layer imposes no
extra restriction either: `zcash_client_sqlite`'s transparent queries carry a
`CoinbaseFilter` (all / coinbase-only / non-coinbase-only) and exclude only *immature*
coinbase.

Consequences: **`z_shieldcoinbase` is not required**, and strategy D's "shield first, then
fan out to create non-coinbase UTXOs" is unnecessary indirection. Once `z_sendmany` can
select transparent inputs, mature transparent coinbase can fund transparent recipients
directly.

Zebra's two sanctioned regtest cheats are exactly this exemption and disabled PoW. There is
no third one — no premine, no `importprivkey`, no faucet.

### Shielded coinbase has no maturity, but is only half-usable

[ZIP 213](https://zips.z.cash/zip-0213) amends the maturity rule "to only apply to
transparent coinbase outputs", and the probe confirms it: Sapling coinbase was credited
within ~6 blocks, against 100 for transparent. That would make shielded coinbase the
cheapest possible warmup — if it worked.

- **Orchard is unusable.** Zebra v5.0.0 builds the block and then **rejects its own block**:
  `submit block failed verification error=… "could not validate orchard proof"`. Zebra
  accepts the address (it logs `miner_address=…` parsed as `Unified(Address([Orchard(…)]))`)
  and then fails verification. This looks like an upstream defect worth reporting; see
  [Upstream issues to file](#upstream-issues-to-file).
- **Sapling mines and is detected, but poisons `z_listunspent`**, which fails for the whole
  wallet with `-20 WalletDb::get_memo failed / Invalid UTF-8: invalid utf-8 sequence`.
  Zallet assumes memo bytes are UTF-8; the coinbase memo is not. One shielded coinbase note
  therefore breaks a method the simulator uses for balance verification.

---

## 2. Other defects the probe exposed

These are independent of the funding question and three of them are **ours**, not upstream.

1. **Coinbase was landing in the wrong account.** `regtest-init.sh` creates a dedicated
   `miner` account (zip32 index 0) and points `ZEBRA_MINING__MINER_ADDRESS` at *its*
   transparent address, while the simulator resolves and spends from an account named
   `hot_wallet` (index 11 in the pre-existing datadir). All 151 UTXOs / 943.75 ZEC belonged
   to `miner`; `hot_wallet` held nothing. So the historical `Insufficient balance (have 0)`
   was, in part, **literally correct** — the simulator was spending from an empty account.
   This confound is why the earlier diagnosis in
   [`zallet-transparent-spending-bug.md`](zallet-transparent-spending-bug.md) could not be
   trusted; the probe re-confirms the alpha.3 defect cleanly, with coinbase paid directly to
   `hot_wallet`.

2. **Requesting a narrow receiver set at diversifier index 0 always fails.** Creating an
   account already generates its index-0 address with **all three** receiver types, so
   `z_getaddressforaccount(uuid, ["orchard","p2pkh"], 0)` returns
   `-4 address at diversifier index 0 was already generated with different receiver types`.
   The simulator asks for exactly that. Omitting the index lets Zallet pick a workable one.

3. **Sapling receivers are not available at every diversifier index.** Roughly half of all
   indices yield no valid Sapling diversifier, and index 0 typically does not:
   `-4 diversifier index 0 cannot generate an address with the requested receivers`. Any
   code that wants a Sapling receiver must let Zallet choose the index. This directly
   contradicts the "always pass `diversifier_index`" guidance added to
   `z_get_address_for_account`'s docstring, which should be softened to "pin the index only
   when you need a *specific*, already-generated address."

4. **`getwalletinfo` is a stub.** It returns all-zero fields and logs
   `TODO: Implement getwalletinfo`. It must not be used for balance verification.

---

## 3. The pin bump

| Entity | Current | Target |
|---|---|---|
| Zallet image | `electriccoinco/zallet:v0.1.0-alpha.3` | `v0.1.0-beta.1` (**no published image — see below**) |
| Zallet commit | `6fc85f68cf5ebe456160c6518255a83129e7d21c` | `5be0f4861feedc47978102c627c6293dea2d7838` |
| Released | 2025-12-15 | 2026-07-12 |

Zebra (`v5.0.0`) and Zaino (`0.4.0-rc.2`) stay as pinned.

### What the bump buys

From the `v0.1.0-beta.1` changelog:

- **`z_sendmany` can spend transparent funds.** Previously it "passed a shielded-only spend
  policy to the proposal builder, so no transparent input could ever be selected, and the
  `AllowFullyTransparent` privacy policy was unreachable." This matches the probe exactly.
- `InputSource::select_spendable_transparent_outputs` and
  `WalletWrite::reserve_next_n_internal_addresses` are implemented; both previously
  defaulted to `unimplemented!()`.
- The `zebra` and `zaino` backends support regtest.

From `v0.1.0-alpha.4` (included transitively): `z_shieldcoinbase` exists, and coinbase
`tx_index` is recorded — the latter matters even though we no longer need shielding, because
`zcash_client_sqlite` defaults an unknown `tx_index` to *non-coinbase*
(`IFNULL(t.tx_index, 1)`), which silently skips the maturity check for such outputs.

### There is no published beta.1 image

This is the open blocker on the bump, and it invalidates the first version of this document,
which said the bump "is expressed by writing `Z3_ZALLET_IMAGE` into
`external/z3/.env.regtest`". Probed registries:

| Reference | Result |
|---|---|
| `electriccoinco/zallet` | newest tag `v0.1.0-alpha.3`, dated 2025-12-15 |
| `electriccoinco/zallet:v0.1.0-alpha.4` / `:v0.1.0-beta.1` | absent |
| `ghcr.io/zcash/zallet`, `ghcr.io/zcash/wallet`, `zcash/zallet` | absent |

Source releases and prebuilt binaries do exist. Two routes:

- **(a) Source build.** `external/z3/docker-compose.build.yml` already defines a `zallet`
  service building `./vendor/zallet` (`target: runtime`, tagged `z3_zallet:local`), fed by
  `scripts/vendor.sh` — whose pin must move from `v0.1.0-alpha.3` to `v0.1.0-beta.1`. Note
  `vendor.sh` still clones `https://github.com/zcash/wallet`. Sanctioned by the Z3 repo, but
  a full Rust build, and `external/` is gitignored so it is not reproducible from this repo
  alone.
- **(b) Release tarball.** `zallet-v0.1.0-beta.1-linux-{amd64,arm64}.tar.gz` (~140 MB, with
  `.asc`, provenance, and SBOM) wrapped in a thin image. Faster, and the `arm64` asset
  yields a **native** image — the current `electriccoinco/zallet:v0.1.0-alpha.3` is
  `linux/amd64` and runs under emulation on this aarch64 host.

Either way the datadir must be fresh: alpha.4 broke the wallet database format.

---

## 4. The working pipeline — measured end to end

Established step by step by [`scripts/experiments/fanout-probe.sh`](../scripts/experiments/fanout-probe.sh),
which prints an OK/FAIL verdict per operation. On the override stack (**Zebra v6.0.0 +
Zaino 0.6.0 + Zallet v0.1.0-beta.1**, fresh datadir, 2026-07-29) all 12 steps pass:

| # | Operation | alpha.3 stack | beta.1 stack |
|---|---|---|---|
| 1 | resolve source account, reuse its existing UA | OK | OK |
| 2 | point `miner_address` at its p2pkh receiver | OK | OK |
| 3 | mine 105 blocks | OK | OK |
| 4 | wallet credits the coinbase | OK | OK |
| 5 | UTXOs reach 100-conf maturity | OK | OK |
| 6 | create 5 sink accounts, reuse their receivers | OK | OK |
| 7 | `z_shieldcoinbase` (account-UUID `from`) | n/a (method absent) | OK |
| 8 | shielding op completes; mine 10 anchor confs | — | OK |
| 9 | `z_sendmany` UA → 5 transparent receivers | **FAIL** (have 0) | OK |
| 10 | fan-out op completes with txid | — | OK |
| 11 | all 5 sinks hold non-coinbase transparent UTXOs | — | OK |
| 12 | sink spends back via its t-addr `from` | — | OK |

### The cheaper variant: mine shielded coinbase (no maturity, no shielding)

With **NU6.2 activated** in the regtest params, coinbase can be mined directly to the hot
wallet's **Orchard receiver**, and the whole front half of the pipeline collapses:

- ZIP 213 limits the 100-block maturity rule to *transparent* coinbase — Orchard coinbase
  was credited within ~6 blocks and spendable at ~10.
- No `z_shieldcoinbase`: the wallet's client-side coinbase rule concerns transparent
  coinbase only. Measured: `z_sendmany` from the UA spent Orchard coinbase directly, to
  both shielded (`FullPrivacy`) and transparent (`AllowRevealedRecipients`) recipients.
- Warmup drops from ~105 blocks to ~16 (a few to fund + 10 anchor confirmations).

NU6.2 is the key: it activates the **fixed Orchard Action circuit** with a per-epoch
verifying key (Zebra 5.0.0 changelog). Without it, Zebra's miner builds Orchard coinbase
proofs with the new circuit and then **rejects its own block** ("could not validate orchard
proof") because a pre-NU6.2 height verifies against the old key. This — not a mining bug —
is why mining to Orchard failed on the original pins. It also requires Zallet ≥ beta.1:
alpha.3's `zcash_protocol 0.7.2` cannot parse the NU6.2 branch id (`5437f330`).

Config (both files; heights must match — Zaino 0.6.0 takes heights from the validator, so
Zebra's file is the source of truth):

```toml
# external/z3/config/regtest/zebra.toml
[network.testnet_parameters.activation_heights]
NU5 = 2
NU6 = 2
"NU6.1" = 3
"NU6.2" = 3

# external/z3/config/regtest/zallet.toml
regtest_nuparams = [ …, "4dec4df0:3", "5437f330:3" ]
```

### Rules the pipeline encodes (all measured)

1. **Never call `z_getaddressforaccount` to "get" an address** — it always *derives* a new
   one at the next Sapling-valid diversifier index, which advances in jumps. On an account
   with no funded address the transparent gap window is indices 0–9, so two or three such
   calls exhaust it and every later one fails with `ReachedGapLimit` at index 10. **This
   resolves the June smoke-run mystery** (fresh accounts erroring at "index 10"): the
   provisioner derived per account and per intent. Read existing addresses from
   `z_listaccounts` instead; account creation always generates the diversifier-0 address
   with every receiver type.
2. **`from` semantics** (beta.1): a UA draws the account's *shielded* funds only
   ([zallet#644](https://github.com/zcash/zallet/issues/644), by design); a bare t-addr
   draws that address's own transparent UTXOs — the only working form for TToT/TToZ;
   `ANY_TADDR` requires `features.legacy_pool_seed_fingerprint`.
3. **~10 confirmations before any input is selectable**, shielded notes and transparent
   UTXOs alike (refused at 3, accepted at ≥ 10 — a 10-conf anchor policy). Younger funds
   produce `-4 Insufficient balance (have 0, …)` while `z_gettotalbalance` plainly shows
   them. Sends must retry while blocks are mined; the wallet's scan can also trail the
   chain, so "have 0" right after mining is normal for a few seconds.
4. **Recipients that must hold transparent value are paid at their extracted p2pkh
   receiver** (`z_listunifiedreceivers`), never at their UA — paying a UA settles shielded.
5. **`z_gettotalbalance` > 0 is not spendability.** The only honest check is a successful
   proposal.

### Where this lives in the simulator

- `src/scenarios/runner/funding.rs` — the common helper: `resolve_account` (reuse existing
  addresses; never derive), `fund_accounts` (shield-if-needed → one fan-out transaction →
  txid), `wait_operation`, and the anchor-retry send.
- `src/rpc/mod.rs` — wrappers added: `z_shield_coinbase`, `z_list_unified_receivers`,
  `z_send_many_with_policy`; `AccountInfo` now carries the `addresses` array so callers can
  reuse instead of derive.
- `scripts/dev/regtest-miner-setup.sh` — points `miner_address` at the hot wallet (the
  account the runner spends from), not a separate `miner` account. Coinbase previously
  landed in an account the simulator never spent from, which made alpha.3's real defect
  much harder to see.

---

## 5. Questions, resolved

1. **Does beta.1 spend transparent coinbase?** No — client-side rule (§ corrections). It
   spends *shielded* coinbase and *non-coinbase* transparent UTXOs; `z_shieldcoinbase`
   bridges the gap when coinbase is transparent.
2. **Gap-limit scope.** Resolved — not an account-scope question at all. The window is per
   account and per scope as upstream documents, but *derivation-index jumps* (Sapling
   diversifier validity) burn ~3–5 indices per call, so ~2–3 unfunded derivations reach
   index 10. Reuse addresses; the limit is then never hit.
3. **Bare t-addr `from`?** Works on beta.1 for non-coinbase UTXOs (probe step 12); rejected
   on alpha.3. The legacy-pool feature is not needed for per-account spending.
4. **Minimum confirmation-rate assertion in the integration test** — still worth doing;
   now unblocked because a working pipeline exists to assert against.

## Upstream issues to file

Per the standing decision, drafted here and **not** filed:

1. **Zebra:** with NU6.2 not yet active, `generate` to an Orchard `miner_address` builds a
   block that fails Zebra's own verification ("could not validate orchard proof") — the
   template builder uses the fixed circuit while verification uses the pre-NU6.2 key. The
   miner should either build with the epoch-correct circuit or refuse the configuration.
   Reproducer: `funding-probe.sh orchard` with no NU6.2 in the regtest params.
2. **Zallet (beta.1):** one shielded coinbase note breaks `z_listunspent` wallet-wide with
   `WalletDb::get_memo failed / Invalid UTF-8` (memo bytes are not required to be UTF-8).
   Measured on alpha.3 (Sapling note) and still present on beta.1 (Orchard note).
3. **Zallet (docs/ergonomics):** account creation pre-generates diversifier 0 with all
   receiver types, so a later narrower request at index 0 fails with a message that reads
   like caller error; and repeated "get an address" calls silently walk the gap window (see
   rule 1 above) — an API for *reading* the primary address would remove the footgun.
4. **Zallet/Zaino (packaging):** no container image is published past v0.1.0-alpha.3, and
   the zaino-backend binary requires Zebra ≥ 6.0.0 (queries the `ironwood` subtree pool)
   with no compatibility note in the release.

---

## 6. Status

| Step | Work | State |
|---|---|---|
| 1 | Probe scripts (`funding-probe.sh`, `fanout-probe.sh`) | done — this document's evidence |
| 2 | beta.1 runtime (release-tarball image, `scripts/dev/zallet-release-image/`) | done — `z3sim/zallet:v0.1.0-beta.1` |
| 3 | Stack bump (Zebra 6.0.0, Zaino 0.6.0, NU6.2 config) + fresh datadir | done — recorded in `z3-commits.lock` overrides |
| 4 | Common funding helper (`src/scenarios/runner/funding.rs`) + RPC wrappers | done — unit-tested |
| 5 | Wire the runner: orchard-coinbase warmup, fan-out provisioning, per-flow `from` forms, confirmation-rate assertion | done — see below |

### How the runner is wired (step 5)

- **Warmup** mines to the hot wallet's **Orchard receiver** (written into
  `ZEBRA_MINING__MINER_ADDRESS` by `scripts/dev/regtest-miner-setup.sh`); the spendability
  check is pool-aware (100-conf maturity for transparent coinbase, ~10-conf anchor for
  shielded) and degrades to a log when `z_listunspent` hits the beta.1 memo defect.
- **Provisioning never derives**: accounts are created, then their creation-time UAs are
  read back in one `z_listaccounts` call and each UA's p2pkh receiver extracted. The
  population's transparent address entries carry the REAL t-addrs, so intent generation
  resolves genuine per-pool sender/recipient addresses.
- **Funding**: one fan-out transaction from the hot wallet funds every active account in
  both pools. The transparent side is COUNT-based — a transparent spend consumes its whole
  UTXO and change returns to the account's *shielded* pool, so each expected transparent
  intent gets its own UTXO (`FundingPlan`).
- **Per-flow `from` forms** in dispatch: TToT = sender's t-addr → t-addr
  (`AllowFullyTransparent`); TToZ = sender's t-addr → recipient UA (`AllowRevealedSenders`);
  ZToT = sweep sender UA → hot wallet, then hot wallet UA → t-addr
  (`AllowRevealedRecipients`); ZToZ = sender UA → recipient UA (`FullPrivacy`).
- **The integration test asserts a minimum confirmation rate** (>0 confirmed and ≥50%),
  closing the "0/60 confirmed looked healthy" hole for good.

Validated end to end on a fresh datadir (2026-07-29): the smoke scenario confirms
**59/60 intents (98.3%)** on the override stack — the single failure is same-account
UTXO contention between concurrent intents, itself a finding. Five more constraints were
measured while wiring (each one produced a distinct live failure first):

1. **`generate` must be chunked** (≤5 blocks/call here): with an Orchard `miner_address`
   every block carries a ~2 s halo2 coinbase proof (emulated host), so one
   `generate(110)` call outlives the 30 s HTTP timeout while Zebra keeps mining
   server-side — the client sees only transport errors and the chain over-mines.
2. **`z_sendmany` rejects duplicate recipient addresses in one transaction**
   (`-8 duplicated recipient address`), so K UTXOs at one address take K sequential
   fan-out rounds.
3. **A transparent spend consumes its whole UTXO and the change returns to the account's
   shielded pool**, so transparent funding is COUNT-based: one UTXO per expected
   transparent intent.
4. **ZEC amounts must be computed in integer zatoshis**: `0.1_f64 × 1.5 =
   0.15000000000000002`, which Zallet rejects (`-3 Invalid amount`, >8 decimals).
5. **Zebra ≥ 6.0.0 requires `getrawtransaction`'s verbosity as a NUMBER** (v5 tolerated a
   JSON boolean; v6 answers `-32602 Invalid params`), which silently broke every
   confirmation wait — sends succeeded while all 60 intents were reported failed.

---

## 7. `run_sweep()`'s two defects, exposed by exercising ZToT in isolation

Both fixed in `src/scenarios/exchange.rs` and `src/rpc/mod.rs`. Neither was visible in the
59/60 result above because that run's flow mix did not isolate ZToT enough to trigger
either one at the funding scale then in use; both reproduced 100% of the time once ZToT was
run as the only flow type.

### 7.1 `z_listunspent`'s wallet-wide memo crash breaks every ZToT intent, not just warmup

The warmup step's own `z_listunspent` call (§ how the runner is wired, above) already
degrades gracefully when it hits the beta.1/beta.2 memo defect (`WalletDb::get_memo
failed`, triggered by any shielded coinbase note's non-UTF8 memo bytes existing anywhere in
the wallet). `run_sweep()`'s own `z_listunspent` call, used to find the notes to consolidate
into the hot wallet, had no equivalent handling — it propagated the error directly, failing
100% of ZToT intents on this override stack, since warmup's own orchard-coinbase mining
guarantees the defect-triggering note exists in every run.

**Fix — measured, not assumed.** `z_listunspent` accepts an `addresses` filter
(confirmed via `rpc.discover`'s parameter schema). Measured directly, back-to-back, same
wallet state: an unfiltered call fails with the memo error every time; a call passing a
non-empty `addresses` array (with a concrete `max_conf`, not `null` — untested and not
relied upon) succeeds every time. `run_sweep()` now calls `z_list_unspent_for_addresses`
with the sweeping account's own address, added to `src/rpc/mod.rs`.

### 7.2 The `addresses` filter scopes by account, not by pool — mixing transparent and shielded balance

Initially assumed (incorrectly) that the `addresses` filter didn't scope results at all,
based on an early manual test against a wallet with 38 accounts accumulated over many prior
sessions, where it returned notes across dozens of unrelated accounts. Retested against a
controlled, freshly-funded wallet (accounts funded with known, precise amounts, ground-truthed
against `z_getbalanceforaccount`): the filter correctly scopes to the queried address's
*account* — the earlier result is presumed to have been an artifact of that wallet's
long, reused history, not a defect in the filter itself, though this was not independently
re-isolated.

What the filter does **not** do is scope by *pool*. Querying with an account's shielded UA
returns that account's notes from every pool it holds — transparent UTXOs included, not
only the shielded ones matching the queried address's type. Measured directly: an account
funded with 100,000,000 zat shielded and 3,000,000 zat transparent (matching what the
funding fan-out allocates to every account regardless of scenario flow mix — see §2) returns
all three notes, correctly attributed to that account, when queried by its shielded UA
alone.

`run_sweep()`'s existing account-scope filter summed every note across both pools into the
sweep amount. But its `z_sendmany` call passes the account's UA as `from`, and a UA `from`
draws shielded funds only ([zallet#644](https://github.com/zcash/zallet/issues/644); already
noted in §4, rule 2) — it cannot reach the account's transparent UTXOs at all. The result
was a sweep amount inflated by the account's transparent balance, systematically exceeding
what the UA-scoped send could actually supply: every ZToT intent in an affected run failed
identically with `Insufficient balance (have <shielded total>, need <shielded + transparent
total, plus fee>)`.

**Fix.** `run_sweep()` now filters to notes whose `pool` is not `"transparent"` (and is
present — a missing `pool` defaults to transparent, the same conservative reading already
used in the warmup check) before summing for the sweep amount and the ZIP-317 fee's note
count.

**Validated end to end**, `health-z2t.yaml` (6 accounts, ZToT only), fresh datadir:

| Run | `--max-in-flight` | Confirmed | Notes |
|---|---|---|---|
| Before either fix | 64 | 0/25 | 100% memo-crash failures (§7.1) |
| After §7.1 only | 64 | 0/25 | 100% inflated-balance failures (§7.2) |
| After both fixes | 4 | 2/6 | Remainder: concurrency-driven rejection + same-account exhaustion, both documented in `docs/z3-concurrent-request-ceiling.md` |
| After both fixes | 1 (serialized) | 5/6 | Remaining failure: same-account exhaustion (structural, not a defect — see that document) |

With both fixes in place and dispatch concurrency kept low, ZToT confirms cleanly; the
residual failures are accounted for by findings already documented elsewhere, not by any
remaining defect in `run_sweep()`.

---

## 8. Load-phase shielded sends had no anchor-confirmation retry, unlike the funding fan-out

`fund_accounts()` (§ how the runner is wired, above) already retries its own `z_sendmany`
call on "Insufficient balance" specifically, mining a block and waiting between attempts
(`send_with_anchor_retries`, `src/scenarios/runner/funding.rs`) — because a shielded
source's notes need roughly 10 confirmations before Zallet treats them as spendable (the
anchor policy noted in §4, rule 3), and a source funded moments earlier has not
necessarily reached that depth yet. The three load-phase functions that also send from a
shielded source — `run_deposit` (used by both TToZ and ZToZ), `run_withdrawal` (ZToT's
second leg), and `run_sweep` (ZToT's first leg) — had no equivalent retry: a single-shot
`z_sendmany` call that failed outright if the source's confirmations had not yet caught up.

This was invisible for TToZ, whose source is a transparent UTXO (no anchor-depth
requirement), and mostly invisible for ZToT's withdrawal leg, whose source is the hot
wallet (funded and confirmed well before the run's own dispatch started). It was fully
exposed running `health-z2z.yaml` (ZToZ only) in isolation: every account's shielded
balance is funded once, at setup, and any ZToZ intent dispatched before that funding
reaches anchor depth fails immediately with `Insufficient balance (have 0, need <amount>)`
— the "have 0" distinguishing it from the funding-inflation defect in §7.2, whose "have"
was always the account's true (non-zero) shielded total.

**Fix.** Added `send_many_with_anchor_retries` (`src/scenarios/exchange.rs`), mirroring
`funding::send_with_anchor_retries`'s retry-only-on-"Insufficient balance" behavior, and
wired it into all three call sites in place of their direct `z_sendmany` /
`z_sendmany_with_policy` calls.

**Validated end to end**, `health-z2z.yaml` (6 accounts, ZToZ only), `--max-in-flight 4`:

| Run | Confirmed | Failure breakdown |
|---|---|---|
| Before the fix | 6/25 | 17× `have 0` (this defect), 2× already-spent-input rejection |
| After the fix | 21/25 | 0× `have 0`; remainder is 3× already-spent-input rejection + 1× confirmation-polling race — both already documented in `docs/z3-concurrent-request-ceiling.md`, not this defect |

The `have 0` failure class is fully eliminated; the residual failures are the same
concurrency-driven class already characterized for the other flows, not a new or
remaining defect in the affected functions.
