# Proposed RPC Method Scope — Z3 Exchange Simulator

**Prepared by Inversed for Zcash Foundation review.**

This document lists the RPC methods we propose to exercise during the engagement.

---

## Context

The simulator drives the Z3 stack (Zebra, Zaino, Zallet) by calling these methods over
JSON-RPC. All testing runs in regtest — a local isolated chain with no real funds.

Method names and component assignments have been verified against the pinned commits:
Zebra `d4cd662`, Zaino `4ddbfd2`, Zallet `05926f3`.

---

## Proposed method list

### Chain and block state — served by Zebra / Zaino

| Method | What it does |
|---|---|
| `getblockchaininfo` | Confirm the node is connected and synced; identify the chain (regtest) |
| `getblockcount` | Current block height — used to count confirmations on deposits |
| `getbestblockhash` | Hash of the current chain tip |
| `getblock` | Fetch a full block's contents — used to detect incoming deposits |
| `getblockhash` | Get a block's hash by height |
| `getblockheader` | Block header data without the full transaction list |

### Transaction and UTXO lookup — served by Zebra / Zaino

| Method | What it does |
|---|---|
| `getrawtransaction` | Fetch a transaction by its ID |
| `gettxout` | Check whether a specific unspent output still exists on-chain |
| `getaddressbalance` | Transparent balance for one or more addresses — useful for deposit monitoring |
| `getaddresstxids` | All transactions involving a transparent address |
| `getaddressutxos` | All unspent outputs for a transparent address |

### Mempool — served by Zebra / Zaino

| Method | What it does |
|---|---|
| `getrawmempool` | All pending transaction IDs — used to monitor mempool fill and saturation |
| `getmempoolinfo` | Mempool size, byte count, and fee statistics |
| Mempool notifications | Push notification when mempool changes (mechanism TBD — was ZMQ in zcashd) |

### Wallet — accounts and addresses — served by Zallet

> **Note on the new account model:** Zallet no longer uses `getnewaddress` or
> `z_getnewaddress`. Instead, you create an *account* (a logical wallet entity, one per
> exchange user) and then derive addresses from it. This is a deliberate design change
> from zcashd.

| Method | What it does |
|---|---|
| `z_getnewaccount` | Create a new wallet account — one per synthetic exchange user |
| `z_getaddressforaccount` | Derive a deposit address from an account |
| `z_listaccounts` | List all accounts in the wallet |
| `listaddresses` | List all addresses in the wallet |
| `validateaddress` | Confirm a transparent address is valid before sending to it |
| `z_validateaddress` | Confirm a shielded address is valid |

### Wallet — balances — served by Zallet

| Method | What it does |
|---|---|
| `z_getbalances` | Balances for all spending authorities — replaces zcashd's `getbalance` and `z_getbalance` |
| `z_gettotalbalance` | Combined transparent + shielded total balance |

### Transaction creation and tracking — served by Zallet

> **Note on fees:** `z_sendmany` no longer accepts a `fee` parameter. The fee is always
> computed automatically using the ZIP 317 formula and cannot be manually overridden.

| Method | What it does |
|---|---|
| `z_sendmany` | Create and broadcast a transaction — supports transparent and shielded outputs in one call |
| `z_getoperationstatus` | Check whether a shielded transaction has finished its ZK proof generation |
| `z_getoperationresult` | Retrieve the result (txid) of a completed shielded transaction |
| `z_listoperationids` | List all pending and completed async operation IDs |

### Transaction broadcast — served by Zebra / Zaino

| Method | What it does |
|---|---|
| `sendrawtransaction` | Broadcast a signed transaction to the network |

### Regtest control — served by Zebra

| Method | What it does |
|---|---|
| `generate` | Mine N blocks on demand — required for advancing the chain and confirming transactions during testing |

---

## Total: 27 methods + mempool notification mechanism

---

## Questions for the Foundation

1. Does this list cover all methods required by the RFP, or are there methods we should
   add?
2. Are there methods on this list that are out of scope and can be deprioritised?
3. What is the mempool notification mechanism in Z3? (In zcashd it was ZMQ publish/subscribe;
   we need to confirm the Z3 equivalent before implementing the notification client.)

---

*Full coverage matrix with component routing, parity status, and implementation tracking:
[`docs/rpc/rpc-coverage-matrix.md`](rpc-coverage-matrix.md)*
