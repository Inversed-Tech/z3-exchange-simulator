use chrono::Utc;
use rand::distributions::{Distribution, WeightedIndex};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::SyntheticPopulation;
use crate::data_model::{
    Account, AccountStatus, ActivityProfile, Address, FlowType, ScenarioConfig, TransactionIntent,
    TransactionStatus, Wallet,
};

const INTENT_SEED_SALT: u64 = 0x9E37_79B9_7F4A_7C15;

pub struct AccountGenerator {
    rng: ChaCha8Rng,
    config: ScenarioConfig,
    profile_dist: WeightedIndex<f64>,
}

impl AccountGenerator {
    pub fn new(config: ScenarioConfig) -> Result<Self, GeneratorError> {
        let weights = [
            config.activity_profiles.low_fraction,
            config.activity_profiles.medium_fraction,
            config.activity_profiles.high_fraction,
        ];
        let profile_dist = WeightedIndex::new(weights)
            .map_err(|e| GeneratorError::InvalidActivityProfileWeights(e.to_string()))?;
        let rng = ChaCha8Rng::seed_from_u64(config.seed);
        Ok(Self {
            rng,
            config,
            profile_dist,
        })
    }

    pub fn generate_population(&mut self) -> Result<SyntheticPopulation, GeneratorError> {
        let n = usize::try_from(self.config.accounts_count)
            .map_err(|_| GeneratorError::AccountCountTooLarge)?;
        let mut accounts = Vec::with_capacity(n);
        let mut wallets = Vec::with_capacity(n);
        let mut accounts_by_id = std::collections::HashMap::with_capacity(n);
        let mut wallets_by_account = std::collections::HashMap::with_capacity(n);
        let mut active_account_ids = Vec::new();

        for i in 0..n {
            let account_id = next_uuid(&mut self.rng);
            let wallet_id = next_uuid(&mut self.rng);
            let status = sample_account_status(&mut self.rng, self.config.accounts_active_fraction);
            let profile = sample_activity_profile(&mut self.rng, &self.profile_dist);

            let account = Account {
                account_id: account_id.clone(),
                status: status.clone(),
                activity_profile: profile,
                wallet_id: wallet_id.clone(),
                created_at: Utc::now(),
            };
            let wallet = Wallet {
                wallet_id,
                account_id: account_id.clone(),
                transparent_addresses: vec![],
                shielded_addresses: vec![],
                created_at: Utc::now(),
            };

            if status == AccountStatus::Active {
                active_account_ids.push(account_id.clone());
            }
            accounts_by_id.insert(account_id.clone(), i);
            wallets_by_account.insert(account_id, i);
            accounts.push(account);
            wallets.push(wallet);
        }

        Ok(SyntheticPopulation {
            accounts,
            wallets,
            active_account_ids,
            accounts_by_id,
            wallets_by_account,
        })
    }
}

pub struct TransactionIntentGenerator {
    rng: ChaCha8Rng,
    flow_dist: WeightedIndex<f64>,
    flow_variants: [FlowType; 4],
    min_amount: u64,
    max_amount: u64,
    active_account_ids: Vec<String>,
}

impl TransactionIntentGenerator {
    pub fn new(
        population: &SyntheticPopulation,
        config: &ScenarioConfig,
    ) -> Result<Self, GeneratorError> {
        let weights = [
            config.flows.transparent_to_transparent,
            config.flows.transparent_to_shielded,
            config.flows.shielded_to_transparent,
            config.flows.shielded_to_shielded,
        ];
        let flow_dist = WeightedIndex::new(weights)
            .map_err(|e| GeneratorError::InvalidFlowWeights(e.to_string()))?;
        let sub_seed = config.seed ^ INTENT_SEED_SALT;
        let rng = ChaCha8Rng::seed_from_u64(sub_seed);
        let active_account_ids = population.active_account_ids.clone();
        Ok(Self {
            rng,
            flow_dist,
            flow_variants: [
                FlowType::TToT,
                FlowType::TToZ,
                FlowType::ZToT,
                FlowType::ZToZ,
            ],
            min_amount: config.amounts.min_zatoshis,
            max_amount: config.amounts.max_zatoshis,
            active_account_ids,
        })
    }

