# Zallet Integration Notes

Integration reference for the Zallet wallet component. Covers build, wallet
initialization, RPC surface, transparent and shielded operations, and open questions to
resolve during integration.

For a plain-English explanation of what Zallet is and its role in the stack, see
[`docs/architecture/z3-overview.md`](../architecture/z3-overview.md).

---

## Repository and pinned commit

| Field | Value |
|---|---|
| Repository | https://github.com/zcash/wallet |
| Pinned commit | `05926f3f3ec1b1d90348ae899628cc0e28547ef3` — see [`z3-commits.lock`](../../z3-commits.lock) |
| Language | Rust (Edition 2024, MSRV 1.85) |
| Binary name | `zallet` |

---

## Prerequisites

- Rust toolchain (`rustup`, stable channel, minimum version 1.85)
- A running Zebra or Zaino instance (the exact connection dependency is TBD — see
  [Connection to other Z3 components](#connection-to-other-z3-components) below)
- Any additional system dependencies TBD — verify from the repository README

---

## Build instructions

```sh
git clone https://github.com/zcash/wallet external/zallet
cd external/zallet
git checkout 05926f3f3ec1b1d90348ae899628cc0e28547ef3
cargo build --release
```

Binary: `external/zallet/target/release/zallet`

---

## Wallet initialization

> TBD — verify the wallet initialization procedure for regtest before Week 4.

Items to identify:

| Item | Status |
|---|---|
| How to create a new wallet in regtest | TBD |
| How to seed the wallet deterministically | TBD |
| How to unlock/decrypt the wallet (if applicable) | TBD |
| Where wallet data is stored | TBD |

A key requirement for the simulator is **deterministic wallet seeding**: given the same
seed value, the same set of accounts and addresses must be generated every run. Confirm
whether Zallet supports this directly or whether the simulator must manage it externally.

---

## Connection to other Z3 components

> TBD — the exact connection topology must be verified during Week 4.

Zallet likely connects to Zaino and/or Zebra for chain data and transaction broadcasting.
Items to confirm:

| Item | Status |
|---|---|
| Does Zallet connect to Zaino, Zebra, or both? | TBD |
| Connection protocol | TBD |
| Required config fields for pointing Zallet at the rest of the stack | TBD |

---

## RPC methods

Verified against pinned commit `05926f3`. Full detail in
[`docs/rpc/rpc-coverage-matrix.md`](../rpc/rpc-coverage-matrix.md).

**Account and address management**

Zallet uses an account model — not the per-address model from zcashd. `getnewaddress`
and `z_getnewaddress` do not exist. The workflow is:
1. Create an account: `z_getnewaccount`
2. Derive an address from it: `z_getaddressforaccount`

| Method | What it does |
|---|---|
| `z_getnewaccount` | Create a new wallet account |
| `z_getaddressforaccount` | Derive a Unified Address from an account |
| `z_listaccounts` | List all accounts |
| `z_getaccount` | Details for a specific account |
| `listaddresses` | List all addresses grouped by source |

**Balances**

`getbalance` and `z_getbalance` do not exist. Replacements:

| Method | What it does |
|---|---|
| `z_getbalances` | Balances for all spending authorities |
| `z_gettotalbalance` | Transparent + shielded total |

**Transaction creation and tracking**

| Method | What it does |
|---|---|
| `z_sendmany` | Send a transaction (transparent and/or shielded outputs). Fee auto-computed via ZIP 317 — `fee` must be `null`. Returns an operation ID. |
| `z_getoperationstatus` | Check status of a pending async operation |
| `z_getoperationresult` | Retrieve result of a completed operation |
| `z_listoperationids` | List all operation IDs |

**Transaction inspection**

| Method | What it does |
|---|---|
| `getrawtransaction` | Fetch a raw transaction by ID |
| `decoderawtransaction` | Decode a raw transaction hex |
| `z_listunspent` | List unspent shielded notes |
| `z_listtransactions` | List transactions, filterable by account |
| `validateaddress` | Validate a transparent address |

---

## Shielded transaction support

Shielded transactions are **fully implemented** in the pinned commit. Both the Sapling
and Orchard value pools are supported. `z_sendmany` handles all flow types:
T→T, T→Z, Z→T, Z→Z.

ZK proof generation happens asynchronously — `z_sendmany` returns an operation ID
immediately, and the result is retrieved via `z_getoperationresult` once proving
completes. The proving time must be measured and recorded per run.

---

## Configuration notes

> TBD — verify config format from the repository README.

Expected items to configure:

- Connection to Zaino/Zebra
- Wallet data directory
- RPC listener address and port
- Log level

---

## Startup order

Based on the expected stack topology:

1. Start Zebra in regtest mode
2. Start Zaino, connected to Zebra
3. **Start Zallet**, connected to Zaino/Zebra
4. Initialize wallet

Verify this sequence during integration.

---

## Known blockers

- Zebra and Zaino integrations must be complete before Zallet integration begins.
- Pinned commit set to the latest main as of 2026-05-18; to be reviewed with the Foundation at kickoff.

---

## Questions for the Zallet team / Foundation

- Does Zallet connect to Zaino, Zebra, or both — and via what protocol?
- Which RPC methods are available in the target commit?
- Is shielded transaction support complete in the target commit, or should we plan for
  transparent-only in Week 4?
- What is the recommended wallet initialization sequence for regtest?
- Is deterministic wallet seeding (same seed → same addresses) supported?
