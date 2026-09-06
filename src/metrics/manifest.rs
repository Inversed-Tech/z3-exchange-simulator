use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::error::MetricsError;
use crate::z3::ImageInfo;

/// The RPC transport timeout and confirmation/operation polling patience
/// actually in effect for a run, recorded so a low confirmation rate can be
/// distinguished from an impatient client.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct RunTimeouts {
    pub rpc_timeout_ms: u64,
    pub operation_poll_interval_ms: u64,
    pub max_operation_wait_ms: u64,
    pub confirmation_poll_interval_ms: u64,
    pub max_confirmation_wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub run_started_at: DateTime<Utc>,
    pub run_completed_at: Option<DateTime<Utc>>,
    pub simulator_commit: String,
    pub zebra_commit: String,
    pub zaino_commit: String,
    pub zallet_commit: String,
    pub scenario_name: String,
    pub scenario_config_hash: String,
    pub target_tps: f64,
    pub timeouts: RunTimeouts,
    /// Wall-clock start time of each lifecycle phase this run passed through
    /// (see `crate::data_model::Phase`), in the order they occurred. Absent
    /// (empty) on manifests written before phase instrumentation landed —
    /// `#[serde(default)]` lets those older manifests keep deserializing.
    #[serde(default)]
    pub phase_boundaries: Vec<PhaseBoundary>,
    /// Wall-clock instant `load_phase()` returned — i.e. the moment the
    /// Drain phase's own work (not the Z3 stack's subsequent teardown)
    /// finished. This, not `run_completed_at` (which includes teardown
    /// time), is the correct end boundary for the Drain phase's *measured*
    /// duration and for `confirmed_tx_throughput`'s elapsed-time window —
    /// both stop counting at this same instant. `None` when the run never
    /// reached this point (setup failed) or predates this field.
    #[serde(default)]
    pub load_and_drain_completed_at: Option<DateTime<Utc>>,
    /// SHA-256 hex digest of this run's effective `docker compose config`
    /// (images, env vars, ports, network layout, container-side paths),
    /// with checkout-location-dependent bind-mount source paths stripped
    /// first — see `z3::Z3Stack::compose_config_hash`. Two checkouts of
    /// identical logical configuration at different filesystem paths
    /// produce the same hash. Empty on manifests written before this field
    /// existed, or when the hash could not be computed (degrades to a
    /// warning rather than failing the run — this is evidence, not a
    /// correctness dependency).
    #[serde(default)]
    pub compose_config_hash: String,
    /// The image (repository:tag) and local content-addressed image ID
    /// Docker actually ran for each stack component — see
    /// `z3::Z3Stack::image_digests`. Empty on manifests written before this
    /// field existed, or when it could not be read.
    #[serde(default)]
    pub image_digests: Vec<ImageInfo>,
    /// Number of logical CPUs available to the process that ran this run,
    /// from `std::thread::available_parallelism()`. `0` on manifests
    /// written before this field existed.
    #[serde(default)]
    pub host_cpu_count: u32,
    /// The host's memory limit in bytes, when running under a constrained
    /// cgroup (a containerized CI runner, for instance). `None` on an
    /// unconstrained bare-metal or VM host, and on manifests written before
    /// this field existed.
    #[serde(default)]
    pub host_memory_limit_bytes: Option<u64>,
    /// Whether this run's starting chain state was freshly reset or reused
    /// from a prior run, and which reset generation it belongs to — see
    /// `StateIdentifier`. Defaulted on manifests written before this field
    /// existed.
    #[serde(default)]
    pub state: StateIdentifier,
    /// The scenario's pass/fail evaluation (see
    /// `scenarios::runner::result::AssertionOutcome`), persisted so the
    /// findings-report pipeline — which loads runs from disk, not from the
    /// in-memory `RunResult` a single invocation returns — can render it.
    /// `None` when the run never reached assertion evaluation (setup
    /// failed) or predates this field.
    #[serde(default)]
    pub assertion: Option<crate::scenarios::runner::result::AssertionOutcome>,
}

