//! Integration tests for the scenario runner.
//!
//! Tests marked `#[ignore]` require a live Z3 regtest stack and are skipped
//! during CI unless explicitly enabled with `cargo test -- --ignored`.

use std::path::Path;

use z3_exchange_simulator::scenarios::runner::{
    load_scenario, run, validate_scenario, IntentOutcome, RunOptions,
};

/// Full end-to-end smoke test — requires a live Z3 regtest stack.
///
/// Run with: `cargo test -- --ignored test_smoke_scenario_via_runner`
#[tokio::test]
#[ignore]
async fn test_smoke_scenario_via_runner() {
    let scenario = load_scenario(Path::new("configs/scenarios/smoke.yaml")).unwrap();
    validate_scenario(&scenario).unwrap();
    let opts = RunOptions::default();
    let result = run(scenario, opts).await.unwrap();

    assert!(
        result.stats.total_attempted > 0,
        "expected at least one intent"
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
    assert_eq!(result.stats.total_attempted, 0);
    assert!(result.outcomes.is_empty());
}
