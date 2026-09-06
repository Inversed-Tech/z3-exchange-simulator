//! Single-line, TTY-aware progress reporting for multi-minute lifecycle
//! phases (warmup mining, the hot-wallet balance wait, funding rounds, and
//! the load phase's dispatch loop) — all of which the runner already tracks
//! a known total/ceiling for, but never surfaced above debug-only tracing
//! (see the Foundation feedback this addresses: a run could sit silent for
//! minutes with no visible indication of what phase it was in or whether it
//! was making progress).
//!
//! On a real terminal, redraws one line in place with `\r`; when stderr is
//! not a TTY (piped, redirected, CI), falls back to one `tracing::info!`
//! line per update instead of spamming a log file with raw carriage returns.

use std::io::IsTerminal;
use std::time::Duration;

use crate::data_model::Phase;

pub struct ProgressLine {
    is_tty: bool,
}

impl ProgressLine {
    pub fn new() -> Self {
        Self {
            is_tty: std::io::stderr().is_terminal(),
        }
    }

    /// Report progress within `phase`. `timeout`, when known, is rendered
    /// alongside `elapsed` so a reader can see how much budget remains, not
    /// just how much time has passed.
    pub fn update(&self, phase: Phase, detail: &str, elapsed: Duration, timeout: Option<Duration>) {
        let line = format!(
            "{phase:?}: {detail} (elapsed {}{})",
            format_duration(elapsed),
            timeout
                .map(|t| format!(", timeout {}", format_duration(t)))
                .unwrap_or_default(),
        );
        if self.is_tty {
            // \x1b[2K clears the line before redrawing, so a shorter line
            // never leaves trailing characters from a longer previous one.
            eprint!("\r\x1b[2K{line}");
        } else {
            tracing::info!("{line}");
        }
    }

    /// Move past the redrawn line once the phase it was reporting on
    /// completes, so subsequent output starts on its own line. A no-op on a
    /// non-TTY, which never redrew in place to begin with.
    pub fn finish(&self) {
        if self.is_tty {
            eprintln!();
        }
    }

    #[cfg(test)]
    fn with_tty(is_tty: bool) -> Self {
        Self { is_tty }
    }
}

impl Default for ProgressLine {
    fn default() -> Self {
        Self::new()
    }
}

fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    if mins > 0 {
        format!("{mins}m{secs:02}s")
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_renders_seconds_only_under_a_minute() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
    }

    #[test]
    fn format_duration_renders_minutes_and_seconds() {
        assert_eq!(format_duration(Duration::from_secs(90)), "1m30s");
    }

    #[test]
    fn format_duration_zero_pads_seconds_under_ten() {
        assert_eq!(format_duration(Duration::from_secs(65)), "1m05s");
    }

    // No capture seam exists for eprint!/tracing output here, so these guard
    // only that each code path (tty vs. non-tty) is reachable without
    // panicking — is_tty's actual routing is exercised end-to-end by the
    // lifecycle/load-phase call sites in live-stack testing.
    #[test]
    fn non_tty_progress_line_update_and_finish_do_not_panic() {
        let p = ProgressLine::with_tty(false);
        p.update(Phase::Warmup, "test detail", Duration::from_secs(5), None);
        p.finish();
    }

    #[test]
    fn tty_progress_line_update_and_finish_do_not_panic() {
        let p = ProgressLine::with_tty(true);
        p.update(
            Phase::Load,
            "test detail",
            Duration::from_secs(5),
            Some(Duration::from_secs(30)),
        );
        p.finish();
    }
}
