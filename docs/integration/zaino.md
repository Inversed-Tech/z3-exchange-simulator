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
| Pinned commit | TBD — to be confirmed at kickoff. See [`z3-commits.lock`](../../z3-commits.lock) |
| Language | Rust |

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

The release binary will be under `external/zaino/target/release/`. Binary name TBD —
verify from the repository.

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

## Known blockers

- Pinned commit not yet confirmed (kickoff dependency).
- Zebra integration (Week 2) must be complete before Zaino integration can begin.

---

## Questions for the Zaino team

- What is the minimum Rust toolchain version for the pinned commit?
- What is the connection mechanism between Zaino and Zebra (protocol, endpoint)?
- Which RPC methods does Zaino handle natively vs. forward to Zebra?
- Does Zaino expose a gRPC interface as well as JSON-RPC?
- What is the recommended regtest configuration?
- Are there known limitations or differences in regtest vs. mainnet behavior?
- Are there example setups in the repository's CI or test suite we can reference?
