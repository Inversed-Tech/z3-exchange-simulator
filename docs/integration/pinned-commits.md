# Pinned Z3 Commits

> Status: Draft — to be fully populated in Week 1 (Task T5).

## Why pinning matters

Every finding in the final report must be reproducible. A finding is only meaningful if
it can be attributed to a specific state of the Z3 stack. Pinning commits at kickoff
ensures that:

- results from different runs can be compared,
- the Foundation and component teams can reproduce any finding,
- benchmark runs across the 12-week engagement are comparable.

## Current pinned commits

See [`z3-commits.lock`](../../z3-commits.lock).

| Component | Commit | Notes |
|---|---|---|
| Zebra | TBD | To be confirmed at kickoff |
| Zaino | TBD | To be confirmed at kickoff |
| Zallet | TBD | To be confirmed at kickoff; repo URL also TBD |

## How to update pins

TBD — update process and team coordination steps to be documented after kickoff.

## How benchmark runs record pins

Each experiment run manifest captures the exact commits used:

```json
{
  "simulator_commit": "TBD",
  "zebra_commit": "TBD",
  "zaino_commit": "TBD",
  "zallet_commit": "TBD",
  "scenario_config_hash": "TBD",
  "run_started_at": "TBD"
}
```

## Open questions

- When exactly will commits be confirmed — at the kickoff call or before?
- Who coordinates a pin update if a component releases a critical fix mid-engagement?