/// One lifecycle phase's start time, as recorded by
/// `crate::scenarios::runner::phase::PhaseTracker`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseBoundary {
    pub phase: crate::data_model::Phase,
    pub started_at: DateTime<Utc>,
}

/// State/snapshot provenance for a run's starting chain: distinguishes a
/// freshly-reset environment from one carrying over state from a prior run,
/// and ties both to a specific reset generation.
///
/// Deliberately cheap rather than exact (no Docker-volume content hash): a
/// reset-epoch counter plus the chain height recorded at that reset, the
/// actual chain height and hot-wallet balance observed at this run's start,
/// and an explicit fresh/reused classification computed from those numbers
/// — sufficient to distinguish fresh/reused/which-reset-generation without
/// leaving the judgment itself for a report reader to infer.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct StateIdentifier {
    /// Incremented once per `scripts/dev/regtest-reset.sh` execution
    /// against this run's specific environment; persisted at
    /// `configs/local/reset-epoch-<env_id>` (gitignored, alongside
    /// `env-id` — see `z3::env_id::reset_epoch_path`), scoped per
    /// environment id so a `--fresh-env` run never reads another
    /// environment's reset provenance. `0` when this specific environment
    /// has never been reset.
    pub reset_epoch: u64,
    /// Chain height observed at the start of this run (before warmup mining).
    pub chain_height_at_start: u64,
    /// Hot wallet's total balance (zatoshis) observed at the end of warmup,
    /// once it is confirmed funded.
    pub hot_wallet_balance_at_start_zat: u64,
    /// Whether this run's starting chain state was freshly reset or carried
    /// over from a prior run since the last reset — see
    /// `StateFreshness::classify`.
    pub freshness: StateFreshness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateFreshness {
    /// This run is the first to observe chain state since the last reset:
    /// `chain_height_at_start` does not exceed the height recorded at reset
    /// time.
    Fresh,
    /// A prior run already advanced the chain since the last reset:
    /// `chain_height_at_start` exceeds the height recorded at reset time.
    Reused,
}

impl Default for StateFreshness {
    /// Matches `StateIdentifier::default()`'s all-zero fields: height 0 does
    /// not exceed height-at-reset 0, so `Fresh` is the classification
    /// `classify` itself would produce for that case.
    fn default() -> Self {
        StateFreshness::Fresh
    }
}

impl StateFreshness {
    pub fn classify(chain_height_at_start: u64, height_at_reset: u64) -> Self {
        if chain_height_at_start <= height_at_reset {
            StateFreshness::Fresh
        } else {
            StateFreshness::Reused
        }
    }
}

/// Reads a `configs/local/reset-epoch-<env_id>` file (see
/// `z3::env_id::reset_epoch_path`, written by `regtest-reset.sh`'s last
/// step for that specific environment): two whitespace-separated fields,
/// `{epoch} {height_at_reset}`. Missing file or a malformed line degrades
/// to `(0, 0)` — "no reset has run against this environment yet" — rather
/// than failing the run, matching `env_id::resolve_env_id`'s and
/// `read_z3_commits`'s forgiving convention for optional, machine-written
/// local state.
pub fn read_reset_state(path: &Path) -> (u64, u64) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return (0, 0);
    };
    let mut fields = content.split_whitespace();
    let epoch = fields.next().and_then(|s| s.parse().ok());
    let height_at_reset = fields.next().and_then(|s| s.parse().ok());
    match (epoch, height_at_reset) {
        (Some(e), Some(h)) => (e, h),
        _ => (0, 0),
    }
}

pub fn write_manifest(path: &Path, manifest: &RunManifest) -> Result<(), MetricsError> {
    let json = serde_json::to_string_pretty(manifest).map_err(MetricsError::Serialization)?;
    std::fs::write(path, json).map_err(MetricsError::Io)?;
    Ok(())
}

pub fn read_manifest(path: &Path) -> Result<RunManifest, MetricsError> {
    let content = std::fs::read_to_string(path).map_err(MetricsError::Io)?;
    serde_json::from_str(&content).map_err(MetricsError::Serialization)
}

