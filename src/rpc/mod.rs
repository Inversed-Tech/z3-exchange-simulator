use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::data_model::{Backend, RpcCall};
use crate::metrics::MetricsRecorder;

// ── Private envelope types ────────────────────────────────────────────────────
//
// Every JSON-RPC call has the same shape going out and coming back.
// These are internal — callers only see the typed result or an RpcError.

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    method: &'a str,
    params: serde_json::Value,
    id: u64,
}

#[derive(Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    error: Option<JsonRpcErrorBody>,
}

#[derive(Deserialize)]
struct JsonRpcErrorBody {
    code: i64,
    message: String,
}

// ── Public error type ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum RpcError {
    /// Network or HTTP failure — couldn't reach the server at all.
    Transport(String),
    /// Server responded but returned a JSON-RPC error object.
    JsonRpc { code: i64, message: String },
    /// Response arrived but couldn't be parsed into the expected type.
    Parse(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Transport(msg) => write!(f, "RPC transport error: {msg}"),
            RpcError::JsonRpc { code, message } => write!(f, "JSON-RPC error {code}: {message}"),
            RpcError::Parse(msg) => write!(f, "RPC response parse error: {msg}"),
        }
    }
}

impl std::error::Error for RpcError {}

// ── Parameter types ───────────────────────────────────────────────────────────

/// `getblock` and `getblockheader` accept either a block hash or a height.
pub enum BlockRef<'a> {
    Hash(&'a str),
    Height(u64),
}

impl Serialize for BlockRef<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            BlockRef::Hash(h) => s.serialize_str(h),
            BlockRef::Height(n) => s.serialize_u64(*n),
        }
    }
}

/// Optional block height range for `getaddresstxids`.
pub struct HeightRange {
    pub start: u64,
    pub end: u64,
}

// ── Response types ────────────────────────────────────────────────────────────
//
// One struct per RPC method return value. We only declare the fields the
// simulator actually uses — serde ignores extra fields from the server.

#[derive(Debug, Clone, Deserialize)]
pub struct BlockchainInfo {
    pub chain: String,
    pub blocks: u64,
    pub headers: u64,
}

