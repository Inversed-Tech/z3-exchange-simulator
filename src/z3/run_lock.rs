//! Concurrency guards for shared per-checkout state.
//!
//! Two locks, scoped differently:
//!
//! - [`RunLock`]/[`acquire`]: per `env_id`, fail-fast. A stable-per-checkout
//!   `env_id` (the default — see `z3::env_id`) means two concurrent
//!   invocations against the same checkout resolve the identical `env_id`,
//!   and therefore the identical Compose project, ports, and subnet. Held for
//!   the duration of a run so a second invocation against the same
//!   environment fails fast instead of colliding with the in-progress one.
//! - [`BootstrapLock`]/[`acquire_bootstrap_lock`]: per `compose_dir`, blocks
//!   rather than failing. Serializes the brief window in which
//!   `Z3Config::ensure_wallet_bootstrapped` reads and writes the checkout's
//!   single shared `.env.regtest` file, so two *different* environments
//!   bootstrapping that same checkout for the first time at once — an
//!   expected scenario, not a mistake — never interleave their writes.
//!
//! Built on `std::fs::File`'s `try_lock`/`lock` (stable since Rust 1.89),
//! which provide exactly the cross-platform advisory-lock primitives this
//! needs — no separate locking crate required.

use std::fs::TryLockError;
use std::path::{Path, PathBuf};

use crate::z3::Z3Error;

/// Holds an exclusive advisory lock on `configs/local/run-{env_id}.lock` for
/// its own lifetime. The OS releases the lock when the held file descriptor
/// closes — including on process termination via signal — so no explicit
/// unlock call is required.
pub struct RunLock {
    _file: std::fs::File,
}

/// Acquire the lock for `env_id`, or return `Z3Error::EnvironmentBusy` if
/// another process already holds it. Two different `env_id`s never contend:
/// each gets its own lock file, matching their already-disjoint Compose
/// projects, ports, and subnets.
pub fn acquire(env_id: &str, cache_dir: &Path) -> Result<RunLock, Z3Error> {
    std::fs::create_dir_all(cache_dir).map_err(Z3Error::RunLockIo)?;
    let path: PathBuf = cache_dir.join(format!("run-{env_id}.lock"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .map_err(Z3Error::RunLockIo)?;
    match file.try_lock() {
        Ok(()) => Ok(RunLock { _file: file }),
        Err(TryLockError::WouldBlock) => Err(Z3Error::EnvironmentBusy {
            env_id: env_id.to_string(),
            lock_path: path,
        }),
        Err(TryLockError::Error(e)) => Err(Z3Error::RunLockIo(e)),
    }
}

/// Holds an exclusive advisory lock on `<compose_dir>/.z3sim-bootstrap.lock`
/// for its own lifetime.
///
/// Unlike [`RunLock`] (scoped per `env_id`, fail-fast — a second run against
/// the SAME environment is a mistake to report immediately), this is scoped
/// per physical `compose_dir` checkout and blocks rather than failing: two
/// environments bootstrapping for the first time against the same checkout
/// (e.g. a `--fresh-env` run started while another run is being bootstrapped
/// for the first time in this checkout) is an expected scenario, not a
/// mistake. It exists because `Z3Config::ensure_wallet_bootstrapped` writes
/// this run's project name/ports into the checkout's single shared
/// `.env.regtest` (`sync_bootstrap_env_file`) and then runs
/// `regtest-init.sh`/`regtest-miner-setup.sh`, which read that file back —
/// interleaving two environments' writes there mid-bootstrap makes one of
/// them create its Docker resources under the OTHER's project name,
/// producing a hard container-name conflict rather than two isolated
/// environments. Serializing the whole write-then-run-scripts sequence
/// closes that gap: each environment's bootstrap runs to completion (using
/// its own values) before the next one starts, and `ensure_wallet_bootstrapped`
/// is idempotent, so this costs nothing beyond a short wait when contended.
pub struct BootstrapLock {
    _file: std::fs::File,
}

/// Acquire the bootstrap lock for `compose_dir`, blocking until any other
/// environment currently bootstrapping the same checkout releases it.
pub fn acquire_bootstrap_lock(compose_dir: &Path) -> Result<BootstrapLock, Z3Error> {
    let path = compose_dir.join(".z3sim-bootstrap.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .map_err(Z3Error::RunLockIo)?;
    file.lock().map_err(Z3Error::RunLockIo)?;
    Ok(BootstrapLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_second_acquire_for_same_env_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let first = acquire("a1b2c3d4", dir.path()).unwrap();

        // A second, independent `File::open` + `try_lock` — advisory locks
        // are per open-file-description, so this exercises real contention.
        let second = acquire("a1b2c3d4", dir.path());
        assert!(matches!(second, Err(Z3Error::EnvironmentBusy { .. })));

        drop(first);
        let third = acquire("a1b2c3d4", dir.path());
        assert!(third.is_ok(), "lock must be released after drop");
    }

    #[test]
    fn does_not_contend_across_different_env_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = acquire("a1b2c3d4", dir.path());
        let b = acquire("00000000", dir.path());
        assert!(a.is_ok());
        assert!(b.is_ok());
    }

    #[test]
    fn bootstrap_lock_serializes_access_to_the_same_compose_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let first = acquire_bootstrap_lock(dir.path()).unwrap();

        let dir_path = dir.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            // Blocks until `first` drops below.
            acquire_bootstrap_lock(&dir_path).unwrap();
        });

        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            !handle.is_finished(),
            "second bootstrap lock acquire should still be blocked while the first is held"
        );

        drop(first);
        handle.join().unwrap();
    }
}
