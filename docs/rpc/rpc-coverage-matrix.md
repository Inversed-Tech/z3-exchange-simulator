# RPC Coverage Matrix

Tracks which RPC methods the simulator exercises, which Z3 component serves each method,
zcashd behavioral parity, and test status. Updated continuously through the engagement.

Method names and component assignments are verified against the pinned commits:
- Zebra `d4cd662` — source: `zebra-rpc/src/methods.rs`
- Zaino `4ddbfd2` — source: `zaino-serve/src/rpc/jsonrpc/service.rs`
- Zallet `05926f3` — source: Zallet JSON-RPC method registrations

---

## Critical caveat

**The "Required by RFP?" column is pending Foundation confirmation.** A proposed method
list has been prepared for Foundation review at
[`docs/rpc/proposed-method-scope.md`](proposed-method-scope.md).

---

## Column definitions

| Column | Meaning |
|---|---|
| **Method** | RPC method name — verified against pinned commit source code. |
| **Component** | Z3 component that serves this method. |
| **Required by RFP?** | Whether this method appears in the Foundation's RFP requirements. Pending confirmation. |
| **zcashd equiv?** | Whether an equivalent method existed in zcashd. |
| **T/Z** | Which address pool: T = transparent, Z = shielded, Both, N/A. |
| **Implemented?** | Whether the simulator's RPC client can call this method yet. |
| **Tested?** | Whether the simulator has successfully called this method against a live Z3 stack. |
| **Parity** | Behavioral parity with zcashd: TBD / Confirmed / Deviation / No-equiv. |
| **Notes** | Caveats, deviations, or open questions specific to this method. |

---

## Status legend

| Symbol | Meaning |
|---|---|
| TBD | Not yet verified or determined |
| Yes / No | Confirmed fact |
| Confirmed | Behavior matches zcashd |
| Deviation | Behavior differs from zcashd — see Notes |
| No-equiv | No zcashd equivalent exists (new in Z3, or method name changed entirely) |

---

## Matrix

### Chain info

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblockchaininfo` | Zebra / Zaino | TBD | Yes | N/A | No | No | TBD | Zaino proxies to Zebra. Key smoke-test method. |
| `getblockcount` | Zebra / Zaino | TBD | Yes | N/A | No | No | TBD | Zaino proxies to Zebra. |
| `getbestblockhash` | Zebra / Zaino | TBD | Yes | N/A | No | No | TBD | Zaino proxies to Zebra. |
| `getbestblockheightandhash` | Zebra | TBD | No | N/A | No | No | No-equiv | Returns height and hash together in one call. Zebra-specific; not in zcashd or Zaino. |

### Block lookup

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getblock` | Zebra / Zaino | TBD | Yes | N/A | No | No | TBD | Zaino proxies to Zebra. |
| `getblockhash` | Zebra | TBD | Yes | N/A | No | No | TBD | Not present in Zaino at pinned commit — call Zebra directly. |
| `getblockheader` | Zebra / Zaino | TBD | Yes | N/A | No | No | TBD | Zaino proxies to Zebra. |

### Transaction lookup

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getrawtransaction` | Zebra / Zaino / Zallet | TBD | Yes | Both | No | No | TBD | Zaino handles natively from its indexer. Present in all three components. |
| `decoderawtransaction` | Zallet | TBD | Yes | Both | No | No | TBD | Only in Zallet; not exposed by Zebra or Zaino at pinned commits. |
| `gettxout` | Zebra / Zaino | TBD | Yes | T | No | No | TBD | Zaino proxies to Zebra. |

### Address-indexed lookup

These methods query chain state by transparent address, without wallet context.
Useful for monitoring deposit addresses independently of the wallet.

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getaddressbalance` | Zebra / Zaino | TBD | Yes | T | No | No | TBD | Zaino handles natively from its indexer. |
| `getaddresstxids` | Zebra / Zaino | TBD | Yes | T | No | No | TBD | Zaino handles natively from its indexer. |
| `getaddressutxos` | Zebra / Zaino | TBD | Yes | T | No | No | TBD | Zaino handles natively from its indexer. |

### Mempool

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `getrawmempool` | Zebra / Zaino | TBD | Yes | Both | No | No | TBD | Zaino handles natively from its indexer. Core signal for mempool saturation. |
| `getmempoolinfo` | Zebra / Zaino | TBD | Yes | N/A | No | No | TBD | Zaino handles natively from its indexer. |

