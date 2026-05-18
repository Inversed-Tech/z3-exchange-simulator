# Zebra Integration Notes

> Status: Draft — to be fully populated in Week 1 (Task T5).

## What Zebra is

Zebra is the Zcash full node implementation maintained by the Zcash Foundation. In this
project, Zebra serves as the base layer: it validates and stores the local regtest chain
state that the simulator drives transactions against.

## Repository

https://github.com/ZcashFoundation/zebra

## Pinned commit

See [`z3-commits.lock`](../../z3-commits.lock). TBD — to be confirmed at kickoff.

## Build instructions

TBD — verify from the Zebra repository README.

## Regtest instructions

TBD — verify how to start Zebra in regtest mode.

## RPC endpoint notes

TBD — verify which RPC methods Zebra exposes directly vs. which are delegated to Zaino.

## Configuration notes

TBD

## Known blockers

None identified yet.

## Questions for the Zebra team

- Which RPC methods does Zebra expose directly in regtest mode?
- What is the recommended regtest startup procedure?
- Are there any known limitations in regtest vs mainnet behavior relevant to this project?
