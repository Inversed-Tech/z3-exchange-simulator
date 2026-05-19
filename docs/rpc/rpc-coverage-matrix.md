# RPC Coverage Matrix

Tracks which RPC methods the simulator exercises, which Z3 component serves each method,
zcashd behavioral parity, and test status. Updated continuously through the engagement.

---

## Critical caveat

**The authoritative RFP method list has not yet been received.** The methods listed below
are drawn from the known zcashd RPC surface and the expected needs of exchange-like
operation. All "Required by RFP?" cells are TBD until the Foundation provides the list.

Component routing (which of Zebra, Zaino, or Zallet serves each method) is also
unverified until integration testing in Weeks 2–4. Do not treat the Component column as
confirmed until marked otherwise.

---

## Column definitions

| Column | Meaning |
|---|---|
| **Method** | RPC method name. Based on zcashd convention; actual Z3 method names may differ. |
| **Category** | Functional grouping — see category list below. |
| **Component** | Expected Z3 component that serves this method. TBD until verified. |
| **Required by RFP?** | Whether this method appears in the Foundation's RFP requirements. TBD until RFP list is received. |
| **zcashd equivalent?** | Whether an equivalent method existed in zcashd. |
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
| No-equiv | No zcashd equivalent exists |

---

## Matrix

### Chain info

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `getblockchaininfo` | chain info | TBD (Zebra/Zaino) | TBD | Yes | N/A | No | No | TBD | Returns chain name, sync status, current height. Key smoke-test method. |
| `getblockcount` | chain info | TBD (Zebra/Zaino) | TBD | Yes | N/A | No | No | TBD | Returns current block height. |
| `getbestblockhash` | chain info | TBD (Zebra/Zaino) | TBD | Yes | N/A | No | No | TBD | Returns hash of the tip block. |

### Block lookup

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `getblock` | block lookup | TBD (Zebra/Zaino) | TBD | Yes | N/A | No | No | TBD | Fetch block by hash or height. Verbosity parameter behavior may differ. |
| `getblockhash` | block lookup | TBD (Zebra/Zaino) | TBD | Yes | N/A | No | No | TBD | Returns block hash at a given height. |
| `getblockheader` | block lookup | TBD (Zebra/Zaino) | TBD | Yes | N/A | No | No | TBD | Returns block header data without full transaction list. |

### Transaction lookup

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `getrawtransaction` | tx lookup | TBD (Zebra/Zaino) | TBD | Yes | Both | No | No | TBD | Fetch raw transaction by txid. Verbose mode returns decoded fields. |
| `gettransaction` | tx lookup | TBD (Zallet) | TBD | Yes | Both | No | No | TBD | Wallet-aware tx lookup including confirmations and watch-only status. |
| `decoderawtransaction` | tx lookup | TBD (Zebra/Zaino) | TBD | Yes | Both | No | No | TBD | Decode a raw transaction hex without broadcasting. |

### Mempool

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `getrawmempool` | mempool | TBD (Zebra/Zaino) | TBD | Yes | Both | No | No | TBD | Returns txids of all mempool transactions. Core signal for mempool saturation. |
| `getmempoolinfo` | mempool | TBD (Zebra/Zaino) | TBD | Yes | N/A | No | No | TBD | Returns mempool size, bytes, and fee information. |
| `getmempoolentry` | mempool | TBD (Zebra/Zaino) | TBD | Yes | Both | No | No | TBD | Returns mempool data for a specific txid. |

### Mempool notifications

The proposal explicitly requires testing RPC client notifications for mempool changes.
The notification mechanism in Z3 is unconfirmed (zcashd used ZMQ; Z3 may differ).

| Method / Mechanism | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| Mempool change notifications | notifications | TBD | TBD | Yes (ZMQ in zcashd) | Both | No | No | TBD | Mechanism in Z3 is unconfirmed. May be ZMQ, gRPC streaming, or another protocol. Must verify during Week 3–4. |
| `getzmqnotifications` | notifications | TBD | TBD | Yes | N/A | No | No | TBD | Returns active ZMQ notification endpoints. Only relevant if Z3 uses ZMQ. |

