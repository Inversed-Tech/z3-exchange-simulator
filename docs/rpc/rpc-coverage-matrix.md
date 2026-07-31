# RPC Coverage Matrix

Tracks which RPC methods the simulator exercises, which Z3 backend serves each method,
test category, zcashd behavioral parity, and implementation status.

All JSON-RPC calls go through the Z3 RPC Router at `:8181` (regtest only). The router
requires HTTP Basic Auth (default `zebra` / `zebra`). The "Backend" column shows which
component the router forwards each call to — this is for documentation and report
attribution only. The simulator does not select backends directly.

---

## Column definitions

| Column | Meaning |
|---|---|
| **Method** | RPC method name |
| **Backend** | Component the RPC Router forwards this call to: Zebra or Zallet |
| **Test category** | Stress = exercised under load; Regtest-control = chain manipulation, scenario-driven, excluded from stress histograms; Smoke = called once for compatibility |
| **zcashd equiv?** | Whether an equivalent method existed in zcashd |
| **T/Z** | Address pool: T = transparent, Z = shielded, Both, N/A |
| **Implemented?** | Whether the simulator's RPC client can call this method yet |
| **Tested?** | Whether the simulator has called this method against a live Z3 stack |
| **Parity** | Behavioral parity with zcashd: TBD / Confirmed / Deviation / No-equiv |
| **Notes** | Caveats or deviations specific to this method |

---

## Status legend

| Symbol | Meaning |
|---|---|
| TBD | Not yet verified or determined |
| Yes / No | Confirmed fact |
| Confirmed | Behavior matches zcashd |
| Deviation | Behavior differs from zcashd — see Notes |
| No-equiv | No zcashd equivalent exists |

> **Tested?** flips to `Yes` once a method has actually been called against a live Z3
> stack (not just exercised in a unit test against a mocked RPC server). As of the
> `smoke` scenario's first live pass (2026-07-29, `docs/regtest-funding-plan.md`), the
> methods on the transparent-to-transparent (T→T) path have live coverage; the rest
> remain `No` until a scenario exercises them.

---

## Matrix

### Chain info

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblockchaininfo` | Zebra | Stress | Yes | N/A | Yes | Yes | TBD | Primary smoke signal; called at the start of every run. |
| `getblockcount` | Zebra | Stress | Yes | N/A | Yes | Yes | TBD | |
| `getbestblockhash` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |
| `getbestblockheightandhash` | Zebra | Stress | No | N/A | Yes | No | No-equiv | Zebra-specific combined tip height+hash. In the Foundation's confirmed stress list. |

### Block lookup

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblock` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |
| `getblockhash` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |
| `getblockheader` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |

### Transaction lookup

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getrawtransaction` | Zebra + Zallet | Stress | Yes | Both | Yes | Yes | TBD | Listed under both backends; routed to Zebra. A Zallet wallet-aware variant is TBD. `verbose` must be serialized as a number, not a JSON boolean, on Zebra >= 6.0.0 (`-32602` otherwise; v5.x tolerated a boolean). |
| `decoderawtransaction` | Zallet | — | Yes | Both | No | No | TBD | Not in Foundation's confirmed list. |
| `gettxout` | Zebra | Stress | Yes | T | Yes | No | TBD | |

### Address-indexed lookup

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getaddressbalance` | Zebra | Stress | Yes | T | Yes | No | TBD | |
| `getaddresstxids` | Zebra | Stress | Yes | T | Yes | No | TBD | |
| `getaddressutxos` | Zebra | Stress | Yes | T | Yes | No | TBD | |

### Mempool

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getrawmempool` | Zebra | Stress | Yes | Both | Yes | No | TBD | Core mempool saturation signal. |
| `getmempoolinfo` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |

### Mempool notifications (gRPC)

These are push-based streaming interfaces, not JSON-RPC methods. Accessed outside the
RPC Router via their respective gRPC endpoints.

| Mechanism | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `Indexer.mempool_change()` | Zebra | — | No | Both | No | No | No-equiv | gRPC server-streaming. Pushes ADDED/INVALIDATED/MINED events per tx. Requires `--features indexer` build and `indexer_listen_addr` config. |
| `GetMempoolStream` | Zaino | — | No | Both | No | No | No-equiv | Zaino LightWallet gRPC. Out of scope this engagement (Zaino covered via its JSON-RPC mirror). |

### Mining and block production

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblocktemplate` | Zebra | Stress | Yes | N/A | Yes | No | TBD | Required to drive regtest block production. |
| `submitblock` | Zebra | Stress | Yes | N/A | No | No | TBD | Submit a mined block. |
| `getblocksubsidy` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getdifficulty` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getmininginfo` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getnetworkhashps` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getnetworksolps` | Zebra | Smoke | Yes | N/A | No | No | TBD | |

