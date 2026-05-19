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
| Binary name | `zebrad` |
| MSRV | 1.85.1 |

---

## Prerequisites

Zebra is a Rust project. Building it requires:

- Rust toolchain (`rustup`, stable channel, minimum version **1.85.1**)
- **libclang / LLVM** (`libclang-dev` on Debian/Ubuntu, `llvm` on macOS via Homebrew)
- A C++ compiler (`g++` or Xcode Command Line Tools on macOS)
- `protoc` (Protocol Buffers compiler) — optional but recommended

Run `make setup` to check that your local Rust toolchain is installed.

---

## Build instructions

```sh
git clone https://github.com/ZcashFoundation/zebra external/zebra
cd external/zebra
git checkout d4cd662c716382f6397d2a730148025a1ca79fec
cargo build --release --features indexer
```

The release binary is at `external/zebra/target/release/zebrad`.

The `indexer` feature enables the gRPC `Indexer` service used for mempool change
notifications. Build without it only if the gRPC stream is not needed.

---

## Starting Zebra in regtest mode

Zebra is configured via a TOML file (`zebrad.toml`). A minimal regtest config:

```toml
[network]
network = "Testnet"
# Additional regtest parameters are set under [network.testnet_parameters]

[rpc]
listen_addr = "127.0.0.1:18232"
indexer_listen_addr = "127.0.0.1:8230"   # gRPC indexer for mempool notifications
# Cookie auth is enabled by default; see "RPC authentication" below

[state]
cache_dir = "/tmp/zebra-regtest"
```

The RPC server is **disabled by default** — `listen_addr` must be explicitly set.

Regtest mode uses the Testnet network type with additional regtest parameters. The
exact `[network.testnet_parameters]` fields for regtest activation need to be verified
from Zebra's CI test configs (`tests/` or `.github/workflows/`).

---

## RPC endpoint

| Item | Value |
|---|---|
| Default RPC port (regtest/testnet) | `127.0.0.1:18232` |
| RPC enabled by default? | No — must set `listen_addr` in `[rpc]` config |
| Authentication | Cookie-based, enabled by default (`enable_cookie_auth = true`) |
| Cookie location | Configurable via `cookie_dir` in `[rpc]` config |

The simulator must read the cookie file to authenticate RPC calls, or disable cookie
auth in the Zebra config for local testing.

For the method list served by Zebra, see the [RPC coverage matrix](../rpc/rpc-coverage-matrix.md).

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

- What are the exact `[network.testnet_parameters]` fields required to activate regtest mode?
- Are there known regtest limitations compared to mainnet that affect RPC behavior?
- Are there example regtest configs in the repository's CI or test suite we can reference?
