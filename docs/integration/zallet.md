# Zallet Integration Notes

> **In normal usage, Zallet runs inside the Z3 Docker Compose stack.** The simulator
> does not start Zallet directly — see [`docs/integration/z3.md`](z3.md) for the primary
> integration reference. This document covers Zallet's standalone configuration for
> reference and for cases where building from source is needed.

Integration reference for the Zallet wallet component. Covers build, wallet
initialization, RPC surface, transparent and shielded operations, and open questions.

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

Wallet initialization in regtest is handled by the Z3 Docker Compose stack's
`./scripts/regtest-init.sh` script (see [`docs/integration/z3.md`](z3.md)).

Open items:
- Whether deterministic seeding (same seed → same accounts/addresses) is supported directly by Zallet, or must be managed externally by the simulator: TBD

---

## Connection to other Z3 components

Within the Docker Compose stack, Zallet connects to Zaino (embedded as a library) and
to Zebra for chain data and transaction broadcasting. Connection config is managed by
the Z3 repo's `config/regtest/zallet.toml`.

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

## Startup order

1. Start Zebra in regtest mode
2. Start Zaino, connected to Zebra
3. Start Zallet, connected to Zaino/Zebra
4. Initialize wallet
