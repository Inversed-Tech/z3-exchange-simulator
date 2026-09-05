//! Integration tests for the scenario runner.
//!
//! Tests marked `#[ignore]` require a live Z3 regtest stack and are skipped
//! during CI unless explicitly enabled with `cargo test -- --ignored`.

use std::path::{Path, PathBuf};

use z3_exchange_simulator::cli::{dispatch, Cli, CliError, Commands, PrintVersionsArgs};
use z3_exchange_simulator::scenarios::runner::{
    load_scenario, run, validate_scenario, IntentOutcome, RunOptions,
};
use z3_exchange_simulator::z3::env_id::compose_project_for_env;
use z3_exchange_simulator::z3::run_lock;
use z3_exchange_simulator::z3::Z3Error;

/// Full end-to-end smoke test — requires a live Z3 regtest stack.
///
/// Run with: `cargo test -- --ignored test_smoke_scenario_via_runner`
#[tokio::test]
#[ignore = "requires live Z3 regtest stack; run with: cargo test -- --ignored test_smoke_scenario_via_runner"]
async fn test_smoke_scenario_via_runner() {
    let scenario = load_scenario(Path::new("configs/scenarios/smoke.yaml")).unwrap();
    validate_scenario(&scenario).unwrap();
    let opts = RunOptions::default();
    let result = run(scenario, opts).await.unwrap();

    assert!(
        result.stats.total_attempted > 0,
        "expected at least one intent"
    );

    // Minimum confirmation rate. Before the measured funding pipeline landed,
    // a run that confirmed 0 of 60 intents still passed this test, because it
    // only asserted that the runner returned Ok — the 100%-failure regression
    // was invisible (docs/regtest-funding-plan.md). `confirmed > 0` alone
    // would catch that; the 50% floor also catches systemic partial failures
    // (e.g. one flow type's `from` form or privacy policy broken) while
    // tolerating individual timeouts under load.
    let confirmed_rate = result.stats.confirmed as f64 / result.stats.total_attempted as f64;
    assert!(
        result.stats.confirmed > 0,
        "no intent confirmed at all — the funding pipeline or spend path is broken \
         (attempted {}, failed {}, timed out {})",
        result.stats.total_attempted,
        result.stats.failed,
        result.stats.timed_out,
    );
    assert!(
        confirmed_rate >= 0.5,
        "confirmation rate {:.0}% below the 50% floor (confirmed {}, attempted {}, \
         failed {}, timed out {})",
        confirmed_rate * 100.0,
        result.stats.confirmed,
        result.stats.total_attempted,
        result.stats.failed,
        result.stats.timed_out,
    );

    // Regression guard: no outcomes should contain "unprovisioned" addresses.
    for outcome in &result.outcomes {
        if let IntentOutcome::Failed { error, .. } = outcome {
            assert!(
                !error.contains("unprovisioned"),
                "unprovisioned address leaked into outcome: {error}"
            );
        }
    }
}

