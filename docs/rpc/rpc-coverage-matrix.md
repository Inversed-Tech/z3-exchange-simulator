# RPC Coverage Matrix

Tracks which RPC methods the simulator exercises, which Z3 backend serves each method,
test category, zcashd behavioral parity, and implementation status.

All calls go through the Z3 RPC Router at `:8181`. The "Backend" column shows which
component the router forwards each call to — this is for documentation and report
attribution only. The simulator does not select backends directly.

---

## Column definitions

| Column | Meaning |
|---|---|
| **Method** | RPC method name |
| **Backend** | Component the RPC Router forwards this call to: Zebra or Zallet |
| **Test category** | Stress = exercised under load; Smoke = called once for compatibility |
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

---

## Matrix

### Chain info

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblockchaininfo` | Zebra | Stress | Yes | N/A | No | No | TBD | Primary smoke signal; called at the start of every run. |
| `getblockcount` | Zebra | Stress | Yes | N/A | No | No | TBD | |
| `getbestblockhash` | Zebra | Stress | Yes | N/A | No | No | TBD | |
| `getbestblockheightandhash` | Zebra | — | No | N/A | No | No | No-equiv | Zebra-specific combined call; not in Foundation's confirmed method list. |

### Block lookup

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblock` | Zebra | Stress | Yes | N/A | No | No | TBD | |
| `getblockhash` | Zebra | Stress | Yes | N/A | No | No | TBD | |
| `getblockheader` | Zebra | Stress | Yes | N/A | No | No | TBD | |

### Transaction lookup

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getrawtransaction` | Zebra + Zallet | Stress | Yes | Both | No | No | TBD | Listed under both backends in Foundation's method scope. |
| `decoderawtransaction` | Zallet | — | Yes | Both | No | No | TBD | Not in Foundation's confirmed list. |
| `gettxout` | Zebra | Stress | Yes | T | No | No | TBD | |

### Address-indexed lookup

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getaddressbalance` | Zebra | Stress | Yes | T | No | No | TBD | |
| `getaddresstxids` | Zebra | Stress | Yes | T | No | No | TBD | |
| `getaddressutxos` | Zebra | Stress | Yes | T | No | No | TBD | |

### Mempool

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getrawmempool` | Zebra | Stress | Yes | Both | No | No | TBD | Core mempool saturation signal. |
| `getmempoolinfo` | Zebra | Stress | Yes | N/A | No | No | TBD | |

### Mempool notifications (gRPC)

These are push-based streaming interfaces, not JSON-RPC methods. Accessed outside the
RPC Router via their respective gRPC endpoints.

| Mechanism | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `Indexer.mempool_change()` | Zebra | — | No | Both | No | No | No-equiv | gRPC server-streaming. Pushes ADDED/INVALIDATED/MINED events per tx. Requires `--features indexer` build and `indexer_listen_addr` config. Scope TBD pending Foundation confirmation. |
| `GetMempoolStream` | Zaino | — | No | Both | No | No | No-equiv | Zaino LightWallet gRPC. Scope TBD pending Foundation confirmation. |

### Mining and block production

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblocktemplate` | Zebra | Stress | Yes | N/A | No | No | TBD | Required to drive regtest block production. |
| `submitblock` | Zebra | Stress | Yes | N/A | No | No | TBD | Submit a mined block. |
| `generate` | Zebra | Smoke | Yes | N/A | No | No | TBD | Mine N blocks immediately in regtest. |
| `getblocksubsidy` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getdifficulty` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getmininginfo` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getnetworkhashps` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getnetworksolps` | Zebra | Smoke | Yes | N/A | No | No | TBD | |

### Network and peers

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getpeerinfo` | Zebra | Stress | Yes | N/A | No | No | TBD | Verify regtest network state. |
| `getnetworkinfo` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `getinfo` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `addnode` | Zebra | Smoke | Yes | N/A | No | No | TBD | |
| `ping` | Zebra | Smoke | Yes | N/A | No | No | TBD | |

### Chain control

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `invalidateblock` | Zebra | Smoke | Yes | N/A | No | No | TBD | Chain reorganization testing. |
| `reconsiderblock` | Zebra | Smoke | Yes | N/A | No | No | TBD | Undo a previous `invalidateblock`. |

### Shielded tree state

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_gettreestate` | Zebra | Stress | Yes | Z | No | No | TBD | Sapling and Orchard commitment tree state at a block. |
| `z_getsubtreesbyindex` | Zebra | Stress | No | Z | No | No | No-equiv | Subtree roots for the note commitment tree — shielded state size signal. |

