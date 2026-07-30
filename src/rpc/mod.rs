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
    #[serde(rename = "account_uuid")]
    pub account: String,
    pub name: Option<String>,
    /// Already-derived addresses (`z_listaccounts` includes them; the
    /// `z_getnewaccount` response does not). Use these instead of deriving:
    /// account creation always generates the diversifier-0 address with every
    /// receiver type, and every `z_getaddressforaccount` call derives a NEW
    /// address at the next Sapling-valid index — on an account with no funded
    /// address the transparent gap window is indices 0..9, so a handful of
    /// "get the address" calls exhausts it (`ReachedGapLimit` at index 10).
    #[serde(default)]
    pub addresses: Vec<AccountAddress>,
}

impl AccountInfo {
    /// The account's primary address: the existing UA with the lowest
    /// diversifier index (the one created with the account). Empty only if
    /// this `AccountInfo` came from a call that omits addresses.
    pub fn primary_address(&self) -> Option<&str> {
        self.addresses
            .iter()
            .min_by_key(|a| a.diversifier_index)
            .map(|a| a.ua.as_str())
    }
}

/// One derived address inside `z_listaccounts`' `addresses` array.
#[derive(Debug, Clone, Deserialize)]
pub struct AccountAddress {
    pub diversifier_index: u64,
    pub ua: String,
}

/// Returned by `z_getaddressforaccount`.
/// `address` is the Unified Address used as the deposit address.
#[derive(Debug, Clone, Deserialize)]
pub struct UnifiedAddress {
    #[serde(rename = "account_uuid")]
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

/// One entry from `z_listunspent`. Verified against Zallet `v0.1.0-alpha.3`
/// and `v0.1.0-beta.1`.
#[derive(Debug, Clone, Deserialize)]
pub struct UnspentNote {
    pub txid: String,
    /// Which pool holds the output: `"transparent"`, `"sapling"`, or
    /// `"orchard"`. Optional defensively — coinbase maturity applies only to
    /// transparent outputs, so a missing pool is treated as transparent (the
    /// stricter reading) by consumers.
    #[serde(default)]
    pub pool: Option<String>,
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

/// Returned by `z_listunifiedreceivers`: the individual receivers inside a
/// Unified Address. Each present pool maps to that pool's standalone address
/// encoding (the `orchard` entry is itself a single-receiver UA).
#[derive(Debug, Clone, Deserialize)]
pub struct UnifiedReceivers {
    pub orchard: Option<String>,
    pub sapling: Option<String>,
    pub p2pkh: Option<String>,
    pub p2sh: Option<String>,
}

/// Returned by `z_shieldcoinbase`. `opid` is polled like a `z_sendmany`
/// operation; the counts describe what the sweep selected.
#[derive(Debug, Clone, Deserialize)]
pub struct ShieldCoinbaseResult {
    #[serde(rename = "remainingUTXOs")]
    pub remaining_utxos: u64,
    #[serde(rename = "remainingValue")]
    pub remaining_value: f64,
    #[serde(rename = "shieldingUTXOs")]
    pub shielding_utxos: u64,
    #[serde(rename = "shieldingValue")]
    pub shielding_value: f64,
    pub opid: String,
}

// ── Additional stress-test response types ─────────────────────────────────────
//
// Types for the stress-test methods added per the Foundation's confirmed list.
// Each declares only the fields the simulator reads. Parameter shapes for a few
// of these (noted on the methods below) are provisional and will be verified
// against the live stack / OpenRPC discovery during integration testing.

/// Returned by `getbestblockheightandhash` (Zebra-specific combined tip call).
#[derive(Debug, Clone, Deserialize)]
pub struct BestBlockHeightAndHash {
    pub height: u64,
    pub hash: String,
}

/// Returned by `getblocktemplate`. Only the fields the simulator uses are declared.
#[derive(Debug, Clone, Deserialize)]
pub struct BlockTemplate {
    pub height: u64,
    pub previousblockhash: String,
}

/// Returned by `z_gettreestate` (Zebra). Sapling and Orchard tree state at a block.
#[derive(Debug, Clone, Deserialize)]
pub struct TreeState {
    pub height: u64,
    pub hash: String,
}

/// One subtree entry from `z_getsubtreesbyindex`.
#[derive(Debug, Clone, Deserialize)]
pub struct Subtree {
    pub root: String,
    pub end_height: u64,
}

/// Returned by `z_getsubtreesbyindex` (Zebra). Note-commitment subtree roots.
#[derive(Debug, Clone, Deserialize)]
pub struct Subtrees {
    pub pool: String,
    pub start_index: u64,
    pub subtrees: Vec<Subtree>,
}

/// Returned by `z_getnotescount` (Zallet). Unspent note counts per shielded pool.
#[derive(Debug, Clone, Deserialize)]
pub struct NotesCount {
    #[serde(default)]
    pub sapling: u64,
    #[serde(default)]
    pub orchard: u64,
}

/// One entry from `z_listtransactions` (Zallet). Minimal projection.
#[derive(Debug, Clone, Deserialize)]
pub struct WalletTransaction {
    pub txid: String,
    #[serde(rename = "account_uuid", default)]
    pub account: Option<String>,
}

/// Returned by `z_viewtransaction` (Zallet). Minimal projection.
#[derive(Debug, Clone, Deserialize)]
pub struct ViewedTransaction {
    pub txid: String,
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
        ("getbestblockheightandhash", Backend::Zebra),
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
        // Zebra — regtest-control
        ("generate", Backend::Zebra),
        ("invalidateblock", Backend::Zebra),
        ("reconsiderblock", Backend::Zebra),
        // Zebra — smoke / compatibility
        ("validateaddress", Backend::Zebra),
        ("z_validateaddress", Backend::Zebra),
        // Zallet — stress-test
        // z_listunifiedreceivers is served by Zallet (it appears in Zallet's
        // own method list); it was previously mislabelled Backend::Zebra here,
        // which skewed per-backend metrics attribution.
        ("z_listunifiedreceivers", Backend::Zallet),
        ("z_shieldcoinbase", Backend::Zallet),
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
    /// Optional HTTP Basic Auth credentials. The Z3 regtest RPC Router requires
    /// these (default `zebra` / `zebra`); mainnet/testnet use cookie auth instead.
    auth: Option<(String, String)>,
    /// When set, every RpcCall is tagged with this backend instead of consulting
    /// the routing table. Used for clients pointed directly at Zaino's JSON-RPC
    /// mirror, where the router's method→backend mapping does not apply.
    backend_override: Option<Backend>,
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
            auth: None,
            backend_override: None,
        }
    }