    pub fn next_intent(
        &mut self,
        run_id: &str,
        population: &SyntheticPopulation,
    ) -> Option<TransactionIntent> {
        if self.active_account_ids.len() < 2 {
            return None;
        }

        let flow_type = self.flow_variants[self.flow_dist.sample(&mut self.rng)].clone();

        let n = self.active_account_ids.len();
        let sender_idx = self.rng.gen_range(0..n);
        let recipient_idx = loop {
            let idx = self.rng.gen_range(0..n);
            if idx != sender_idx {
                break idx;
            }
        };
        let sender_account_id = &self.active_account_ids[sender_idx];
        let recipient_account_id = &self.active_account_ids[recipient_idx];

        let sender_address =
            resolve_address(population, sender_account_id, &flow_type, Side::Sender);
        let recipient_address = resolve_address(
            population,
            recipient_account_id,
            &flow_type,
            Side::Recipient,
        );

        let amount_zatoshis = self.rng.gen_range(self.min_amount..=self.max_amount);
        let intent_id = next_uuid(&mut self.rng);

        Some(TransactionIntent {
            intent_id,
            run_id: run_id.to_string(),
            account_id: sender_account_id.to_string(),
            recipient_account_id: recipient_account_id.to_string(),
            sender_address,
            recipient_address,
            amount_zatoshis,
            fee_zatoshis: 0,
            flow_type,
            status: TransactionStatus::Pending,
            created_at: Utc::now(),
            submitted_at: None,
        })
    }
}

enum Side {
    Sender,
    Recipient,
}

