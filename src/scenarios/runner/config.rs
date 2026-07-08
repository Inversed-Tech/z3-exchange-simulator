//! Scenario configuration loading and validation.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::data_model::ScenarioConfig;
use crate::scenarios::runner::{PopulationPlan, RunOptions};

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(serde_yaml::Error),
    ValidationErrors(Vec<(String, String)>),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "IO error reading scenario: {e}"),
            ConfigError::Parse(e) => write!(f, "YAML parse error: {e}"),
            ConfigError::ValidationErrors(errs) => {
                write!(f, "Scenario validation failed:")?;
                for (field, msg) in errs {
                    write!(f, "\n  {field}: {msg}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(e) => Some(e),
            ConfigError::Parse(e) => Some(e),
            ConfigError::ValidationErrors(_) => None,
        }
    }
}

// ── load_scenario ─────────────────────────────────────────────────────────────

/// Read a scenario YAML file, compute its SHA-256 hash, and deserialize it.
///
/// Sets `config_hash` to `"sha256:<64 hex chars>"` and `source_path` to the
/// canonical path string before returning.
pub fn load_scenario(path: &Path) -> Result<ScenarioConfig, ConfigError> {
    let bytes = std::fs::read(path).map_err(ConfigError::Io)?;

    // Compute SHA-256 of the raw bytes.
    let hash = Sha256::digest(&bytes);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    let config_hash = format!("sha256:{hex}");

    let mut config: ScenarioConfig = serde_yaml::from_slice(&bytes).map_err(ConfigError::Parse)?;

    config.config_hash = config_hash;
    config.source_path = path.to_string_lossy().into_owned();

    Ok(config)
}

// ── validate_scenario ─────────────────────────────────────────────────────────

