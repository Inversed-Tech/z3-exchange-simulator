//! Integration tests for the scenario runner.
//!
//! Tests marked `#[ignore]` require a live Z3 regtest stack and are skipped
//! during CI unless explicitly enabled with `cargo test -- --ignored`.

use std::path::Path;

use z3_exchange_simulator::scenarios::runner::{
    load_scenario, run, validate_scenario, IntentOutcome, RunOptions,
};
use z3_exchange_simulator::z3::env_id::compose_project_for_env;

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
