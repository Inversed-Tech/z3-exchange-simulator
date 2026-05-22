use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Enums ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Active,
    Inactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityProfile {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressType {
    Transparent,
    Sapling,
    Orchard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressPurpose {
    Deposit,
    Change,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowType {
    #[serde(rename = "t_to_t")]
    TToT,
    #[serde(rename = "t_to_z")]
    TToZ,
    #[serde(rename = "z_to_t")]
    ZToT,
    #[serde(rename = "z_to_z")]
    ZToZ,
}

impl FlowType {
    pub fn is_shielded(&self) -> bool {
        matches!(self, FlowType::TToZ | FlowType::ZToT | FlowType::ZToZ)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    Pending,
    Submitted,
    Confirming,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositStatus {
    Pending,
    Detected,
    Confirming,
    Credited,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WithdrawalStatus {
    Requested,
    Processing,
    Broadcast,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepStatus {
    Pending,
    Processing,
    Broadcast,
    Confirmed,
    Failed,
}

/// Which Z3 backend the RPC Router forwarded a call to.
/// Derived from the method routing table, not from which URL was called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    Zebra,
    Zallet,
    Unknown,
}

// ── Structs ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub account_id: String,
    pub status: AccountStatus,
    pub activity_profile: ActivityProfile,
    pub wallet_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub wallet_id: String,
    pub account_id: String,
    pub transparent_addresses: Vec<Address>,
    pub shielded_addresses: Vec<Address>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Address {
    pub address_id: String,
    pub wallet_id: String,
    pub address: String,
    pub address_type: AddressType,
    pub purpose: AddressPurpose,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Balance {
    pub wallet_id: String,
    pub transparent_confirmed: u64,
    pub transparent_unconfirmed: u64,
    pub shielded_confirmed: u64,
    pub shielded_unconfirmed: u64,
    pub total_confirmed: u64,
    pub at_block_height: u64,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionIntent {
    pub intent_id: String,
    pub run_id: String,
    pub account_id: String,
    pub sender_address: String,
    pub recipient_address: String,
    pub amount_zatoshis: u64,
    pub fee_zatoshis: u64,
    pub flow_type: FlowType,
    pub status: TransactionStatus,
    pub created_at: DateTime<Utc>,
    pub submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResult {
    pub intent_id: String,
    pub txid: Option<String>,
    pub status: TransactionStatus,
    pub confirmations: u64,
    pub broadcast_latency_ms: Option<u64>,
    pub proving_time_ms: Option<u64>,
    pub confirmed_at_height: Option<u64>,
    pub error: Option<String>,
    pub rpc_call_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deposit {
    pub deposit_id: String,
    pub account_id: String,
    pub deposit_address: String,
    pub amount_zatoshis: u64,
    pub txid: Option<String>,
    pub status: DepositStatus,
    pub required_confirmations: u64,
    pub current_confirmations: u64,
    pub created_at: DateTime<Utc>,
    pub credited_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Withdrawal {
    pub withdrawal_id: String,
    pub account_id: String,
    pub destination_address: String,
    pub amount_zatoshis: u64,
    pub fee_zatoshis: u64,
    pub status: WithdrawalStatus,
    pub txid: Option<String>,
    pub intent_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub broadcast_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sweep {
    pub sweep_id: String,
    pub source_addresses: Vec<String>,
    pub destination_address: String,
    pub total_amount_zatoshis: u64,
    pub fee_zatoshis: u64,
    pub status: SweepStatus,
    pub txid: Option<String>,
    pub intent_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcCall {
    pub call_id: String,
    pub run_id: String,
    pub method: String,
    pub backend: Backend,
    pub params_hash: Option<String>,
    pub request_at: DateTime<Utc>,
    pub response_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<u64>,
    pub success: bool,
    pub error_code: Option<i64>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowConfig {
    pub transparent_to_transparent: f64,
    pub transparent_to_shielded: f64,
    pub shielded_to_transparent: f64,
    pub shielded_to_shielded: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub record_rpc_calls: bool,
    pub record_component_logs: bool,
    pub metric_sampling_interval_secs: u64,
    pub mempool_saturation_threshold: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioConfig {
    pub name: String,
    pub description: String,
    pub seed: u64,
    pub accounts_count: u64,
    pub accounts_active_fraction: f64,
    pub load_duration_seconds: u64,
    pub load_target_tps: f64,
    pub flows: FlowConfig,
    pub confirmations_deposit_required: u64,
    pub observability: ObservabilityConfig,
    pub config_hash: String,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSample {
    pub run_id: String,
    pub timestamp: DateTime<Utc>,
    pub metric_name: String,
    pub value: f64,
    pub labels: HashMap<String, String>,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> DateTime<Utc> {
        Utc::now()
    }

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de>>(v: &T) -> T {
        serde_json::from_str(&serde_json::to_string(v).unwrap()).unwrap()
    }

    #[test]
    fn account() {
        let v = Account {
            account_id: "acc-1".into(),
            status: AccountStatus::Active,
            activity_profile: ActivityProfile::Medium,
            wallet_id: "wal-1".into(),
            created_at: ts(),
        };
        let back = roundtrip(&v);
        assert_eq!(v.account_id, back.account_id);
        assert_eq!(v.status, back.status);
        assert_eq!(v.activity_profile, back.activity_profile);
    }

    #[test]
    fn wallet() {
        let v = Wallet {
            wallet_id: "wal-1".into(),
            account_id: "acc-1".into(),
            transparent_addresses: vec![],
            shielded_addresses: vec![],
            created_at: ts(),
        };
        let back = roundtrip(&v);
        assert_eq!(v.wallet_id, back.wallet_id);
    }

    #[test]
    fn address() {
        let v = Address {
            address_id: "addr-1".into(),
            wallet_id: "wal-1".into(),
            address: "t1abc".into(),
            address_type: AddressType::Transparent,
            purpose: AddressPurpose::Deposit,
            created_at: ts(),
            last_used_at: None,
        };
        let back = roundtrip(&v);
        assert_eq!(v.address, back.address);
        assert_eq!(v.address_type, back.address_type);
        assert_eq!(v.purpose, back.purpose);
    }

    #[test]
    fn balance() {
        let v = Balance {
            wallet_id: "wal-1".into(),
            transparent_confirmed: 100_000_000,
            transparent_unconfirmed: 0,
            shielded_confirmed: 50_000_000,
            shielded_unconfirmed: 0,
            total_confirmed: 150_000_000,
            at_block_height: 42,
            recorded_at: ts(),
        };
        let back = roundtrip(&v);
        assert_eq!(v.total_confirmed, back.total_confirmed);
        assert_eq!(v.at_block_height, back.at_block_height);
    }

    #[test]
    fn transaction_intent() {
        let v = TransactionIntent {
            intent_id: "int-1".into(),
            run_id: "run-1".into(),
            account_id: "acc-1".into(),
            sender_address: "t1sender".into(),
            recipient_address: "t1recipient".into(),
            amount_zatoshis: 5_000_000,
            fee_zatoshis: 1_000,
            flow_type: FlowType::TToT,
            status: TransactionStatus::Pending,
            created_at: ts(),
            submitted_at: None,
        };
        let back = roundtrip(&v);
        assert_eq!(v.flow_type, back.flow_type);
        assert_eq!(v.amount_zatoshis, back.amount_zatoshis);
    }

    #[test]
    fn transaction_result() {
        let v = TransactionResult {
            intent_id: "int-1".into(),
            txid: Some("deadbeef".into()),
            status: TransactionStatus::Confirmed,
            confirmations: 12,
            broadcast_latency_ms: Some(8),
            proving_time_ms: None,
            confirmed_at_height: Some(100),
            error: None,
            rpc_call_ids: vec!["c-1".into(), "c-2".into()],
        };
        let back = roundtrip(&v);
        assert_eq!(v.txid, back.txid);
        assert_eq!(v.confirmations, back.confirmations);
        assert_eq!(v.rpc_call_ids, back.rpc_call_ids);
    }

    #[test]
    fn deposit() {
        let v = Deposit {
            deposit_id: "dep-1".into(),
            account_id: "acc-1".into(),
            deposit_address: "t1dep".into(),
            amount_zatoshis: 1_000_000,
            txid: None,
            status: DepositStatus::Pending,
            required_confirmations: 10,
            current_confirmations: 0,
            created_at: ts(),
            credited_at: None,
        };
        let back = roundtrip(&v);
        assert_eq!(v.status, back.status);
        assert_eq!(v.required_confirmations, back.required_confirmations);
    }

    #[test]
    fn withdrawal() {
        let v = Withdrawal {
            withdrawal_id: "wdl-1".into(),
            account_id: "acc-1".into(),
            destination_address: "t1dest".into(),
            amount_zatoshis: 2_000_000,
            fee_zatoshis: 1_000,
            status: WithdrawalStatus::Requested,
            txid: None,
            intent_id: Some("int-1".into()),
            created_at: ts(),
            broadcast_at: None,
        };
        let back = roundtrip(&v);
        assert_eq!(v.status, back.status);
        assert_eq!(v.amount_zatoshis, back.amount_zatoshis);
    }

    #[test]
    fn sweep() {
        let v = Sweep {
            sweep_id: "swp-1".into(),
            source_addresses: vec!["t1a".into(), "t1b".into()],
            destination_address: "t1hot".into(),
            total_amount_zatoshis: 3_000_000,
            fee_zatoshis: 2_000,
            status: SweepStatus::Pending,
            txid: None,
            intent_ids: vec!["int-1".into(), "int-2".into()],
            created_at: ts(),
        };
        let back = roundtrip(&v);
        assert_eq!(v.source_addresses, back.source_addresses);
        assert_eq!(v.intent_ids, back.intent_ids);
    }

    #[test]
    fn rpc_call() {
        let v = RpcCall {
            call_id: "c-1".into(),
            run_id: "run-1".into(),
            method: "getblockchaininfo".into(),
            backend: Backend::Zebra,
            params_hash: None,
            request_at: ts(),
            response_at: None,
            latency_ms: Some(12),
            success: true,
            error_code: None,
            error_message: None,
        };
        let back = roundtrip(&v);
        assert_eq!(v.backend, back.backend);
        assert_eq!(v.latency_ms, back.latency_ms);
        assert!(back.success);
    }

    #[test]
    fn scenario_config() {
        let v = ScenarioConfig {
            name: "smoke".into(),
            description: "minimal run".into(),
            seed: 42,
            accounts_count: 10,
            accounts_active_fraction: 1.0,
            load_duration_seconds: 60,
            load_target_tps: 1.0,
            flows: FlowConfig {
                transparent_to_transparent: 1.0,
                transparent_to_shielded: 0.0,
                shielded_to_transparent: 0.0,
                shielded_to_shielded: 0.0,
            },
            confirmations_deposit_required: 10,
            observability: ObservabilityConfig {
                record_rpc_calls: true,
                record_component_logs: true,
                metric_sampling_interval_secs: 5,
                mempool_saturation_threshold: 500,
            },
            config_hash: "abc123".into(),
            source_path: "configs/scenarios/smoke.yaml".into(),
        };
        let back = roundtrip(&v);
        assert_eq!(v.name, back.name);
        assert_eq!(v.seed, back.seed);
        assert_eq!(
            v.flows.transparent_to_transparent,
            back.flows.transparent_to_transparent
        );
        assert_eq!(
            v.observability.mempool_saturation_threshold,
            back.observability.mempool_saturation_threshold
        );
    }

    #[test]
    fn metric_sample() {
        let mut labels = HashMap::new();
        labels.insert("method".into(), "getblockcount".into());
        labels.insert("backend".into(), "Zebra".into());
        let v = MetricSample {
            run_id: "run-1".into(),
            timestamp: ts(),
            metric_name: "rpc_latency_ms".into(),
            value: 12.5,
            labels,
        };
        let back = roundtrip(&v);
        assert_eq!(v.metric_name, back.metric_name);
        assert_eq!(v.value, back.value);
        assert_eq!(v.labels["method"], back.labels["method"]);
    }

    #[test]
    fn flow_type_is_shielded() {
        assert!(!FlowType::TToT.is_shielded());
        assert!(FlowType::TToZ.is_shielded());
        assert!(FlowType::ZToT.is_shielded());
        assert!(FlowType::ZToZ.is_shielded());
    }

    // ── Wire format: exact JSON string for every variant ─────────────────────

    #[test]
    fn flow_type_wire_format_all_variants() {
        assert_eq!(
            serde_json::to_string(&FlowType::TToT).unwrap(),
            r#""t_to_t""#
        );
        assert_eq!(
            serde_json::to_string(&FlowType::TToZ).unwrap(),
            r#""t_to_z""#
        );
        assert_eq!(
            serde_json::to_string(&FlowType::ZToT).unwrap(),
            r#""z_to_t""#
        );
        assert_eq!(
            serde_json::to_string(&FlowType::ZToZ).unwrap(),
            r#""z_to_z""#
        );
    }

    #[test]
    fn account_status_wire_format() {
        assert_eq!(
            serde_json::to_string(&AccountStatus::Active).unwrap(),
            r#""active""#
        );
        assert_eq!(
            serde_json::to_string(&AccountStatus::Inactive).unwrap(),
            r#""inactive""#
        );
    }

    #[test]
    fn activity_profile_wire_format() {
        assert_eq!(
            serde_json::to_string(&ActivityProfile::Low).unwrap(),
            r#""low""#
        );
        assert_eq!(
            serde_json::to_string(&ActivityProfile::Medium).unwrap(),
            r#""medium""#
        );
        assert_eq!(
            serde_json::to_string(&ActivityProfile::High).unwrap(),
            r#""high""#
        );
    }

    #[test]
    fn address_type_wire_format() {
        assert_eq!(
            serde_json::to_string(&AddressType::Transparent).unwrap(),
            r#""transparent""#
        );
        assert_eq!(
            serde_json::to_string(&AddressType::Sapling).unwrap(),
            r#""sapling""#
        );
        assert_eq!(
            serde_json::to_string(&AddressType::Orchard).unwrap(),
            r#""orchard""#
        );
    }

    #[test]
    fn address_purpose_wire_format() {
        assert_eq!(
            serde_json::to_string(&AddressPurpose::Deposit).unwrap(),
            r#""deposit""#
        );
        assert_eq!(
            serde_json::to_string(&AddressPurpose::Change).unwrap(),
            r#""change""#
        );
        assert_eq!(
            serde_json::to_string(&AddressPurpose::Internal).unwrap(),
            r#""internal""#
        );
    }

    #[test]
    fn transaction_status_wire_format() {
        assert_eq!(
            serde_json::to_string(&TransactionStatus::Pending).unwrap(),
            r#""pending""#
        );
        assert_eq!(
            serde_json::to_string(&TransactionStatus::Submitted).unwrap(),
            r#""submitted""#
        );
        assert_eq!(
            serde_json::to_string(&TransactionStatus::Confirming).unwrap(),
            r#""confirming""#
        );
        assert_eq!(
            serde_json::to_string(&TransactionStatus::Confirmed).unwrap(),
            r#""confirmed""#
        );
        assert_eq!(
            serde_json::to_string(&TransactionStatus::Failed).unwrap(),
            r#""failed""#
        );
    }

    #[test]
    fn deposit_status_wire_format() {
        assert_eq!(
            serde_json::to_string(&DepositStatus::Pending).unwrap(),
            r#""pending""#
        );
        assert_eq!(
            serde_json::to_string(&DepositStatus::Detected).unwrap(),
            r#""detected""#
        );
        assert_eq!(
            serde_json::to_string(&DepositStatus::Confirming).unwrap(),
            r#""confirming""#
        );
        assert_eq!(
            serde_json::to_string(&DepositStatus::Credited).unwrap(),
            r#""credited""#
        );
        assert_eq!(
            serde_json::to_string(&DepositStatus::Failed).unwrap(),
            r#""failed""#
        );
    }

    #[test]
    fn withdrawal_status_wire_format() {
        assert_eq!(
            serde_json::to_string(&WithdrawalStatus::Requested).unwrap(),
            r#""requested""#
        );
        assert_eq!(
            serde_json::to_string(&WithdrawalStatus::Processing).unwrap(),
            r#""processing""#
        );
        assert_eq!(
            serde_json::to_string(&WithdrawalStatus::Broadcast).unwrap(),
            r#""broadcast""#
        );
        assert_eq!(
            serde_json::to_string(&WithdrawalStatus::Confirmed).unwrap(),
            r#""confirmed""#
        );
        assert_eq!(
            serde_json::to_string(&WithdrawalStatus::Failed).unwrap(),
            r#""failed""#
        );
    }

    #[test]
    fn sweep_status_wire_format() {
        assert_eq!(
            serde_json::to_string(&SweepStatus::Pending).unwrap(),
            r#""pending""#
        );
        assert_eq!(
            serde_json::to_string(&SweepStatus::Processing).unwrap(),
            r#""processing""#
        );
        assert_eq!(
            serde_json::to_string(&SweepStatus::Broadcast).unwrap(),
            r#""broadcast""#
        );
        assert_eq!(
            serde_json::to_string(&SweepStatus::Confirmed).unwrap(),
            r#""confirmed""#
        );
        assert_eq!(
            serde_json::to_string(&SweepStatus::Failed).unwrap(),
            r#""failed""#
        );
    }

    #[test]
    fn backend_wire_format_all_variants() {
        // Backend has no rename_all — variants serialize as their Rust names (PascalCase).
        assert_eq!(
            serde_json::to_string(&Backend::Zebra).unwrap(),
            r#""Zebra""#
        );
        assert_eq!(
            serde_json::to_string(&Backend::Zallet).unwrap(),
            r#""Zallet""#
        );
        assert_eq!(
            serde_json::to_string(&Backend::Unknown).unwrap(),
            r#""Unknown""#
        );
    }

    // ── Deserialization ───────────────────────────────────────────────────────

    #[test]
    fn enum_variants_deserialize_from_wire_strings() {
        assert_eq!(
            serde_json::from_str::<FlowType>(r#""t_to_z""#).unwrap(),
            FlowType::TToZ
        );
        assert_eq!(
            serde_json::from_str::<Backend>(r#""Unknown""#).unwrap(),
            Backend::Unknown
        );
        assert_eq!(
            serde_json::from_str::<DepositStatus>(r#""credited""#).unwrap(),
            DepositStatus::Credited
        );
        assert_eq!(
            serde_json::from_str::<WithdrawalStatus>(r#""broadcast""#).unwrap(),
            WithdrawalStatus::Broadcast
        );
    }

    #[test]
    fn unknown_enum_discriminant_is_rejected() {
        // None of these enums use #[serde(other)], so unknown variants must fail.
        assert!(serde_json::from_str::<AccountStatus>(r#""suspended""#).is_err());
        assert!(serde_json::from_str::<FlowType>(r#""t_to_everything""#).is_err());
        assert!(serde_json::from_str::<Backend>(r#""Zaino""#).is_err());
        assert!(serde_json::from_str::<DepositStatus>(r#""bounced""#).is_err());
    }

    // ── Option<T> fields ─────────────────────────────────────────────────────

    #[test]
    fn address_last_used_at_none_serializes_as_null() {
        let addr = Address {
            address_id: "a".into(),
            wallet_id: "w".into(),
            address: "t1x".into(),
            address_type: AddressType::Transparent,
            purpose: AddressPurpose::Deposit,
            created_at: ts(),
            last_used_at: None,
        };
        let json: serde_json::Value = serde_json::to_value(&addr).unwrap();
        assert!(json["last_used_at"].is_null());
    }

    #[test]
    fn address_last_used_at_some_roundtrips() {
        let t = DateTime::parse_from_rfc3339("2024-06-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let addr = Address {
            address_id: "a".into(),
            wallet_id: "w".into(),
            address: "t1x".into(),
            address_type: AddressType::Transparent,
            purpose: AddressPurpose::Deposit,
            created_at: t,
            last_used_at: Some(t),
        };
        let back = roundtrip(&addr);
        assert_eq!(back.last_used_at, Some(t));
    }

    #[test]
    fn transaction_result_all_option_fields_some() {
        let v = TransactionResult {
            intent_id: "i".into(),
            txid: Some("beefcafe".into()),
            status: TransactionStatus::Confirmed,
            confirmations: 15,
            broadcast_latency_ms: Some(42),
            proving_time_ms: Some(3_000),
            confirmed_at_height: Some(999),
            error: None,
            rpc_call_ids: vec![],
        };
        let back = roundtrip(&v);
        assert_eq!(back.txid, Some("beefcafe".into()));
        assert_eq!(back.broadcast_latency_ms, Some(42));
        assert_eq!(back.proving_time_ms, Some(3_000));
        assert_eq!(back.confirmed_at_height, Some(999));
    }

    #[test]
    fn transaction_result_all_option_fields_none() {
        let v = TransactionResult {
            intent_id: "i".into(),
            txid: None,
            status: TransactionStatus::Failed,
            confirmations: 0,
            broadcast_latency_ms: None,
            proving_time_ms: None,
            confirmed_at_height: None,
            error: Some("rpc timeout".into()),
            rpc_call_ids: vec![],
        };
        let back = roundtrip(&v);
        assert_eq!(back.txid, None);
        assert_eq!(back.broadcast_latency_ms, None);
        assert_eq!(back.proving_time_ms, None);
        assert_eq!(back.confirmed_at_height, None);
        assert_eq!(back.error, Some("rpc timeout".into()));
    }

    #[test]
    fn rpc_call_option_fields_all_some() {
        let t = DateTime::parse_from_rfc3339("2024-06-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let v = RpcCall {
            call_id: "c".into(),
            run_id: "r".into(),
            method: "z_getbalances".into(),
            backend: Backend::Zallet,
            params_hash: Some("sha256:abcdef".into()),
            request_at: t,
            response_at: Some(t),
            latency_ms: Some(55),
            success: false,
            error_code: Some(-32_601),
            error_message: Some("Method not found".into()),
        };
        let back = roundtrip(&v);
        assert_eq!(back.params_hash, Some("sha256:abcdef".into()));
        assert_eq!(back.response_at, Some(t));
        assert_eq!(back.error_code, Some(-32_601));
        assert_eq!(back.error_message, Some("Method not found".into()));
        assert!(!back.success);
    }

    #[test]
    fn transaction_intent_submitted_at_option_roundtrips() {
        let t = DateTime::parse_from_rfc3339("2024-06-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut v = TransactionIntent {
            intent_id: "i".into(),
            run_id: "r".into(),
            account_id: "a".into(),
            sender_address: "t1s".into(),
            recipient_address: "t1r".into(),
            amount_zatoshis: 1_000,
            fee_zatoshis: 100,
            flow_type: FlowType::TToT,
            status: TransactionStatus::Pending,
            created_at: ts(),
            submitted_at: None,
        };
        assert_eq!(roundtrip(&v).submitted_at, None);
        v.submitted_at = Some(t);
        assert_eq!(roundtrip(&v).submitted_at, Some(t));
    }

    // ── DateTime sub-second precision ─────────────────────────────────────────

    #[test]
    fn datetime_utc_preserves_subsecond_precision() {
        // Nanosecond precision must survive the JSON roundtrip.
        let dt = DateTime::parse_from_rfc3339("2024-01-15T10:30:00.123456789Z")
            .unwrap()
            .with_timezone(&Utc);
        let json = serde_json::to_string(&dt).unwrap();
        let back: DateTime<Utc> = serde_json::from_str(&json).unwrap();
        assert_eq!(dt, back);
    }

    // ── u64 zatoshi boundaries ────────────────────────────────────────────────

    #[test]
    fn zatoshi_amount_zero_roundtrips() {
        let v = Balance {
            wallet_id: "w".into(),
            transparent_confirmed: 0,
            transparent_unconfirmed: 0,
            shielded_confirmed: 0,
            shielded_unconfirmed: 0,
            total_confirmed: 0,
            at_block_height: 0,
            recorded_at: ts(),
        };
        let back = roundtrip(&v);
        assert_eq!(back.transparent_confirmed, 0);
        assert_eq!(back.total_confirmed, 0);
        assert_eq!(back.at_block_height, 0);
    }

    #[test]
    fn zatoshi_amount_one_roundtrips() {
        let v = TransactionIntent {
            intent_id: "i".into(),
            run_id: "r".into(),
            account_id: "a".into(),
            sender_address: "t1s".into(),
            recipient_address: "t1r".into(),
            amount_zatoshis: 1,
            fee_zatoshis: 1,
            flow_type: FlowType::TToT,
            status: TransactionStatus::Pending,
            created_at: ts(),
            submitted_at: None,
        };
        let back = roundtrip(&v);
        assert_eq!(back.amount_zatoshis, 1);
        assert_eq!(back.fee_zatoshis, 1);
    }

    #[test]
    fn zatoshi_supply_cap_roundtrips() {
        // 21,000,000 ZEC × 100,000,000 zatoshis/ZEC — fits comfortably in u64.
        const SUPPLY_CAP: u64 = 2_100_000_000_000_000;
        let v = Deposit {
            deposit_id: "d".into(),
            account_id: "a".into(),
            deposit_address: "t1x".into(),
            amount_zatoshis: SUPPLY_CAP,
            txid: None,
            status: DepositStatus::Pending,
            required_confirmations: 10,
            current_confirmations: 0,
            created_at: ts(),
            credited_at: None,
        };
        let back = roundtrip(&v);
        assert_eq!(back.amount_zatoshis, SUPPLY_CAP);
    }

    // ── ScenarioConfig full nested roundtrip ──────────────────────────────────

    #[test]
    fn scenario_config_hash_and_source_path_survive_roundtrip() {
        let v = ScenarioConfig {
            name: "burst".into(),
            description: "burst load scenario".into(),
            seed: 99_999,
            accounts_count: 5_000,
            accounts_active_fraction: 0.8,
            load_duration_seconds: 300,
            load_target_tps: 50.0,
            flows: FlowConfig {
                transparent_to_transparent: 0.4,
                transparent_to_shielded: 0.2,
                shielded_to_transparent: 0.2,
                shielded_to_shielded: 0.2,
            },
            confirmations_deposit_required: 10,
            observability: ObservabilityConfig {
                record_rpc_calls: true,
                record_component_logs: false,
                metric_sampling_interval_secs: 10,
                mempool_saturation_threshold: 500,
            },
            config_hash: "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
                .into(),
            source_path: "/workspace/configs/scenarios/burst.yaml".into(),
        };
        let back = roundtrip(&v);
        assert_eq!(back.config_hash, v.config_hash);
        assert_eq!(back.source_path, v.source_path);
        assert_eq!(back.accounts_count, 5_000);
        assert_eq!(back.flows.shielded_to_shielded, 0.2);
        assert!(!back.observability.record_component_logs);
    }
}
