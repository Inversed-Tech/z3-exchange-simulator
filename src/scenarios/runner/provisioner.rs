//! Population provisioning: create Zallet accounts and addresses for all
//! synthetic accounts in the population.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::data_model::{Address, AddressPurpose, AddressType, MetricSample, ScenarioConfig};
use crate::metrics::MetricsRecorder;
use crate::rpc::{RpcClient, RpcError};
use crate::synthetic::generators::{AccountGenerator, GeneratorError};
use crate::synthetic::{PopulationError, SyntheticPopulation};

// ── Types ─────────────────────────────────────────────────────────────────────

/// The result of provisioning: a fully-populated synthetic population with
/// Zallet UUIDs and addresses filled in, plus a reference to the hot wallet UUID.
pub struct ProvisionedPopulation {
    pub population: SyntheticPopulation,
    /// Maps simulator `account_id` → Zallet account UUID.
    pub zallet_uuids: Arc<HashMap<String, String>>,
    /// Maps simulator `account_id` → Unified Address (the `from` address for z_sendmany).
    pub zallet_addresses: Arc<HashMap<String, String>>,
    pub hot_wallet_uuid: String,
}

/// Coarse capacity planning numbers, used for dry-run output.
pub struct PopulationPlan {
    pub account_count: u64,
    pub active_count: u64,
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ProvisionerError {
    Rpc(RpcError),
    Population(PopulationError),
    Generator(GeneratorError),
    Join(tokio::task::JoinError),
}

impl fmt::Display for ProvisionerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProvisionerError::Rpc(e) => write!(f, "RPC error during provisioning: {e}"),
            ProvisionerError::Population(e) => write!(f, "Population error: {e}"),
            ProvisionerError::Generator(e) => write!(f, "Generator error: {e}"),
            ProvisionerError::Join(e) => write!(f, "Task join error: {e}"),
        }
    }
}

impl std::error::Error for ProvisionerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProvisionerError::Rpc(e) => Some(e),
            ProvisionerError::Population(e) => Some(e),
            ProvisionerError::Generator(e) => Some(e),
            ProvisionerError::Join(e) => Some(e),
        }
    }
}

// ── provision ─────────────────────────────────────────────────────────────────

const PROVISION_CONCURRENCY: usize = 16;

