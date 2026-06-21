//! Synthetic account and transaction data generators.
//!
//! Produces deterministic, seeded populations of accounts and wallets, plus
//! on-demand streams of transaction intents at configurable flow-type distributions.
//! Given the same scenario seed, the same population and intent sequence are always
//! reproduced.

pub mod fixtures;
pub mod generators;

pub use fixtures::{write_fixtures, FixtureError};
pub use generators::{AccountGenerator, GeneratorError, TransactionIntentGenerator};

use std::collections::HashMap;

use crate::data_model::{Account, Address, AddressType, Wallet};

/// A fully generated synthetic population of accounts and wallets.
///
/// Constructed by [`AccountGenerator::generate_population`]. Not publicly constructable
/// except through the test-only [`SyntheticPopulation::new_for_test`] helper.
pub struct SyntheticPopulation {
    pub accounts: Vec<Account>,
    pub wallets: Vec<Wallet>,
    pub active_account_ids: Vec<String>,
    accounts_by_id: HashMap<String, usize>,
    wallets_by_account: HashMap<String, usize>,
}

impl SyntheticPopulation {
    pub fn account_by_id(&self, id: &str) -> Option<&Account> {
        self.accounts_by_id.get(id).map(|&i| &self.accounts[i])
    }

    pub fn wallet_for_account(&self, account_id: &str) -> Option<&Wallet> {
        self.wallets_by_account
            .get(account_id)
            .map(|&i| &self.wallets[i])
    }

    pub fn add_address(
        &mut self,
        account_id: &str,
        address: Address,
    ) -> Result<(), PopulationError> {
        let i = *self
            .wallets_by_account
            .get(account_id)
            .ok_or_else(|| PopulationError::AccountNotFound(account_id.to_string()))?;
        match address.address_type {
            AddressType::Transparent => self.wallets[i].transparent_addresses.push(address),
            AddressType::Sapling | AddressType::Orchard => {
                self.wallets[i].shielded_addresses.push(address)
            }
        }
        Ok(())
    }

    pub fn active_count(&self) -> usize {
        self.active_account_ids.len()
    }

    #[cfg(test)]
    pub fn new_for_test(accounts: Vec<Account>, wallets: Vec<Wallet>) -> Self {
        use crate::data_model::AccountStatus;
        let mut accounts_by_id = HashMap::with_capacity(accounts.len());
        let mut wallets_by_account = HashMap::with_capacity(wallets.len());
        let mut active_account_ids = Vec::new();
        for (i, account) in accounts.iter().enumerate() {
            accounts_by_id.insert(account.account_id.clone(), i);
            if account.status == AccountStatus::Active {
                active_account_ids.push(account.account_id.clone());
            }
        }
        for (i, wallet) in wallets.iter().enumerate() {
            wallets_by_account.insert(wallet.account_id.clone(), i);
        }
        Self {
            accounts,
            wallets,
            active_account_ids,
            accounts_by_id,
            wallets_by_account,
        }
    }
}

/// Errors returned by [`SyntheticPopulation`] methods.
#[derive(Debug)]
pub enum PopulationError {
    AccountNotFound(String),
}

impl std::fmt::Display for PopulationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PopulationError::AccountNotFound(id) => {
                write!(f, "No account found with id '{id}'")
            }
        }
    }
}

