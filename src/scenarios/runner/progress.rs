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
    /// Counts `update()` calls so a caller-side regression test (e.g.
    /// `funding.rs`'s `wait_operation` test) can assert progress actually
    /// fired repeatedly during a multi-iteration wait, without needing to
    /// intercept the real `eprint!`/`tracing::info!` output — see
    /// `progress.rs`'s own tests for why capturing that output directly
    /// isn't pursued here.
    #[cfg(test)]
    update_count: std::sync::atomic::AtomicUsize,
}

impl ProgressLine {
    pub fn new() -> Self {
        Self {
            is_tty: std::io::stderr().is_terminal(),
            #[cfg(test)]
            update_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Report progress within `phase`. `timeout`, when known, is rendered
    /// alongside `elapsed` so a reader can see how much budget remains, not
    /// just how much time has passed.
    pub fn update(&self, phase: Phase, detail: &str, elapsed: Duration, timeout: Option<Duration>) {
        #[cfg(test)]
        self.update_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

    /// Test-only constructor with an injected `is_tty`, for determinism —
    /// used both by this module's own tests and by other modules' tests
    /// (e.g. `funding.rs`) that need a `ProgressLine` without depending on
    /// the real terminal check.
    #[cfg(test)]
    pub(crate) fn with_tty(is_tty: bool) -> Self {
        Self {
            is_tty,
            update_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Number of `update()` calls observed so far — see the `update_count`
    /// field doc comment.
    #[cfg(test)]
    pub(crate) fn update_count(&self) -> usize {
        self.update_count.load(std::sync::atomic::Ordering::Relaxed)
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

    // Deliberate, explicit decision (not an oversight): `eprint!` writes
    // directly to the process's real stderr file descriptor, which a normal
    // `#[test]` cannot intercept on stable Rust without either an OS-level fd
    // redirection (a new dependency, e.g. `gag`) or restructuring this
    // module around an injectable sink — disproportionate engineering for a
    // single-branch `if self.is_tty { .. } else { .. }` that is easy to
    // verify correct by inspection and has no history of regressing. These
    // tests therefore only guard that each code path is reachable without
    // panicking; `update_count()` (test-only) lets *callers* of
    // `ProgressLine` (e.g. `funding::wait_operation`'s tests) assert
    // `update()` was actually invoked an expected number of times, which is
    // the property those call sites' own regression tests need — without
    // requiring output capture at all.
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

    #[test]
    fn update_count_reflects_the_number_of_update_calls() {
        let p = ProgressLine::with_tty(false);
        assert_eq!(p.update_count(), 0);
        p.update(Phase::Funding, "a", Duration::from_secs(1), None);
        p.update(Phase::Funding, "b", Duration::from_secs(2), None);
        assert_eq!(p.update_count(), 2);
    }
}