#[derive(Debug, Default, Deserialize)]
struct ComponentPin {
    #[serde(default)]
    commit: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Overrides {
    #[serde(default)]
    zebra: Option<ComponentPin>,
    #[serde(default)]
    zaino: Option<ComponentPin>,
    #[serde(default)]
    zallet: Option<ComponentPin>,
}

#[derive(Debug, Default, Deserialize)]
struct CommitsLock {
    #[serde(default)]
    zebra: Option<ComponentPin>,
    #[serde(default)]
    zaino: Option<ComponentPin>,
    #[serde(default)]
    zallet: Option<ComponentPin>,
    #[serde(default)]
    overrides: Overrides,
}

fn effective_commit(primary: Option<ComponentPin>, over: Option<ComponentPin>) -> String {
    // The `overrides:` block, when present for a component, records the commit
    // actually in effect at run time (applied via `Z3_*_IMAGE` env vars) — the
    // frozen top-level pin exists for contractual attribution but may not be
    // spendable at all (see docs/regtest-funding-plan.md). A run's manifest
    // must record what actually ran, not merely the first pin encountered in
    // the file.
    over.and_then(|o| o.commit)
        .or_else(|| primary.and_then(|p| p.commit))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Reads the commit actually in effect for each component: the `overrides:`
/// entry when one exists for that component, otherwise the frozen top-level
/// pin. Malformed or unreadable lock files degrade to `"unknown"` for every
/// field rather than panicking.
pub fn read_z3_commits(lock_path: &Path) -> (String, String, String) {
    let content = std::fs::read_to_string(lock_path).unwrap_or_default();
    let parsed: CommitsLock = serde_yaml::from_str(&content).unwrap_or_default();
    (
        effective_commit(parsed.zebra, parsed.overrides.zebra),
        effective_commit(parsed.zaino, parsed.overrides.zaino),
        effective_commit(parsed.zallet, parsed.overrides.zallet),
    )
}

/// The commit this binary was built from, embedded at compile time by `build.rs`.
///
/// Deliberately not a runtime `git rev-parse HEAD` call: that reports the
/// working tree's *current* commit, which silently diverges from the commit
/// actually running whenever the binary isn't rebuilt after a later commit.
pub fn read_simulator_commit() -> String {
    env!("SIMULATOR_GIT_COMMIT").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_manifest_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let m = RunManifest {
            run_id: "20260610T000000Z-smoke".into(),
            run_started_at: Utc::now(),
            run_completed_at: None,
            simulator_commit: "abc123".into(),
            zebra_commit: "zebra-sha".into(),
            zaino_commit: "zaino-sha".into(),
            zallet_commit: "zallet-sha".into(),
            scenario_name: "smoke".into(),
            scenario_config_hash: "sha256:deadbeef".into(),
            target_tps: 10.0,
            timeouts: RunTimeouts::default(),
            phase_boundaries: Vec::new(),
            load_and_drain_completed_at: None,
            compose_config_hash: String::new(),
            image_digests: Vec::new(),
            host_cpu_count: 0,
            host_memory_limit_bytes: None,
            state: StateIdentifier::default(),
            assertion: None,
        };
        write_manifest(&path, &m).unwrap();
        let back = read_manifest(&path).unwrap();
        assert_eq!(back.run_id, m.run_id);
        assert_eq!(back.zebra_commit, "zebra-sha");
        assert!(back.run_completed_at.is_none());
    }

    #[test]
    fn read_z3_commits_parses_lock_file() {
        let fixture = r#"zebra:
  repo: https://github.com/ZcashFoundation/zebra
  branch: main
  commit: sha-z
  status: verified
zaino:
  repo: https://github.com/zingolabs/zaino
  branch: main
  commit: sha-i
  status: verified
zallet:
  repo: https://github.com/nuttycom/zallet
  branch: main
  commit: sha-t
  status: verified
"#;
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("z3-commits.lock");
        std::fs::write(&lock, fixture).unwrap();
        let (z, i, t) = read_z3_commits(&lock);
        assert_eq!(z, "sha-z");
        assert_eq!(i, "sha-i");
        assert_eq!(t, "sha-t");
    }

    #[test]
    fn read_z3_commits_prefers_override_when_present() {
        // Mirrors the real z3-commits.lock shape: a frozen top-level pin that
        // cannot produce a non-zero confirmation rate, and an `overrides:`
        // block recording what the simulator actually runs against. A run's
        // manifest must reflect the latter, not the former — this is the bug
        // a live run (20260731T133332Z-smoke) exposed: the manifest reported
        // the frozen pin while the stack was demonstrably running overrides.
        let fixture = r#"zebra:
  commit: frozen-zebra
zaino:
  commit: frozen-zaino
zallet:
  commit: frozen-zallet

overrides:
  zebra:
    commit: override-zebra
  zallet:
    commit: override-zallet
"#;
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("z3-commits.lock");
        std::fs::write(&lock, fixture).unwrap();
        let (z, i, t) = read_z3_commits(&lock);
        assert_eq!(z, "override-zebra", "zebra has an override — must win");
        assert_eq!(
            i, "frozen-zaino",
            "zaino has no override — frozen pin stands"
        );
        assert_eq!(t, "override-zallet", "zallet has an override — must win");
    }

    #[test]
    fn read_z3_commits_against_real_lock_file_returns_the_active_overrides() {
        // Regression test tied to the actual repo file, not a fixture — this
        // is what a real run's manifest will now record. If the overrides
        // block is ever bumped again (as it was beta.1 -> beta.2 today) this
        // test's expected values must be updated alongside it.
        let lock_path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/z3-commits.lock"));
        let (zebra, zaino, zallet) = read_z3_commits(lock_path);
        assert_eq!(zebra, "bb41d69013edbfa8594bb097fa751f47eeb31445");
        assert_eq!(zaino, "17963672d0c2cad97dd12bd38bbf1b6fd232c8c5");
        assert_eq!(zallet, "bd7f020eb9e1de6f79da947e8102281832b05f83");
    }

    #[test]
    fn read_z3_commits_returns_unknown_when_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("missing.lock");
        let (z, i, t) = read_z3_commits(&lock);
        assert_eq!(z, "unknown");
        assert_eq!(i, "unknown");
        assert_eq!(t, "unknown");
    }

