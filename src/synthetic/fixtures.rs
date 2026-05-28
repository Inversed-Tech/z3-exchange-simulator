use std::path::Path;

use super::SyntheticPopulation;

/// Writes `accounts.json` and `wallets.json` to `out_dir`.
///
/// Creates `out_dir` and all parent directories if they do not exist.
/// Existing files are overwritten silently.
pub fn write_fixtures(
    population: &SyntheticPopulation,
    out_dir: &Path,
) -> Result<(), FixtureError> {
    std::fs::create_dir_all(out_dir).map_err(FixtureError::Io)?;

    let accounts_json =
        serde_json::to_string_pretty(&population.accounts).map_err(FixtureError::Serialisation)?;
    std::fs::write(out_dir.join("accounts.json"), accounts_json).map_err(FixtureError::Io)?;

    let wallets_json =
        serde_json::to_string_pretty(&population.wallets).map_err(FixtureError::Serialisation)?;
    std::fs::write(out_dir.join("wallets.json"), wallets_json).map_err(FixtureError::Io)?;

    Ok(())
}

/// Errors returned by [`write_fixtures`].
#[derive(Debug)]
pub enum FixtureError {
    Io(std::io::Error),
    Serialisation(serde_json::Error),
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FixtureError::Io(e) => write!(f, "Failed to write fixture file: {e}"),
            FixtureError::Serialisation(e) => write!(f, "Failed to serialise fixture data: {e}"),
        }
    }
}

impl std::error::Error for FixtureError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{
        Account, AccountStatus, ActivityProfile, ActivityProfileConfig, AmountRangeConfig,
        FlowConfig, ObservabilityConfig, ScenarioConfig,
    };
    use crate::synthetic::generators::AccountGenerator;
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_population(n: usize) -> SyntheticPopulation {
        let config = ScenarioConfig {
            name: "test".into(),
            description: "test".into(),
            seed: 99,
            accounts_count: n as u64,
            accounts_active_fraction: 1.0,
            load_duration_seconds: 60,
            load_target_tps: 1.0,
            flows: FlowConfig {
                transparent_to_transparent: 1.0,
                transparent_to_shielded: 0.0,
                shielded_to_transparent: 0.0,
                shielded_to_shielded: 0.0,
            },
            activity_profiles: ActivityProfileConfig {
                low_fraction: 0.5,
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
        };
        let mut gen = AccountGenerator::new(config).unwrap();
        gen.generate_population().unwrap()
    }

    fn make_empty_population() -> SyntheticPopulation {
        SyntheticPopulation::new_for_test(vec![], vec![])
    }

    fn make_single_account_population() -> SyntheticPopulation {
        use crate::data_model::Wallet;
        let accounts = vec![Account {
            account_id: "acc-test".into(),
            status: AccountStatus::Active,
            activity_profile: ActivityProfile::Low,
            wallet_id: "wal-test".into(),
            created_at: Utc::now(),
        }];
        let wallets = vec![Wallet {
            wallet_id: "wal-test".into(),
            account_id: "acc-test".into(),
            transparent_addresses: vec![],
            shielded_addresses: vec![],
            created_at: Utc::now(),
        }];
        SyntheticPopulation::new_for_test(accounts, wallets)
    }

    #[test]
    fn write_fixtures_creates_accounts_json() {
        let dir = TempDir::new().unwrap();
        let pop = make_population(5);
        write_fixtures(&pop, dir.path()).unwrap();
        assert!(dir.path().join("accounts.json").exists());
    }

    #[test]
    fn write_fixtures_creates_wallets_json() {
        let dir = TempDir::new().unwrap();
        let pop = make_population(5);
        write_fixtures(&pop, dir.path()).unwrap();
        assert!(dir.path().join("wallets.json").exists());
    }

    #[test]
    fn write_fixtures_account_count_matches() {
        let dir = TempDir::new().unwrap();
        let pop = make_population(7);
        write_fixtures(&pop, dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("accounts.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 7);
    }

    #[test]
    fn write_fixtures_account_ids_match() {
        let dir = TempDir::new().unwrap();
        let pop = make_population(5);
        let expected_ids: Vec<String> = pop.accounts.iter().map(|a| a.account_id.clone()).collect();
        write_fixtures(&pop, dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("accounts.json")).unwrap();
        let parsed: Vec<Account> = serde_json::from_str(&content).unwrap();
        let actual_ids: Vec<String> = parsed.iter().map(|a| a.account_id.clone()).collect();
        assert_eq!(expected_ids, actual_ids);
    }

    #[test]
    fn write_fixtures_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let pop = make_single_account_population();
        write_fixtures(&pop, dir.path()).unwrap();
        let accounts_1 = std::fs::read_to_string(dir.path().join("accounts.json")).unwrap();
        let wallets_1 = std::fs::read_to_string(dir.path().join("wallets.json")).unwrap();
        write_fixtures(&pop, dir.path()).unwrap();
        let accounts_2 = std::fs::read_to_string(dir.path().join("accounts.json")).unwrap();
        let wallets_2 = std::fs::read_to_string(dir.path().join("wallets.json")).unwrap();
        assert_eq!(accounts_1, accounts_2);
        assert_eq!(wallets_1, wallets_2);
    }

    #[test]
    fn write_fixtures_creates_output_dir() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("nested").join("output");
        let pop = make_population(3);
        write_fixtures(&pop, &nested).unwrap();
        assert!(nested.join("accounts.json").exists());
        assert!(nested.join("wallets.json").exists());
    }

    #[test]
    fn write_fixtures_pretty_json() {
        let dir = TempDir::new().unwrap();
        let pop = make_population(2);
        write_fixtures(&pop, dir.path()).unwrap();
        let content = std::fs::read_to_string(dir.path().join("accounts.json")).unwrap();
        assert!(
            content.contains('\n'),
            "expected pretty-printed JSON with newlines"
        );
    }

    #[test]
    fn write_fixtures_empty_population() {
        let dir = TempDir::new().unwrap();
        let pop = make_empty_population();
        write_fixtures(&pop, dir.path()).unwrap();
        let accounts = std::fs::read_to_string(dir.path().join("accounts.json")).unwrap();
        let wallets = std::fs::read_to_string(dir.path().join("wallets.json")).unwrap();
        assert_eq!(accounts.trim(), "[]");
        assert_eq!(wallets.trim(), "[]");
    }
}
