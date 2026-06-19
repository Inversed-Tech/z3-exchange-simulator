# Zaino Integration Notes

> **Zaino is directly testable in Z3.** It runs as its own container exposing a
> lightwalletd-compatible `CompactTxStreamer` gRPC service and a zcashd-style JSON-RPC
> mirror. The simulator covers Zaino through its **JSON-RPC mirror** (regtest host port
> `:28237`); the gRPC streaming surface is documented but out of scope for this
> engagement. See [`docs/integration/z3.md`](z3.md) for the primary stack reference.

Integration reference for Zaino's role in the Z3 stack and its standalone configuration.

For a plain-English explanation of Zaino's role, see
[`docs/architecture/z3-overview.md`](../architecture/z3-overview.md).

---

## Repository and pinned commit

| Field | Value |
|---|---|
| Repository | https://github.com/zingolabs/zaino |
| Version | `0.4.0-rc.2` (image `zingodevops/zainod:0.4.0-rc.2`, `-no-tls` variant in regtest) |
| Pinned commit | `0cf4fd5008a7536e3495e3e377073faac1cb28f3` — see [`z3-commits.lock`](../../z3-commits.lock) |
| Language | Rust |
| Binary name | `zainod` |
| MSRV | 1.95.0 |

---

## Zaino's role in the Z3 stack

Zaino is the indexing layer. In Z3 it runs as its own container and exposes two
network interfaces, both directly reachable from the host:

**1. LightWallet gRPC (`CompactTxStreamer`)**
The lightwalletd-compatible gRPC service used by light wallet clients (e.g. Zingo),
on regtest host port `28137` (plaintext `h2c`, no client auth). The proto files can be
fetched from the Z3 repo via `scripts/vendor.sh zaino`, and the Z3 integration docs
include a `grpcurl` quickstart. **Out of scope for this engagement** (see Open items).

**2. zcashd-style JSON-RPC mirror**
A JSON-RPC endpoint on regtest host port `28237` that mirrors zcashd-style methods
backed by the indexer. **This is the surface the simulator exercises for Zaino.** The
simulator points a dedicated RPC client at it
([`RpcClient::for_zaino_mirror`](../../src/rpc/mod.rs)), which tags every call
`Backend::Zaino` so Zaino's latency is attributed to Zaino rather than folded into
Zallet response times.

> Zaino is also embedded as a library inside Zallet for the wallet's own indexing;
> that internal use is separate from the directly-testable interfaces above.

---

## Ports (regtest)

| Interface | Host port | Protocol | Used by simulator? |
|---|---|---|---|
| LightWallet gRPC (`CompactTxStreamer`) | 28137 | gRPC (h2c) | Not this engagement (documented) |
| JSON-RPC mirror | 28237 | JSON-RPC | **Yes** — primary Zaino coverage |

Mainnet uses `8137` / `8237` and testnet `18137` / `18237`; the simulator reads the
active network's ports from [`z3-contract.yaml`](z3.md) rather than hardcoding them.

---

## Driving the JSON-RPC mirror

```sh
# zcashd-style JSON-RPC mirror, regtest:
curl -d '{"method":"getinfo","params":[],"id":1}' http://127.0.0.1:28237
```

The simulator issues its typed Zebra-style read methods (e.g. `get_blockchain_info`,
`get_raw_transaction`) against a `for_zaino_mirror` client to record per-call latency
and success against the `Zaino` backend. See
[`docs/integration/z3.md`](z3.md) and the Z3 repo's `docs/regtest.md` for endpoint and
client details.

---

## Standalone build (reference)

If building Zaino from source independently:

```sh
git clone https://github.com/zingolabs/zaino external/zaino
cd external/zaino
git checkout 0cf4fd5008a7536e3495e3e377073faac1cb28f3   # 0.4.0-rc.2
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

## Open items

- Authentication on the JSON-RPC mirror in regtest: confirm whether the mirror requires
  credentials (the RPC Router uses `zebra`/`zebra`; the mirror is a separate endpoint).
- Direct gRPC `CompactTxStreamer` coverage (`GetMempoolStream` / `GetMempoolTx`,
  block streaming) is documented but out of scope for this engagement; if added later,
  vendor the proto files via `scripts/vendor.sh zaino` (fetched fresh, not committed).
