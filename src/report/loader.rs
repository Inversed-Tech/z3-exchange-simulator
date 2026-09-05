//! Loads a completed run directory into a typed, in-memory model.
//!
//! Deliberately does not support runs predating `intents.jsonl` / the
//! `timeouts` field on `RunManifest` — see docs/architecture/observability.md
//! for why those runs' data (e.g. `simulator_commit`) cannot be trusted, and
//! the project decision to regenerate rather than carry two data formats
//! through the report pipeline.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::data_model::{IntentRecord, MetricSample, RpcCall};
use crate::metrics::RunManifest;

use super::error::ReportError;

/// One fully-loaded run: the manifest plus every record from its three JSONL
/// logs. `parse_warnings` records lines that failed to deserialize (skipped,
/// not fatal to the load) so the report can surface incomplete evidence
/// explicitly rather than silently under-counting.
#[derive(Debug)]
pub struct RunData {
    pub run_dir: PathBuf,
    pub manifest: RunManifest,
    pub rpc_calls: Vec<RpcCall>,
    pub intents: Vec<IntentRecord>,
    pub metrics: Vec<MetricSample>,
    pub parse_warnings: Vec<String>,
}

fn require_file(run_dir: &Path, file: &'static str) -> Result<PathBuf, ReportError> {
    let path = run_dir.join(file);
    if !path.exists() {
        return Err(ReportError::MissingFile {
            run_dir: run_dir.to_path_buf(),
            file,
        });
    }
    Ok(path)
}

