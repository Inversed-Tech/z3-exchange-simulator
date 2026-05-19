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
| Binary name | `zainod` |
| MSRV | 1.95.0 |

---

## Prerequisites

Zaino is a Rust project. Building it requires:

- Rust toolchain (`rustup`, stable channel, minimum version **1.95.0**)
- A running Zebra instance (Zaino connects to Zebra; see [`zebra.md`](zebra.md))
- No additional system dependencies beyond the Rust toolchain

---

## Build instructions

```sh
git clone https://github.com/zingolabs/zaino external/zaino
cd external/zaino
git checkout 4ddbfd29c9f0e74f20b4d5bf81f51042aae4302a
cargo build --release
```

The release binary is at `external/zaino/target/release/zainod`.

---

## How Zaino connects to Zebra

Zaino connects to Zebra via **JSON-RPC** (standard HTTP POST). No special Zebra
configuration is required — Zaino uses Zebra's standard RPC endpoint.

| Item | Value |
|---|---|
| Connection protocol | JSON-RPC over HTTP |
| Zebra endpoint | Zebra's RPC `listen_addr` (default `127.0.0.1:18232` for regtest) |
| Zaino config field | `validator_jsonrpc_listen_address` |
| Authentication | `validator_cookie_path` (recommended) or `validator_user` / `validator_password` |
| Zebra special config needed? | No — standard RPC endpoint is sufficient |

Zaino config snippet pointing at Zebra (example config uses port 18231 — this is the
port Zebra is configured to listen on, not a fixed default):

```toml
[validator_settings]
validator_jsonrpc_listen_address = "127.0.0.1:18231"
validator_cookie_path = "/path/to/zebra/cookie"
```

Note: sensitive fields (`password`, `cookie`, `secret`) cannot be set via environment
variables — they must be in the config file.

---

## RPC and data access

Zaino exposes two interfaces to clients:

| Interface | Default address | Purpose |
|---|---|---|
| JSON-RPC | Configured via `json_server_settings` | Zcash JSON-RPC compatibility layer |
| gRPC (LightWallet protocol) | `127.0.0.1:8137` | Compact block and transaction streaming |

The JSON-RPC interface provides backwards-compatible Zcash RPC methods. The gRPC
interface implements the LightWallet protocol (`CompactTxStreamer`) for light clients.

For the simulator, we use Zaino's **JSON-RPC interface**.

### Method routing

Zaino handles two classes of methods differently:

**Natively indexed** (Zaino computes these itself from its chain index):

| Method |
|---|
| `getrawtransaction` |
| `getaddressbalance` |
| `getaddresstxids` |
| `getaddressutxos` |
| `getrawmempool` |
| `getmempoolinfo` |

**Proxied to Zebra** (Zaino forwards these calls to Zebra's JSON-RPC and returns the result):

| Method |
|---|
| `getblockchaininfo` |
| `getblockcount` |
| `getbestblockhash` |
| `getblock` |
| `getblockheader` |
| `gettxout` |
| `sendrawtransaction` |
| `validateaddress` |
| `z_validateaddress` |

`getblockhash` is not present in Zaino at the pinned commit — route directly to Zebra.

Full detail in the [RPC coverage matrix](../rpc/rpc-coverage-matrix.md).

---

## Configuration notes

Zaino uses a **TOML** config file (`zainod.toml`). Key fields:

```toml
network = "Regtest"            # or "Mainnet" / "Testnet"

[validator_settings]
validator_jsonrpc_listen_address = "127.0.0.1:18232"   # Zebra RPC endpoint
validator_cookie_path = "/path/to/zebra/cookie"        # Zebra auth cookie

[json_server_settings]
# JSON-RPC listener config (port TBD — verify from example config)

[grpc_settings]
listen_address = "127.0.0.1:8137"   # Zaino's own gRPC port
```

Log level is set via the `RUST_LOG` environment variable (e.g. `RUST_LOG=zaino=info,zainod=info`).

Config can also be overridden via environment variables with prefix `ZAINO__` and double
underscore separators for nesting — except sensitive fields (password, cookie) which must
be in the config file.

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

- What is the exact JSON-RPC listener port in the default regtest config?
- Which RPC methods does Zaino handle natively vs. forward to Zebra?
- Are there known limitations or differences in regtest vs. mainnet behavior?
- Are there example regtest configs in the repository's CI or test suite we can reference?
