# Zaino Integration Notes

> **Zaino is not a direct JSON-RPC target for the simulator.** It operates in two
> modes within the Z3 stack: as a library embedded inside Zallet (for indexing), and
> as a standalone gRPC container for light clients. See
> [`docs/integration/z3.md`](z3.md) for the primary stack integration reference.

Integration reference for Zaino's role in the Z3 stack and its standalone configuration.

For a plain-English explanation of Zaino's role, see
[`docs/architecture/z3-overview.md`](../architecture/z3-overview.md).

---

## Repository and pinned commit

| Field | Value |
|---|---|
| Repository | https://github.com/zingolabs/zaino |
| Pinned commit | `93a9495336e7ee6f28ab1b02d1959a23b459f035` (candidate — pending Z3 update confirmation) — see [`z3-commits.lock`](../../z3-commits.lock) |
| Branch | `dev` |
| Language | Rust |
| Binary name | `zainod` |
| MSRV | 1.95.0 |

---

## Zaino's role in the Z3 stack

Zaino operates in two modes simultaneously within the Z3 Docker Compose stack:

**1. Library inside Zallet**
Zallet embeds Zaino's indexing code directly as a Rust library dependency. When the
simulator calls Zallet's wallet methods (balance queries, transaction history, note
counts), Zaino's indexing runs transparently inside the Zallet process. This means
Zaino's latency is implicit in Zallet method response times — the simulator does not
call Zaino directly for these operations.

**2. Standalone gRPC container**
Zaino also runs as a separate Docker container exposing the LightWallet gRPC interface
(`CompactTxStreamer`) on port 8137 for light wallet clients (e.g. Zingo). The simulator
does not call this interface during normal operation.

Whether to test Zaino's gRPC interface directly is pending Foundation confirmation.

---

## Ports

| Interface | Port | Protocol | Used by simulator? |
|---|---|---|---|
| LightWallet gRPC | 8137 | gRPC | No (pending Foundation confirmation) |
| JSON-RPC | Configured via `json_server_settings` | JSON-RPC | No — Zaino is not in the JSON-RPC routing path |

---

## Standalone build (reference)

If building Zaino from source independently:

```sh
git clone https://github.com/zingolabs/zaino external/zaino
cd external/zaino
git checkout 93a9495336e7ee6f28ab1b02d1959a23b459f035
cargo build --release
```

The release binary is at `external/zaino/target/release/zainod`.

**Prerequisites:** Rust toolchain minimum **1.95.0** (highest MSRV in the stack). No
additional system dependencies beyond the Rust toolchain.

---

## How Zaino connects to Zebra (standalone)

When running standalone (not via Z3 Docker Compose), Zaino connects to Zebra via
JSON-RPC. Config snippet:

```toml
[validator_settings]
validator_jsonrpc_listen_address = "127.0.0.1:18231"
validator_cookie_path = "/path/to/zebra/cookie"
```

Note: sensitive fields (`password`, `cookie`) must be in the config file, not env vars.

---

## Open questions

- Foundation confirmation on whether Zaino's gRPC interface should be tested directly
