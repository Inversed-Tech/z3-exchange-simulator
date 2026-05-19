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
| Pinned commit | `d4cd662c716382f6397d2a730148025a1ca79fec` (main, 2026-05-07) — see [`z3-commits.lock`](../../z3-commits.lock) |
| Language | Rust |
| Binary name | Likely `zebrad` — verify from build output |

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

The binary will be at `external/zebra/target/release/zebrad` (verify from build output).

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

## RPC methods (confirmed at pinned commit)

Source: `zebra-rpc/src/methods.rs`

Simulator-relevant methods: `getblockchaininfo`, `getblockcount`, `getbestblockhash`,
`getbestblockheightandhash`, `getblock`, `getblockhash`, `getblockheader`,
`getrawtransaction`, `gettxout`, `getaddressbalance`, `getaddresstxids`,
`getaddressutxos`, `getrawmempool`, `getmempoolinfo`, `sendrawtransaction`,
`validateaddress`, `z_validateaddress`, `generate`.

Full detail in [`docs/rpc/rpc-coverage-matrix.md`](../rpc/rpc-coverage-matrix.md).

---

## Known blockers

None currently. Pinned commit is set; integration work can begin.

---

## Questions for the Zebra team

- What is the minimum Rust toolchain version required for the pinned commit?
- What is the recommended regtest config file structure?
- Which RPC methods does Zebra expose directly, and which require Zaino?
- Are there known regtest limitations compared to mainnet that affect RPC behavior?
- Are there example regtest setups in the repository's CI or test suite we can reference?
