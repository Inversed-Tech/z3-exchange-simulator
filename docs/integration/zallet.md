# Zallet Integration Notes

> Status: Draft — to be fully populated in Week 4 (Task T5 initial, Week 4 deep integration).

## What Zallet is

Zallet is the wallet component of the Z3 stack. It handles address generation, balance
tracking, transaction creation, transaction signing, and transaction broadcasting. Zallet is
the successor to the deprecated zcashd wallet.

## Repository

TBD — the proposal references Zallet via the zcashd deprecation notice
(https://z.cash/support/zcashd-deprecation/) but does not give a direct GitHub URL.
This must be confirmed with the Zcash Foundation before Week 4 integration begins.

## Pinned commit

See [`z3-commits.lock`](../../z3-commits.lock). TBD — to be confirmed at kickoff.

## Build instructions

TBD — verify once the repository URL is confirmed.

## Wallet initialization

TBD — verify how to initialize a Zallet wallet in regtest mode.

## RPC methods

TBD — verify which RPC methods Zallet exposes for:
- address generation (transparent and shielded),
- balance queries,
- transaction creation,
- transaction signing,
- transaction broadcasting.

## Transparent wallet operations

TBD

## Shielded wallet operations

TBD

## Configuration notes

TBD

## Known blockers

- Repository URL not yet confirmed.

## Questions for the Zallet team

- What is the correct GitHub repository URL for Zallet?
- Which RPC methods are supported in the target commit?
- How does Zallet connect to Zaino and/or Zebra?
- What is the recommended initialization sequence in regtest?
- Is transparent-only operation supported if shielded support is incomplete?
