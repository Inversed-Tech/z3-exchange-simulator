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

## Regtest override set (2026-07-29)

The frozen pins above cannot produce a non-zero confirmation rate: Zallet
`v0.1.0-alpha.3`'s `z_sendmany` passes a shielded-only spend policy to the proposal
builder, so it cannot spend from any pool (measured;
[`scripts/experiments/funding-probe.sh`](../../scripts/experiments/funding-probe.sh)).
Every live run instead uses this upstream-coherent override set, applied via
`Z3_{ZEBRA,ZAINO,ZALLET}_IMAGE` in `external/z3/.env.regtest` (the Z3 compose commit
stays as pinned above):

| Entity | Version / image tag | Pinned commit |
|---|---|---|
| Zebra | `v6.0.0` (`zfnd/zebra:6.0.0`) | `bb41d69013edbfa8594bb097fa751f47eeb31445` |
| Zaino | `0.6.0` (`zingodevops/zainod:0.6.0-no-tls`) | `17963672d0c2cad97dd12bd38bbf1b6fd232c8c5` |
| Zallet | `v0.1.0-beta.1` (`z3sim/zallet:v0.1.0-beta.1`) | `5be0f4861feedc47978102c627c6293dea2d7838` |

No upstream Zallet image exists past `v0.1.0-alpha.3`; the beta.1 image is built
locally from the official release tarball by
[`scripts/dev/zallet-release-image/build.sh`](../../scripts/dev/zallet-release-image/build.sh).
Full evidence trail in [`docs/regtest-funding-plan.md`](../regtest-funding-plan.md).

`src/metrics/manifest.rs::read_z3_commits` prefers a component's commit under this
override block over its frozen pin, so `manifest.json` for any run against this stack
records the versions actually exercised, not the non-functional frozen ones.

---

## How benchmark runs record pins

Every experiment run captures a manifest at `experiments/runs/<run-id>/manifest.json`.
The manifest records the exact state of all components at the time of the run:

```json
{
  "run_id": "<timestamp-or-uuid>",
  "run_started_at": "<ISO-8601 timestamp>",
  "run_completed_at": "<ISO-8601 timestamp, or null while the run is in flight>",
  "simulator_commit": "<git SHA of this repository, via `git rev-parse HEAD`>",
  "zebra_commit": "<SHA from z3-commits.lock — override commit if set, else the frozen pin>",
  "zaino_commit": "<SHA from z3-commits.lock — override commit if set, else the frozen pin>",
  "zallet_commit": "<SHA from z3-commits.lock — override commit if set, else the frozen pin>",
  "scenario_name": "<name from scenario YAML>",
  "scenario_config_hash": "<SHA-256 of scenario YAML content>",
  "target_tps": "<load_target_tps from the scenario YAML>"
}
```

This matches `RunManifest` in `src/metrics/manifest.rs`. The manifest is generated
automatically at the start of each run (component commits filled in by
`read_z3_commits`, preferring an `overrides:` entry over its frozen pin — see above). A
finding in the report always references a `run_id`, which in turn references this
manifest.
