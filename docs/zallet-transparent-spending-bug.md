# Zallet Alpha: Transparent UTXOs Tracked But Not Spendable via `z_sendmany`

**Status:** Resolved — fixed in Zallet `v0.1.0-beta.1` (see "Root cause — CONFIRMED"
below). The simulator runs against the beta.1 override stack recorded in
`z3-commits.lock`; this bug applies only to the frozen `v0.1.0-alpha.3` pin, no longer
what any live run uses.  
**Impact (alpha.3):** Simulator load phase confirmed 0 transactions (100% failure rate)  
**Was a blocker for (alpha.3):** All production scenarios (smoke, steady_state, ramp,
burst, mixed)

---

## Context

The Z3 exchange simulator drives Zebra + Zaino + Zallet in regtest to measure real
transaction throughput and RPC latency. The load phase works as follows:

1. Zebra mines coinbase blocks to the hot wallet's transparent address (P2PKH)
2. After 110 warmup blocks the hot wallet has mature transparent ZEC
3. The simulator provisions 10 synthetic accounts and calls `z_sendmany` from the
   hot wallet UA to distribute funds to each synthetic account
4. Synthetic accounts then run deposit/withdrawal loops

Step 3 fails. Every `z_sendmany` returns error -4 "Insufficient balance (have 0, need X
including fee)" even though the wallet demonstrably holds hundreds of ZEC.

---

## Evidence

### What Zallet reports correctly

```
z_gettotalbalance → {"transparent": "718.75", "private": "0.00", "total": "718.75"}

z_listunspent → 115 UTXOs
  {
    "txid": "bd07382f...",
    "pool": "transparent",
    "confirmations": 115,
    "is_watch_only": false,
    "address": "tmQ7m4yReKjuGQx11KxvvHgSi2xjGjGioYM",
    "account_uuid": "ba0d919b-02d2-40dd-8141-714ea65804c1",
    "value": 6.25
  }
```

The wallet database confirms this directly:

```sql
SELECT COUNT(*) FROM transparent_received_outputs;  -- 115
SELECT COUNT(*) FROM transactions;                  -- 115
```

### What z_sendmany returns

```json
{
  "method": "z_sendmany",
  "params": ["<hot-wallet-UA>", [{"address": "<synth-orchard-UA>", "amount": 0.001}], null, null, "AllowRevealedSenders"],
  "error": {"code": -4, "message": "Failed to propose transaction: Insufficient balance (have 0, need 110000 including fee)"}
}
```

This holds regardless of:
- Privacy policy (`AllowRevealedSenders`, `AllowFullyTransparent`, `FullPrivacy`, etc.)
- Whether the recipient is a shielded UA or a transparent taddr
- Number of mature coinbase UTXOs (tested with 16, 100+)

### Alternative `from` formats tried

| `from` parameter | Error |
|---|---|
| Raw taddr (`tmQ7...`) | -5 "Invalid from address, no payment source found" |
| `ANY_TADDR` | -11 "The legacy account is currently unsupported" |
| UA with `[orchard, p2pkh]` at diversifier 0 | -4 "Insufficient balance (have 0)" |
| UA with `[orchard, p2pkh]` + diversifier explicit | -4 "Insufficient balance (have 0)" |

### Methods that would bypass the issue

- `z_shieldcoinbase` — not found: error -32601 "Method not found"
- `dumpprivkey` — not available (would allow manual raw tx signing)

### Available Zallet RPC methods (full list)

`getrawtransaction`, `getwalletinfo`, `help`, `listaddresses`, `rpc.discover`, `stop`,
`walletlock`, `walletpassphrase`, `z_getaccount`, `z_getaddressforaccount`,
`z_getnewaccount`, `z_getnotescount`, `z_getoperationresult`, `z_getoperationstatus`,
`z_gettotalbalance`, `z_listaccounts`, `z_listoperationids`, `z_listtransactions`,
`z_listunifiedreceivers`, `z_listunspent`, `z_recoveraccounts`, `z_sendmany`,
`z_viewtransaction`

---

## Root cause — CONFIRMED (supersedes the hypotheses below)