### Network and peers

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getpeerinfo` | Zebra | Stress | Yes | N/A | Yes | No | TBD | Verify regtest network state. |
| `getnetworkinfo` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getinfo` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `addnode` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `ping` | Zebra | Smoke | Yes | N/A | No | No | TBD | |

### Regtest-control (chain manipulation)

Scenario-driven, regtest-only. Excluded from stress latency histograms — these shape
the test rather than being part of the measured workload.

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `generate` | Zebra | Regtest-control | Yes | N/A | Yes | Yes | TBD | Mine N blocks immediately; used in warmup and confirmation steps. |
| `invalidateblock` | Zebra | Regtest-control | Yes | N/A | Yes | No | TBD | Drives chain-reorganization scenarios (`src/scenarios/regtest_control.rs`); no live `reorg` scenario is shipped yet, so not yet run against a live stack. |
| `reconsiderblock` | Zebra | Regtest-control | Yes | N/A | Yes | No | TBD | Restores a branch invalidated by `invalidateblock`; same live-test status as above. |

### Shielded tree state

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_gettreestate` | Zebra | Stress | Yes | Z | Yes | No | TBD | Sapling and Orchard commitment tree state at a block. |
| `z_getsubtreesbyindex` | Zebra | Stress | No | Z | Yes | No | No-equiv | Subtree roots for the note commitment tree — shielded state size signal. |

### Address validation

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `validateaddress` | Zebra | Smoke | Yes | T | Yes | No | TBD | |
| `z_validateaddress` | Zebra | Smoke | Yes | Z | No | No | TBD | Not present in Zallet at pinned commit. |
| `z_listunifiedreceivers` | Zebra + Zallet | Smoke | No | Both | Yes | Yes | No-equiv | Lists individual receivers within a Unified Address. Listed under both backends. Used by the provisioner to extract each account's p2pkh receiver. |

### Transaction broadcast

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `sendrawtransaction` | Zebra | Stress | Yes | Both | Yes | No | TBD | |

### Node control

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `rpc.discover` | Zebra + Zallet | Smoke | No | N/A | No | No | No-equiv | OpenRPC service discovery. |
| `stop` | Zebra + Zallet | Smoke | Yes | N/A | No | No | TBD | Graceful shutdown — smoke only; not called during load runs. |

### Wallet — accounts and addresses

Zallet uses an account model that replaces zcashd's per-address generation.
`getnewaddress` and `z_getnewaddress` do not exist in Zallet.

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_getnewaccount` | Zallet | Stress | No | Both | Yes | Yes | No-equiv | Replaces `getnewaddress` + `z_getnewaddress`. |
| `z_getaddressforaccount` | Zallet | Stress | No | Both | Yes | No | No-equiv | Derives a Unified Address from an account. Deliberately NOT called by the provisioner or the hot-wallet resolution path — each call derives a new address, and on an unfunded account the transparent gap window is indices 0-9 (`docs/zallet-transparent-gap-limit.md`); those paths read an account's existing creation-time address from `z_listaccounts` instead. |
| `z_listaccounts` | Zallet | Stress | No | Both | Yes | Yes | No-equiv | |
| `z_getaccount` | Zallet | Stress | No | Both | Yes | No | No-equiv | |
| `listaddresses` | Zallet | Stress | No | Both | Yes | No | No-equiv | |