### Address validation

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `validateaddress` | Zebra | Smoke | Yes | T | No | No | TBD | |
| `z_validateaddress` | Zebra | Smoke | Yes | Z | No | No | TBD | Not present in Zallet at pinned commit. |
| `z_listunifiedreceivers` | Zebra + Zallet | Smoke | No | Both | No | No | No-equiv | Lists individual receivers within a Unified Address. Listed under both backends. |

### Transaction broadcast

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `sendrawtransaction` | Zebra | Stress | Yes | Both | No | No | TBD | |

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
| `z_getnewaccount` | Zallet | Stress | No | Both | No | No | No-equiv | Replaces `getnewaddress` + `z_getnewaddress`. |
| `z_getaddressforaccount` | Zallet | Stress | No | Both | No | No | No-equiv | Derives a Unified Address from an account. |
| `z_listaccounts` | Zallet | Stress | No | Both | No | No | No-equiv | |
| `z_getaccount` | Zallet | Stress | No | Both | No | No | No-equiv | |
| `listaddresses` | Zallet | Stress | No | Both | No | No | No-equiv | |

### Wallet — balance

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_getbalances` | Zallet | — | No | Both | No | No | No-equiv | Not in Foundation's confirmed list. Replaces `getbalance` + `z_getbalance`. |
| `z_gettotalbalance` | Zallet | Stress | Yes | Both | No | No | TBD | |

### Transaction creation

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_sendmany` | Zallet | Stress | Yes | Both | No | No | Deviation | `fee` must be `null`; auto-computed via ZIP 317. Returns operation ID immediately (async). |

### Async operations

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_getoperationstatus` | Zallet | Stress | Yes | Z | No | No | TBD | |
| `z_getoperationresult` | Zallet | Stress | Yes | Z | No | No | TBD | |
| `z_listoperationids` | Zallet | Stress | Yes | Z | No | No | TBD | |

### Wallet — transaction history and notes

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_listunspent` | Zallet | Stress | Yes | Z | No | No | Deviation | `amount` renamed to `value`; new fields added vs zcashd. |
| `z_listtransactions` | Zallet | Stress | No | Both | No | No | No-equiv | Not in zcashd. |
| `z_getnotescount` | Zallet | Stress | No | Z | No | No | No-equiv | Count of unspent notes — shielded state size signal. |
| `z_viewtransaction` | Zallet | Stress | Yes | Both | No | No | TBD | Decode full wallet transaction details. |
| `z_recoveraccounts` | Zallet | Stress | No | Both | No | No | No-equiv | Recover accounts from wallet seed. |
| `getrawtransaction` | Zebra + Zallet | Stress | Yes | Both | No | No | TBD | See Transaction lookup section above. |

### Wallet — management

| Method | Backend | Test category | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getwalletinfo` | Zallet | Smoke | Yes | N/A | No | No | TBD | |
| `walletlock` | Zallet | Smoke | Yes | N/A | No | No | TBD | |
| `walletpassphrase` | Zallet | Smoke | Yes | N/A | No | No | TBD | |
| `help` | Zallet | Smoke | Yes | N/A | No | No | TBD | |

---

## Removed or replaced from zcashd

| Old method | Status in Z3 | Replacement |
|---|---|---|
| `getnewaddress` | Removed | `z_getnewaccount` + `z_getaddressforaccount` |
| `z_getnewaddress` | Removed | `z_getnewaccount` + `z_getaddressforaccount` |
| `getbalance` | Removed | `z_getbalances` |
| `z_getbalance` | Removed | `z_getbalances` |
| `gettransaction` | Not found in Z3 | None identified |
| `getmempoolentry` | Not found in Z3 | None identified |
| `sendtoaddress` | Not found in Z3 | Use `z_sendmany` with a single recipient |
| `createrawtransaction` | Not in Zallet | PCZT support planned for future release |
| `signrawtransaction` | Not in Zallet | PCZT support planned for future release |

---

## Open questions

1. Which zcashd methods have no Z3 equivalent beyond those already listed above?
2. Zaino gRPC scope: should the simulator test `GetMempoolStream` / `GetMempoolTx` directly? Pending Foundation confirmation.
