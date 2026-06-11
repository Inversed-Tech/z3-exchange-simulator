use chrono::Utc;
use std::path::{Path, PathBuf};

use super::error::MetricsError;

#[derive(Debug)]
pub struct RunDir {
    pub path: PathBuf,
    pub run_id: String,
}

fn sanitize_scenario_name(name: &str) -> String {
    let s = name.to_lowercase();
    let sanitized: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    sanitized
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub(crate) fn make_run_id(scenario_name: &str) -> String {
    let now = Utc::now();
    format!(
        "{}Z-{}",
        now.format("%Y%m%dT%H%M%S"),
        sanitize_scenario_name(scenario_name)
    )
}

impl RunDir {
    pub fn create(base: &Path, scenario_name: &str) -> Result<Self, MetricsError> {
        let base_id = make_run_id(scenario_name);
        for attempt in 0..=9u32 {
            let run_id = if attempt == 0 {
                base_id.clone()
            } else {
                format!("{base_id}-{attempt}")
            };
            let path = base.join(&run_id);
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    std::fs::create_dir_all(path.join("component_logs"))
                        .map_err(MetricsError::Io)?;
                    return Ok(RunDir { path, run_id });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(MetricsError::Io(e)),
            }
        }
        Err(MetricsError::InvalidRunDir(base.join(&base_id)))
    }

    pub fn component_logs_dir(&self) -> PathBuf {
        self.path.join("component_logs")
    }

    pub fn rpc_calls_path(&self) -> PathBuf {
        self.path.join("rpc_calls.jsonl")
    }

    pub fn metrics_path(&self) -> PathBuf {
        self.path.join("metrics.jsonl")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.path.join("manifest.json")
    }

    pub fn summary_path(&self) -> PathBuf {
        self.path.join("summary.md")
    }

    pub fn scenario_yaml_path(&self) -> PathBuf {
        self.path.join("scenario.yaml")
    }

    pub fn copy_scenario_yaml(&self, source: &Path) -> Result<(), MetricsError> {
        if source.as_os_str().is_empty() {
            return Ok(());
        }
        std::fs::copy(source, self.scenario_yaml_path()).map_err(MetricsError::Io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_run_dir_generates_expected_path() {
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "smoke").unwrap();
        assert!(rd.path.exists());
        assert!(rd.run_id.ends_with("-smoke"));
        assert!(rd.path.join("component_logs").is_dir());
    }

    #[test]
    fn create_run_dir_handles_collision() {
        let base = tempfile::tempdir().unwrap();
        let expected_id = make_run_id("smoke");
        std::fs::create_dir(base.path().join(&expected_id)).unwrap();
        let rd = RunDir::create(base.path(), "smoke").unwrap();
        assert_ne!(
            rd.run_id, expected_id,
            "collision not resolved: still returned the pre-existing id"
        );
        assert!(rd.path.exists());
        assert!(rd.path.join("component_logs").is_dir());
    }

    #[test]
    fn run_id_is_sortable_by_timestamp() {
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "test").unwrap();
        let id = &rd.run_id;
        assert!(id.len() >= 16, "run_id too short: {id}");
        assert!(
            id[..8].chars().all(|c| c.is_ascii_digit()),
            "date not all digits: {id}"
        );
        assert_eq!(&id[8..9], "T", "missing T separator: {id}");
        assert!(
            id[9..15].chars().all(|c| c.is_ascii_digit()),
            "time not all digits: {id}"
        );
        assert_eq!(&id[15..16], "Z", "missing Z suffix: {id}");
        assert!(
            id[16..].starts_with("-test"),
            "missing scenario suffix: {id}"
        );
        let rd2 = RunDir::create(base.path(), "test").unwrap();
        assert!(
            rd.run_id <= rd2.run_id,
            "run_ids not in order: {} > {}",
            rd.run_id,
            rd2.run_id
        );
    }

    #[test]
    fn sanitize_scenario_name_handles_special_chars() {
        assert_eq!(sanitize_scenario_name("Steady State"), "steady-state");
        assert_eq!(sanitize_scenario_name("burst!v2"), "burst-v2");
        assert_eq!(sanitize_scenario_name("--bad--"), "bad");
        assert_eq!(sanitize_scenario_name("ok"), "ok");
    }

    #[test]
    fn copy_scenario_yaml_copies_file() {
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "smoke").unwrap();
        let src = base.path().join("smoke.yaml");
        std::fs::write(&src, "name: smoke\n").unwrap();
        rd.copy_scenario_yaml(&src).unwrap();
        assert!(rd.scenario_yaml_path().exists());
        assert_eq!(
            std::fs::read_to_string(rd.scenario_yaml_path()).unwrap(),
            "name: smoke\n"
        );
    }

    #[test]
    fn copy_scenario_yaml_skips_empty_source() {
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "smoke").unwrap();
        let empty = std::path::Path::new("");
        rd.copy_scenario_yaml(empty).unwrap();
        assert!(
            !rd.scenario_yaml_path().exists(),
            "no file should be created for empty source"
        );
    }

    #[test]
    fn path_helpers_return_correct_filenames() {
        let base = tempfile::tempdir().unwrap();
        let rd = RunDir::create(base.path(), "paths").unwrap();
        assert_eq!(rd.rpc_calls_path().file_name().unwrap(), "rpc_calls.jsonl");
        assert_eq!(rd.metrics_path().file_name().unwrap(), "metrics.jsonl");
        assert_eq!(rd.manifest_path().file_name().unwrap(), "manifest.json");
        assert_eq!(rd.summary_path().file_name().unwrap(), "summary.md");
        assert_eq!(
            rd.scenario_yaml_path().file_name().unwrap(),
            "scenario.yaml"
        );
        assert_eq!(
            rd.component_logs_dir().file_name().unwrap(),
            "component_logs"
        );
    }

    #[test]
    fn create_exhausts_retries_and_returns_invalid_run_dir() {
        let base = tempfile::tempdir().unwrap();
        // Get the base_id that create() will use; pre-occupy all 10 slots.
        let base_id = make_run_id("exhaust");
        std::fs::create_dir(base.path().join(&base_id)).unwrap();
        for i in 1..=9u32 {
            std::fs::create_dir(base.path().join(format!("{base_id}-{i}"))).unwrap();
        }
        let err = RunDir::create(base.path(), "exhaust").unwrap_err();
        assert!(
            matches!(err, MetricsError::InvalidRunDir(_)),
            "expected InvalidRunDir, got: {err:?}"
        );
    }
}
