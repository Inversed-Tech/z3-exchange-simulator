# Zaino Integration Notes

Integration reference for the Zaino blockchain data and indexing layer. Covers build,
configuration, connection to Zebra, RPC surface, and open questions to resolve during
integration.

For a plain-English explanation of what Zaino is and its role in the stack, see
[`docs/architecture/z3-overview.md`](../architecture/z3-overview.md).

---

## Repository and pinned commit

| Field | Value |
|---|---|
| Repository | https://github.com/zingolabs/zaino |
| Pinned commit | `4ddbfd29c9f0e74f20b4d5bf81f51042aae4302a` (dev, 2026-05-12) — see [`z3-commits.lock`](../../z3-commits.lock) |
| Language | Rust |
| Binary name | TBD — verify from build output |

---

## Prerequisites

Zaino is a Rust project. Building it requires:

- Rust toolchain (`rustup`, stable channel — verify minimum version from the repository)
- A running Zebra instance (Zaino connects to Zebra; see [`zebra.md`](zebra.md))
- Any additional system dependencies TBD — verify from the Zaino repository README

---

## Build instructions

> TBD — verify exact steps from the Zaino repository README.

Expected approach (standard for a Rust binary project):

```sh
# Clone at the pinned commit (once z3-commits.lock is populated)
git clone https://github.com/zingolabs/zaino external/zaino
cd external/zaino
git checkout <pinned-commit>

# Build
cargo build --release
```

The release binary will be under `external/zaino/target/release/`. Verify the binary
name from the repository (TBD).

---

## How Zaino connects to Zebra

> TBD — verify the connection mechanism and required configuration before integration.

Zaino is expected to connect to a running Zebra node to read chain state. Items to
identify:

| Item | Status |
|---|---|
| Connection protocol (gRPC, HTTP, socket?) | TBD |
| Zebra endpoint that Zaino connects to | TBD |
| Zaino config fields for pointing at Zebra | TBD |
| Whether Zebra needs special config to accept Zaino | TBD |

**How to verify:** Check the Zaino repository README, `docs/`, and example config files.
Look at Zaino's CI setup to see how it starts Zebra as a dependency.

---

## RPC and data access

> TBD — the exact division of RPC methods between Zaino and Zebra must be verified
> during integration.

Key questions for the coverage matrix:

| Question | Status |
|---|---|
| Which RPC methods does Zaino expose to clients? | TBD |
| Which methods does Zaino originate (handles itself)? | TBD |
| Which methods does Zaino forward to Zebra? | TBD |
| Does Zaino expose a gRPC interface in addition to JSON-RPC? | TBD |
| Default RPC/gRPC ports | TBD |

**How to verify:** Review Zaino's source (look for an RPC handler module) and its
documentation. Cross-reference with the Zebra RPC method list to map the routing.

---

## Configuration notes

> TBD — verify config format and required fields from the repository.

Expected items to configure:

- Zebra connection endpoint
- Zaino RPC listener address and port
- Log level
- Chain data / cache directory (if applicable)

---

## Startup order

Expected: **Zebra must be running before Zaino starts.** Zaino likely fails or exits if
it cannot connect to Zebra at startup. Verify this assumption during integration.

The recommended startup sequence for local development is expected to be:
1. Start Zebra in regtest mode
2. Wait for Zebra RPC to be ready
3. Start Zaino, pointing it at Zebra
4. Start Zallet (see [`zallet.md`](zallet.md))

---

## RPC methods (confirmed at pinned commit)

Source: `zaino-serve/src/rpc/jsonrpc/service.rs`

Simulator-relevant methods: `getblockchaininfo`, `getblockcount`, `getbestblockhash`,
`getblock`, `getblockheader`, `getrawtransaction`, `gettxout`, `getaddressbalance`,
`getaddresstxids`, `getaddressutxos`, `getrawmempool`, `getmempoolinfo`,
`sendrawtransaction`, `validateaddress`, `z_validateaddress`.

Note: `getblockhash` is present in Zebra but not confirmed in Zaino at the pinned commit.

Full detail in [`docs/rpc/rpc-coverage-matrix.md`](../rpc/rpc-coverage-matrix.md).

---

## Known blockers

- Zebra integration must be complete before Zaino integration can begin.
- Pinned commit is on the `dev` branch — confirm with Foundation that this is the intended branch.

---

## Questions for the Zaino team

- What is the minimum Rust toolchain version for the pinned commit?
- What is the connection mechanism between Zaino and Zebra (protocol, endpoint)?
- Which RPC methods does Zaino handle natively vs. forward to Zebra?
- Does Zaino expose a gRPC interface as well as JSON-RPC?
- What is the recommended regtest configuration?
- Are there known limitations or differences in regtest vs. mainnet behavior?
- Are there example setups in the repository's CI or test suite we can reference?