/// Two independently-identified environments (Track 2) never collide on
/// Compose project, network, or volume names, verified against real `docker`
/// output — not merely against the value stored in `Z3Config`.
///
/// Simulates two independent checkouts by pointing each run's
/// `env_id_cache_path`/`run_lock_dir` at its own tempdir (each resolves and
/// caches its own `env_id` there, exactly as two separate checkouts would),
/// then runs both concurrently against the one real Docker daemon.
///
/// Run with: `cargo test -- --ignored test_two_environments_do_not_collide`
#[tokio::test]
#[ignore = "requires live Z3 regtest stack; run with: cargo test -- --ignored test_two_environments_do_not_collide"]
async fn test_two_environments_do_not_collide() {
    fn opts_for_env(cache_dir: &Path) -> RunOptions {
        let output_base = cache_dir.join("runs");
        std::fs::create_dir_all(&output_base).unwrap();
        RunOptions {
            output_base,
            env_id_cache_path: cache_dir.join("env-id"),
            run_lock_dir: cache_dir.to_path_buf(),
            ..RunOptions::default()
        }
    }

    let cache_a = tempfile::TempDir::new().unwrap();
    let cache_b = tempfile::TempDir::new().unwrap();

    let scenario_a = load_scenario(Path::new("configs/scenarios/smoke.yaml")).unwrap();
    let scenario_b = load_scenario(Path::new("configs/scenarios/smoke.yaml")).unwrap();

    let (result_a, result_b) = tokio::join!(
        run(scenario_a, opts_for_env(cache_a.path())),
        run(scenario_b, opts_for_env(cache_b.path())),
    );
    let result_a = result_a.unwrap();
    let result_b = result_b.unwrap();

    let env_id_a = std::fs::read_to_string(cache_a.path().join("env-id")).unwrap();
    let env_id_b = std::fs::read_to_string(cache_b.path().join("env-id")).unwrap();
    assert_ne!(
        env_id_a, env_id_b,
        "two independent checkouts must not resolve the same env_id"
    );

    let project_a = compose_project_for_env(env_id_a.trim());
    let project_b = compose_project_for_env(env_id_b.trim());

    // Both environments produced confirmed transactions independently —
    // neither's funding/warmup/load was starved by the other running
    // concurrently on the same host.
    assert!(
        result_a.stats.confirmed > 0,
        "environment A confirmed nothing"
    );
    assert!(
        result_b.stats.confirmed > 0,
        "environment B confirmed nothing"
    );

    // Docker-stats resource sampling (Z3Stack::spawn_resource_sampling)
    // actually recorded samples while scoped to each environment's own
    // derived project name, not just `z3-regtest` — the concrete claim
    // behind the "docker stats samples appear correctly regardless of the
    // generated project name" acceptance criterion.
    for (cache, result) in [(&cache_a, &result_a), (&cache_b, &result_b)] {
        let metrics_path = cache
            .path()
            .join("runs")
            .join(&result.run_id)
            .join("metrics.jsonl");
        let metrics = std::fs::read_to_string(&metrics_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", metrics_path.display()));
        let has_resource_sample = metrics
            .lines()
            .any(|line| line.contains("process_cpu_percent") || line.contains("process_memory_mb"));
        assert!(
            has_resource_sample,
            "expected at least one process_cpu_percent/process_memory_mb sample in {}",
            metrics_path.display()
        );
    }

    // Each project's containers are gone after its own run tore down (both
    // runs call Z3Stack::stop()); critically, tearing down A's containers
    // must never have touched B's, and vice versa — assert neither project
    // has any container at all post-run, which would also be true if `-p`
    // wiring were broken and one run's `down` had size-effected the other's
    // still-running containers into a stopped-but-present state instead of
    // being removed by its own teardown.
    for project in [&project_a, &project_b] {
        let output = std::process::Command::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("label=com.docker.compose.project={project}"),
                "--format",
                "{{.Names}}",
            ])
            .output()
            .unwrap();
        let names = String::from_utf8_lossy(&output.stdout);
        assert!(
            names.trim().is_empty(),
            "expected no leftover containers for project {project}, found: {names}"
        );
    }
}

/// `regtest-reset.sh`'s preview (Track 2) must show the real, resolved
/// project/network/volume names it is about to delete — printed
/// unconditionally, before the script's own `--yes` confirmation gate — not
/// a generic placeholder a reader would have to reconstruct by hand.
///
/// Non-destructive: `--yes` is never passed, so the script always declines
/// (exit 1) right after printing the preview, without deleting anything.
///
/// Run with: `cargo test -- --ignored test_regtest_reset_preview_lists_real_names`
#[test]
#[ignore = "requires a live external/z3 checkout; run with: cargo test -- --ignored test_regtest_reset_preview_lists_real_names"]
fn test_regtest_reset_preview_lists_real_names() {
    let output = std::process::Command::new("bash")
        .arg("scripts/dev/regtest-reset.sh")
        .output()
        .expect("failed to invoke scripts/dev/regtest-reset.sh");

    assert_eq!(
        output.status.code(),
        Some(1),
        "expected the script to decline without --yes (stderr: {})",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("About to reset environment:"),
        "preview header missing: {stdout}"
    );
    assert!(
        stdout.contains("network:"),
        "network name missing from preview: {stdout}"
    );
    assert!(
        stdout.contains("volumes:"),
        "volume names missing from preview: {stdout}"
    );
    assert!(
        stdout.contains("z3-regtest") || stdout.contains("z3-sim-"),
        "preview did not show a real, resolved project name, only a generic \
         placeholder was expected to be absent: {stdout}"
    );
}