/// Returned by `getblock` with verbosity 1 (default).
/// `tx` contains the txids of all transactions in the block.
#[derive(Debug, Clone, Deserialize)]
pub struct Block {
    pub hash: String,
    pub height: u64,
    pub confirmations: i64,
    pub time: u64,
    pub tx: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockHeader {
    pub hash: String,
    pub height: u64,
    pub confirmations: i64,
    pub time: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MempoolInfo {
    pub size: u64,
    pub bytes: u64,
}

/// Returned by `getrawtransaction` with `verbose=true`.
/// `confirmations` is absent when the transaction is still in the mempool.
#[derive(Debug, Clone, Deserialize)]
pub struct RawTransaction {
    pub txid: String,
    pub hex: String,
    pub confirmations: Option<u64>,
}

/// Returned by `gettxout`. The method returns JSON null when the output is
/// spent or does not exist — use `get_tx_out` which returns `Option<TxOut>`.
#[derive(Debug, Clone, Deserialize)]
pub struct TxOut {
    pub confirmations: u64,
    /// Value in ZEC. The Zcash RPC returns this as a float; convert to
    /// zatoshis by multiplying by 1e8 and rounding.
    pub value: f64,
}

/// Returned by `getaddressbalance`. Amounts are in zatoshis.
#[derive(Debug, Clone, Deserialize)]
pub struct AddressBalance {
    pub balance: i64,
    pub received: i64,
}

/// One entry from `getaddressutxos`.
#[derive(Debug, Clone, Deserialize)]
pub struct Utxo {
    pub address: String,
    pub txid: String,
    #[serde(rename = "outputIndex")]
    pub output_index: u32,
    pub satoshis: u64,
    pub height: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PeerInfo {
    pub addr: String,
    pub version: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddressValidation {
    pub isvalid: bool,
    pub address: Option<String>,
}

// ── Zallet response types ─────────────────────────────────────────────────────

/// Returned by `z_getnewaccount` and `z_getaccount`.
/// Zallet identifies accounts by UUID string, not a numeric index.
#[derive(Debug, Clone, Deserialize)]
pub struct AccountInfo {
    pub account: String,
    pub name: Option<String>,
}

/// Returned by `z_getaddressforaccount`.
/// `address` is the Unified Address used as the deposit address.
#[derive(Debug, Clone, Deserialize)]
pub struct UnifiedAddress {
    pub account: String,
    pub address: String,
    pub receiver_types: Vec<String>,
}

/// One entry from `listaddresses`.
#[derive(Debug, Clone, Deserialize)]
pub struct AddressEntry {
    pub source: String,
    pub address: Option<String>,
}

/// Returned by `z_gettotalbalance`. Amounts are ZEC strings (e.g. `"0.50000000"`).
#[derive(Debug, Clone, Deserialize)]
pub struct TotalBalance {
    pub transparent: String,
    pub private: String,
    pub total: String,
}

/// Returned by `z_getbalances`. Per-pool balances in zatoshis.
#[derive(Debug, Clone, Deserialize)]
pub struct Balances {
    pub transparent: u64,
    pub sapling: u64,
    pub orchard: u64,
}

/// The outcome detail inside a completed `OperationStatus` or `OperationResult`.
#[derive(Debug, Clone, Deserialize)]
pub struct OperationResultDetail {
    pub txid: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OperationError {
    pub code: i64,
    pub message: String,
}

/// One entry from `z_getoperationstatus`. `status` is one of:
/// `"queued"`, `"executing"`, `"success"`, `"failed"`, `"cancelled"`.
#[derive(Debug, Clone, Deserialize)]
pub struct OperationStatus {
    pub id: String,
    pub status: String,
    pub result: Option<OperationResultDetail>,
    pub error: Option<OperationError>,
}

impl OperationStatus {
    pub fn is_complete(&self) -> bool {
        matches!(self.status.as_str(), "success" | "failed" | "cancelled")
    }

    pub fn txid(&self) -> Option<&str> {
        self.result.as_ref().map(|r| r.txid.as_str())
    }
}

/// One entry from `z_getoperationresult` — same shape as `OperationStatus`
/// but only returned once the operation has finished.
#[derive(Debug, Clone, Deserialize)]
pub struct OperationResult {
    pub id: String,
    pub status: String,
    pub result: Option<OperationResultDetail>,
    pub error: Option<OperationError>,
}

/// One entry from `z_listunspent`. Verified against Zallet `v0.1.0-alpha.3`.
#[derive(Debug, Clone, Deserialize)]
pub struct UnspentNote {
    pub txid: String,
    pub confirmations: u64,
    pub address: Option<String>,
    #[serde(rename = "account_uuid")]
    pub account: String,
    pub value: f64,
    #[serde(rename = "valueZat")]
    pub value_zat: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WalletInfo {
    pub walletversion: u64,
    pub balance: f64,
}

/// A recipient entry for `z_sendmany`.
/// `amount` is in ZEC. `memo` is optional hex-encoded memo field.
#[derive(Debug, Clone, Serialize)]
pub struct Recipient {
    pub address: String,
    pub amount: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

// ── Routing table ─────────────────────────────────────────────────────────────
//
// Maps every method name to the backend the RPC Router forwards it to.
// This lets us populate RpcCall.backend without inspecting the response.

fn routing_table() -> HashMap<&'static str, Backend> {
    HashMap::from([
        // Zebra — stress-test
        ("getblockchaininfo", Backend::Zebra),
        ("getblockcount", Backend::Zebra),
        ("getbestblockhash", Backend::Zebra),
        ("getblock", Backend::Zebra),
        ("getblockhash", Backend::Zebra),
        ("getblockheader", Backend::Zebra),
        ("getblocktemplate", Backend::Zebra),
        ("getrawmempool", Backend::Zebra),
        ("getmempoolinfo", Backend::Zebra),
        ("getrawtransaction", Backend::Zebra),
        ("gettxout", Backend::Zebra),
        ("getaddressbalance", Backend::Zebra),
        ("getaddresstxids", Backend::Zebra),
        ("getaddressutxos", Backend::Zebra),
        ("getpeerinfo", Backend::Zebra),
        ("sendrawtransaction", Backend::Zebra),
        ("submitblock", Backend::Zebra),
        ("z_gettreestate", Backend::Zebra),
        ("z_getsubtreesbyindex", Backend::Zebra),
        // Zebra — smoke-test
        ("generate", Backend::Zebra),
        ("validateaddress", Backend::Zebra),
        ("z_validateaddress", Backend::Zebra),
        ("z_listunifiedreceivers", Backend::Zebra),
        // Zallet — stress-test
        ("z_getnewaccount", Backend::Zallet),
        ("z_getaddressforaccount", Backend::Zallet),
        ("z_listaccounts", Backend::Zallet),
        ("z_getaccount", Backend::Zallet),
        ("listaddresses", Backend::Zallet),
        ("z_gettotalbalance", Backend::Zallet),
        ("z_sendmany", Backend::Zallet),
        ("z_getoperationstatus", Backend::Zallet),
        ("z_getoperationresult", Backend::Zallet),
        ("z_listoperationids", Backend::Zallet),
        ("z_listunspent", Backend::Zallet),
        ("z_listtransactions", Backend::Zallet),
        ("z_getnotescount", Backend::Zallet),
        ("z_viewtransaction", Backend::Zallet),
        ("z_recoveraccounts", Backend::Zallet),
        // Zallet — smoke-test
        ("getwalletinfo", Backend::Zallet),
        ("walletlock", Backend::Zallet),
        ("walletpassphrase", Backend::Zallet),
    ])
}

// ── RpcClient ─────────────────────────────────────────────────────────────────

pub struct RpcClient {
    http: reqwest::Client,
    base_url: String,
    run_id: String,
    metrics: Option<Arc<dyn MetricsRecorder>>,
    routing: HashMap<&'static str, Backend>,
    call_counter: AtomicU64,
}

impl RpcClient {
    pub fn new(
        base_url: impl Into<String>,
        run_id: impl Into<String>,
        metrics: Option<Arc<dyn MetricsRecorder>>,
        timeout: Option<Duration>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout.unwrap_or(Duration::from_secs(30)))
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            base_url: base_url.into(),
            run_id: run_id.into(),
            metrics,
            routing: routing_table(),
            call_counter: AtomicU64::new(0),
        }
    }

    /// Send one JSON-RPC call, parse the result, and record an RpcCall entry.
    ///
    /// This is the single chokepoint all public methods go through — timing,
    /// error classification, and metrics recording all happen here.
    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<T, RpcError> {
        let n = self.call_counter.fetch_add(1, Ordering::Relaxed);
        let call_id = format!("{method}-{n}");
        let backend = self
            .routing
            .get(method)
            .cloned()
            .unwrap_or(Backend::Unknown);
        let request_at = Utc::now();

        // Run the HTTP round-trip inside an async block so we can use `?`
        // for early returns while still recording the RpcCall in all paths.
        let outcome: Result<T, RpcError> = async {
            let resp = self
                .http
                .post(&self.base_url)
                .json(&JsonRpcRequest {
                    method,
                    params,
                    id: n,
                })
                .send()
                .await
                .map_err(|e| RpcError::Transport(e.to_string()))?;

            let rpc_resp = resp
                .json::<JsonRpcResponse<T>>()
                .await
                .map_err(|e| RpcError::Parse(e.to_string()))?;

            if let Some(err) = rpc_resp.error {
                return Err(RpcError::JsonRpc {
                    code: err.code,
                    message: err.message,
                });
            }
            rpc_resp
                .result
                .ok_or_else(|| RpcError::Parse("response had neither result nor error".into()))
        }
        .await;

        let response_at = Utc::now();
        let latency_ms = (response_at - request_at).num_milliseconds().max(0) as u64;

        let (error_code, error_message) = match &outcome {
            Ok(_) => (None, None),
            Err(RpcError::JsonRpc { code, message }) => (Some(*code), Some(message.clone())),
            Err(e) => (None, Some(e.to_string())),
        };

        if let Some(m) = &self.metrics {
            m.record_rpc_call(RpcCall {
                call_id,
                run_id: self.run_id.clone(),
                method: method.to_string(),
                backend,
                params_hash: None,
                request_at,
                response_at: Some(response_at),
                latency_ms: Some(latency_ms),
                success: outcome.is_ok(),
                error_code,
                error_message,
            });
        }

        outcome
    }

    /// Like `call`, but treats a JSON null result as `Ok(None)` instead of an
    /// error. Used for methods like `gettxout` where null is a valid response.
    async fn call_nullable<T: for<'de> Deserialize<'de>>(
        &self,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<Option<T>, RpcError> {
        let n = self.call_counter.fetch_add(1, Ordering::Relaxed);
        let call_id = format!("{method}-{n}");
        let backend = self
            .routing
            .get(method)
            .cloned()
            .unwrap_or(Backend::Unknown);
        let request_at = Utc::now();

        let outcome: Result<Option<T>, RpcError> = async {
            let resp = self
                .http
                .post(&self.base_url)
                .json(&JsonRpcRequest {
                    method,
                    params,
                    id: n,
                })
                .send()
                .await
                .map_err(|e| RpcError::Transport(e.to_string()))?;

            let rpc_resp = resp
                .json::<JsonRpcResponse<T>>()
                .await
                .map_err(|e| RpcError::Parse(e.to_string()))?;

            if let Some(err) = rpc_resp.error {
                return Err(RpcError::JsonRpc {
                    code: err.code,
                    message: err.message,
                });
            }
            Ok(rpc_resp.result)
        }
        .await;

        let response_at = Utc::now();
        let latency_ms = (response_at - request_at).num_milliseconds().max(0) as u64;

        let (error_code, error_message) = match &outcome {
            Ok(_) => (None, None),
            Err(RpcError::JsonRpc { code, message }) => (Some(*code), Some(message.clone())),
            Err(e) => (None, Some(e.to_string())),
        };

        if let Some(m) = &self.metrics {
            m.record_rpc_call(RpcCall {
                call_id,
                run_id: self.run_id.clone(),
                method: method.to_string(),
                backend,
                params_hash: None,
                request_at,
                response_at: Some(response_at),
                latency_ms: Some(latency_ms),
                success: outcome.is_ok(),
                error_code,
                error_message,
            });
        }

        outcome
    }

    // ── Zebra methods ─────────────────────────────────────────────────────────

    pub async fn get_blockchain_info(&self) -> Result<BlockchainInfo, RpcError> {
        self.call("getblockchaininfo", serde_json::json!([])).await
    }

    pub async fn get_block_count(&self) -> Result<u64, RpcError> {
        self.call("getblockcount", serde_json::json!([])).await
    }

    pub async fn get_best_block_hash(&self) -> Result<String, RpcError> {
        self.call("getbestblockhash", serde_json::json!([])).await
    }

    /// Fetch a full block by hash or height. `tx` contains all transaction IDs.
    pub async fn get_block(&self, block_ref: BlockRef<'_>) -> Result<Block, RpcError> {
        // Verbosity 1: return JSON with txids (not full transaction objects).
        self.call("getblock", serde_json::json!([block_ref, 1]))
            .await
    }

    pub async fn get_block_hash(&self, height: u64) -> Result<String, RpcError> {
        self.call("getblockhash", serde_json::json!([height])).await
    }

    pub async fn get_block_header(&self, block_ref: BlockRef<'_>) -> Result<BlockHeader, RpcError> {
        self.call("getblockheader", serde_json::json!([block_ref]))
            .await
    }

    pub async fn get_raw_mempool(&self) -> Result<Vec<String>, RpcError> {
        self.call("getrawmempool", serde_json::json!([])).await
    }

    pub async fn get_mempool_info(&self) -> Result<MempoolInfo, RpcError> {
        self.call("getmempoolinfo", serde_json::json!([])).await
    }

    pub async fn get_raw_transaction(
        &self,
        txid: &str,
        verbose: bool,
    ) -> Result<RawTransaction, RpcError> {
        self.call("getrawtransaction", serde_json::json!([txid, verbose]))
            .await
    }

    /// Returns `None` if the output has already been spent or does not exist.
    pub async fn get_tx_out(&self, txid: &str, index: u32) -> Result<Option<TxOut>, RpcError> {
        self.call_nullable("gettxout", serde_json::json!([txid, index]))
            .await
    }

    pub async fn get_address_balance(
        &self,
        addresses: &[&str],
    ) -> Result<AddressBalance, RpcError> {
        self.call(
            "getaddressbalance",
            serde_json::json!([{ "addresses": addresses }]),
        )
        .await
    }

    pub async fn get_address_txids(
        &self,
        addresses: &[&str],
        range: Option<HeightRange>,
    ) -> Result<Vec<String>, RpcError> {
        let params = match range {
            Some(r) => {
                serde_json::json!([{ "addresses": addresses, "start": r.start, "end": r.end }])
            }
            None => serde_json::json!([{ "addresses": addresses }]),
        };
        self.call("getaddresstxids", params).await
    }

    pub async fn get_address_utxos(&self, addresses: &[&str]) -> Result<Vec<Utxo>, RpcError> {
        self.call(
            "getaddressutxos",
            serde_json::json!([{ "addresses": addresses }]),
        )
        .await
    }

    pub async fn get_peer_info(&self) -> Result<Vec<PeerInfo>, RpcError> {
        self.call("getpeerinfo", serde_json::json!([])).await
    }

    /// Broadcast a signed raw transaction. Returns the txid on success.
    pub async fn send_raw_transaction(&self, tx_hex: &str) -> Result<String, RpcError> {
        self.call("sendrawtransaction", serde_json::json!([tx_hex]))
            .await
    }

    pub async fn validate_address(&self, address: &str) -> Result<AddressValidation, RpcError> {
        self.call("validateaddress", serde_json::json!([address]))
            .await
    }

    /// Mine `num_blocks` blocks immediately. Regtest only.
    pub async fn generate(&self, num_blocks: u32) -> Result<Vec<String>, RpcError> {
        self.call("generate", serde_json::json!([num_blocks])).await
    }

    // ── Zallet methods ────────────────────────────────────────────────────────

    /// Create a new wallet account. `name` is a human-readable label stored in
    /// Zallet. Returns the account UUID used in all subsequent Zallet calls.
    pub async fn z_get_new_account(&self, name: &str) -> Result<AccountInfo, RpcError> {
        self.call("z_getnewaccount", serde_json::json!([name]))
            .await
    }

    /// Derive a Unified Address (deposit address) for an account.
    pub async fn z_get_address_for_account(
        &self,
        account: &str,
    ) -> Result<UnifiedAddress, RpcError> {
        self.call("z_getaddressforaccount", serde_json::json!([account]))
            .await
    }

    pub async fn z_list_accounts(&self) -> Result<Vec<AccountInfo>, RpcError> {
        self.call("z_listaccounts", serde_json::json!([])).await
    }

    pub async fn z_get_account(&self, account: &str) -> Result<AccountInfo, RpcError> {
        self.call("z_getaccount", serde_json::json!([account]))
            .await
    }

    pub async fn list_addresses(&self) -> Result<Vec<AddressEntry>, RpcError> {
        self.call("listaddresses", serde_json::json!([])).await
    }

    pub async fn z_get_total_balance(&self) -> Result<TotalBalance, RpcError> {
        self.call("z_gettotalbalance", serde_json::json!([])).await
    }

    pub async fn z_get_balances(&self) -> Result<Balances, RpcError> {
        self.call("z_getbalances", serde_json::json!([])).await
    }

    /// Send funds to one or more recipients. Fee is always `null` (auto-computed
    /// via ZIP 317 — the simulator must not pre-specify fees).
    /// Returns an operation ID string — poll with `z_get_operation_status`.
    pub async fn z_send_many(
        &self,
        from: &str,
        recipients: &[Recipient],
    ) -> Result<String, RpcError> {
        self.call(
            "z_sendmany",
            serde_json::json!([from, recipients, null, null]),
        )
        .await
    }

    /// Poll the status of one or more async operations.
    /// An operation is done when `OperationStatus::is_complete()` is true.
    pub async fn z_get_operation_status(
        &self,
        op_ids: &[&str],
    ) -> Result<Vec<OperationStatus>, RpcError> {
        self.call("z_getoperationstatus", serde_json::json!([op_ids]))
            .await
    }

    /// Fetch the final result of completed operations. Only call this after
    /// `z_get_operation_status` confirms the operation has finished.
    pub async fn z_get_operation_result(
        &self,
        op_ids: &[&str],
    ) -> Result<Vec<OperationResult>, RpcError> {
        self.call("z_getoperationresult", serde_json::json!([op_ids]))
            .await
    }

    /// List all operation IDs. Pass `status` to filter (e.g. `Some("executing")`).
    pub async fn z_list_operation_ids(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<String>, RpcError> {
        let params = match status {
            Some(s) => serde_json::json!([s]),
            None => serde_json::json!([]),
        };
        self.call("z_listoperationids", params).await
    }

    /// List unspent shielded notes. `min_conf` filters by minimum confirmations;
    /// `max_conf` is optional (pass `None` for no upper bound).
    pub async fn z_list_unspent(
        &self,
        min_conf: u32,
        max_conf: Option<u32>,
    ) -> Result<Vec<UnspentNote>, RpcError> {
        let params = match max_conf {
            Some(max) => serde_json::json!([min_conf, max]),
            None => serde_json::json!([min_conf]),
        };
        self.call("z_listunspent", params).await
    }

    pub async fn get_wallet_info(&self) -> Result<WalletInfo, RpcError> {
        self.call("getwalletinfo", serde_json::json!([])).await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::data_model::MetricSample;

    // Minimal MetricsRecorder that just collects calls into a vec.
    struct MockRecorder {
        calls: Mutex<Vec<RpcCall>>,
    }

    impl MockRecorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(vec![]),
            })
        }

        fn recorded_calls(&self) -> Vec<RpcCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl MetricsRecorder for MockRecorder {
        fn record_rpc_call(&self, call: RpcCall) {
            self.calls.lock().unwrap().push(call);
        }
        fn record_metric(&self, _: MetricSample) {}
    }

    fn client(url: &str) -> RpcClient {
        RpcClient::new(url, "test-run", None, None)
    }

    fn client_with_recorder(url: &str, rec: Arc<MockRecorder>) -> RpcClient {
        RpcClient::new(url, "test-run", Some(rec as Arc<dyn MetricsRecorder>), None)
    }

    // ── get_blockchain_info ───────────────────────────────────────────────────

    #[tokio::test]
    async fn get_blockchain_info_parses_success_response() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "chain": "regtest", "blocks": 42, "headers": 42 },
                "error": null,
                "id": 1
            })))
            .mount(&server)
            .await;

        let info = client(&server.uri()).get_blockchain_info().await.unwrap();
        assert_eq!(info.chain, "regtest");
        assert_eq!(info.blocks, 42);
        assert_eq!(info.headers, 42);
    }

    #[tokio::test]
    async fn get_blockchain_info_records_success_rpc_call() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "chain": "regtest", "blocks": 1, "headers": 1 },
                "error": null,
                "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        client_with_recorder(&server.uri(), rec.clone())
            .get_blockchain_info()
            .await
            .unwrap();

        let calls = rec.recorded_calls();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.method, "getblockchaininfo");
        assert_eq!(call.backend, Backend::Zebra);
        assert_eq!(call.run_id, "test-run");
        assert!(call.success);
        assert!(call.error_code.is_none());
        assert!(call.latency_ms.is_some());
        assert!(call.response_at.is_some());
    }

    #[tokio::test]
    async fn get_blockchain_info_returns_json_rpc_error() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": null,
                "error": { "code": -32601, "message": "Method not found" },
                "id": 1
            })))
            .mount(&server)
            .await;

        let err = client(&server.uri())
            .get_blockchain_info()
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::JsonRpc { code: -32601, .. }));
    }

    #[tokio::test]
    async fn get_blockchain_info_records_json_rpc_error_in_rpc_call() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": null,
                "error": { "code": -32601, "message": "Method not found" },
                "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let _ = client_with_recorder(&server.uri(), rec.clone())
            .get_blockchain_info()
            .await;

        let calls = rec.recorded_calls();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert!(!call.success);
        assert_eq!(call.error_code, Some(-32601));
        assert_eq!(call.error_message.as_deref(), Some("Method not found"));
    }

    #[tokio::test]
    async fn get_blockchain_info_returns_transport_error_on_connection_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let err = client(&format!("http://{addr}"))
            .get_blockchain_info()
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Transport(_)));
    }

    #[tokio::test]
    async fn transport_error_still_records_rpc_call() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let rec = MockRecorder::new();
        let _ = client_with_recorder(&format!("http://{addr}"), rec.clone())
            .get_blockchain_info()
            .await;

        let calls = rec.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].success);
        assert!(calls[0].error_message.is_some());
    }

    // ── routing table ─────────────────────────────────────────────────────────

    #[test]
    fn routing_table_maps_zebra_methods_correctly() {
        let table = routing_table();
        for method in [
            "getblockchaininfo",
            "getblockcount",
            "getrawmempool",
            "sendrawtransaction",
        ] {
            assert_eq!(
                table.get(method),
                Some(&Backend::Zebra),
                "expected Zebra for {method}"
            );
        }
    }

    #[test]
    fn routing_table_maps_zallet_methods_correctly() {
        let table = routing_table();
        for method in [
            "z_getnewaccount",
            "z_sendmany",
            "z_getoperationstatus",
            "z_listunspent",
        ] {
            assert_eq!(
                table.get(method),
                Some(&Backend::Zallet),
                "expected Zallet for {method}"
            );
        }
    }

    #[test]
    fn unknown_method_falls_back_to_backend_unknown() {
        let client = RpcClient::new("http://127.0.0.1:8181", "r", None, None);
        let backend = client
            .routing
            .get("made_up_method")
            .cloned()
            .unwrap_or(Backend::Unknown);
        assert_eq!(backend, Backend::Unknown);
    }

    // ── call_id uniqueness ────────────────────────────────────────────────────

    #[test]
    fn call_counter_increments_per_call() {
        let client = RpcClient::new("http://127.0.0.1:8181", "r", None, None);
        let a = client.call_counter.fetch_add(1, Ordering::Relaxed);
        let b = client.call_counter.fetch_add(1, Ordering::Relaxed);
        assert_ne!(a, b);
    }

    // ── Zebra method response parsing ─────────────────────────────────────────

    #[tokio::test]
    async fn get_block_count_returns_height() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": 100, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;
        assert_eq!(client(&server.uri()).get_block_count().await.unwrap(), 100);
    }

    #[tokio::test]
    async fn get_block_parses_hash_and_tx_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "hash": "abc123",
                    "height": 50,
                    "confirmations": 10,
                    "time": 1_700_000_000u64,
                    "tx": ["txid1", "txid2"]
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let block = client(&server.uri())
            .get_block(BlockRef::Height(50))
            .await
            .unwrap();
        assert_eq!(block.hash, "abc123");
        assert_eq!(block.height, 50);
        assert_eq!(block.tx, vec!["txid1", "txid2"]);
    }

    #[tokio::test]
    async fn get_mempool_info_parses_size_and_bytes() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "size": 42, "bytes": 98_000 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let info = client(&server.uri()).get_mempool_info().await.unwrap();
        assert_eq!(info.size, 42);
        assert_eq!(info.bytes, 98_000);
    }

    #[tokio::test]
    async fn get_raw_transaction_parses_confirmed_tx() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "abc", "hex": "deadbeef", "confirmations": 6 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let tx = client(&server.uri())
            .get_raw_transaction("abc", true)
            .await
            .unwrap();
        assert_eq!(tx.txid, "abc");
        assert_eq!(tx.confirmations, Some(6));
    }

    #[tokio::test]
    async fn get_raw_transaction_handles_mempool_tx_with_no_confirmations() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "abc", "hex": "deadbeef" },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let tx = client(&server.uri())
            .get_raw_transaction("abc", true)
            .await
            .unwrap();
        assert_eq!(tx.confirmations, None);
    }

    #[tokio::test]
    async fn get_tx_out_returns_some_when_utxo_exists() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "confirmations": 3, "value": 0.5 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let out = client(&server.uri()).get_tx_out("abc", 0).await.unwrap();
        assert!(out.is_some());
        assert_eq!(out.unwrap().confirmations, 3);
    }

    #[tokio::test]
    async fn get_tx_out_returns_none_when_utxo_spent() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": null, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;
        let out = client(&server.uri()).get_tx_out("abc", 0).await.unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn get_address_balance_parses_zatoshi_amounts() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "balance": 100_000_000i64, "received": 200_000_000i64 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let bal = client(&server.uri())
            .get_address_balance(&["t1abc"])
            .await
            .unwrap();
        assert_eq!(bal.balance, 100_000_000);
        assert_eq!(bal.received, 200_000_000);
    }

    #[tokio::test]
    async fn get_address_txids_returns_txid_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["txid1", "txid2"],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let txids = client(&server.uri())
            .get_address_txids(&["t1abc"], None)
            .await
            .unwrap();
        assert_eq!(txids, vec!["txid1", "txid2"]);
    }

    #[tokio::test]
    async fn get_address_txids_with_range_includes_height_params() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        // Capture the request body so we can assert start/end are included.
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(serde_json::json!({
                "params": [{ "addresses": ["t1abc"], "start": 10, "end": 20 }]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [], "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        client(&server.uri())
            .get_address_txids(&["t1abc"], Some(HeightRange { start: 10, end: 20 }))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_address_utxos_parses_utxo_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "address": "t1abc",
                    "txid": "deadbeef",
                    "outputIndex": 0,
                    "satoshis": 50_000_000u64,
                    "height": 100
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let utxos = client(&server.uri())
            .get_address_utxos(&["t1abc"])
            .await
            .unwrap();
        assert_eq!(utxos.len(), 1);
        assert_eq!(utxos[0].satoshis, 50_000_000);
        assert_eq!(utxos[0].output_index, 0);
    }

    #[tokio::test]
    async fn validate_address_parses_valid_address() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "isvalid": true, "address": "t1abc" },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let v = client(&server.uri())
            .validate_address("t1abc")
            .await
            .unwrap();
        assert!(v.isvalid);
        assert_eq!(v.address.as_deref(), Some("t1abc"));
    }

    #[tokio::test]
    async fn validate_address_parses_invalid_address() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "isvalid": false },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let v = client(&server.uri()).validate_address("bad").await.unwrap();
        assert!(!v.isvalid);
        assert!(v.address.is_none());
    }

    // ── Zallet method response parsing ────────────────────────────────────────

    #[tokio::test]
    async fn z_get_new_account_returns_account_uuid() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "account": "uuid-1234", "name": null },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let info = client(&server.uri())
            .z_get_new_account("test-account")
            .await
            .unwrap();
        assert_eq!(info.account, "uuid-1234");
    }

    #[tokio::test]
    async fn z_get_address_for_account_returns_unified_address() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "account": "uuid-1234",
                    "address": "u1depositaddress",
                    "receiver_types": ["orchard", "sapling", "p2pkh"]
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let ua = client(&server.uri())
            .z_get_address_for_account("uuid-1234")
            .await
            .unwrap();
        assert_eq!(ua.address, "u1depositaddress");
        assert_eq!(ua.account, "uuid-1234");
        assert!(ua.receiver_types.contains(&"orchard".to_string()));
    }

    #[tokio::test]
    async fn z_list_accounts_returns_account_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [
                    { "account": "uuid-1", "name": "Alice" },
                    { "account": "uuid-2", "name": null }
                ],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let accounts = client(&server.uri()).z_list_accounts().await.unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].account, "uuid-1");
        assert_eq!(accounts[0].name.as_deref(), Some("Alice"));
        assert!(accounts[1].name.is_none());
    }

    #[tokio::test]
    async fn z_get_balances_parses_per_pool_zatoshi_amounts() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "transparent": 100_000_000u64, "sapling": 0u64, "orchard": 50_000_000u64 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let bal = client(&server.uri()).z_get_balances().await.unwrap();
        assert_eq!(bal.transparent, 100_000_000);
        assert_eq!(bal.orchard, 50_000_000);
        assert_eq!(bal.sapling, 0);
    }

    #[tokio::test]
    async fn z_get_total_balance_parses_zec_strings() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "transparent": "0.50000000",
                    "private": "1.00000000",
                    "total": "1.50000000"
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let bal = client(&server.uri()).z_get_total_balance().await.unwrap();
        assert_eq!(bal.transparent, "0.50000000");
        assert_eq!(bal.total, "1.50000000");
    }

    #[tokio::test]
    async fn z_send_many_returns_operation_id() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-abcdef",
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let recipients = vec![Recipient {
            address: "u1dest".into(),
            amount: 0.5,
            memo: None,
        }];
        let op_id = client(&server.uri())
            .z_send_many("uuid-1234", &recipients)
            .await
            .unwrap();
        assert_eq!(op_id, "opid-abcdef");
    }

    #[tokio::test]
    async fn z_send_many_sends_null_fee() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        // Assert that the params contain `null` for fee (3rd and 4th positional args).
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(serde_json::json!({
                "params": ["uuid-1234", [{"address": "u1dest", "amount": 0.5}], null, null]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "opid-xyz", "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let recipients = vec![Recipient {
            address: "u1dest".into(),
            amount: 0.5,
            memo: None,
        }];
        client(&server.uri())
            .z_send_many("uuid-1234", &recipients)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn z_get_operation_status_parses_executing_operation() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{ "id": "opid-1", "status": "executing", "result": null, "error": null }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let ops = client(&server.uri())
            .z_get_operation_status(&["opid-1"])
            .await
            .unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].status, "executing");
        assert!(!ops[0].is_complete());
        assert!(ops[0].txid().is_none());
    }

    #[tokio::test]
    async fn z_get_operation_status_parses_successful_operation() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-1",
                    "status": "success",
                    "result": { "txid": "finaltxid" },
                    "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let ops = client(&server.uri())
            .z_get_operation_status(&["opid-1"])
            .await
            .unwrap();
        assert!(ops[0].is_complete());
        assert_eq!(ops[0].txid(), Some("finaltxid"));
    }

    #[tokio::test]
    async fn z_list_operation_ids_without_filter() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["opid-1", "opid-2"],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let ids = client(&server.uri())
            .z_list_operation_ids(None)
            .await
            .unwrap();
        assert_eq!(ids, vec!["opid-1", "opid-2"]);
    }

    #[tokio::test]
    async fn z_list_operation_ids_with_status_filter() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(serde_json::json!({
                "params": ["executing"]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["opid-1"],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let ids = client(&server.uri())
            .z_list_operation_ids(Some("executing"))
            .await
            .unwrap();
        assert_eq!(ids, vec!["opid-1"]);
    }

    #[tokio::test]
    async fn z_list_unspent_parses_note_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "txid": "abc",
                    "confirmations": 6,
                    "account_uuid": "uuid-1",
                    "address": "u1addr",
                    "value": 0.5,
                    "valueZat": 50_000_000u64
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let notes = client(&server.uri()).z_list_unspent(1, None).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].txid, "abc");
        assert_eq!(notes[0].account, "uuid-1");
        assert_eq!(notes[0].address.as_deref(), Some("u1addr"));
        assert_eq!(notes[0].value_zat, 50_000_000);
    }

    #[tokio::test]
    async fn z_list_unspent_address_may_be_null() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "txid": "def",
                    "confirmations": 1,
                    "account_uuid": "uuid-2",
                    "address": null,
                    "value": 1.0,
                    "valueZat": 100_000_000u64
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let notes = client(&server.uri()).z_list_unspent(1, None).await.unwrap();
        assert!(notes[0].address.is_none());
    }

    #[tokio::test]
    async fn z_get_operation_result_parses_successful_operation() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "opid-1",
                    "status": "success",
                    "result": { "txid": "finaltxid" },
                    "error": null
                }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let ops = client(&server.uri())
            .z_get_operation_result(&["opid-1"])
            .await
            .unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].status, "success");
        assert_eq!(
            ops[0].result.as_ref().map(|r| r.txid.as_str()),
            Some("finaltxid")
        );
    }

    #[tokio::test]
    async fn get_tx_out_null_result_records_success_in_rpc_call() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": null, "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let rec = MockRecorder::new();
        let out = client_with_recorder(&server.uri(), rec.clone())
            .get_tx_out("abc", 0)
            .await
            .unwrap();
        assert!(out.is_none());
        let calls = rec.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].success);
        assert!(calls[0].error_code.is_none());
    }

    #[tokio::test]
    async fn generate_returns_block_hashes() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": ["hash1", "hash2", "hash3"],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let hashes = client(&server.uri()).generate(3).await.unwrap();
        assert_eq!(hashes.len(), 3);
        assert_eq!(hashes[0], "hash1");
    }
}
