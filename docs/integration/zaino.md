# Zaino Integration Notes

> Status: Draft — to be fully populated in Week 3 (Task T5 initial, Week 3 deep integration).

## What Zaino is

Zaino is the blockchain data access, indexing, and RPC passthrough layer. In this project,
Zaino is expected to sit between the simulator's RPC client and Zebra, exposing chain data
and forwarding certain RPC calls.

## Repository

https://github.com/zingolabs/zaino

## Pinned commit

See [`z3-commits.lock`](../../z3-commits.lock). TBD — to be confirmed at kickoff.

## Build instructions

TBD — verify from the Zaino repository README.

## How Zaino connects to Zebra

TBD — verify the expected Zaino → Zebra connection mechanism and configuration.

## RPC and data access notes

TBD — verify which RPC methods Zaino exposes, which it forwards, and which it originates.

## Configuration notes

TBD

## Known blockers

None identified yet.

## Questions for the Zaino team

- Which RPC methods does Zaino expose?
- Which methods does it forward to Zebra vs. handle itself?
- What is the expected startup order (Zebra first, then Zaino)?
- Are there any known limitations relevant to this project's regtest usage?