impl std::error::Error for PopulationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{
        AccountStatus, ActivityProfile, ActivityProfileConfig, AmountRangeConfig, FlowConfig,
        ObservabilityConfig, ScenarioConfig,
    };
    use chrono::Utc;

    fn make_account(id: &str, status: AccountStatus) -> Account {
        Account {
            account_id: id.into(),
            status,
            activity_profile: ActivityProfile::Low,
            wallet_id: format!("wal-{id}"),
            created_at: Utc::now(),
        }
    }

    fn make_wallet(account_id: &str) -> Wallet {
        Wallet {
            wallet_id: format!("wal-{account_id}"),
            account_id: account_id.into(),
            transparent_addresses: vec![],
            shielded_addresses: vec![],
            created_at: Utc::now(),
        }
    }

    fn make_address(address_type: AddressType) -> Address {
        use crate::data_model::AddressPurpose;
        Address {
            address_id: "addr-1".into(),
            wallet_id: "wal-acc-1".into(),
            address: "t1test".into(),
            address_type,
            purpose: AddressPurpose::Deposit,
            created_at: Utc::now(),
            last_used_at: None,
        }
    }

    fn make_scenario_config(accounts_count: u64, active_fraction: f64) -> ScenarioConfig {
        ScenarioConfig {
            name: "test".into(),
            description: "test scenario".into(),
            seed: 1,
            accounts_count,
            accounts_active_fraction: active_fraction,
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
            warmup_blocks: 10,
        }
    }

    fn test_population() -> SyntheticPopulation {
        let accounts = vec![
            make_account("acc-1", AccountStatus::Active),
            make_account("acc-2", AccountStatus::Active),
            make_account("acc-3", AccountStatus::Inactive),
        ];
        let wallets = vec![
            make_wallet("acc-1"),
            make_wallet("acc-2"),
            make_wallet("acc-3"),
        ];
        SyntheticPopulation::new_for_test(accounts, wallets)
    }

    #[test]
    fn population_account_by_id_found() {
        let pop = test_population();
        let acc = pop.account_by_id("acc-1").unwrap();
        assert_eq!(acc.account_id, "acc-1");
    }

    #[test]
    fn population_account_by_id_not_found() {
        let pop = test_population();
        assert!(pop.account_by_id("nonexistent").is_none());
    }

    #[test]
    fn population_wallet_for_account_found() {
        let pop = test_population();
        let w = pop.wallet_for_account("acc-2").unwrap();
        assert_eq!(w.account_id, "acc-2");
    }

    #[test]
    fn population_wallet_for_account_not_found() {
        let pop = test_population();
        assert!(pop.wallet_for_account("nonexistent").is_none());
    }

    #[test]
    fn population_add_address_transparent() {
        let mut pop = test_population();
        let addr = make_address(AddressType::Transparent);
        pop.add_address("acc-1", addr).unwrap();
        assert_eq!(
            pop.wallet_for_account("acc-1")
                .unwrap()
                .transparent_addresses
                .len(),
            1
        );
        assert!(pop
            .wallet_for_account("acc-1")
            .unwrap()
            .shielded_addresses
            .is_empty());
    }

    #[test]
    fn population_add_address_shielded_sapling() {
        let mut pop = test_population();
        let addr = make_address(AddressType::Sapling);
        pop.add_address("acc-1", addr).unwrap();
        assert_eq!(
            pop.wallet_for_account("acc-1")
                .unwrap()
                .shielded_addresses
                .len(),
            1
        );
        assert!(pop
            .wallet_for_account("acc-1")
            .unwrap()
            .transparent_addresses
            .is_empty());
    }

    #[test]
    fn population_add_address_shielded_orchard() {
        let mut pop = test_population();
        let addr = make_address(AddressType::Orchard);
        pop.add_address("acc-1", addr).unwrap();
        assert_eq!(
            pop.wallet_for_account("acc-1")
                .unwrap()
                .shielded_addresses
                .len(),
            1
        );
        assert!(pop
            .wallet_for_account("acc-1")
            .unwrap()
            .transparent_addresses
            .is_empty());
    }

    #[test]
    fn population_add_address_unknown_account() {
        let mut pop = test_population();
        let addr = make_address(AddressType::Transparent);
        let err = pop.add_address("unknown", addr).unwrap_err();
        assert!(matches!(err, PopulationError::AccountNotFound(_)));
    }

    #[test]
    fn population_active_count() {
        let pop = test_population();
        assert_eq!(pop.active_count(), 2);
    }

    #[test]
    fn population_active_account_ids_all_active() {
        let config = make_scenario_config(10, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        assert_eq!(pop.active_count(), 10);
    }

    #[test]
    fn population_active_account_ids_none_active() {
        let config = make_scenario_config(10, 0.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        assert_eq!(pop.active_count(), 0);
    }

    #[test]
    fn population_new_for_test_builds_correct_indexes() {
        let accounts = vec![
            make_account("a1", AccountStatus::Active),
            make_account("a2", AccountStatus::Active),
            make_account("a3", AccountStatus::Inactive),
        ];
        let wallets = vec![make_wallet("a1"), make_wallet("a2"), make_wallet("a3")];
        let pop = SyntheticPopulation::new_for_test(accounts, wallets);
        assert_eq!(pop.account_by_id("a1").unwrap().account_id, "a1");
        assert_eq!(pop.account_by_id("a2").unwrap().account_id, "a2");
        assert_eq!(pop.account_by_id("a3").unwrap().account_id, "a3");
        assert_eq!(pop.wallet_for_account("a1").unwrap().account_id, "a1");
        assert_eq!(pop.wallet_for_account("a2").unwrap().account_id, "a2");
        assert_eq!(pop.wallet_for_account("a3").unwrap().account_id, "a3");
        assert_eq!(pop.active_count(), 2);
    }
}
