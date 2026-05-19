# Pinned Z3 Commits

Reference for why commit pinning matters, how the current pins are managed, and how
each simulator run records the exact versions it ran against.

---

## Why pinning matters

A finding is only meaningful if it is reproducible. Without pinning, a bug reported
against "the current Zebra" cannot be verified a week later if Zebra has since changed.

Pinning specific commits of Zebra, Zaino, and Zallet at the start of the engagement
ensures that:

- every benchmark run produces results comparable to every other run,
- the Foundation and component teams can reproduce any finding exactly,
- the findings report has a clear, verifiable scope ("this is what we found against
  these specific versions").

The pinned commits are fixed for the duration of the engagement. All findings in the
report are anchored to these specific versions.

---

## Current pinned commits

All pins live in [`z3-commits.lock`](../../z3-commits.lock) at the repository root.
This file is the authoritative source; the table below is a human-readable summary.

| Component | Repository | Pinned commit | Branch |
|---|---|---|---|
| Zebra | https://github.com/ZcashFoundation/zebra | `d4cd662c716382f6397d2a730148025a1ca79fec` | main |
| Zaino | https://github.com/zingolabs/zaino | `4ddbfd29c9f0e74f20b4d5bf81f51042aae4302a` | dev |
| Zallet | https://github.com/zcash/wallet | `05926f3f3ec1b1d90348ae899628cc0e28547ef3` | main |

These commits are fixed for the engagement. The `scripts/dev/clone-z3.sh` script clones
each component at its pinned commit.

---

## How benchmark runs record pins

Every experiment run captures a manifest at `experiments/runs/<run-id>/manifest.json`.
The manifest records the exact state of all components at the time of the run:

```json
{
  "run_id": "<timestamp-or-uuid>",
  "run_started_at": "<ISO-8601 timestamp>",
  "simulator_commit": "<git SHA of this repository>",
  "zebra_commit": "<SHA from z3-commits.lock>",
  "zaino_commit": "<SHA from z3-commits.lock>",
  "zallet_commit": "<SHA from z3-commits.lock>",
  "scenario_name": "<name from scenario YAML>",
  "scenario_config_hash": "<SHA-256 of scenario YAML content>"
}
```

This manifest is generated automatically at the start of each run. A finding in the
report always references a `run_id`, which in turn references this manifest.

---