    /// Build a client pointed at Zaino's zcashd-style JSON-RPC mirror (regtest
    /// host port `:28237`). Every call it records is tagged `Backend::Zaino`, so
    /// Zaino's latency is attributed to Zaino rather than folded into Zallet. The
    /// typed Zebra-style read methods (`get_blockchain_info`, `get_raw_transaction`,
    /// …) can be issued against it to exercise the mirror.
    pub fn for_zaino_mirror(
        base_url: impl Into<String>,
        run_id: impl Into<String>,
        metrics: Option<Arc<dyn MetricsRecorder>>,
        timeout: Option<Duration>,
    ) -> Self {
        Self::new(base_url, run_id, metrics, timeout).with_backend_override(Backend::Zaino)
    }

    /// Attach HTTP Basic Auth credentials, sent on every request. Required by the
    /// Z3 regtest RPC Router (default `zebra` / `zebra`).
    pub fn with_basic_auth(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.auth = Some((username.into(), password.into()));
        self
    }

    /// Tag every recorded call with a fixed backend instead of consulting the
    /// routing table. Used for the Zaino JSON-RPC mirror client.
    pub fn with_backend_override(mut self, backend: Backend) -> Self {
        self.backend_override = Some(backend);
        self
    }

    /// Resolve the backend for a method: the override if set, else the routing table.
    fn backend_for(&self, method: &str) -> Backend {
        self.backend_override.clone().unwrap_or_else(|| {
            self.routing
                .get(method)
                .cloned()
                .unwrap_or(Backend::Unknown)
        })
    }

