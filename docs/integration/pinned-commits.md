# Pinned Z3 Commits

All findings in the report are anchored to specific, fixed commits of each Z3 component.
This makes every result reproducible: anyone with the same commits can re-run any
scenario and verify any finding.

## What is pinned

The **primary pin** is the Z3 meta-repository commit. The simulator drives the Z3
Docker Compose stack at this exact commit, which determines the Docker image versions
of each component. The component images are pinned by tag (and, where possible, by
sha256 digest), so the running versions are fixed regardless of later tag movement.

Individual component commits are recorded as reference for the findings report.
They are not used directly to build or run anything — the Z3 Docker images determine
the actual running versions.

All pins live in [`z3-commits.lock`](../../z3-commits.lock) at the repository root.

| Entity | Repository | Version / image tag | Pinned commit | Status |
|---|---|---|---|---|
| Z3 stack | https://github.com/ZcashFoundation/z3 | `main` | `dfb9d0eae6d3f67a3c184e0d8fcb1166e7740724` | Confirmed (Gustavo); frozen for the engagement |
| Zebra | https://github.com/ZcashFoundation/zebra | `v5.0.0` (`zfnd/zebra:5.0.0`) | `1e6519ea91e2d3035c20aadd4d9a40dcac2eed3a` | Confirmed — tag verified to resolve to this commit |
| Zaino | https://github.com/zingolabs/zaino | `0.4.0-rc.2` (`zingodevops/zainod:0.4.0-rc.2`, `-no-tls`) | `0cf4fd5008a7536e3495e3e377073faac1cb28f3` | Confirmed — tag verified to resolve to this commit |
| Zallet | https://github.com/zcash/wallet | `v0.1.0-alpha.3` (`electriccoinco/zallet:v0.1.0-alpha.3`) | `6fc85f68cf5ebe456160c6518255a83129e7d21c` | Confirmed by Foundation — see provenance note below |

> **Zallet provenance note.** The `v0.1.0-alpha.3` git tag resolves to
> `f0db32d23de36b9a8e0c48b4438d22ab076aca58`; the commit cited above is one commit later
> on the same line of history. We cite the Foundation-provided commit for attribution;
> the running version is fixed by the image tag/digest regardless.

---

## How benchmark runs record pins

Every experiment run captures a manifest at `experiments/runs/<run-id>/manifest.json`.
The manifest records the exact state of all components at the time of the run:

```json
{
  "run_id": "<timestamp-or-uuid>",
  "run_started_at": "<ISO-8601 timestamp>",
  "run_completed_at": "<ISO-8601 timestamp, null until the run finishes>",
  "simulator_commit": "<git SHA the running binary was compiled from, embedded at build time>",
  "zebra_commit": "<SHA from z3-commits.lock — the override commit when one exists, else the frozen pin>",
  "zaino_commit": "<SHA from z3-commits.lock — the override commit when one exists, else the frozen pin>",
  "zallet_commit": "<SHA from z3-commits.lock — the override commit when one exists, else the frozen pin>",
  "scenario_name": "<name from scenario YAML>",
  "scenario_config_hash": "<SHA-256 of scenario YAML content>",
  "target_tps": "<load_target_tps from the scenario YAML>",
  "timeouts": {
    "rpc_timeout_ms": "<effective RPC transport timeout>",
    "operation_poll_interval_ms": "<effective async-operation poll interval>",
    "max_operation_wait_ms": "<effective async-operation wait budget>",
    "confirmation_poll_interval_ms": "<effective confirmation poll interval>",
    "max_confirmation_wait_ms": "<effective confirmation wait budget>"
  }
}
```

This manifest is generated automatically at the start of each run (`run_completed_at`
and the final content are written when the run finishes). A finding in the report
always references a `run_id`, which in turn references this manifest. See
[`src/metrics/manifest.rs`](../../src/metrics/manifest.rs) for the exact schema and
[`docs/architecture/observability.md`](../architecture/observability.md) for the
`timeouts` rationale.