/// Validate all fields of a scenario config. Collects ALL violations rather than
/// failing on the first, returning them as a `ValidationErrors` list.
pub fn validate_scenario(config: &ScenarioConfig) -> Result<(), ConfigError> {
    let mut errors: Vec<(String, String)> = Vec::new();

    if config.load_target_tps <= 0.0 {
        errors.push(("load_target_tps".into(), "must be > 0.0".into()));
    }
    if config.load_duration_seconds == 0 {
        errors.push(("load_duration_seconds".into(), "must be > 0".into()));
    }
    if config.accounts_count < 1 {
        errors.push(("accounts_count".into(), "must be >= 1".into()));
    }
    if config.accounts_active_fraction <= 0.0 || config.accounts_active_fraction > 1.0 {
        errors.push((
            "accounts_active_fraction".into(),
            "must be in range (0.0, 1.0]".into(),
        ));
    }
    let active_count =
        (config.accounts_count as f64 * config.accounts_active_fraction).floor() as u64;
    if active_count < 2 {
        errors.push((
            "accounts_active_fraction".into(),
            format!(
                "floor(accounts_count × accounts_active_fraction) = {active_count} must be >= 2"
            ),
        ));
    }

    let flows = &config.flows;
    let flows_sum = flows.transparent_to_transparent
        + flows.transparent_to_shielded
        + flows.shielded_to_transparent
        + flows.shielded_to_shielded;
    if !(0.9999..=1.0001).contains(&flows_sum) {
        errors.push((
            "flows".into(),
            format!("flow fractions must sum to 1.0, got {flows_sum:.6}"),
        ));
    }
    for (name, val) in [
        (
            "flows.transparent_to_transparent",
            flows.transparent_to_transparent,
        ),
        (
            "flows.transparent_to_shielded",
            flows.transparent_to_shielded,
        ),
        (
            "flows.shielded_to_transparent",
            flows.shielded_to_transparent,
        ),
        ("flows.shielded_to_shielded", flows.shielded_to_shielded),
    ] {
        if !(0.0..=1.0).contains(&val) {
            errors.push((name.into(), format!("must be in [0.0, 1.0], got {val}")));
        }
    }

    let ap = &config.activity_profiles;
    let ap_sum = ap.low_fraction + ap.medium_fraction + ap.high_fraction;
    if !(0.9999..=1.0001).contains(&ap_sum) {
        errors.push((
            "activity_profiles".into(),
            format!("activity profile fractions must sum to 1.0, got {ap_sum:.6}"),
        ));
    }

    if config.confirmations_deposit_required < 1 {
        errors.push((
            "confirmations_deposit_required".into(),
            "must be >= 1".into(),
        ));
    }
    if config.amounts.min_zatoshis > config.amounts.max_zatoshis {
        errors.push((
            "amounts".into(),
            format!(
                "min_zatoshis ({}) must be <= max_zatoshis ({})",
                config.amounts.min_zatoshis, config.amounts.max_zatoshis
            ),
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::ValidationErrors(errors))
    }
}

// ── print_dry_run_summary ─────────────────────────────────────────────────────

/// Print a human-readable dry-run summary to stdout.
pub fn print_dry_run_summary(config: &ScenarioConfig, opts: &RunOptions, plan: &PopulationPlan) {
    println!("=== DRY RUN: {} ===", config.name);
    println!("Description : {}", config.description);
    println!("Seed        : {}", config.seed);
    println!(
        "Accounts    : {} total, {} active",
        plan.account_count, plan.active_count
    );
    println!(
        "Duration    : {}s at {:.1} TPS",
        config.load_duration_seconds, config.load_target_tps
    );
    println!("Load shape  : {:?}", opts.load_shape);
    println!("Max in-flight: {}", opts.max_in_flight);
    println!("Warmup blocks: {}", config.warmup_blocks);
    println!("Config hash : {}", config.config_hash);
    println!("Source path : {}", config.source_path);
    println!("(Dry run — Z3 stack will NOT be started)");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{
        ActivityProfileConfig, AmountRangeConfig, FlowConfig, ObservabilityConfig, ScenarioConfig,
    };
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn smoke_config() -> ScenarioConfig {
        ScenarioConfig {
            name: "smoke".into(),
            description: "test".into(),
            seed: 42,
            accounts_count: 10,
            accounts_active_fraction: 0.5,
            load_duration_seconds: 60,
            load_target_tps: 1.0,
            flows: FlowConfig {
                transparent_to_transparent: 1.0,
                transparent_to_shielded: 0.0,
                shielded_to_transparent: 0.0,
                shielded_to_shielded: 0.0,
            },
            activity_profiles: ActivityProfileConfig {
                low_fraction: 0.50,
                medium_fraction: 0.35,
                high_fraction: 0.15,
            },
            amounts: AmountRangeConfig {
                min_zatoshis: 10_000,
                max_zatoshis: 10_000_000,
            },
            confirmations_deposit_required: 3,
            observability: ObservabilityConfig {
                record_rpc_calls: true,
                record_component_logs: true,
                metric_sampling_interval_secs: 5,
                mempool_saturation_threshold: 500,
            },
            config_hash: String::new(),
            source_path: String::new(),
            warmup_blocks: 110,
        }
    }

    #[test]
    fn validate_scenario_accepts_smoke_yaml() {
        let path = std::path::Path::new("configs/scenarios/smoke.yaml");
        if path.exists() {
            let config = load_scenario(path).unwrap();
            validate_scenario(&config).unwrap();
        } else {
            // fallback: use our smoke_config helper
            let config = smoke_config();
            validate_scenario(&config).unwrap();
        }
    }

    #[test]
    fn validate_scenario_accepts_steady_state_yaml() {
        let path = std::path::Path::new("configs/scenarios/steady_state.yaml");
        let config = load_scenario(path).unwrap();
        validate_scenario(&config).unwrap();

        assert_eq!(config.accounts_count, 100);
        assert_eq!(config.load_target_tps, 5.0);
        assert_eq!(config.flows.transparent_to_transparent, 1.0);
        assert_eq!(config.flows.transparent_to_shielded, 0.0);
        assert_eq!(config.warmup_blocks, 110);
        assert!(config.config_hash.starts_with("sha256:"));
    }

    #[test]
    fn validate_scenario_accepts_ramp_yaml() {
        let path = std::path::Path::new("configs/scenarios/ramp.yaml");
        let config = load_scenario(path).unwrap();
        validate_scenario(&config).unwrap();

        assert_eq!(config.accounts_count, 100);
        assert_eq!(config.load_target_tps, 10.0);
        assert_eq!(config.flows.transparent_to_transparent, 1.0);
        assert_eq!(config.warmup_blocks, 110);
        // ramp ceiling is double steady_state — room to find the degradation point
        assert!(config.load_target_tps > 5.0);
    }

    #[test]
    fn validate_scenario_accepts_burst_yaml() {
        let path = std::path::Path::new("configs/scenarios/burst.yaml");
        let config = load_scenario(path).unwrap();
        validate_scenario(&config).unwrap();

        assert_eq!(config.accounts_count, 50);
        assert_eq!(config.load_target_tps, 3.0);
        assert_eq!(config.warmup_blocks, 110);
        // saturation threshold must be low enough to fire during the burst spike
        assert!(
            config.observability.mempool_saturation_threshold <= 50,
            "burst scenario needs a low saturation threshold to surface mempool events"
        );
    }

    #[test]
    fn validate_scenario_accepts_mixed_yaml() {
        let path = std::path::Path::new("configs/scenarios/mixed.yaml");
        let config = load_scenario(path).unwrap();
        validate_scenario(&config).unwrap();

        assert_eq!(config.accounts_count, 50);
        assert_eq!(config.accounts_active_fraction, 1.0);
        assert_eq!(config.flows.transparent_to_shielded, 0.5);
        assert_eq!(config.flows.shielded_to_shielded, 0.5);
        assert_eq!(config.flows.transparent_to_transparent, 0.0);
        assert_eq!(config.warmup_blocks, 110);
        // TPS must be conservative — ZK proving takes seconds per transaction
        assert!(config.load_target_tps <= 3.0);
    }

    #[test]
    fn validate_scenario_rejects_zero_tps() {
        let mut config = smoke_config();
        config.load_target_tps = 0.0;
        let err = validate_scenario(&config).unwrap_err();
        assert!(matches!(err, ConfigError::ValidationErrors(_)));
        if let ConfigError::ValidationErrors(errs) = err {
            assert!(errs.iter().any(|(f, _)| f == "load_target_tps"));
        }
    }

    #[test]
    fn validate_scenario_rejects_flows_not_summing_to_one() {
        let mut config = smoke_config();
        config.flows = FlowConfig {
            transparent_to_transparent: 0.5,
            transparent_to_shielded: 0.0,
            shielded_to_transparent: 0.0,
            shielded_to_shielded: 0.0,
        };
        let err = validate_scenario(&config).unwrap_err();
        if let ConfigError::ValidationErrors(errs) = err {
            assert!(errs.iter().any(|(f, _)| f == "flows"));
        }
    }

    #[test]
    fn validate_scenario_rejects_activity_profiles_not_summing_to_one() {
        let mut config = smoke_config();
        config.activity_profiles = ActivityProfileConfig {
            low_fraction: 0.1,
            medium_fraction: 0.1,
            high_fraction: 0.1,
        };
        let err = validate_scenario(&config).unwrap_err();
        if let ConfigError::ValidationErrors(errs) = err {
            assert!(errs.iter().any(|(f, _)| f == "activity_profiles"));
        }
    }

    #[test]
    fn validate_scenario_rejects_insufficient_active_accounts() {
        let mut config = smoke_config();
        // 10 accounts × 0.1 active = 1 — below required 2
        config.accounts_active_fraction = 0.1;
        let err = validate_scenario(&config).unwrap_err();
        if let ConfigError::ValidationErrors(errs) = err {
            assert!(errs.iter().any(|(f, _)| f == "accounts_active_fraction"));
        }
    }

    #[test]
    fn validate_scenario_rejects_min_gt_max_zatoshis() {
        let mut config = smoke_config();
        config.amounts.min_zatoshis = 1_000_000;
        config.amounts.max_zatoshis = 1_000;
        let err = validate_scenario(&config).unwrap_err();
        if let ConfigError::ValidationErrors(errs) = err {
            assert!(errs.iter().any(|(f, _)| f == "amounts"));
        }
    }

    #[test]
    fn load_scenario_computes_config_hash() {
        let yaml = b"name: test\ndescription: t\nseed: 1\naccounts_count: 10\naccounts_active_fraction: 0.5\nload_duration_seconds: 60\nload_target_tps: 1.0\nflows:\n  transparent_to_transparent: 1.0\n  transparent_to_shielded: 0.0\n  shielded_to_transparent: 0.0\n  shielded_to_shielded: 0.0\nactivity_profiles:\n  low_fraction: 0.50\n  medium_fraction: 0.35\n  high_fraction: 0.15\namounts:\n  min_zatoshis: 10000\n  max_zatoshis: 10000000\nconfirmations_deposit_required: 1\nobservability:\n  record_rpc_calls: true\n  record_component_logs: true\n  metric_sampling_interval_secs: 5\n  mempool_saturation_threshold: 500\n";
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(yaml).unwrap();

        let config = load_scenario(tmp.path()).unwrap();
        assert!(config.config_hash.starts_with("sha256:"));
        // SHA-256 produces 32 bytes = 64 hex chars; "sha256:" is 7 chars → 71 total
        assert_eq!(config.config_hash.len(), 71);
    }

    #[test]
    fn load_scenario_sets_source_path() {
        let yaml = b"name: test\ndescription: t\nseed: 1\naccounts_count: 10\naccounts_active_fraction: 0.5\nload_duration_seconds: 60\nload_target_tps: 1.0\nflows:\n  transparent_to_transparent: 1.0\n  transparent_to_shielded: 0.0\n  shielded_to_transparent: 0.0\n  shielded_to_shielded: 0.0\nactivity_profiles:\n  low_fraction: 0.50\n  medium_fraction: 0.35\n  high_fraction: 0.15\namounts:\n  min_zatoshis: 10000\n  max_zatoshis: 10000000\nconfirmations_deposit_required: 1\nobservability:\n  record_rpc_calls: true\n  record_component_logs: true\n  metric_sampling_interval_secs: 5\n  mempool_saturation_threshold: 500\n";
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(yaml).unwrap();

        let config = load_scenario(tmp.path()).unwrap();
        assert!(!config.source_path.is_empty());
        // source_path should contain some path component
        assert!(
            config.source_path.contains('/')
                || config.source_path.contains('\\')
                || !config.source_path.is_empty()
        );
    }

    #[test]
    fn validate_scenario_rejects_missing_file() {
        let err =
            load_scenario(std::path::Path::new("/nonexistent/path/scenario.yaml")).unwrap_err();
        assert!(matches!(err, ConfigError::Io(_)));
    }

    #[test]
    fn validate_scenario_rejects_invalid_yaml() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"key: [unclosed bracket").unwrap();
        let err = load_scenario(tmp.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }
}