/// Parses each non-empty line of `path` as `T`, collecting successes and
/// recording a warning (rather than failing the whole load) for lines that
/// don't deserialize — consistent with how the simulator's own
/// `generate_summary` treats malformed JSONL lines.
fn parse_jsonl<T: serde::de::DeserializeOwned>(
    path: &Path,
    file_label: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<T>, ReportError> {
    let file = std::fs::File::open(path).map_err(ReportError::Io)?;
    let mut out = Vec::new();
    for (line_no, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(ReportError::Io)?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(&line) {
            Ok(record) => out.push(record),
            Err(e) => warnings.push(format!(
                "{file_label}:{}: malformed line skipped: {e}",
                line_no + 1
            )),
        }
    }
    Ok(out)
}

/// Loads one run directory. Requires `manifest.json`, `rpc_calls.jsonl`,
/// `intents.jsonl`, and `metrics.jsonl` to exist — a run missing any of these
/// predates the current schema and is rejected rather than silently
/// analyzed with partial data.
pub fn load_run(run_dir: &Path) -> Result<RunData, ReportError> {
    let manifest_path = require_file(run_dir, "manifest.json")?;
    let rpc_calls_path = require_file(run_dir, "rpc_calls.jsonl")?;
    let intents_path = require_file(run_dir, "intents.jsonl")?;
    let metrics_path = require_file(run_dir, "metrics.jsonl")?;

    let manifest_content = std::fs::read_to_string(&manifest_path).map_err(ReportError::Io)?;
    let manifest: RunManifest =
        serde_json::from_str(&manifest_content).map_err(|source| ReportError::InvalidManifest {
            run_dir: run_dir.to_path_buf(),
            source,
        })?;

    let mut parse_warnings = Vec::new();
    let rpc_calls = parse_jsonl(&rpc_calls_path, "rpc_calls.jsonl", &mut parse_warnings)?;
    let intents = parse_jsonl(&intents_path, "intents.jsonl", &mut parse_warnings)?;
    let metrics = parse_jsonl(&metrics_path, "metrics.jsonl", &mut parse_warnings)?;

    Ok(RunData {
        run_dir: run_dir.to_path_buf(),
        manifest,
        rpc_calls,
        intents,
        metrics,
        parse_warnings,
    })
}

/// Loads every run directory in `run_dirs`, in order. Fails on the first
/// unloadable run rather than silently dropping it from the report.
pub fn load_runs(run_dirs: &[PathBuf]) -> Result<Vec<RunData>, ReportError> {
    if run_dirs.is_empty() {
        return Err(ReportError::NoRunsProvided);
    }
    run_dirs.iter().map(|d| load_run(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{Backend, FlowType};
    use crate::metrics::RunTimeouts;
    use chrono::Utc;

    fn write_run(dir: &Path, corrupt_intents_line: bool) {
        let manifest = RunManifest {
            run_id: "test-run".into(),
            run_started_at: Utc::now(),
            run_completed_at: Some(Utc::now()),
            simulator_commit: "abc123".into(),
            zebra_commit: "z".into(),
            zaino_commit: "i".into(),
            zallet_commit: "t".into(),
            scenario_name: "smoke".into(),
            scenario_config_hash: "sha256:x".into(),
            target_tps: 1.0,
            timeouts: RunTimeouts::default(),
            phase_boundaries: Vec::new(),
            load_and_drain_completed_at: None,
        };
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        let call = RpcCall {
            call_id: "c-1".into(),
            run_id: "test-run".into(),
            method: "getblockcount".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: Utc::now(),
            response_at: Some(Utc::now()),
            latency_ms: Some(5),
            success: true,
            error_code: None,
            error_message: None,
            phase: crate::data_model::Phase::Unknown,
        };
        std::fs::write(
            dir.join("rpc_calls.jsonl"),
            format!("{}\n", serde_json::to_string(&call).unwrap()),
        )
        .unwrap();

        let intent = IntentRecord {
            run_id: "test-run".into(),
            intent_id: "i-1".into(),
            flow_type: FlowType::TToT,
            outcome: "confirmed".into(),
            error: None,
            timeout_context: None,
            recorded_at: Utc::now(),
        };
        let mut intents_content = format!("{}\n", serde_json::to_string(&intent).unwrap());
        if corrupt_intents_line {
            intents_content.push_str("{not valid json\n");
        }
        std::fs::write(dir.join("intents.jsonl"), intents_content).unwrap();

        std::fs::write(dir.join("metrics.jsonl"), "").unwrap();
    }

    #[test]
    fn load_run_reads_all_files() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), false);
        let data = load_run(dir.path()).unwrap();
        assert_eq!(data.manifest.run_id, "test-run");
        assert_eq!(data.rpc_calls.len(), 1);
        assert_eq!(data.intents.len(), 1);
        assert!(data.metrics.is_empty());
        assert!(data.parse_warnings.is_empty());
    }

    #[test]
    fn load_run_records_malformed_line_as_warning_not_error() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), true);
        let data = load_run(dir.path()).unwrap();
        assert_eq!(data.intents.len(), 1, "the one valid line must still load");
        assert_eq!(data.parse_warnings.len(), 1);
        assert!(data.parse_warnings[0].contains("intents.jsonl"));
    }

    #[test]
    fn load_run_missing_manifest_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), false);
        std::fs::remove_file(dir.path().join("manifest.json")).unwrap();
        let err = load_run(dir.path()).unwrap_err();
        assert!(matches!(
            err,
            ReportError::MissingFile {
                file: "manifest.json",
                ..
            }
        ));
    }

    #[test]
    fn load_run_missing_intents_jsonl_is_an_error_not_a_silent_skip() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), false);
        std::fs::remove_file(dir.path().join("intents.jsonl")).unwrap();
        let err = load_run(dir.path()).unwrap_err();
        assert!(matches!(
            err,
            ReportError::MissingFile {
                file: "intents.jsonl",
                ..
            }
        ));
    }

    #[test]
    fn load_run_old_format_manifest_without_timeouts_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        write_run(dir.path(), false);
        // Simulate an old-format manifest: no `timeouts` field at all.
        let old_manifest = serde_json::json!({
            "run_id": "old-run",
            "run_started_at": Utc::now().to_rfc3339(),
            "run_completed_at": null,
            "simulator_commit": "abc",
            "zebra_commit": "z",
            "zaino_commit": "i",
            "zallet_commit": "t",
            "scenario_name": "smoke",
            "scenario_config_hash": "sha256:x",
            "target_tps": 1.0
        });
        std::fs::write(
            dir.path().join("manifest.json"),
            serde_json::to_string(&old_manifest).unwrap(),
        )
        .unwrap();
        let err = load_run(dir.path()).unwrap_err();
        assert!(matches!(err, ReportError::InvalidManifest { .. }));
    }

    #[test]
    fn load_runs_empty_list_is_an_error() {
        let err = load_runs(&[]).unwrap_err();
        assert!(matches!(err, ReportError::NoRunsProvided));
    }

    #[test]
    fn load_runs_loads_multiple_directories() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        write_run(dir1.path(), false);
        write_run(dir2.path(), false);
        let runs = load_runs(&[dir1.path().to_path_buf(), dir2.path().to_path_buf()]).unwrap();
        assert_eq!(runs.len(), 2);
    }
}