### Wallet — address generation

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `getnewaddress` | wallet/address | Zallet | TBD | Yes | T | No | No | TBD | Generate a new transparent address. |
| `z_getnewaddress` | wallet/address | Zallet | TBD | Yes | Z | No | No | TBD | Generate a new shielded address. Address type (Orchard/Sapling) TBD — verify from Zallet. |
| `validateaddress` | wallet/address | TBD (Zallet/Zaino) | TBD | Yes | T | No | No | TBD | Validate a transparent address. |
| `z_validateaddress` | wallet/address | TBD (Zallet/Zaino) | TBD | Yes | Z | No | No | TBD | Validate a shielded address. |

### Wallet — balance

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `getbalance` | wallet/balance | Zallet | TBD | Yes | T | No | No | TBD | Query transparent balance for the wallet or a specific address. |
| `z_getbalance` | wallet/balance | Zallet | TBD | Yes | Z | No | No | TBD | Query shielded balance for a specific z-address. |
| `z_gettotalbalance` | wallet/balance | Zallet | TBD | Yes | Both | No | No | TBD | Returns transparent, private, and total balance in one call. |

### Transaction creation

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `sendtoaddress` | tx creation | Zallet | TBD | Yes | T | No | No | TBD | Create and broadcast a transparent-to-transparent transaction. |
| `z_sendmany` | tx creation | Zallet | TBD | Yes | Both | No | No | TBD | Create a transaction with multiple outputs, supports transparent and shielded. Primary method for mixed flows. |
| `createrawtransaction` | tx creation | TBD (Zebra/Zaino) | TBD | Yes | T | No | No | TBD | Construct a raw unsigned transaction. |
| `signrawtransaction` | tx creation | TBD (Zallet) | TBD | Yes | T | No | No | TBD | Sign a raw transaction with wallet keys. |

### Async operations

Shielded transactions in Zcash require proving time (zero-knowledge proof generation).
zcashd handled this by returning an operation ID immediately and making the result
available asynchronously. Whether Zallet uses the same pattern must be verified.

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `z_getoperationstatus` | async ops | Zallet | TBD | Yes | Z | No | No | TBD | Returns current status of one or more async operations by ID. |
| `z_getoperationresult` | async ops | Zallet | TBD | Yes | Z | No | No | TBD | Returns and removes result of completed async operations. |
| `z_listoperationids` | async ops | Zallet | TBD | Yes | Z | No | No | TBD | Lists all known async operation IDs. |

### Transaction broadcast

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `sendrawtransaction` | tx broadcast | TBD (Zebra/Zaino) | TBD | Yes | Both | No | No | TBD | Broadcast a signed raw transaction to the network. |

### Regtest-specific

These methods are only available or meaningful in regtest mode. Required for the
simulator to control block production and chain state during test scenarios.

| Method | Category | Component | Required by RFP? | zcashd equiv? | T/Z | Implemented? | Tested? | Parity | Notes |
|---|---|---|---|---|---|---|---|---|---|
| `generate` | regtest | TBD (Zebra) | TBD | Yes | N/A | No | No | TBD | Mine N blocks immediately. Required for advancing the chain and confirming transactions in regtest. Exact method name in Z3 TBD — may be `generatetoaddress` or a Zebra-specific command. |

---

## Categories

| Category | Description |
|---|---|
| chain info | Current blockchain state: height, hash, sync status |
| block lookup | Fetching block data by hash or height |
| tx lookup | Fetching transaction data by txid |
| mempool | Mempool state and transaction inspection |
| notifications | RPC client push notifications for chain/mempool events |
| wallet/address | Address generation and validation |
| wallet/balance | Balance queries by address or total |
| tx creation | Transaction construction, signing, and async shielded operations |
| async ops | Async operation tracking for shielded proving |
| tx broadcast | Submitting signed transactions |
| regtest | Block generation and chain control (regtest only) |

---

## Open questions

1. What is the complete RFP method list? This is the critical input for this matrix.
2. Which component serves each method (Zebra, Zaino, or Zallet)?
3. What is the mempool notification mechanism in Z3 (ZMQ, gRPC, other)?
4. Does Zallet use the async operation pattern for shielded transactions?
5. Which shielded address pool does `z_getnewaddress` use (Orchard, Sapling)?
6. Does `generate` (or equivalent) work in Zebra regtest? What is the exact method name?
7. Which zcashd methods have no Z3 equivalent in the target commit?
