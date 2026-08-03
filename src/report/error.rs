use std::path::PathBuf;

#[derive(Debug)]
pub enum ReportError {
    Io(std::io::Error),
    /// A required file is missing from a run directory. Old-format runs
    /// (predating `intents.jsonl` and `manifest.json`'s `timeouts` field) are
    /// deliberately not supported — see docs/architecture/observability.md.
    MissingFile {
        run_dir: PathBuf,
        file: &'static str,
    },
    /// `manifest.json` failed to deserialize into `RunManifest`. The most
    /// common cause is an old-format manifest missing the `timeouts` field.
    InvalidManifest {
        run_dir: PathBuf,
        source: serde_json::Error,
    },
    NoRunsProvided,
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::Io(e) => write!(f, "report I/O error: {e}"),
            ReportError::MissingFile { run_dir, file } => write!(
                f,
                "{}: missing {file} — this run predates the current report schema \
                 (see docs/architecture/observability.md) and cannot be included",
                run_dir.display()
            ),
            ReportError::InvalidManifest { run_dir, source } => write!(
                f,
                "{}: manifest.json could not be parsed as the current schema \
                 (likely an old-format manifest missing `timeouts`): {source}",
                run_dir.display()
            ),
            ReportError::NoRunsProvided => write!(f, "no run directories provided"),
        }
    }
}

impl std::error::Error for ReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReportError::Io(e) => Some(e),
            ReportError::InvalidManifest { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_missing_file_names_the_file_and_run_dir() {
        let e = ReportError::MissingFile {
            run_dir: PathBuf::from("/runs/20260610T000000Z-smoke"),
            file: "intents.jsonl",
        };
        let s = format!("{e}");
        assert!(s.contains("intents.jsonl"), "got: {s}");
        assert!(s.contains("20260610T000000Z-smoke"), "got: {s}");
    }

    #[test]
    fn display_invalid_manifest_contains_source() {
        let source = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let e = ReportError::InvalidManifest {
            run_dir: PathBuf::from("/runs/x"),
            source,
        };
        assert!(format!("{e}").contains("manifest.json"));
    }

    #[test]
    fn display_no_runs_provided() {
        assert_eq!(
            format!("{}", ReportError::NoRunsProvided),
            "no run directories provided"
        );
    }

    #[test]
    fn source_io_is_some() {
        let e = ReportError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "x"));
        assert!(std::error::Error::source(&e).is_some());
    }

    #[test]
    fn source_missing_file_is_none() {
        let e = ReportError::MissingFile {
            run_dir: PathBuf::from("/x"),
            file: "manifest.json",
        };
        assert!(std::error::Error::source(&e).is_none());
    }
}
