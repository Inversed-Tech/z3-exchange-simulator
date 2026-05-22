# Z3 Stack Overview

Plain-English overview of the Z3 stack for technical contributors who are new to Zcash.
No prior blockchain knowledge assumed.

---

## Purpose of this document

This document explains what each Z3 component is, how they fit together, what the Z3
meta-repository is, what regtest and RPC mean in practice, and where the simulator sits
in relation to the stack. It is a starting point for contributors, not a complete
reference.

For integration-level detail (how to run the stack, ports, config), see
[`docs/integration/z3.md`](../integration/z3.md).

---

## Background: what is Zcash?

Zcash is a cryptocurrency with strong privacy features. Like Bitcoin, it has a public
blockchain and a peer-to-peer network. Unlike Bitcoin, it supports **shielded
transactions** — transfers where the sender, receiver, and amount are cryptographically
hidden from public view using zero-knowledge proofs.

For this project, the key distinction is:

- **Transparent transactions (T→T):** sender, receiver, and amount are publicly visible
  on-chain, similar to Bitcoin.
- **Shielded transactions (Z→Z, T→Z, Z→T):** one or both sides of the transfer are
  private, using Zcash's shielded address pool.

The simulator must exercise both types, because exchanges handle both and each makes
different demands on the Z3 stack.

---

## The Z3 stack

Z3 refers to three components being developed by the Zcash Foundation and its partners
as the modern replacement for the deprecated `zcashd` reference client — plus a
**meta-repository** that ties them together as a deployable stack.

### Zebra

**What it is:** The Zcash full node.

A full node maintains a complete, up-to-date copy of the Zcash blockchain and
participates in the peer-to-peer network. Zebra validates every transaction and block
and serves as the source of truth for chain state.

**Role in this project:** Zebra is the base layer. Every transaction the simulator sends
ultimately flows through Zebra for validation and inclusion in the chain. In regtest mode,
Zebra runs a private local chain that we control entirely.

**Repository:** https://github.com/ZcashFoundation/zebra

---

### Zaino

**What it is:** The blockchain indexing layer.

Zaino provides indexed access to chain data — address balances, transaction history, UTXO
sets — and exposes the LightWallet gRPC interface (`CompactTxStreamer`) for light wallet
clients.

**Role in this project:** Zaino operates in two modes within the Z3 stack:

1. **As a library inside Zallet** — Zallet embeds Zaino's indexing code directly. When
   the simulator calls Zallet's wallet methods, Zaino's indexing runs transparently inside
   the same process.
2. **As a standalone gRPC container** — Zaino runs separately to serve light wallet
   clients via the LightWallet gRPC protocol on port 8137.

The simulator does not call Zaino directly via JSON-RPC. Zaino's latency is implicit in
Zallet method response times.

**Repository:** https://github.com/zingolabs/zaino

---

### Zallet

**What it is:** The wallet component.

A wallet manages private keys, derives addresses, tracks balances, constructs
transactions, and broadcasts them to the network. Zallet is the next-generation wallet
designed to replace the wallet functionality previously bundled inside `zcashd`.

**Role in this project:** The simulator uses Zallet for all wallet operations: generating
synthetic deposit addresses, querying balances, constructing and signing transactions
(both transparent and shielded), and broadcasting them.

**Repository:** https://github.com/zcash/wallet

---

### Z3 meta-repository

**What it is:** A Docker Compose orchestration repository.

The Z3 meta-repository (`https://github.com/ZcashFoundation/z3`) is not a Rust codebase
— it ties together Zebra, Zaino, and Zallet into a single deployable Docker Compose
stack. It provides:

- Pre-built Docker images for each component
- Configuration files for mainnet, testnet, and regtest
- A regtest initialization script
- An **RPC Router** (regtest only): a single JSON-RPC endpoint at `:8181` that
  transparently forwards calls to the correct backend

**The simulator drives the Z3 stack by starting this Docker Compose stack and issuing
all RPC calls to the RPC Router.** See [`docs/integration/z3.md`](../integration/z3.md).

**Repository:** https://github.com/ZcashFoundation/z3

---

## What regtest means

Regtest (short for "regression test mode") is a local, isolated blockchain that you run
on your own machine. It has no connection to the public Zcash network.

In regtest mode you can:

- start a fresh chain from block zero,
- mine new blocks on demand (no real proof-of-work required),
- control all accounts and funds,
- reset the chain state between experiments,
- run deterministically with a fixed seed,
- reproduce any result without depending on mainnet conditions.

**All simulator testing in this project runs in regtest.** There is no mainnet usage,
no real ZEC, and no real user data at any point.

---

## What RPC means in this context

RPC stands for Remote Procedure Call. In this project it means: the simulator sends
a structured request to a Z3 component over a local network connection, and the component
returns a structured response.

In practice, RPC calls look like HTTP POST requests with a JSON body:

```json
{
  "method": "getblockchaininfo",
  "params": [],
  "id": 1
}
```

The response is also JSON:

```json
{
  "result": { "chain": "regtest", "blocks": 42, ... },
  "error": null,
  "id": 1
}
```

---

## How the simulator interacts with Z3

The simulator drives the Z3 stack by issuing all RPC calls to the **RPC Router** — a
single endpoint at `:8181` that is part of the Z3 regtest stack. The router handles
forwarding each call to the correct backend (Zebra or Zallet) transparently.

At a high level:

1. The **scenario runner** reads a scenario config and provisions synthetic accounts.
2. It uses **Zallet** (via the router) to generate deposit addresses, create transactions,
   and query balances.
3. It uses **Zebra** (via the router) for chain data: block heights, transaction lookup,
   mempool state, and regtest block production.
4. The **metrics module** records per-method latency, success/failure, and resource usage
   throughout.

---

## Actual deployment topology

```
┌──────────────────────────────────────────────┐
│  z3-exchange-simulator (this project)        │
│                                              │
│  ┌──────────┐    ┌────────────────────────┐  │
│  │ Scenario │───▶│   RPC Client           │  │
│  │ Runner   │    │  (src/rpc/)            │  │
│  └──────────┘    └──────────┬─────────────┘  │
│                             │                │
└─────────────────────────────┼────────────────┘
                              │ JSON-RPC
                              ▼
                    ┌─────────────────┐
                    │   RPC Router    │  :8181
                    │  (Z3 regtest)   │
                    └────────┬────────┘
                             │ routes by method name
              ┌──────────────┴──────────────┐
              ▼                             ▼
        ┌──────────┐                  ┌──────────┐
        │  Zebra   │ :18232           │  Zallet  │ :28232
        │(full node│                  │ (wallet) │
        └──────────┘                  └────┬─────┘
              │                           │ Zaino embedded
              │                      ┌────▼──────┐
              └──────────────────────│   Chain   │
                                     │  (regtest)│
                                     └───────────┘
                                     
         ┌──────────┐
         │  Zaino   │ :8137  ← gRPC only, for light clients
         │  (gRPC)  │            not called by simulator
         └──────────┘
```

---

## zcashd and the Z3 transition

`zcashd` is the original Zcash reference client. It bundled the full node, wallet, and
RPC server into a single binary. It is now deprecated.

The Z3 stack replaces `zcashd` by splitting those responsibilities:

| Old (`zcashd`) | New (Z3) |
|---|---|
| Full node | Zebra |
| Blockchain data / indexing | Zaino (as Zallet library + standalone gRPC) |
| Wallet | Zallet |
| Single RPC endpoint | RPC Router (routes to Zebra or Zallet) |

The RPC coverage matrix tracks which `zcashd` RPC methods have Z3 equivalents and where
there are behavioral differences.