fn next_uuid(rng: &mut ChaCha8Rng) -> String {
    let bytes: [u8; 16] = rng.gen();
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

fn sample_account_status(rng: &mut ChaCha8Rng, active_fraction: f64) -> AccountStatus {
    if rng.gen::<f64>() < active_fraction {
        AccountStatus::Active
    } else {
        AccountStatus::Inactive
    }
}

fn sample_activity_profile(rng: &mut ChaCha8Rng, dist: &WeightedIndex<f64>) -> ActivityProfile {
    match dist.sample(rng) {
        0 => ActivityProfile::Low,
        1 => ActivityProfile::Medium,
        2 => ActivityProfile::High,
        _ => unreachable!(),
    }
}

fn resolve_address(
    population: &SyntheticPopulation,
    account_id: &str,
    flow_type: &FlowType,
    side: Side,
) -> String {
    let wallet = match population.wallet_for_account(account_id) {
        Some(w) => w,
        None => return format!("unprovisioned:{account_id}"),
    };

    let pool: &[Address] = match (flow_type, side) {
        (FlowType::TToT | FlowType::TToZ, Side::Sender) => &wallet.transparent_addresses,
        (FlowType::TToT | FlowType::ZToT, Side::Recipient) => &wallet.transparent_addresses,
        (FlowType::ZToT | FlowType::ZToZ, Side::Sender) => &wallet.shielded_addresses,
        (FlowType::TToZ | FlowType::ZToZ, Side::Recipient) => &wallet.shielded_addresses,
    };

    if pool.is_empty() {
        return format!("unprovisioned:{account_id}");
    }

    pool[0].address.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{
        ActivityProfileConfig, AmountRangeConfig, FlowConfig, ObservabilityConfig, ScenarioConfig,
    };
    use crate::synthetic::SyntheticPopulation;

    fn make_config(seed: u64, accounts_count: u64, active_fraction: f64) -> ScenarioConfig {
        ScenarioConfig {
            name: "test".into(),
            description: "test scenario".into(),
            seed,
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
        }
    }

    fn make_config_with_flows(
        seed: u64,
        accounts_count: u64,
        t_to_t: f64,
        t_to_z: f64,
        z_to_t: f64,
        z_to_z: f64,
    ) -> ScenarioConfig {
        let mut c = make_config(seed, accounts_count, 1.0);
        c.flows = FlowConfig {
            transparent_to_transparent: t_to_t,
            transparent_to_shielded: t_to_z,
            shielded_to_transparent: z_to_t,
            shielded_to_shielded: z_to_z,
        };
        c
    }

    fn make_config_with_amounts(
        seed: u64,
        accounts_count: u64,
        min: u64,
        max: u64,
    ) -> ScenarioConfig {
        let mut c = make_config(seed, accounts_count, 1.0);
        c.amounts = AmountRangeConfig {
            min_zatoshis: min,
            max_zatoshis: max,
        };
        c
    }

    fn is_valid_uuid_v4(s: &str) -> bool {
        if s.len() != 36 {
            return false;
        }
        let b = s.as_bytes();
        if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return false;
        }
        if b[14] != b'4' {
            return false;
        }
        if !matches!(b[19], b'8' | b'9' | b'a' | b'b') {
            return false;
        }
        s.chars()
            .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c))
    }

    // ── AccountGenerator tests ────────────────────────────────────────────────

    #[test]
    fn account_generator_determinism() {
        let config1 = make_config(42, 20, 0.5);
        let config2 = make_config(42, 20, 0.5);
        let mut gen1 = AccountGenerator::new(config1).unwrap();
        let mut gen2 = AccountGenerator::new(config2).unwrap();
        let pop1 = gen1.generate_population().unwrap();
        let pop2 = gen2.generate_population().unwrap();
        let ids1: Vec<&str> = pop1
            .accounts
            .iter()
            .map(|a| a.account_id.as_str())
            .collect();
        let ids2: Vec<&str> = pop2
            .accounts
            .iter()
            .map(|a| a.account_id.as_str())
            .collect();
        assert_eq!(ids1, ids2);
    }

    #[test]
    fn account_generator_population_size() {
        let config = make_config(1, 50, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        assert_eq!(pop.accounts.len(), 50);
        assert_eq!(pop.wallets.len(), 50);
    }

    #[test]
    fn account_generator_wallet_account_correspondence() {
        let config = make_config(1, 20, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        for i in 0..pop.accounts.len() {
            assert_eq!(pop.accounts[i].wallet_id, pop.wallets[i].wallet_id);
            assert_eq!(pop.wallets[i].account_id, pop.accounts[i].account_id);
        }
    }

    #[test]
    fn account_generator_active_fraction_one() {
        let config = make_config(1, 20, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        assert!(pop
            .accounts
            .iter()
            .all(|a| a.status == AccountStatus::Active));
    }

    #[test]
    fn account_generator_active_fraction_zero() {
        let config = make_config(1, 20, 0.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        assert!(pop
            .accounts
            .iter()
            .all(|a| a.status == AccountStatus::Inactive));
    }

    #[test]
    fn account_generator_active_fraction_half() {
        let config = make_config(42, 1000, 0.5);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        let active = pop
            .accounts
            .iter()
            .filter(|a| a.status == AccountStatus::Active)
            .count();
        assert!(active >= 450 && active <= 550, "active={active}");
    }

    #[test]
    fn account_generator_activity_profile_distribution() {
        let mut config = make_config(42, 1000, 1.0);
        config.activity_profiles = ActivityProfileConfig {
            low_fraction: 0.5,
            medium_fraction: 0.35,
            high_fraction: 0.15,
        };
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        let low = pop
            .accounts
            .iter()
            .filter(|a| a.activity_profile == ActivityProfile::Low)
            .count();
        let medium = pop
            .accounts
            .iter()
            .filter(|a| a.activity_profile == ActivityProfile::Medium)
            .count();
        let high = pop
            .accounts
            .iter()
            .filter(|a| a.activity_profile == ActivityProfile::High)
            .count();
        assert!(low >= 450 && low <= 550, "low={low}");
        assert!(medium >= 300 && medium <= 400, "medium={medium}");
        assert!(high >= 100 && high <= 200, "high={high}");
    }

    #[test]
    fn account_generator_ids_are_uuids() {
        let config = make_config(7, 20, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        for account in &pop.accounts {
            assert!(
                is_valid_uuid_v4(&account.account_id),
                "invalid account_id: {}",
                account.account_id
            );
            assert!(
                is_valid_uuid_v4(&account.wallet_id),
                "invalid wallet_id: {}",
                account.wallet_id
            );
        }
    }

    #[test]
    fn account_generator_wallet_addresses_empty() {
        let config = make_config(1, 10, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        for wallet in &pop.wallets {
            assert!(wallet.transparent_addresses.is_empty());
            assert!(wallet.shielded_addresses.is_empty());
        }
    }

    #[test]
    fn account_generator_invalid_weights() {
        let mut config = make_config(1, 10, 1.0);
        config.activity_profiles = ActivityProfileConfig {
            low_fraction: 0.0,
            medium_fraction: 0.0,
            high_fraction: 0.0,
        };
        let result = AccountGenerator::new(config);
        assert!(matches!(
            result,
            Err(GeneratorError::InvalidActivityProfileWeights(_))
        ));
    }

    #[test]
    fn account_generator_negative_weights() {
        let mut config = make_config(1, 10, 1.0);
        config.activity_profiles = ActivityProfileConfig {
            low_fraction: -0.1,
            medium_fraction: 0.5,
            high_fraction: 0.6,
        };
        let result = AccountGenerator::new(config);
        assert!(matches!(
            result,
            Err(GeneratorError::InvalidActivityProfileWeights(_))
        ));
    }

    #[test]
    fn account_generator_zero_accounts() {
        let config = make_config(1, 0, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        let pop = gen.generate_population().unwrap();
        assert_eq!(pop.accounts.len(), 0);
        assert_eq!(pop.wallets.len(), 0);
        assert_eq!(pop.active_count(), 0);
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn account_generator_count_too_large() {
        let config = make_config(1, u64::MAX, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        assert!(matches!(
            gen.generate_population(),
            Err(GeneratorError::AccountCountTooLarge)
        ));
    }

    // ── TransactionIntentGenerator tests ─────────────────────────────────────

    fn make_population_for_intents(n: usize) -> SyntheticPopulation {
        let config = make_config(42, n as u64, 1.0);
        let mut gen = AccountGenerator::new(config).unwrap();
        gen.generate_population().unwrap()
    }

    #[test]
    fn intent_generator_determinism() {
        let pop1 = make_population_for_intents(10);
        let pop2 = make_population_for_intents(10);
        let config1 = make_config(42, 10, 1.0);
        let config2 = make_config(42, 10, 1.0);
        let mut gen1 = TransactionIntentGenerator::new(&pop1, &config1).unwrap();
        let mut gen2 = TransactionIntentGenerator::new(&pop2, &config2).unwrap();
        let ids1: Vec<String> = (0..100)
            .map(|_| gen1.next_intent("r", &pop1).unwrap().intent_id)
            .collect();
        let ids2: Vec<String> = (0..100)
            .map(|_| gen2.next_intent("r", &pop2).unwrap().intent_id)
            .collect();
        assert_eq!(ids1, ids2);
    }

    #[test]
    fn intent_generator_none_with_zero_active() {
        let pop = SyntheticPopulation::new_for_test(vec![], vec![]);
        let config = make_config(1, 0, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        assert!(gen.next_intent("r", &pop).is_none());
    }

    #[test]
    fn intent_generator_none_with_one_active() {
        use crate::data_model::{AccountStatus, ActivityProfile};
        use chrono::Utc;
        let accounts = vec![Account {
            account_id: "a1".into(),
            status: AccountStatus::Active,
            activity_profile: ActivityProfile::Low,
            wallet_id: "w1".into(),
            created_at: Utc::now(),
        }];
        let wallets = vec![Wallet {
            wallet_id: "w1".into(),
            account_id: "a1".into(),
            transparent_addresses: vec![],
            shielded_addresses: vec![],
            created_at: Utc::now(),
        }];
        let pop = SyntheticPopulation::new_for_test(accounts, wallets);
        let config = make_config(1, 1, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        assert!(gen.next_intent("r", &pop).is_none());
    }

    #[test]
    fn intent_generator_distinct_pair() {
        let pop = make_population_for_intents(10);
        let config = make_config(42, 10, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..1000 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert_ne!(intent.account_id, intent.recipient_account_id);
        }
    }

    #[test]
    fn intent_generator_flow_distribution() {
        let pop = make_population_for_intents(10);
        let config = make_config_with_flows(1, 10, 1.0, 0.0, 0.0, 0.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..1000 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert_eq!(intent.flow_type, FlowType::TToT);
        }
    }

    #[test]
    fn intent_generator_flow_distribution_all_t_to_z() {
        let pop = make_population_for_intents(10);
        let config = make_config_with_flows(1, 10, 0.0, 1.0, 0.0, 0.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..1000 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert_eq!(intent.flow_type, FlowType::TToZ);
        }
    }

    #[test]
    fn intent_generator_flow_distribution_all_z_to_t() {
        let pop = make_population_for_intents(10);
        let config = make_config_with_flows(1, 10, 0.0, 0.0, 1.0, 0.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..1000 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert_eq!(intent.flow_type, FlowType::ZToT);
        }
    }

    #[test]
    fn intent_generator_flow_distribution_all_z_to_z() {
        let pop = make_population_for_intents(10);
        let config = make_config_with_flows(1, 10, 0.0, 0.0, 0.0, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..1000 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert_eq!(intent.flow_type, FlowType::ZToZ);
        }
    }

    #[test]
    fn intent_generator_flow_distribution_mixed() {
        let pop = make_population_for_intents(10);
        let config = make_config_with_flows(1, 10, 0.25, 0.25, 0.25, 0.25);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        let mut counts = [0usize; 4];
        for _ in 0..1000 {
            let intent = gen.next_intent("r", &pop).unwrap();
            match intent.flow_type {
                FlowType::TToT => counts[0] += 1,
                FlowType::TToZ => counts[1] += 1,
                FlowType::ZToT => counts[2] += 1,
                FlowType::ZToZ => counts[3] += 1,
            }
        }
        for (i, &c) in counts.iter().enumerate() {
            assert!(c >= 200 && c <= 300, "flow[{i}]={c}");
        }
    }

    #[test]
    fn intent_generator_amount_in_range() {
        let pop = make_population_for_intents(10);
        let config = make_config(42, 10, 1.0);
        let min = config.amounts.min_zatoshis;
        let max = config.amounts.max_zatoshis;
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..1000 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert!(intent.amount_zatoshis >= min && intent.amount_zatoshis <= max);
        }
    }

    #[test]
    fn intent_generator_amount_fixed_range() {
        let pop = make_population_for_intents(10);
        let config = make_config_with_amounts(1, 10, 500_000, 500_000);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..100 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert_eq!(intent.amount_zatoshis, 500_000);
        }
    }

    #[test]
    fn intent_generator_intent_id_is_uuid() {
        let pop = make_population_for_intents(10);
        let config = make_config(1, 10, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..20 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert!(
                is_valid_uuid_v4(&intent.intent_id),
                "invalid intent_id: {}",
                intent.intent_id
            );
        }
    }

    #[test]
    fn intent_generator_fee_is_zero() {
        let pop = make_population_for_intents(10);
        let config = make_config(1, 10, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..100 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert_eq!(intent.fee_zatoshis, 0);
        }
    }

    #[test]
    fn intent_generator_status_is_pending() {
        let pop = make_population_for_intents(10);
        let config = make_config(1, 10, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..100 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert_eq!(intent.status, TransactionStatus::Pending);
        }
    }

    #[test]
    fn intent_generator_submitted_at_is_none() {
        let pop = make_population_for_intents(10);
        let config = make_config(1, 10, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..100 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert!(intent.submitted_at.is_none());
        }
    }

    #[test]
    fn intent_generator_run_id_propagated() {
        let pop = make_population_for_intents(10);
        let config = make_config(1, 10, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        let intent = gen.next_intent("my-run-id", &pop).unwrap();
        assert_eq!(intent.run_id, "my-run-id");
    }

    #[test]
    fn intent_generator_sender_is_active_account() {
        let pop = make_population_for_intents(10);
        let config = make_config(42, 10, 1.0);
        let mut gen = TransactionIntentGenerator::new(&pop, &config).unwrap();
        for _ in 0..200 {
            let intent = gen.next_intent("r", &pop).unwrap();
            assert!(pop.active_account_ids.contains(&intent.account_id));
        }
    }

    #[test]
    fn intent_generator_rng_is_seeded_independently_of_account_generator() {
        let config5 = make_config(42, 5, 1.0);
        let config10 = make_config(42, 10, 1.0);
        let mut gen5 = AccountGenerator::new(config5.clone()).unwrap();
        let mut gen10 = AccountGenerator::new(config10.clone()).unwrap();
        let pop5 = gen5.generate_population().unwrap();
        let pop10 = gen10.generate_population().unwrap();
        let mut intent_gen5 = TransactionIntentGenerator::new(&pop5, &config5).unwrap();
        let mut intent_gen10 = TransactionIntentGenerator::new(&pop10, &config10).unwrap();
        let intent5 = intent_gen5.next_intent("r", &pop5).unwrap();
        let intent10 = intent_gen10.next_intent("r", &pop10).unwrap();
        assert_eq!(intent5.flow_type, intent10.flow_type);
    }

    #[test]
    fn intent_generator_invalid_flow_weights() {
        let pop = make_population_for_intents(10);
        let config = make_config_with_flows(1, 10, 0.0, 0.0, 0.0, 0.0);
        let result = TransactionIntentGenerator::new(&pop, &config);
        assert!(matches!(result, Err(GeneratorError::InvalidFlowWeights(_))));
    }
}

/// Errors returned by [`AccountGenerator`] and [`TransactionIntentGenerator`].
#[derive(Debug)]
pub enum GeneratorError {
    InvalidActivityProfileWeights(String),
    InvalidFlowWeights(String),
    AccountCountTooLarge,
}

impl std::fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeneratorError::InvalidActivityProfileWeights(msg) => {
                write!(f, "Invalid activity profile weights: {msg}")
            }
            GeneratorError::InvalidFlowWeights(msg) => {
                write!(f, "Invalid flow weights: {msg}")
            }
            GeneratorError::AccountCountTooLarge => write!(
                f,
                "accounts_count exceeds the maximum value representable as usize on this platform"
            ),
        }
    }
}

impl std::error::Error for GeneratorError {}