### Mempool notifications

The proposal explicitly requires testing RPC client notifications for mempool changes.
The notification mechanism in Z3 is unconfirmed (zcashd used ZMQ; Z3 may differ).

| Method / Mechanism | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| Mempool change notifications | TBD | TBD | Yes (ZMQ in zcashd) | Both | No | No | TBD | Mechanism in Z3 unconfirmed. May be ZMQ, gRPC streaming, or another protocol. |

### Wallet — accounts and addresses

Zallet uses an account model that replaces the per-address generation in zcashd.
An account is created first (`z_getnewaccount`), then addresses are derived from it
(`z_getaddressforaccount`). `getnewaddress` and `z_getnewaddress` do not exist in Zallet.

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_getnewaccount` | Zallet | TBD | No | Both | No | No | No-equiv | Replaces `getnewaddress` + `z_getnewaddress`. Creates a named account; derive addresses from it with `z_getaddressforaccount`. |
| `z_getaddressforaccount` | Zallet | TBD | No | Both | No | No | No-equiv | Derives a Unified Address (Orchard + transparent receivers) from an account. |
| `z_listaccounts` | Zallet | TBD | No | Both | No | No | No-equiv | Lists all accounts in the wallet. |
| `z_getaccount` | Zallet | TBD | No | Both | No | No | No-equiv | Returns details for a specific account. |
| `listaddresses` | Zallet | TBD | No | Both | No | No | No-equiv | Lists all wallet addresses grouped by source. |

### Wallet — balance

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_getbalances` | Zallet | TBD | No | Both | No | No | No-equiv | Replaces `getbalance` + `z_getbalance`. Returns balances for all spending authorities. |
| `z_gettotalbalance` | Zallet | TBD | Yes | Both | No | No | TBD | Transparent + shielded total. Same name as zcashd. |

### Wallet — address validation

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `validateaddress` | Zebra / Zaino / Zallet | TBD | Yes | T | No | No | TBD | Zaino proxies to Zebra. Present in all three components. |
| `z_validateaddress` | Zebra / Zaino | TBD | Yes | Z | No | No | TBD | Zaino proxies to Zebra. Not confirmed in Zallet at pinned commit. |

### Transaction creation

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_sendmany` | Zallet | TBD | Yes | Both | No | No | Deviation | `fee` parameter must be `null`; fee is always computed automatically via ZIP 317. Supports transparent and shielded outputs in one call. Returns an operation ID immediately (async). |

### Async operations

Shielded transactions require ZK proof generation, which takes time. Zallet handles this
asynchronously: `z_sendmany` returns an operation ID immediately, and these methods
are used to track it through to completion.

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_getoperationstatus` | Zallet | TBD | Yes | Z | No | No | TBD | Returns current status of one or more async operations by ID. |
| `z_getoperationresult` | Zallet | TBD | Yes | Z | No | No | TBD | Returns and removes result of completed async operations. |
| `z_listoperationids` | Zallet | TBD | Yes | Z | No | No | TBD | Lists all known operation IDs, optionally filtered by status. |

### Transaction broadcast

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `sendrawtransaction` | Zebra / Zaino | TBD | Yes | Both | No | No | TBD | Zaino proxies to Zebra. |

### Wallet — transaction history

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `z_listunspent` | Zallet | TBD | Yes | Z | No | No | Deviation | Lists unspent shielded notes. Response format changed from zcashd: `amount` renamed to `value`; new fields added. |
| `z_listtransactions` | Zallet | TBD | No | Both | No | No | No-equiv | Lists transactions filterable by account and block range. Not in zcashd. |

### Regtest

These methods are only meaningful in regtest mode. Required for controlling block
production and chain state during test scenarios.

| Method | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|
| `generate` | Zebra | TBD | Yes | N/A | No | No | TBD | Mine N blocks immediately. Confirmed present in Zebra at pinned commit. |

---

## Removed or replaced from zcashd

These methods existed in zcashd but are not present in Z3 at the pinned commits.

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

1. What is the complete RFP method list from the Foundation? See [`docs/rpc/proposed-method-scope.md`](proposed-method-scope.md).
2. What is the mempool notification mechanism in Z3 — ZMQ, gRPC streaming, or something else?
3. Is `z_validateaddress` present in Zallet at the pinned commit?
4. Which zcashd methods have no Z3 equivalent beyond those already listed above?
