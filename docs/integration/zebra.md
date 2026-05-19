# Zebra Integration Notes

Integration reference for the Zebra full node. Covers build, regtest setup, RPC
configuration, and open questions to resolve during integration.

For a plain-English explanation of what Zebra is and its role in the stack, see
[`docs/architecture/z3-overview.md`](../architecture/z3-overview.md).

---

## Repository and pinned commit

| Field | Value |
|---|---|
| Repository | https://github.com/ZcashFoundation/zebra |
| Pinned commit | TBD — to be confirmed at kickoff. See [`z3-commits.lock`](../../z3-commits.lock) |
| Language | Rust |

---

## Prerequisites

Zebra is a Rust project. Building it requires:

- Rust toolchain (`rustup`, stable channel — verify minimum version from the repository's
  `rust-toolchain.toml` or README)
- Standard system build tools (C compiler, `pkg-config`, etc. — exact requirements TBD,
  verify from repository)

Run `make setup` to check that your local Rust toolchain is installed.

---

## Build instructions

> TBD — verify exact steps from the Zebra repository README.

Expected approach (standard for a Rust binary project):

```sh
# Clone at the pinned commit (once z3-commits.lock is populated)
git clone https://github.com/ZcashFoundation/zebra external/zebra
cd external/zebra
git checkout <pinned-commit>

# Build
cargo build --release
```

The release binary will be under `external/zebra/target/release/`. The binary name is
TBD — verify from the repository (likely `zebrad` based on convention, but do not assume).

---

## Starting Zebra in regtest mode

> TBD — verify the exact config and flags from the Zebra repository documentation.

Zebra is configured via a TOML file (verify format and required fields from the
repository). Key settings to identify:

- How to set the network to `regtest`
- RPC listener address and port
- Chain data directory
- Log level

**How to verify:** Check the Zebra repository for:
- `README.md` or `book/` documentation on regtest setup
- Example config files (look for `zebrad.toml` or similar in `zebrad/` or `tests/`)
- CI scripts that start Zebra in regtest (`tests/`, `.github/workflows/`)

---

## RPC endpoint

Zebra exposes a JSON-RPC endpoint that the simulator will call. Details to confirm:

| Item | Status |
|---|---|
| Default RPC port (regtest) | TBD |
| Authentication required? | TBD |
| Methods exposed directly by Zebra | TBD — see RPC coverage matrix |
| Methods delegated to Zaino | TBD |

**How to verify:** Check Zebra's RPC documentation and the `zebrad/src/components/rpc/`
source directory for the list of implemented methods.

---

## Connection to other components

In the expected topology, Zaino connects to Zebra to retrieve chain data. Zallet may
also connect to Zebra directly for some operations (TBD).

Expected startup order: **Zebra first**, then Zaino, then Zallet. Verify this assumption
during integration.

---

## Known blockers

- Pinned commit not yet confirmed (kickoff dependency).

---

## Questions for the Zebra team

- What is the minimum Rust toolchain version required for the pinned commit?
- What is the recommended regtest config file structure?
- Which RPC methods does Zebra expose directly, and which require Zaino?
- Are there known regtest limitations compared to mainnet that affect RPC behavior?
- Are there example regtest setups in the repository's CI or test suite we can reference?
