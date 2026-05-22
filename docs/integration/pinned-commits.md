# Pinned Z3 Commits

Reference for why commit pinning matters, how the current pins are managed, and how
each simulator run records the exact versions it ran against.

---

## Why pinning matters

A finding is only meaningful if it is reproducible. Without pinning, a bug reported
against "the current Z3" cannot be verified a week later if the stack has since changed.

Pinning ensures that:

- every benchmark run produces results comparable to every other run,
- the Foundation and component teams can reproduce any finding exactly,
- the findings report has a clear, verifiable scope.

The pinned commits are fixed for the duration of the engagement. All findings in the
report are anchored to these specific versions.

---

## What is pinned

The **primary pin** is the Z3 meta-repository commit. The simulator drives the Z3
Docker Compose stack at this exact commit, which determines the Docker image versions
of each component.

Individual component commits are recorded as reference for the findings report.
They are not used directly to build or run anything — the Z3 Docker images determine
the actual running versions.

All pins live in [`z3-commits.lock`](../../z3-commits.lock) at the repository root.

| Entity | Repository | Pinned commit | Status |
|---|---|---|---|
| Z3 stack | https://github.com/ZcashFoundation/z3 | TBD | Pending Foundation confirmation (Gustavo) |
| Zebra | https://github.com/ZcashFoundation/zebra | `aba329d6dca884f6d42bb4d36bda0010a071c2fc` | Candidate — pending Z3 update |
| Zaino | https://github.com/zingolabs/zaino | `93a9495336e7ee6f28ab1b02d1959a23b459f035` | Candidate — pending Z3 update |
| Zallet | https://github.com/zcash/wallet | `6fc85f68cf5ebe456160c6518255a83129e7d21c` | Candidate — pending Z3 update |

---

## How benchmark runs record pins

Every experiment run captures a manifest at `experiments/runs/<run-id>/manifest.json`.
The manifest records the exact state of all components at the time of the run:

```json
{
  "run_id": "<timestamp-or-uuid>",
  "run_started_at": "<ISO-8601 timestamp>",
  "simulator_commit": "<git SHA of this repository>",
  "z3_commit": "<SHA from z3-commits.lock>",
  "zebra_commit": "<SHA from z3-commits.lock>",
  "zaino_commit": "<SHA from z3-commits.lock>",
  "zallet_commit": "<SHA from z3-commits.lock>",
  "scenario_name": "<name from scenario YAML>",
  "scenario_config_hash": "<SHA-256 of scenario YAML content>"
}
```

This manifest is generated automatically at the start of each run. A finding in the
report always references a `run_id`, which in turn references this manifest.