// ── z3sim print-versions (Track 1) ───────────────────────────────────────────
//
// Exercises `print_versions_command`'s actual runtime wiring (arg struct ->
// env_id resolution -> RunLock -> Z3Config::for_run ->
// ensure_wallet_bootstrapped's precondition check), not just the
// `parse_compose_images` helper it eventually calls — via `dispatch()`, the
// same entry point a real `z3sim print-versions` invocation goes through.

fn print_versions_args(env_id_cache_path: PathBuf, run_lock_dir: PathBuf) -> PrintVersionsArgs {
    PrintVersionsArgs {
        // Guaranteed not to exist — see `live_run_args`'s identical rationale
        // above: this must fail fast at `Z3Config::check_preconditions`
        // rather than ever reaching a real `external/z3` checkout.
        compose_dir: Some(PathBuf::from(
            "target/nonexistent-external-z3-for-tests-do-not-create",
        )),
        env_id_cache_path: Some(env_id_cache_path),
        run_lock_dir: Some(run_lock_dir),
    }
}

#[tokio::test]
async fn test_print_versions_fails_fast_on_nonexistent_compose_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let args = print_versions_args(tmp.path().join("env-id"), tmp.path().to_path_buf());
    let cli = Cli {
        command: Commands::PrintVersions(args),
        verbose: false,
        quiet: false,
    };

    let result = dispatch(cli).await;
    assert!(
        matches!(result, Err(CliError::Z3(Z3Error::ComposeDirNotFound(_)))),
        "expected ComposeDirNotFound, got: {result:?}"
    );
}

/// FINDING-2 regression guard: `print-versions` must hold the same per-`env_id`
/// `RunLock` `z3sim run` does, so a concurrent invocation against an
/// in-progress environment fails fast instead of racing Docker operations
/// against it. Pre-seeds the cached `env_id` and holds the lock externally
/// (mirroring `run_lock`'s own `rejects_second_acquire_for_same_env_id`
/// test), so this never depends on Docker or a real checkout — the
/// `EnvironmentBusy` check must fire before `compose_dir` (deliberately
/// nonexistent here) is ever touched.
#[tokio::test]
async fn test_print_versions_respects_environment_busy_lock() {
    let tmp = tempfile::TempDir::new().unwrap();
    let env_id_cache_path = tmp.path().join("env-id");
    std::fs::write(&env_id_cache_path, "a1b2c3d4").unwrap();
    let run_lock_dir = tmp.path().to_path_buf();

    let _held = run_lock::acquire("a1b2c3d4", &run_lock_dir)
        .expect("failed to pre-acquire the lock this test depends on");

    let args = print_versions_args(env_id_cache_path, run_lock_dir);
    let cli = Cli {
        command: Commands::PrintVersions(args),
        verbose: false,
        quiet: false,
    };

    let result = dispatch(cli).await;
    assert!(
        matches!(result, Err(CliError::Z3(Z3Error::EnvironmentBusy { .. }))),
        "expected EnvironmentBusy, got: {result:?}"
    );
}

/// Live-stack acceptance check for the track's headline deliverable —
/// requires an already-bootstrapped `external/z3` checkout (`make bootstrap`
/// or an equivalent prior `z3sim run`/`z3sim print-versions`).
///
/// Run with: `cargo test -- --ignored test_print_versions_reports_all_services`
#[test]
#[ignore = "requires an already-bootstrapped external/z3 checkout; run with: cargo test -- --ignored test_print_versions_reports_all_services"]
fn test_print_versions_reports_all_services() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_z3sim"))
        .arg("print-versions")
        .output()
        .expect("failed to invoke z3sim print-versions");
    assert!(
        output.status.success(),
        "expected exit 0, got {:?} (stderr: {})",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for label in ["Zebra", "Zaino", "Zallet", "RPC Router"] {
        assert!(
            stdout.contains(label),
            "missing {label} in print-versions output: {stdout}"
        );
    }
}

// ── scripts/dev/bootstrap.sh dependency check ────────────────────────────────
//
// These exercise `bootstrap.sh --check-only` (Phase 1 only — no clone, build,
// or Docker) by handing the script a constructed `PATH` containing symlinks
// to every tool it looks for except the one(s) under test, so `command -v`
// genuinely fails for the hidden tool rather than merely being distracted by
// an unrelated shadow earlier in the real PATH. Fast and side-effect-free —
// not `#[ignore]`-gated, unlike the live-stack tests above.

