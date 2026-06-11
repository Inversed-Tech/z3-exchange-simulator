use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::error::MetricsError;

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

pub fn read_z3_commits(lock_path: &Path) -> (String, String, String) {
    let content = std::fs::read_to_string(lock_path).unwrap_or_default();
    let extract = |section: &str| -> String {
        let mut in_section = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(&format!("{section}:")) {
                in_section = true;
            } else if in_section && trimmed.starts_with("commit:") {
                return trimmed["commit:".len()..].trim().to_string();
            } else if in_section && !trimmed.is_empty() && !line.starts_with(' ') {
                // Must check `line` (not `trimmed`) — trimmed never starts with a space.
                in_section = false;
            }
        }
        "unknown".to_string()
    };
    (extract("zebra"), extract("zaino"), extract("zallet"))
}

pub fn read_simulator_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
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