> **Resolved.** The `v0.1.0-beta.1` changelog (2026-07-12) states the cause directly:
> `z_sendmany` "passed a shielded-only spend policy to the proposal builder, so no
> transparent input could ever be selected, and the `AllowFullyTransparent` privacy policy
> was unreachable." beta.1 also implements `InputSource::select_spendable_transparent_outputs`
> and `WalletWrite::reserve_next_n_internal_addresses`, both previously `unimplemented!()`.
> So this was hypothesis 3 below: transparent input selection was genuinely not wired up.
>
> **Two further facts change the fix path:**
>
> 1. **Coinbase still cannot be spent to a transparent output, even on beta.1 — but on
>    regtest that is Zallet's client-side policy, not consensus.** On mainnet the rule is
>    consensus (transparent coinbase must be spent by a transaction with no transparent
>    outputs; [ZIP-213](https://zips.z.cash/zip-0213)); on Regtest Zebra hardcodes the
>    exemption (`.with_unshielded_coinbase_spends(true)` in `Parameters::new_regtest()`),
>    yet Zallet's proposal engine excludes transparent coinbase from transparent-output
>    proposals regardless — measured on beta.1 with 209 mature UTXOs. Since regtest funds
>    are entirely coinbase, the pipeline is: mine → 100-block maturity →
>    `z_shieldcoinbase` → `z_sendmany` — or mine shielded (Orchard) coinbase and skip both
>    the maturity wait and the shielding (see `regtest-funding-plan.md`).
> 2. **`z_shieldcoinbase` was added in `v0.1.0-alpha.4`**, which also fixed coinbase
>    `tx_index` recording so coinbase outputs can be identified at all. Our pin (alpha.3)
>    has neither.
>
> Upgrading to `v0.1.0-beta.1` is therefore the fix, with the caveat that alpha.4 broke the
> wallet database format (fresh datadir required). beta.1 is also the first release whose
> zebra/zaino backends support regtest.
>
> See [`zallet-transparent-gap-limit.md`](zallet-transparent-gap-limit.md) for the full
> analysis, including why the gap limit is *not* implicated.

## Root cause hypothesis (original, superseded)

Zallet's `z_sendmany` implementation calls `zcash_client_backend::propose_transaction`
internally. The proposal engine selects inputs from the wallet's tracked pools. Based on
the evidence:

- `z_listunspent` queries `transparent_received_outputs` directly → sees 115 UTXOs ✓
- `z_gettotalbalance` queries `v_received_outputs` view (includes transparent pool) → sees 718 ZEC ✓
- `z_sendmany` proposal selects 0 inputs from the transparent pool → reports "have 0" ✗

The most likely cause is one of:
1. `propose_transaction` in this version of `zcash_client_backend` does not select
   transparent (coinbase) UTXOs when the `from` address is a Unified Address
2. Coinbase outputs are excluded from the spending proposal (e.g. treated as
   "not yet mature" by a different maturity definition than `z_listunspent` uses)
3. The transparent input selection is not yet wired up in Zallet alpha's `z_sendmany`
   implementation

Zallet version: `6fc85f68cf5ebe456160c6518255a83129e7d21c`

---

## What does pass

The warmup check (`z_gettotalbalance > 0`) gives a **false positive**: the wallet shows
funded but spending is broken. Everything upstream of `z_sendmany` works correctly:

- ✅ Zebra mines to the configured transparent address
- ✅ Zaino indexes all blocks
- ✅ Zallet's transparent polling task fetches and records coinbase UTXOs
- ✅ `z_listunspent` returns all UTXOs with correct confirmations and account association
- ✅ `z_gettotalbalance` returns correct total
- ✅ The simulator's lifecycle, provisioner, background miner, and scenario runner all
     work correctly and complete without errors
- ✅ RPC latency is measured correctly for all methods
- ✅ 348 unit tests pass

The only broken step is: **spending from those transparent UTXOs via `z_sendmany`**.

---

## How the integration test appears to pass

`cargo test --test integration test_smoke_scenario_via_runner -- --ignored`

The Rust test asserts that the runner completes without returning `Err(...)`. It does
not assert a minimum confirmed-transaction count. The run finishes with:

```
Confirmed: 0 (0.0%)
Failed: 60 (100.0%)
z_sendmany: error -4 × 60
```

The test binary exits `Ok(())` because the load phase ran to completion without
panicking. The 0% confirmation rate is captured in the run report but is not a test
assertion.

---

## Fix path

The fix belongs in Zallet (or its `zcash_client_backend` dependency), not in the
simulator. Options:

1. **Wait for a Zallet release** that supports transparent spending via `z_sendmany`.
   Once available, update the `Z3_ZALLET_IMAGE` pin in `external/z3/docker-compose.yml`
   and re-run — no simulator code changes needed.

2. **Ask the Zallet team** whether there is a supported workflow for spending
   transparent coinbase outputs in this alpha (e.g. a different RPC, a wallet unlock
   step, or a required `walletpassphrase` call before spending).

3. **Implement `z_shieldcoinbase` support** in the simulator once Zallet exposes it:
   shield the coinbase outputs to Orchard first, then use `z_sendmany` from the
   shielded balance. The spending proposal for Orchard notes is expected to work.

---

## Simulator code changes made during this investigation

All changes are in the simulator repo, not in `external/z3`:

| File | Change | Status |
|---|---|---|
| `src/rpc/mod.rs` | Added optional `diversifier_index` param to `z_get_address_for_account` | ✅ Merged |
| `src/scenarios/runner/lifecycle.rs` | Find hot wallet by `name == "hot_wallet"` instead of list position | ✅ Merged |
| `src/scenarios/runner/lifecycle.rs` | Use `diversifier_index=0` to retrieve the existing funded address | ✅ Merged |
| `src/scenarios/runner/dispatch.rs` | Background miner task (replaces per-transaction `generate()`) | ✅ Merged |
| `src/scenarios/exchange.rs` | Remove per-transaction `generate()` calls | ✅ Merged |
| `external/z3/scripts/init-regtest-fresh.sh` | Replace broken Python bech32m decoder with SQLite wallet.db query | ✅ Merged |
| `external/z3/scripts/init-regtest-fresh.sh` | Use `docker compose up -d zebra` instead of `restart` to apply new env vars | ✅ Merged |