/// Every external tool `bootstrap.sh`'s dependency check or its own script
/// body (`sort`, `awk`, etc.) may invoke, resolved via the CURRENT process's
/// real `PATH` before any test constructs a restricted one.
fn resolve_on_real_path(tool: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var("PATH").ok()?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(tool))
        .find(|candidate| candidate.is_file())
}

/// Build a temp directory containing symlinks to every tool
/// `bootstrap.sh --check-only` needs, except `hidden` — then that directory
/// is the entire `PATH` handed to the script, so `command -v <hidden>` fails
/// exactly as it would on a host that genuinely lacks it, without silently
/// breaking coreutils the script's own body depends on (`sort`, `head`,
/// `awk`, `uname`, `df`, ...).
fn path_with_tools_hidden(hidden: &[&str]) -> (tempfile::TempDir, String) {
    const NEEDED: &[&str] = &[
        "docker",
        "cargo",
        "rage-keygen",
        "openssl",
        "curl",
        "jq",
        "pkg-config",
        "cc",
        "awk",
        "sort",
        "head",
        "uname",
        "dirname",
        "tr",
        "grep",
        "df",
        "bash",
    ];
    let dir = tempfile::TempDir::new().unwrap();
    for &tool in NEEDED {
        if hidden.contains(&tool) {
            continue;
        }
        if let Some(real) = resolve_on_real_path(tool) {
            let _ = std::os::unix::fs::symlink(&real, dir.path().join(tool));
        }
    }
    // `sed` is deliberately NOT a real symlink: this host's own `/usr/bin/sed`
    // may be BSD sed (no `--version` support), which would make
    // `test_bootstrap_succeeds_when_all_dependencies_present` fail on macOS
    // even though this test isn't about sed at all. Provide a stub that
    // answers `--version` like GNU sed (satisfying bootstrap.sh's check) and
    // otherwise delegates to the real `sed`, so the "everything present"
    // case is deterministic across hosts rather than host-sed-dependent.
    if !hidden.contains(&"sed") {
        if let Some(real_sed) = resolve_on_real_path("sed") {
            let stub = format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'sed (GNU sed) 4.9'\n  exit 0\nfi\nexec {} \"$@\"\n",
                real_sed.display()
            );
            let stub_path = dir.path().join("sed");
            std::fs::write(&stub_path, stub).unwrap();
            let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
            std::fs::set_permissions(&stub_path, perms).unwrap();
        }
    }
    let path = dir.path().display().to_string();
    (dir, path)
}

/// `disk_ok`: whether to satisfy the disk-space check via
/// `Z3_BOOTSTRAP_MIN_DISK_KB` (trivially low) rather than the real ~20GB
/// floor — the CI/dev machine's actual free disk space is incidental host
/// state a test asserting on binary-presence behavior shouldn't depend on.
fn run_bootstrap_check_only(path: &str, disk_ok: bool) -> std::process::Output {
    let mut cmd = std::process::Command::new("bash");
    cmd.arg("scripts/dev/bootstrap.sh")
        .arg("--check-only")
        .env("PATH", path);
    if disk_ok {
        cmd.env("Z3_BOOTSTRAP_MIN_DISK_KB", "1");
    }
    cmd.output()
        .expect("failed to invoke scripts/dev/bootstrap.sh")
}

#[test]
fn test_bootstrap_dependency_check_fails_fast_on_missing_binary() {
    let (_dir, path) = path_with_tools_hidden(&["jq"]);
    let output = run_bootstrap_check_only(&path, true);
    assert_eq!(
        output.status.code(),
        Some(2),
        "expected exit 2 for a missing dependency, got status {:?} (stdout: {}, stderr: {})",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("jq"),
        "missing-dependency message did not name jq: {stdout}"
    );
}

#[test]
fn test_bootstrap_reports_all_missing_dependencies_not_just_first() {
    let (_dir, path) = path_with_tools_hidden(&["jq", "rage-keygen"]);
    let output = run_bootstrap_check_only(&path, true);
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("jq"), "expected jq named: {stdout}");
    assert!(
        stdout.contains("rage-keygen"),
        "expected rage-keygen named: {stdout}"
    );
}

