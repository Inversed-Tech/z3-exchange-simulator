use std::path::PathBuf;

#[derive(Debug)]
pub enum MetricsError {
    Io(std::io::Error),
    Serialization(serde_json::Error),
    GitCommand(String),
    InvalidRunDir(PathBuf),
}

impl std::fmt::Display for MetricsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricsError::Io(e) => write!(f, "metrics I/O error: {e}"),
            MetricsError::Serialization(e) => write!(f, "metrics serialization error: {e}"),
            MetricsError::GitCommand(s) => write!(f, "git command failed: {s}"),
            MetricsError::InvalidRunDir(p) => write!(f, "invalid run dir: {}", p.display()),
        }
    }
}

impl std::error::Error for MetricsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MetricsError::Io(e) => Some(e),
            MetricsError::Serialization(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    fn io_err() -> MetricsError {
        MetricsError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ))
    }

    fn serde_err() -> MetricsError {
        MetricsError::Serialization(serde_json::from_str::<serde_json::Value>("bad").unwrap_err())
    }

    #[test]
    fn display_io_contains_prefix_and_cause() {
        let s = format!("{}", io_err());
        assert!(s.starts_with("metrics I/O error:"), "got: {s}");
        assert!(s.contains("file not found"), "got: {s}");
    }

    #[test]
    fn display_serialization_contains_prefix() {
        let s = format!("{}", serde_err());
        assert!(s.starts_with("metrics serialization error:"), "got: {s}");
    }

    #[test]
    fn display_git_command_contains_message() {
        let s = format!("{}", MetricsError::GitCommand("rev-parse failed".into()));
        assert_eq!(s, "git command failed: rev-parse failed");
    }

    #[test]
    fn display_invalid_run_dir_contains_path() {
        let p = std::path::PathBuf::from("/runs/20260610T000000Z-smoke");
        let s = format!("{}", MetricsError::InvalidRunDir(p));
        assert!(s.contains("invalid run dir"), "got: {s}");
        assert!(s.contains("20260610T000000Z-smoke"), "got: {s}");
    }

    #[test]
    fn source_io_is_some() {
        assert!(io_err().source().is_some());
    }

    #[test]
    fn source_serialization_is_some() {
        assert!(serde_err().source().is_some());
    }

    #[test]
    fn source_git_command_is_none() {
        assert!(MetricsError::GitCommand("x".into()).source().is_none());
    }

    #[test]
    fn source_invalid_run_dir_is_none() {
        assert!(MetricsError::InvalidRunDir(std::path::PathBuf::from("/x"))
            .source()
            .is_none());
    }
}