    /// Build a POST request to the RPC endpoint, applying Basic Auth if configured.
    fn request(&self) -> reqwest::RequestBuilder {
        let builder = self.http.post(&self.base_url);
        match &self.auth {
            Some((user, pass)) => builder.basic_auth(user, Some(pass)),
            None => builder,
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
        let backend = self.backend_for(method);
        let request_at = Utc::now();

        // Run the HTTP round-trip inside an async block so we can use `?`
        // for early returns while still recording the RpcCall in all paths.
        let outcome: Result<T, RpcError> = async {
            let resp = self
                .request()
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
        let backend = self.backend_for(method);
        let request_at = Utc::now();

        let outcome: Result<Option<T>, RpcError> = async {
            let resp = self
                .request()
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
        // `verbose` is serialized as a NUMBER (0/1): Zebra ≥ 6.0.0 declares the
        // parameter as a number and rejects a JSON boolean with -32602 "Invalid
        // params" (v5.x tolerated the boolean). zcashd's own signature is also
        // numeric.
        let verbose = u8::from(verbose);
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

    /// Mark a block (and its descendants) as invalid, rolling the chain back to
    /// its parent. Regtest chain-reorganization control. Returns `()` on success.
    pub async fn invalidate_block(&self, block_hash: &str) -> Result<(), RpcError> {
        self.call_nullable::<serde_json::Value>("invalidateblock", serde_json::json!([block_hash]))
            .await
            .map(|_| ())
    }

    /// Undo a previous `invalidateblock`, restoring the block for reconsideration.
    /// Regtest chain-reorganization control. Returns `()` on success.
    pub async fn reconsider_block(&self, block_hash: &str) -> Result<(), RpcError> {
        self.call_nullable::<serde_json::Value>("reconsiderblock", serde_json::json!([block_hash]))
            .await
            .map(|_| ())
    }

    /// Chain tip height and hash in a single call (Zebra-specific).
    pub async fn get_best_block_height_and_hash(&self) -> Result<BestBlockHeightAndHash, RpcError> {
        self.call("getbestblockheightandhash", serde_json::json!([]))
            .await
    }

    /// Fetch a block template. Used to drive regtest block production.
    pub async fn get_block_template(&self) -> Result<BlockTemplate, RpcError> {
        self.call("getblocktemplate", serde_json::json!([])).await
    }

    /// Submit a mined block. Returns `None` when the block is accepted, or
    /// `Some(reason)` when the node rejects it (e.g. `"duplicate"`, `"rejected"`).
    pub async fn submit_block(&self, block_hex: &str) -> Result<Option<String>, RpcError> {
        self.call_nullable("submitblock", serde_json::json!([block_hex]))
            .await
    }

    /// Sapling and Orchard commitment tree state at a block (hash or height).
    pub async fn z_get_treestate(&self, block_ref: BlockRef<'_>) -> Result<TreeState, RpcError> {
        self.call("z_gettreestate", serde_json::json!([block_ref]))
            .await
    }

    /// Note-commitment subtree roots. `pool` is `"sapling"` or `"orchard"`;
    /// `limit` is optional (pass `None` for the server default).
    pub async fn z_get_subtrees_by_index(
        &self,
        pool: &str,
        start_index: u64,
        limit: Option<u64>,
    ) -> Result<Subtrees, RpcError> {
        let params = match limit {
            Some(l) => serde_json::json!([pool, start_index, l]),
            None => serde_json::json!([pool, start_index]),
        };
        self.call("z_getsubtreesbyindex", params).await
    }

    // ── Zallet methods ────────────────────────────────────────────────────────

    /// Create a new wallet account. `name` is a human-readable label stored in
    /// Zallet. Returns the account UUID used in all subsequent Zallet calls.
    pub async fn z_get_new_account(&self, name: &str) -> Result<AccountInfo, RpcError> {
        self.call("z_getnewaccount", serde_json::json!([name]))
            .await
    }

    /// Derive a NEW Unified Address for an account with the given receiver types.
    ///
    /// This method always *derives* — there is no "get the existing address" mode.
    /// To reuse an address that already exists (the usual need), read it from
    /// [`Self::z_list_accounts`] via [`AccountInfo::primary_address`] instead;
    /// account creation always generates the diversifier-0 address with every
    /// receiver type.
    ///
    /// The transparent gap limit is 10 consecutive *unfunded* external derivations
    /// per `(account, key scope)` — not wallet-global — sliding forward whenever a
    /// derived address receives a mined output. The trap (measured on beta.1, and
    /// the resolution of the "index 10 on fresh accounts" failure in
    /// `experiments/runs/20260630T131145Z-smoke/`): each call without an explicit
    /// `diversifier_index` derives at the next *Sapling-valid* index, which
    /// advances in jumps, so an unfunded account's 0..9 transparent window is
    /// exhausted within a few calls when `receiver_types` includes `p2pkh`.
    /// Failures interleave per-account (one account erring while others derive
    /// fine), confirming the per-account scope. See
    /// `docs/zallet-transparent-gap-limit.md`.
    ///
    /// Pinning `diversifier_index` is not a way to fetch an existing address
    /// either: index 0 is pre-generated with *all* receiver types (a narrower
    /// request errors with "already generated with different receiver types"),
    /// and only ~half of all indices have a valid Sapling diversifier.
    pub async fn z_get_address_for_account(
        &self,
        account: &str,
        receiver_types: &[&str],
        diversifier_index: Option<u64>,
    ) -> Result<UnifiedAddress, RpcError> {
        let params = match diversifier_index {
            Some(idx) => serde_json::json!([account, receiver_types, idx]),
            None => serde_json::json!([account, receiver_types]),
        };
        self.call("z_getaddressforaccount", params).await
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

    /// `include_watchonly` must be `true` — Zallet alpha requires it (temporary restriction).
    pub async fn z_get_total_balance(&self) -> Result<TotalBalance, RpcError> {
        self.call("z_gettotalbalance", serde_json::json!([null, true]))
            .await
    }

    /// Send funds to one or more recipients. Fee is always `null` (auto-computed
    /// via ZIP 317 — the simulator must not pre-specify fees).
    /// Returns an operation ID string — poll with `z_get_operation_status`.
    ///
    /// Semantics of `from` (measured on Zallet v0.1.0-beta.1):
    /// - a Unified Address draws the account's SHIELDED funds only (zallet#644,
    ///   by design) — use it for ZToZ/ZToT sends;
    /// - a bare t-addr draws that address's own transparent UTXOs — the only
    ///   form that works for TToT/TToZ sends;
    /// - `ANY_TADDR` requires `features.legacy_pool_seed_fingerprint`.
    ///
    /// Inputs (notes and transparent UTXOs alike) need ~10 confirmations before
    /// the proposal engine selects them; younger funds yield error -4
    /// "Insufficient balance (have 0, ...)" even when the balance shows them.
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

    /// `z_send_many` with an explicit ZIP 315 privacy policy (e.g.
    /// `"AllowRevealedRecipients"` for a shielded source paying transparent
    /// receivers, `"AllowFullyTransparent"` for t-to-t). The plain
    /// [`Self::z_send_many`] leaves the policy at the server default
    /// (`FullPrivacy`), which rejects any transaction that reveals value.
    pub async fn z_send_many_with_policy(
        &self,
        from: &str,
        recipients: &[Recipient],
        privacy_policy: &str,
    ) -> Result<String, RpcError> {
        self.call(
            "z_sendmany",
            serde_json::json!([from, recipients, null, null, privacy_policy]),
        )
        .await
    }

    /// Sweep mature transparent coinbase into the shielded pool. Zallet's
    /// proposal engine refuses to spend coinbase UTXOs to transparent outputs
    /// even on regtest (where consensus allows it), so this is the only exit
    /// for transparent coinbase. Available from Zallet v0.1.0-alpha.4.
    ///
    /// `from` must be an account UUID or a wallet-owned t-addr; Zallet rejects
    /// zcashd's `"*"` wildcard. `to` is a shielded address (a UA works).
    /// Poll the returned `opid` like a `z_sendmany` operation.
    pub async fn z_shield_coinbase(
        &self,
        from: &str,
        to: &str,
    ) -> Result<ShieldCoinbaseResult, RpcError> {
        self.call("z_shieldcoinbase", serde_json::json!([from, to]))
            .await
    }

    /// Split a Unified Address into its per-pool receivers. Needed to extract
    /// the transparent receiver for TToT/TToZ recipients (paying the UA itself
    /// would settle shielded) and the Orchard receiver for Zebra's
    /// `mining.miner_address`.
    pub async fn z_list_unified_receivers(
        &self,
        unified_address: &str,
    ) -> Result<UnifiedReceivers, RpcError> {
        self.call(
            "z_listunifiedreceivers",
            serde_json::json!([unified_address]),
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

    /// Count of unspent notes per shielded pool — a shielded state-size signal.
    pub async fn z_get_notes_count(&self) -> Result<NotesCount, RpcError> {
        self.call("z_getnotescount", serde_json::json!([])).await
    }

    /// List wallet transactions.
    ///
    /// NOTE: the exact parameter signature (account / count / from filters) is
    /// provisional and must be confirmed against the live stack / `rpc.discover`.
    /// Called here with no filter.
    pub async fn z_list_transactions(&self) -> Result<Vec<WalletTransaction>, RpcError> {
        self.call("z_listtransactions", serde_json::json!([])).await
    }

    /// Decode and return full details of a wallet transaction.
    pub async fn z_view_transaction(&self, txid: &str) -> Result<ViewedTransaction, RpcError> {
        self.call("z_viewtransaction", serde_json::json!([txid]))
            .await
    }

    /// Recover accounts from the wallet seed — used during wallet-reset scenarios.
    ///
    /// NOTE: the exact parameter signature is provisional and must be confirmed
    /// against the live stack / `rpc.discover`. Called here with no arguments;
    /// returns the recovered accounts.
    pub async fn z_recover_accounts(&self) -> Result<Vec<AccountInfo>, RpcError> {
        self.call("z_recoveraccounts", serde_json::json!([])).await
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
                "result": { "account_uuid": "uuid-1234", "name": null },
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
                    "account_uuid": "uuid-1234",
                    "address": "u1depositaddress",
                    "receiver_types": ["orchard", "sapling", "p2pkh"]
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let ua = client(&server.uri())
            .z_get_address_for_account("uuid-1234", &["orchard"], None)
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
                    { "account_uuid": "uuid-1", "name": "Alice" },
                    { "account_uuid": "uuid-2", "name": null }
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
    async fn z_get_total_balance_sends_include_watchonly_true() {
        // Zallet alpha requires include_watchonly=true; verify the client sends it.
        // The mock only matches when params are exactly [null, true] — a reverted
        // call site (empty params) would get no match and return a transport error,
        // causing the test to fail.
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(serde_json::json!({
                "method": "z_gettotalbalance",
                "params": [null, true]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "transparent": "0.00000000",
                    "private": "0.00000000",
                    "total": "0.00000000"
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        // Would fail if params were wrong (no matching mock → error).
        client(&server.uri()).z_get_total_balance().await.unwrap();
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

    // ── Added stress-test methods ─────────────────────────────────────────────

    #[test]
    fn routing_table_maps_getbestblockheightandhash_to_zebra() {
        assert_eq!(
            routing_table().get("getbestblockheightandhash"),
            Some(&Backend::Zebra)
        );
    }

    #[tokio::test]
    async fn get_best_block_height_and_hash_parses() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "height": 100, "hash": "abc" }, "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let r = client(&server.uri())
            .get_best_block_height_and_hash()
            .await
            .unwrap();
        assert_eq!(r.height, 100);
        assert_eq!(r.hash, "abc");
    }

    #[tokio::test]
    async fn get_block_template_parses_height_and_prev_hash() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "height": 5, "previousblockhash": "prev", "extra": "ignored" },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let t = client(&server.uri()).get_block_template().await.unwrap();
        assert_eq!(t.height, 5);
        assert_eq!(t.previousblockhash, "prev");
    }

    #[tokio::test]
    async fn submit_block_returns_none_on_acceptance() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": null, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;
        let r = client(&server.uri())
            .submit_block("deadbeef")
            .await
            .unwrap();
        assert!(r.is_none());
    }

    #[tokio::test]
    async fn submit_block_returns_reason_on_rejection() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": "duplicate", "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let r = client(&server.uri())
            .submit_block("deadbeef")
            .await
            .unwrap();
        assert_eq!(r.as_deref(), Some("duplicate"));
    }

    #[tokio::test]
    async fn z_get_treestate_parses() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "height": 9, "hash": "h9", "sapling": {}, "orchard": {} },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let ts = client(&server.uri())
            .z_get_treestate(BlockRef::Height(9))
            .await
            .unwrap();
        assert_eq!(ts.height, 9);
        assert_eq!(ts.hash, "h9");
    }

    #[tokio::test]
    async fn z_get_subtrees_by_index_parses_and_sends_limit() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(serde_json::json!({
                "params": ["orchard", 0, 10]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "pool": "orchard",
                    "start_index": 0,
                    "subtrees": [{ "root": "r1", "end_height": 42 }]
                },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let s = client(&server.uri())
            .z_get_subtrees_by_index("orchard", 0, Some(10))
            .await
            .unwrap();
        assert_eq!(s.pool, "orchard");
        assert_eq!(s.subtrees.len(), 1);
        assert_eq!(s.subtrees[0].end_height, 42);
    }

    #[tokio::test]
    async fn z_get_notes_count_parses_pools() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "sapling": 3, "orchard": 7 }, "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let n = client(&server.uri()).z_get_notes_count().await.unwrap();
        assert_eq!(n.sapling, 3);
        assert_eq!(n.orchard, 7);
    }

    #[tokio::test]
    async fn z_list_transactions_parses_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [
                    { "txid": "t1", "account_uuid": "uuid-1" },
                    { "txid": "t2" }
                ],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let txs = client(&server.uri()).z_list_transactions().await.unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].txid, "t1");
        assert_eq!(txs[0].account.as_deref(), Some("uuid-1"));
        assert!(txs[1].account.is_none());
    }

