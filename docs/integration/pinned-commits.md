# Pinned Z3 Commits

Reference for why commit pinning matters, how the current pins are managed, how to
update them, and how each simulator run records the exact versions it ran against.

---

## Why pinning matters

A finding is only meaningful if it is reproducible. Without pinning, a bug reported
against "the current Zebra" cannot be verified a week later if Zebra has since changed.

Pinning specific commits of Zebra, Zaino, and Zallet at the start of the engagement
ensures that:

- every benchmark run produces results comparable to every other run,
- the Foundation and component teams can reproduce any finding exactly,
- the findings report has a clear, verifiable scope ("this is what we found against
  these specific versions"),
- if a component releases a fix mid-engagement, we can make a deliberate, coordinated
  decision about whether to rebase.

---

## Current pinned commits

All pins live in [`z3-commits.lock`](../../z3-commits.lock) at the repository root.
This file is the authoritative source; the table below is a human-readable summary.

| Component | Repository | Pinned commit | Status |
|---|---|---|---|
| Zebra | https://github.com/ZcashFoundation/zebra | TBD | Pending kickoff |
| Zaino | https://github.com/zingolabs/zaino | TBD | Pending kickoff |
| Zallet | TBD | TBD | Repo URL also pending |

Commits will be agreed with the Zcash Foundation at the kickoff call and written into
`z3-commits.lock` immediately after. The `scripts/dev/clone-z3.sh` script will then be
updated to clone each component at its pinned commit.

---

## How to update pins

Pin updates are rare and require team coordination. The process:

1. **Identify the need.** A pin update is warranted if a critical bug fix in a component
   is required to unblock the simulator, and waiting until the end of the engagement is
   not viable.

2. **Discuss with the Foundation.** Any pin change should be agreed with the Foundation
   before it happens, since it affects the scope of the findings report.

3. **Update `z3-commits.lock`.** Change the relevant `commit` field to the new hash.
   Add a `reason` field explaining why the pin changed:

   ```yaml
   zebra:
     repo: https://github.com/ZcashFoundation/zebra
     commit: <new-hash>
     previous_commit: <old-hash>
     pin_updated: "YYYY-MM-DD"
     reason: "Required fix for regtest RPC stability (issue #XXXX)"
   ```

4. **Re-run integration tests.** Verify the simulator still works correctly against the
   new commit before continuing benchmark runs.

5. **Note the change in the findings report.** The report must clearly state if pins
   changed mid-engagement and why.

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

## Open questions

| Question | Owner | Status |
|---|---|---|
| When will commits be confirmed — at kickoff or before? | Foundation | TBD |
| Who coordinates a pin update if a critical fix lands mid-engagement? | Oded / Foundation | TBD |
| Should the Zallet repo URL be confirmed before or at the kickoff call? | Foundation | TBD |
