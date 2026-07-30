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