    #[tokio::test]
    async fn z_view_transaction_parses() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "txid": "vt1", "spends": [], "outputs": [] },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let v = client(&server.uri())
            .z_view_transaction("vt1")
            .await
            .unwrap();
        assert_eq!(v.txid, "vt1");
    }

    #[tokio::test]
    async fn with_basic_auth_sends_authorization_header() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        // base64("zebra:zebra") == "emVicmE6emVicmE=". The mock only matches when
        // the Authorization header is present, so a missing header fails the call.
        Mock::given(matchers::method("POST"))
            .and(matchers::header("Authorization", "Basic emVicmE6emVicmE="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "chain": "regtest", "blocks": 1, "headers": 1 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let client =
            RpcClient::new(&server.uri(), "test-run", None, None).with_basic_auth("zebra", "zebra");
        // Succeeds only if the Authorization header was sent.
        client.get_blockchain_info().await.unwrap();
    }

    #[tokio::test]
    async fn zaino_mirror_client_tags_calls_with_zaino_backend() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": { "chain": "regtest", "blocks": 1, "headers": 1 },
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;

        let rec = MockRecorder::new();
        let client = RpcClient::for_zaino_mirror(
            &server.uri(),
            "test-run",
            Some(rec.clone() as Arc<dyn MetricsRecorder>),
            None,
        );
        // getblockchaininfo routes to Zebra via the table, but the Zaino mirror
        // override must win, attributing the call to Zaino.
        client.get_blockchain_info().await.unwrap();

        let calls = rec.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].backend, Backend::Zaino);
    }

    #[test]
    fn routing_table_maps_regtest_control_methods_to_zebra() {
        let table = routing_table();
        for method in ["generate", "invalidateblock", "reconsiderblock"] {
            assert_eq!(table.get(method), Some(&Backend::Zebra), "for {method}");
        }
    }

    #[tokio::test]
    async fn invalidate_block_succeeds_on_null_result() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(serde_json::json!({
                "method": "invalidateblock", "params": ["blockhash"]
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": null, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;
        client(&server.uri())
            .invalidate_block("blockhash")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reconsider_block_succeeds_on_null_result() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::body_partial_json(serde_json::json!({
                "method": "reconsiderblock", "params": ["blockhash"]
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "result": null, "error": null, "id": 1 })),
            )
            .mount(&server)
            .await;
        client(&server.uri())
            .reconsider_block("blockhash")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn invalidate_block_propagates_json_rpc_error() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": null,
                "error": { "code": -5, "message": "Block not found" },
                "id": 1
            })))
            .mount(&server)
            .await;
        let err = client(&server.uri())
            .invalidate_block("nope")
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::JsonRpc { code: -5, .. }));
    }

    #[tokio::test]
    async fn z_recover_accounts_parses_account_list() {
        use wiremock::{matchers, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{ "account_uuid": "uuid-r", "name": "recovered" }],
                "error": null, "id": 1
            })))
            .mount(&server)
            .await;
        let accts = client(&server.uri()).z_recover_accounts().await.unwrap();
        assert_eq!(accts.len(), 1);
        assert_eq!(accts[0].account, "uuid-r");
    }
}