    #[test]
    fn manifest_is_pretty_printed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.json");
        write_manifest(
            &path,
            &RunManifest {
                run_id: "test".into(),
                run_started_at: Utc::now(),
                run_completed_at: None,
                simulator_commit: "".into(),
                zebra_commit: "".into(),
                zaino_commit: "".into(),
                zallet_commit: "".into(),
                scenario_name: "".into(),
                scenario_config_hash: "".into(),
                target_tps: 0.0,
                timeouts: RunTimeouts::default(),
                phase_boundaries: Vec::new(),
                load_and_drain_completed_at: None,
                compose_config_hash: String::new(),
                image_digests: Vec::new(),
                host_cpu_count: 0,
                host_memory_limit_bytes: None,
                state: StateIdentifier::default(),
                assertion: None,
            },
        )
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains('\n'));
    }

    #[test]
    fn read_simulator_commit_returns_nonempty() {
        let commit = read_simulator_commit();
        assert!(!commit.is_empty());
    }

    #[test]
    fn state_freshness_classify() {
        assert_eq!(StateFreshness::classify(100, 100), StateFreshness::Fresh);
        assert_eq!(StateFreshness::classify(99, 100), StateFreshness::Fresh);
        assert_eq!(StateFreshness::classify(101, 100), StateFreshness::Reused);
    }

    #[test]
    fn read_reset_state_parses_epoch_and_height() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reset-epoch");
        std::fs::write(&path, "3 12345\n").unwrap();
        assert_eq!(read_reset_state(&path), (3, 12345));
    }

    #[test]
    fn read_reset_state_defaults_when_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist");
        assert_eq!(read_reset_state(&path), (0, 0));
    }

    #[test]
    fn read_reset_state_defaults_when_file_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reset-epoch");
        std::fs::write(&path, "not-a-number\n").unwrap();
        assert_eq!(read_reset_state(&path), (0, 0));
    }

    #[test]
    fn two_phase_write_completed_at_updates_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let started = Utc::now();
        let mut m = RunManifest {
            run_id: "phase-test".into(),
            run_started_at: started,
            run_completed_at: None,
            simulator_commit: "abc".into(),
            zebra_commit: "z".into(),
            zaino_commit: "i".into(),
            zallet_commit: "t".into(),
            scenario_name: "phase".into(),
            scenario_config_hash: "hash".into(),
            target_tps: 5.0,
            timeouts: RunTimeouts::default(),
            phase_boundaries: Vec::new(),
            load_and_drain_completed_at: None,
            compose_config_hash: String::new(),
            image_digests: Vec::new(),
            host_cpu_count: 0,
            host_memory_limit_bytes: None,
            state: StateIdentifier::default(),
            assertion: None,
        };
        write_manifest(&path, &m).unwrap();
        let partial = read_manifest(&path).unwrap();
        assert!(partial.run_completed_at.is_none());

        let completed = Utc::now();
        m.run_completed_at = Some(completed);
        write_manifest(&path, &m).unwrap();
        let final_m = read_manifest(&path).unwrap();
        assert!(final_m.run_completed_at.is_some());
        assert_eq!(
            final_m.run_completed_at.unwrap().timestamp(),
            completed.timestamp()
        );
    }

    #[test]
    fn run_completed_at_some_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let completed = Utc::now();
        let m = RunManifest {
            run_id: "complete-test".into(),
            run_started_at: Utc::now(),
            run_completed_at: Some(completed),
            simulator_commit: "".into(),
            zebra_commit: "".into(),
            zaino_commit: "".into(),
            zallet_commit: "".into(),
            scenario_name: "".into(),
            scenario_config_hash: "".into(),
            target_tps: 0.0,
            timeouts: RunTimeouts::default(),
            phase_boundaries: Vec::new(),
            load_and_drain_completed_at: None,
            compose_config_hash: String::new(),
            image_digests: Vec::new(),
            host_cpu_count: 0,
            host_memory_limit_bytes: None,
            state: StateIdentifier::default(),
            assertion: None,
        };
        write_manifest(&path, &m).unwrap();
        let back = read_manifest(&path).unwrap();
        assert!(back.run_completed_at.is_some());
        assert_eq!(
            back.run_completed_at.unwrap().timestamp(),
            completed.timestamp()
        );
    }

    #[test]
    fn read_manifest_missing_file_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let err = read_manifest(&path).unwrap_err();
        assert!(
            matches!(err, MetricsError::Io(_)),
            "expected Io error, got: {err:?}"
        );
    }

    #[test]
    fn read_manifest_invalid_json_returns_serialization_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not valid json {{{{").unwrap();
        let err = read_manifest(&path).unwrap_err();
        assert!(
            matches!(err, MetricsError::Serialization(_)),
            "expected Serialization error, got: {err:?}"
        );
    }

    #[test]
    fn read_z3_commits_partial_sections_return_unknown() {
        let fixture = r#"zebra:
  repo: https://github.com/ZcashFoundation/zebra
  branch: main
  commit: sha-z
  status: verified
"#;
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("partial.lock");
        std::fs::write(&lock, fixture).unwrap();
        let (z, i, t) = read_z3_commits(&lock);
        assert_eq!(z, "sha-z", "zebra section present — must be parsed");
        assert_eq!(i, "unknown", "zaino section absent — must be unknown");
        assert_eq!(t, "unknown", "zallet section absent — must be unknown");
    }

    #[test]
    fn read_z3_commits_section_without_commit_line_returns_unknown() {
        let fixture = r#"zebra:
  repo: https://github.com/ZcashFoundation/zebra
  branch: main
  status: verified
zaino:
  commit: sha-i
zallet:
  commit: sha-t
"#;
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("nocommit.lock");
        std::fs::write(&lock, fixture).unwrap();
        let (z, i, t) = read_z3_commits(&lock);
        assert_eq!(
            z, "unknown",
            "zebra section has no commit: line — must be unknown"
        );
        assert_eq!(i, "sha-i");
        assert_eq!(t, "sha-t");
    }
}
