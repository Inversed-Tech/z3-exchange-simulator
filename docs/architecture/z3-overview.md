# Z3 Stack Overview

Plain-English overview of the Z3 stack for technical contributors who are new to Zcash.
No prior blockchain knowledge assumed.

---

## Purpose of this document

This document explains what each Z3 component is, how they fit together, what regtest
and RPC mean in practice, and where the simulator sits in relation to the stack. It is a
starting point for contributors, not a complete reference.

For integration-level detail (build instructions, config, known blockers), see the
per-component notes under [`docs/integration/`](../integration/).

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
as the modern replacement for the deprecated `zcashd` reference client.

### Zebra

**What it is:** The Zcash full node.

A full node is a program that maintains a complete, up-to-date copy of the Zcash
blockchain and participates in the peer-to-peer network. Zebra validates every transaction
and block according to the Zcash protocol rules and serves as the source of truth for
chain state.

**Role in this project:** Zebra is the base layer. Every transaction the simulator sends
ultimately flows through Zebra for validation and inclusion in the chain. In regtest mode
(see below), Zebra runs a private local chain that we control entirely.

**Repository:** https://github.com/ZcashFoundation/zebra

---

### Zaino

**What it is:** The blockchain data access and indexing layer.

Zaino sits between clients (like wallets or the simulator) and Zebra. It indexes chain
data, answers queries about blocks and transactions, and acts as an RPC passthrough for
certain methods. Think of it as a read-optimised service layer that shields clients from
having to talk directly to the full node for every data request.

**Role in this project:** The simulator is expected to use Zaino for chain data retrieval
and some RPC calls. Which methods Zaino handles vs. forwards to Zebra must be confirmed
during integration.

**Repository:** https://github.com/zingolabs/zaino

---

### Zallet

**What it is:** The wallet component.

A wallet manages private keys, derives addresses, tracks balances, constructs
transactions, and broadcasts them to the network. Zallet is the next-generation wallet
designed to replace the wallet functionality that was previously bundled inside `zcashd`.

**Role in this project:** The simulator uses Zallet for all wallet operations: generating
synthetic deposit addresses, querying balances, constructing and signing transactions
(both transparent and shielded), and broadcasting them.

**Repository:** https://github.com/zcash/wallet

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

This is the same interface style used by `zcashd` and Bitcoin Core. Most Z3 components
expose an RPC endpoint that the simulator's RPC client module calls directly.

The **RPC coverage matrix** (`docs/rpc/rpc-coverage-matrix.md`) tracks which methods the
simulator exercises, which component serves each, and how behavior compares to `zcashd`.

---

## How the simulator interacts with Z3

The simulator drives the Z3 stack by issuing RPC calls to its components. At a high
level:

1. The **scenario runner** reads a scenario config and provisions synthetic accounts.
2. It uses **Zallet** to generate deposit addresses, create transactions, and query
   balances.
3. **Zaino** provides chain data (block heights, transaction status, mempool state).
4. **Zebra** is the underlying node that validates and records everything on the local
   regtest chain.
5. The **metrics module** records latency, success/failure, and resource usage throughout.

---

## Expected local development topology

```
┌─────────────────────────────────────────┐
│  z3-exchange-simulator (this project)   │
│                                         │
│  ┌──────────┐    ┌───────────────────┐  │
│  │ Scenario │───▶│   RPC Client      │  │
│  │ Runner   │    │  (src/rpc/)       │  │
│  └──────────┘    └────────┬──────────┘  │
│                           │             │
└───────────────────────────┼─────────────┘
                            │ RPC calls
          ┌─────────────────┼─────────────────┐
          │                 │                  │
          ▼                 ▼                  ▼
      ┌────────┐       ┌─────────┐       ┌──────────┐
      │ Zallet │       │  Zaino  │       │  Zebra   │
      │(wallet)│       │(indexer)│       │ (node)   │
      └────────┘       └────┬────┘       └────┬─────┘
                            │                 │
                            └────────▶────────┘
                                   │
                         ┌─────────▼──────────┐
                         │  local regtest chain│
                         └────────────────────┘
```

**Important:** The exact routing between these components — which methods go to Zallet,
which to Zaino, which directly to Zebra — is not yet verified. The diagram above reflects
the expected topology based on each component's stated role. Actual routing will be
confirmed and corrected during integration testing.

---

## zcashd and the Z3 transition

`zcashd` is the original Zcash reference client. It bundled the full node, wallet, and
RPC server into a single binary. It is now deprecated.

The Z3 stack replaces `zcashd` by splitting those responsibilities:

| Old (`zcashd`) | New (Z3) |
|---|---|
| Full node | Zebra |
| Blockchain data / indexing | Zaino |
| Wallet | Zallet |

The RPC coverage matrix tracks which `zcashd` RPC methods have Z3 equivalents and where
there are behavioral differences.

---

## Open architecture questions

These must be answered during integration testing. Until then, treat any claim about
routing or method support as provisional.

- Which RPC methods does Zebra expose directly in regtest?
- Which methods does Zaino handle vs. forward to Zebra?
- Which methods does Zallet expose, and does it talk to Zaino, Zebra, or both?
- What is the startup order for the three components?
- Are there regtest-specific limitations in any component vs. mainnet?
