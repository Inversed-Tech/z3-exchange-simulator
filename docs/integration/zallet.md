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
| Language | Rust (expected, based on the broader Z3 stack) |

---

## Prerequisites

> TBD — verify once the repository URL is confirmed.

Expected requirements (based on the Rust-based Z3 stack):

- Rust toolchain (`rustup`, stable — verify minimum version from the repository)
- A running Zebra or Zaino instance (the exact connection dependency is TBD)
- Any additional system dependencies TBD

---

## Build instructions

> TBD — verify from the Zallet repository README once the URL is confirmed.

Expected approach:

```sh
git clone https://github.com/zcash/wallet external/zallet
cd external/zallet
git checkout 05926f3f3ec1b1d90348ae899628cc0e28547ef3

# Build
cargo build --release
```

Binary name and path TBD.

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

> TBD — the complete list of Zallet RPC methods must be verified from the repository.
> Do not rely on the table below being accurate until confirmed.

Expected wallet operations (based on zcashd equivalents — subject to change):

| Operation | Expected method | Transparent | Shielded | Status |
|---|---|---|---|---|
| Generate new address | TBD | Yes | Yes | TBD |
| Query balance | TBD | Yes | Yes | TBD |
| Create and send transaction | TBD | Yes | Yes | TBD |
| Check async operation status | TBD | — | Yes | TBD |
| List unspent outputs | TBD | Yes | TBD | TBD |
| Import address / viewing key | TBD | TBD | TBD | TBD |

The RPC coverage matrix at [`docs/rpc/rpc-coverage-matrix.md`](../rpc/rpc-coverage-matrix.md)
will be the authoritative record once method names are verified.

---

## Transparent wallet operations

> TBD — verify once the Zallet repository is accessible.

Transparent operations use standard Zcash transparent addresses (t-addresses), which
behave similarly to Bitcoin addresses. The simulator needs:

- Address generation for synthetic deposit accounts
- Balance queries per address
- Transaction creation (T→T transfers)
- Transaction broadcast

Whether Zallet exposes these as separate RPC methods or through a unified interface is TBD.

---

## Shielded wallet operations

> TBD — verify once the Zallet repository is accessible.

Shielded operations use Zcash shielded addresses (z-addresses / Orchard/Sapling pools).
The simulator needs:

- Shielded address generation
- Shielded balance queries
- Shielded transaction creation (T→Z, Z→T, Z→Z)
- Async operation tracking (shielded transactions typically require proving time)

Key question: does the target pinned commit fully support shielded operations, or is
transparent-only a realistic Week 4 baseline? Confirm at kickoff.

---

## Configuration notes

> TBD — verify config format from the repository once URL is confirmed.

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

Verify this sequence during Week 4 integration.

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