### Wallet — balance

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_gettotalbalance` | Zallet | Stress | Yes | Both | Yes | Yes | Deviation | Requires `include_watchonly=true` at the pinned Zallet commit. A non-zero balance is not proof of spendability — the only honest check is a successful send proposal (`docs/regtest-funding-plan.md`). |

### Transaction creation

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_sendmany` | Zallet | Stress | Yes | Both | Yes | Yes | Deviation | `fee` must be `null`; auto-computed via ZIP 317. Returns operation ID immediately (async). Rejects duplicate recipient addresses within one call (`-8`), which shapes the funding fan-out into multiple sequential rounds (`docs/regtest-funding-plan.md`). |
| `z_shieldcoinbase` | Zallet | Stress | Yes | Both | Yes | No | TBD | Shields transparent coinbase into the account's shielded pool; only invoked by the funding path when transparent coinbase is present and the shielded balance is insufficient — a no-op on the recommended Orchard-coinbase warmup path, so not yet exercised live. |

### Async operations

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_getoperationstatus` | Zallet | Stress | Yes | Z | Yes | Yes | TBD | |
| `z_getoperationresult` | Zallet | Stress | Yes | Z | Yes | Yes | TBD | |
| `z_listoperationids` | Zallet | Stress | Yes | Z | Yes | No | TBD | |

### Wallet — transaction history and notes

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_listunspent` | Zallet | Stress | Yes | Z | Yes | Yes | Deviation | `amount` renamed to `value`; new fields added vs zcashd, including a `pool` field (`transparent`/`sapling`/`orchard`) added at v0.1.0-beta.1. Live-tested during warmup's spendability check, which also surfaced a beta.1 defect: `WalletDb::get_memo` fails with a UTF-8 error once a shielded coinbase note exists, returning JSON-RPC error -20 (see `docs/regtest-funding-plan.md`). The simulator degrades to a log rather than failing the run. |
| `z_listtransactions` | Zallet | Stress | No | Both | Yes | No | No-equiv | Not in zcashd. |
| `z_getnotescount` | Zallet | Stress | No | Z | Yes | No | No-equiv | Count of unspent notes — shielded state size signal. |
| `z_viewtransaction` | Zallet | Stress | Yes | Both | Yes | No | TBD | Decode full wallet transaction details. |
| `z_recoveraccounts` | Zallet | Stress | No | Both | Yes | No | No-equiv | Recover accounts from wallet seed. |
| `getrawtransaction` | Zebra + Zallet | Stress | Yes | Both | Yes | No | TBD | See Transaction lookup section above. |

### Wallet — management

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getwalletinfo` | Zallet | Smoke | Yes | N/A | Yes | Yes | Deviation | Called every warmup as a Zallet liveness check (response content is not inspected). At the pinned Zallet commit the method is a stub returning all-zero fields (`docs/regtest-funding-plan.md`) — a Z3-side gap worth reporting, not a simulator defect. |
| `walletlock` | Zallet | Smoke | Yes | N/A | No | No | TBD | |
| `walletpassphrase` | Zallet | Smoke | Yes | N/A | No | No | TBD | |
| `help` | Zallet | Smoke | Yes | N/A | No | No | TBD | |

---

## Removed or replaced from zcashd

| Old method | Status in Z3 | Replacement |
|---|---|---|
| `getnewaddress` | Removed | `z_getnewaccount` + `z_getaddressforaccount` |
| `z_getnewaddress` | Removed | `z_getnewaccount` + `z_getaddressforaccount` |
| `getbalance` | Removed | `z_gettotalbalance` |
| `z_getbalance` | Removed | `z_gettotalbalance` |
| `z_getbalances` | Not in Foundation's confirmed list | `z_gettotalbalance` (used by the simulator instead) |
| `gettransaction` | Not found in Z3 | None identified |
| `getmempoolentry` | Not found in Z3 | None identified |
| `sendtoaddress` | Not found in Z3 | Use `z_sendmany` with a single recipient |
| `createrawtransaction` | Not in Zallet | PCZT support planned for future release |
| `signrawtransaction` | Not in Zallet | PCZT support planned for future release |

---

## Zaino coverage

Zaino is exercised directly via its zcashd-style **JSON-RPC mirror** (regtest host port
`:28237`); see [`docs/integration/zaino.md`](../integration/zaino.md). Its
lightwalletd-compatible **CompactTxStreamer gRPC** surface (regtest `:28137`) is
documented but out of scope for the current engagement.