#[test]
fn test_bootstrap_detects_non_gnu_sed() {
    // A BSD-sed-shaped stub: `--version` is not a recognized option, so it
    // exits non-zero with a usage message — exactly `/usr/bin/sed`'s real
    // behavior on macOS, reproduced here so the assertion doesn't depend on
    // which `sed` this host actually has.
    let (dir, path) = path_with_tools_hidden(&["sed"]);
    let bsd_sed_stub = "#!/bin/sh\necho 'sed: illegal option -- -' >&2\nexit 1\n";
    let stub_path = dir.path().join("sed");
    std::fs::write(&stub_path, bsd_sed_stub).unwrap();
    let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&stub_path, perms).unwrap();

    let output = run_bootstrap_check_only(&path, true);
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("GNU sed"),
        "expected the GNU sed advisory: {stdout}"
    );
}

#[test]
fn test_bootstrap_succeeds_when_all_dependencies_present() {
    let (_dir, path) = path_with_tools_hidden(&[]);
    let output = run_bootstrap_check_only(&path, true);
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0 with every dependency present (stdout: {}, stderr: {})",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_bootstrap_pandoc_absent_is_advisory_not_error() {
    // pandoc is deliberately absent from NEEDED (never symlinked in), so
    // every `path_with_tools_hidden` case already exercises this — assert it
    // explicitly against the "hide nothing else" case so a passing dependency
    // check with no pandoc is confirmed exit 0, not silently masked by some
    // other missing tool.
    let (_dir, path) = path_with_tools_hidden(&[]);
    let output = run_bootstrap_check_only(&path, true);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("pandoc not found"),
        "expected the pandoc advisory line: {stdout}"
    );
}

/// FINDING-3 regression guard: `bootstrap.sh`'s documented exit-code contract
/// (0 / 1 / 2) must hold for Phase 2/3 failures too, not just Phase 1's own
/// `exit 2`. Runs a COPY of the real `bootstrap.sh` (so this exercises the
/// actual `run_step` wrapper, not a re-implementation of it) rooted at a temp
/// dir whose `scripts/dev/clone-z3.sh` is a stub that exits 5 — a code that
/// is neither 1 nor 2, so if `bootstrap.sh` let a Phase 2 script's raw exit
/// code propagate (the pre-fix behavior under bare `set -e`), this would
/// observe `5`, not `1`.
#[test]
fn test_bootstrap_normalizes_phase2_failure_exit_code_to_one() {
    let tmp = tempfile::TempDir::new().unwrap();
    let scripts_dir = tmp.path().join("scripts/dev");
    std::fs::create_dir_all(&scripts_dir).unwrap();

    std::fs::copy("scripts/dev/bootstrap.sh", scripts_dir.join("bootstrap.sh")).unwrap();
    let clone_stub = scripts_dir.join("clone-z3.sh");
    std::fs::write(&clone_stub, "#!/usr/bin/env bash\nexit 5\n").unwrap();
    let mut perms = std::fs::metadata(&clone_stub).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&clone_stub, perms).unwrap();

    let (_dir, path) = path_with_tools_hidden(&[]);
    let output = std::process::Command::new("bash")
        .arg(scripts_dir.join("bootstrap.sh"))
        .env("PATH", path)
        .env("Z3_BOOTSTRAP_MIN_DISK_KB", "1")
        .output()
        .expect("failed to invoke the copied bootstrap.sh");

    assert_eq!(
        output.status.code(),
        Some(1),
        "expected Phase 2's raw exit 5 to be normalized to 1, got status {:?} (stdout: {}, stderr: {})",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Dry-run test — does NOT start the Z3 stack and must always pass.
#[tokio::test]
async fn test_dry_run_does_not_start_z3() {
    let scenario = load_scenario(Path::new("configs/scenarios/smoke.yaml")).unwrap();
    validate_scenario(&scenario).unwrap();
    let opts = RunOptions {
        dry_run: true,
        ..RunOptions::default()
    };
    let result = run(scenario, opts).await.unwrap();
    assert!(result.dry_run, "expected dry_run=true in result");
    assert!(
        result.output_dir.is_none(),
        "dry-run must not populate output_dir"
    );
    assert_eq!(result.stats.total_attempted, 0);
    assert!(result.outcomes.is_empty());
}