/// Generate a synthetic population, create Zallet accounts for each member,
/// derive Unified Addresses, and add both transparent and Orchard address
/// entries to each wallet.
pub async fn provision(
    rpc: Arc<RpcClient>,
    scenario: &ScenarioConfig,
    run_id: &str,
    metrics: Arc<dyn MetricsRecorder>,
    hot_wallet_uuid_override: Option<String>,
) -> Result<ProvisionedPopulation, ProvisionerError> {
    // Step 1: generate the synthetic population (takes ownership of scenario clone).
    let mut generator =
        AccountGenerator::new(scenario.clone()).map_err(ProvisionerError::Generator)?;
    let mut population = generator
        .generate_population()
        .map_err(ProvisionerError::Generator)?;

    // Step 2: resolve the hot wallet UUID.
    let hot_wallet_uuid = if let Some(uuid) = hot_wallet_uuid_override {
        uuid
    } else {
        // Adopt the first pre-existing Zallet account as the hot wallet. Z3's
        // regtest-init.sh creates this account and configures Zebra's miner_address
        // to its transparent receiver, so generate() calls during warmup fund it.
        let existing = rpc.z_list_accounts().await.map_err(ProvisionerError::Rpc)?;
        if let Some(first) = existing.into_iter().next() {
            first.account
        } else {
            rpc.z_get_new_account("hot_wallet")
                .await
                .map_err(ProvisionerError::Rpc)?
                .account
        }
    };

    // Step 3: for each account, spawn a semaphore-bounded task that creates
    // a Zallet account and derives a Unified Address.
    let sem = Arc::new(Semaphore::new(PROVISION_CONCURRENCY));
    let mut tasks: JoinSet<Result<(String, String, String), RpcError>> = JoinSet::new();

    for account in &population.accounts {
        let permit = sem.clone().acquire_owned().await.unwrap();
        let rpc_clone = rpc.clone();
        let account_id = account.account_id.clone();

        tasks.spawn(async move {
            let _permit = permit;
            let account_info = rpc_clone.z_get_new_account(&account_id).await?;
            let zallet_uuid = account_info.account.clone();
            let ua = rpc_clone.z_get_address_for_account(&zallet_uuid, &["orchard"], None).await?;
            Ok((account_id, zallet_uuid, ua.address))
        });
    }

    // Step 4: collect task results.
    let mut provisioned: Vec<(String, String, String)> = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let inner = result.map_err(ProvisionerError::Join)?;
        let tuple = inner.map_err(ProvisionerError::Rpc)?;
        provisioned.push(tuple);
    }

    // Step 5: for each provisioned account, add transparent and Orchard
    // address entries to the population's wallet.
    let mut zallet_uuids: HashMap<String, String> = HashMap::new();
    let mut zallet_addresses: HashMap<String, String> = HashMap::new();

    for (account_id, zallet_uuid, ua_address) in &provisioned {
        zallet_uuids.insert(account_id.clone(), zallet_uuid.clone());
        zallet_addresses.insert(account_id.clone(), ua_address.clone());

        let wallet_id = population
            .wallet_for_account(account_id)
            .map(|w| w.wallet_id.clone())
            .unwrap_or_default();

        // Transparent entry
        population
            .add_address(
                account_id,
                Address {
                    address_id: format!("addr-{account_id}-t"),
                    wallet_id: wallet_id.clone(),
                    address: ua_address.clone(),
                    address_type: AddressType::Transparent,
                    purpose: AddressPurpose::Deposit,
                    created_at: Utc::now(),
                    last_used_at: None,
                },
            )
            .map_err(ProvisionerError::Population)?;

        // Orchard entry
        population
            .add_address(
                account_id,
                Address {
                    address_id: format!("addr-{account_id}-z"),
                    wallet_id,
                    address: ua_address.clone(),
                    address_type: AddressType::Orchard,
                    purpose: AddressPurpose::Deposit,
                    created_at: Utc::now(),
                    last_used_at: None,
                },
            )
            .map_err(ProvisionerError::Population)?;
    }

    // Step 6: emit provisioning metric.
    metrics.record_metric(MetricSample {
        run_id: run_id.to_string(),
        timestamp: Utc::now(),
        metric_name: "accounts_provisioned".to_string(),
        value: provisioned.len() as f64,
        labels: Default::default(),
    });

    Ok(ProvisionedPopulation {
        population,
        zallet_uuids: Arc::new(zallet_uuids),
        zallet_addresses: Arc::new(zallet_addresses),
        hot_wallet_uuid,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

    use crate::data_model::{
        ActivityProfileConfig, AmountRangeConfig, FlowConfig, MetricSample, ObservabilityConfig,
        RpcCall, ScenarioConfig,
    };
    use crate::metrics::{MetricsRecorder, NullRecorder};
    use crate::rpc::RpcClient;

    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn test_scenario() -> ScenarioConfig {
        ScenarioConfig {
            name: "test".into(),
            description: "test scenario".into(),
            seed: 1,
            accounts_count: 2,
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
                low_fraction: 0.50,
                medium_fraction: 0.35,
                high_fraction: 0.15,
            },
            amounts: AmountRangeConfig {
                min_zatoshis: 10_000,
                max_zatoshis: 10_000_000,
            },
            confirmations_deposit_required: 1,
            observability: ObservabilityConfig {
                record_rpc_calls: false,
                record_component_logs: false,
                metric_sampling_interval_secs: 5,
                mempool_saturation_threshold: 500,
            },
            config_hash: String::new(),
            source_path: String::new(),
            warmup_blocks: 0,
        }
    }

    struct MockRecorder {
        samples: Mutex<Vec<MetricSample>>,
    }

    impl MockRecorder {
        fn new() -> Self {
            Self {
                samples: Mutex::new(Vec::new()),
            }
        }

        fn samples(&self) -> Vec<MetricSample> {
            self.samples.lock().unwrap().clone()
        }
    }

    impl MetricsRecorder for MockRecorder {
        fn record_rpc_call(&self, _: RpcCall) {}
        fn record_metric(&self, sample: MetricSample) {
            self.samples.lock().unwrap().push(sample);
        }
    }

    async fn mount_account_mocks(server: &MockServer) {
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({"method": "z_getnewaccount"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"account_uuid": "uuid-1234", "name": null},
                "error": null, "id": 1
            })))
            .mount(server)
            .await;

        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(
                serde_json::json!({"method": "z_getaddressforaccount"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "account_uuid": "uuid-1234",
                    "address": "u1testaddress",
                    "receiver_types": ["orchard", "sapling", "p2pkh"]
                },
                "error": null, "id": 1
            })))
            .mount(server)
            .await;
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn provision_creates_accounts_and_addresses() {
        let server = MockServer::start().await;
        mount_account_mocks(&server).await;

        let rpc = Arc::new(RpcClient::new(&server.uri(), "test-run", None, None));
        let scenario = test_scenario();

        let result = provision(
            rpc,
            &scenario,
            "test-run",
            Arc::new(NullRecorder),
            Some("hw-uuid".to_string()),
        )
        .await
        .expect("provision should succeed");

        assert_eq!(
            result.zallet_uuids.len(),
            scenario.accounts_count as usize,
            "one Zallet UUID per synthetic account"
        );
        assert_eq!(result.hot_wallet_uuid, "hw-uuid");
    }

    #[tokio::test]
    async fn provisioner_constructs_address_struct_correctly() {
        let server = MockServer::start().await;
        mount_account_mocks(&server).await;

        let rpc = Arc::new(RpcClient::new(&server.uri(), "test-run", None, None));
        let scenario = test_scenario();

        let result = provision(
            rpc,
            &scenario,
            "test-run",
            Arc::new(NullRecorder),
            Some("hw-uuid".to_string()),
        )
        .await
        .expect("provision should succeed");

        // Every synthetic account_id must map to the UUID returned by the mock.
        for account in &result.population.accounts {
            let uuid = result
                .zallet_uuids
                .get(&account.account_id)
                .expect("account_id must appear in zallet_uuids");
            assert_eq!(uuid, "uuid-1234", "Zallet UUID must match mock response");
        }
    }

    #[tokio::test]
    async fn provision_records_metric() {
        let server = MockServer::start().await;
        mount_account_mocks(&server).await;

        let rpc = Arc::new(RpcClient::new(&server.uri(), "test-run", None, None));
        let scenario = test_scenario();
        let recorder = Arc::new(MockRecorder::new());
        let metrics: Arc<dyn MetricsRecorder> = recorder.clone();

        provision(
            rpc,
            &scenario,
            "test-run",
            metrics,
            Some("hw-uuid".to_string()),
        )
        .await
        .expect("provision should succeed");

        let samples = recorder.samples();
        let metric = samples
            .iter()
            .find(|s| s.metric_name == "accounts_provisioned")
            .expect("accounts_provisioned metric must be emitted");
        assert_eq!(
            metric.value, scenario.accounts_count as f64,
            "accounts_provisioned must equal accounts_count"
        );
    }

    #[tokio::test]
    async fn provision_fails_on_rpc_error() {
        let server = MockServer::start().await;
        // No mocks mounted — z_listaccounts returns 404, which the RPC client
        // treats as an error, causing provision() to return ProvisionerError::Rpc.

        let rpc = Arc::new(RpcClient::new(&server.uri(), "test-run", None, None));
        let scenario = test_scenario();

        let result = provision(rpc, &scenario, "test-run", Arc::new(NullRecorder), None).await;
        let err = match result {
            Ok(_) => panic!("provision must fail when RPC returns an error"),
            Err(e) => e,
        };

        assert!(
            matches!(err, ProvisionerError::Rpc(_)),
            "expected ProvisionerError::Rpc, got: {err}"
        );
    }
}
