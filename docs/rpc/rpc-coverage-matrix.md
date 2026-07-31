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

> **Tested? evidence.** As of 2026-07-31, 16 real runs exist under
> `experiments/runs/` (`smoke` and single-flow `flow-check-{ttoz,ztot,ztoz}` probes;
> `steady-state`/`ramp`/`burst`/`mixed` have not been run yet — see
> `docs/scenarios/scenario-design.md`). Aggregated across all 16 runs' `rpc_calls.jsonl`,
> the 16 methods below show at least one recorded call and are marked `Tested? = Yes`
> below; every other method in this matrix has not been exercised by any run on disk
> and stays `No`. Call counts (successes/total) as observed:
>
> `generate` 235/235 · `getblockchaininfo` 8/8 · `getblockcount` 78/78 ·
> `getmempoolinfo` 111/111 · `getrawmempool` 111/111 · `getrawtransaction` 1800/1800 ·
> `getwalletinfo` 8/8 · `z_getnewaccount` 80/80 · `z_getoperationresult` 106/106 ·
> `z_getoperationstatus` 1699/1732 · `z_gettotalbalance` 94/121 · `z_listaccounts` 16/76 ·
> `z_listunifiedreceivers` 88/88 · `z_listunspent` 0/68 · `z_sendmany` 341/550
>
> **One exception, flagged rather than silently included:** `z_getbalances` also
> appears (44/120) in one older run (`20260731T100505Z-flow-check-ztot`), despite no
> call site for that method existing anywhere in the RPC client source at the commit
> its manifest claimed (`src/rpc/mod.rs` only implements `z_get_total_balance`).
> **Explained and fixed, not just flagged:** the run's manifest recorded
> `simulator_commit` via a runtime `git rev-parse HEAD` call, which reports the
> working tree's *current* commit rather than the commit the running binary was
> actually built from — so a stale, not-yet-rebuilt binary's run got mislabeled with
> a newer commit than the one that produced it. `read_simulator_commit()` now embeds
> the commit at compile time instead (`build.rs`), so this cannot recur; see
> `docs/architecture/observability.md`. This specific historical run's data should
> still not be mixed with current-commit runs in the findings report, since its true
> originating commit is now unrecoverable — but the mechanism itself is resolved.
> `z_getbalances` is **not** marked `Tested?` below.
>
> "Tested? = Yes" here means "observed at least once in real evidence," not "passes
> reliably" — several of these methods have substantial failure rates in the same
> data (e.g. `z_listunspent` 0/68, `z_sendmany` 341/550) documented elsewhere
> (`docs/zallet-transparent-spending-bug.md`, `docs/regtest-funding-plan.md`).

---

## Matrix

### Chain info

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblockchaininfo` | Zebra | Stress | Yes | N/A | Yes | Yes | TBD | Primary smoke signal; called at the start of every run. |
| `getblockcount` | Zebra | Stress | Yes | N/A | Yes | Yes | TBD | |
| `getbestblockhash` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |
| `getbestblockheightandhash` | Zebra | Stress | No | N/A | No | No | No-equiv | Zebra-specific combined tip height+hash. In the Foundation's confirmed stress list. |

### Block lookup

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblock` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |
| `getblockhash` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |
| `getblockheader` | Zebra | Stress | Yes | N/A | Yes | No | TBD | |

### Transaction lookup

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getrawtransaction` | Zebra + Zallet | Stress | Yes | Both | Yes | Yes | TBD | Listed under both backends; routed to Zebra. A Zallet wallet-aware variant is TBD. |
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
| `getrawmempool` | Zebra | Stress | Yes | Both | Yes | Yes | TBD | Core mempool saturation signal. |
| `getmempoolinfo` | Zebra | Stress | Yes | N/A | Yes | Yes | TBD | |

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
| `getblocktemplate` | Zebra | Stress | Yes | N/A | No | No | TBD | Required to drive regtest block production. |
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
| `invalidateblock` | Zebra | Regtest-control | Yes | N/A | No | No | TBD | Drives chain-reorganization scenarios. |
| `reconsiderblock` | Zebra | Regtest-control | Yes | N/A | No | No | TBD | Restores a branch invalidated by `invalidateblock`. |

### Shielded tree state

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_gettreestate` | Zebra | Stress | Yes | Z | No | No | TBD | Sapling and Orchard commitment tree state at a block. |
| `z_getsubtreesbyindex` | Zebra | Stress | No | Z | No | No | No-equiv | Subtree roots for the note commitment tree — shielded state size signal. |

### Address validation

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `validateaddress` | Zebra | Smoke | Yes | T | Yes | No | TBD | |
| `z_validateaddress` | Zebra | Smoke | Yes | Z | No | No | TBD | Not present in Zallet at pinned commit. |
| `z_listunifiedreceivers` | Zebra + Zallet | Smoke | No | Both | No | Yes | No-equiv | Lists individual receivers within a Unified Address. Listed under both backends. |

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
| `z_getaddressforaccount` | Zallet | Stress | No | Both | Yes | No | No-equiv | Derives a Unified Address from an account. |
| `z_listaccounts` | Zallet | Stress | No | Both | Yes | Yes | No-equiv | |
| `z_getaccount` | Zallet | Stress | No | Both | Yes | No | No-equiv | |
| `listaddresses` | Zallet | Stress | No | Both | Yes | No | No-equiv | |

### Wallet — balance

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_gettotalbalance` | Zallet | Stress | Yes | Both | Yes | Yes | TBD | Requires `include_watchonly=true` at the pinned Zallet commit. |

### Transaction creation

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_sendmany` | Zallet | Stress | Yes | Both | Yes | Yes | Deviation | `fee` must be `null`; auto-computed via ZIP 317. Returns operation ID immediately (async). |

### Async operations

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_getoperationstatus` | Zallet | Stress | Yes | Z | Yes | Yes | TBD | |
| `z_getoperationresult` | Zallet | Stress | Yes | Z | Yes | Yes | TBD | |
| `z_listoperationids` | Zallet | Stress | Yes | Z | Yes | No | TBD | |

### Wallet — transaction history and notes

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_listunspent` | Zallet | Stress | Yes | Z | Yes | Yes | Deviation | `amount` renamed to `value`; new fields added vs zcashd. |
| `z_listtransactions` | Zallet | Stress | No | Both | No | No | No-equiv | Not in zcashd. |
| `z_getnotescount` | Zallet | Stress | No | Z | No | No | No-equiv | Count of unspent notes — shielded state size signal. |
| `z_viewtransaction` | Zallet | Stress | Yes | Both | No | No | TBD | Decode full wallet transaction details. |
| `z_recoveraccounts` | Zallet | Stress | No | Both | No | No | No-equiv | Recover accounts from wallet seed. |
| `getrawtransaction` | Zebra + Zallet | Stress | Yes | Both | Yes | Yes | TBD | See Transaction lookup section above. |

### Wallet — management

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getwalletinfo` | Zallet | Smoke | Yes | N/A | Yes | Yes | TBD | |
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
