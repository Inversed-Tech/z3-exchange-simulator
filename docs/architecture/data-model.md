# Data Model

Defines the core data types used across the simulator. All values are synthetic — no
real accounts, keys, addresses, or funds appear anywhere.

Field types use a simple notation that maps directly to Rust: `string`, `u64`, `i64`,
`f64`, `bool`, `timestamp` (RFC 3339), `Option<T>` (nullable), `Vec<T>` (list),
`enum(...)` (enumeration). Fields marked **Zcash-specific** have semantics tied to the
Zcash protocol and will need verification against the actual Z3 stack.

> **Amount convention:** All ZEC amounts are stored as `u64` in **zatoshis**
> (1 ZEC = 100,000,000 zatoshis). Never use floating-point for amounts — rounding
> errors in financial calculations are silent and compounding.

---

## Account

Represents one synthetic exchange user. The simulator provisions a population of
accounts at startup according to the scenario config.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `account_id` | `string` | Stable synthetic identifier (e.g. UUID) | No |
| `status` | `enum(active, inactive)` | Whether this account generates activity during the run | No |
| `activity_profile` | `enum(low, medium, high)` | Transaction frequency relative to other active accounts | No |
| `wallet_id` | `string` | Reference to this account's Wallet | No |
| `created_at` | `timestamp` | Synthetic creation time | No |

One account maps to one wallet in the current design. This may evolve if multi-wallet
exchange behaviour becomes relevant.

---

## Wallet

Represents a Zallet wallet instance associated with one account. Holds the account's
addresses across both address pools.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `wallet_id` | `string` | Stable identifier | No |
| `account_id` | `string` | Back-reference to Account | No |
| `transparent_addresses` | `Vec<Address>` | T-addresses belonging to this wallet | Yes — transparent address pool |
| `shielded_addresses` | `Vec<Address>` | Z-addresses belonging to this wallet | Yes — shielded address pool |
| `created_at` | `timestamp` | When the wallet was initialised in Zallet | No |

---

## Address

Represents a single Zcash address, either transparent or shielded.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `address_id` | `string` | Internal simulator identifier | No |
| `wallet_id` | `string` | Wallet this address belongs to | No |
| `address` | `string` | The Zcash address string | Yes — format is Zcash-specific |
| `address_type` | `enum(transparent, sapling, orchard)` | Which address pool | Yes — pool distinction is Zcash-specific |
| `purpose` | `enum(deposit, change, internal)` | How the simulator uses this address | No |
| `created_at` | `timestamp` | When the address was generated via Zallet | No |
| `last_used_at` | `Option<timestamp>` | Last time a transaction involved this address | No |

**Note on address types:** Zallet at the pinned commit supports both Sapling and Orchard
shielded address pools.

---

## Balance

A snapshot of a wallet's balance at a specific block height, as returned by Zallet.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `wallet_id` | `string` | Wallet this balance belongs to | No |
| `transparent_confirmed` | `u64` | Confirmed transparent balance, in zatoshis | Yes — zatoshi unit |
| `transparent_unconfirmed` | `u64` | Pending transparent balance (in mempool) | Yes |
| `shielded_confirmed` | `u64` | Confirmed shielded balance, in zatoshis | Yes — shielded pool |
| `shielded_unconfirmed` | `u64` | Pending shielded balance | Yes |
| `total_confirmed` | `u64` | Sum of transparent and shielded confirmed | No |
| `at_block_height` | `u64` | Chain height at the time of query | Yes — chain-specific concept |
| `recorded_at` | `timestamp` | Wall-clock time of the query | No |

---

## TransactionIntent

A planned transaction, created by the scenario runner before it is submitted. Represents
what the simulator *wants* to do, before interacting with Zallet.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `intent_id` | `string` | Stable identifier | No |
| `run_id` | `string` | Which simulator run this belongs to | No |
| `account_id` | `string` | Account initiating the transaction | No |
| `sender_address` | `string` | Source address | No |
| `recipient_address` | `string` | Destination address | No |
| `amount_zatoshis` | `u64` | Amount to send, excluding fee | Yes — zatoshi unit |
| `fee_zatoshis` | `u64` | Transaction fee | Yes |
| `flow_type` | `enum(t_to_t, t_to_z, z_to_t, z_to_z)` | Transparent/shielded classification | Yes — Zcash pool distinction |
| `status` | `enum(pending, submitted, confirming, confirmed, failed)` | Lifecycle stage | No |
| `created_at` | `timestamp` | When the intent was created | No |
| `submitted_at` | `Option<timestamp>` | When it was sent to Zallet | No |

---

## TransactionResult

The observed outcome of a submitted TransactionIntent, populated after the RPC call
returns and updated as the transaction confirms.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `intent_id` | `string` | Links back to TransactionIntent | No |
| `txid` | `Option<string>` | On-chain transaction ID, present if broadcast succeeded | Yes — Zcash txid format |
| `status` | `enum(broadcast, confirming, confirmed, failed)` | Current state | No |
| `confirmations` | `u64` | Number of confirmations at last poll | Yes — chain-specific depth |
| `broadcast_latency_ms` | `Option<u64>` | Time from submission to node acceptance | No |
| `proving_time_ms` | `Option<u64>` | ZK proof generation time, shielded txs only | Yes — Zcash-specific; ZK proving is unique to shielded transactions |
| `confirmed_at_height` | `Option<u64>` | Block height when transaction confirmed | Yes |
| `error` | `Option<string>` | Error message if the transaction failed | No |
| `rpc_call_ids` | `Vec<string>` | IDs of RpcCall records involved in this transaction | No |

---

## Deposit

A simulated user deposit to the exchange: funds arriving at a deposit address.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `deposit_id` | `string` | Stable identifier | No |
| `account_id` | `string` | Account making the deposit | No |
| `deposit_address` | `string` | Transparent address assigned to this deposit | Yes — typically transparent for exchange deposits |
| `amount_zatoshis` | `u64` | Amount deposited | Yes |
| `txid` | `Option<string>` | On-chain transaction ID once detected | Yes |
| `status` | `enum(pending, detected, confirming, credited, failed)` | Lifecycle stage | No |
| `required_confirmations` | `u64` | From scenario config | Yes — Zcash confirmation semantics |
| `current_confirmations` | `u64` | As of last poll | Yes |
| `created_at` | `timestamp` | When the deposit was initiated in the simulation | No |
| `credited_at` | `Option<timestamp>` | When the exchange balance was updated | No |

---

## Withdrawal

A simulated user withdrawal from the exchange: funds sent from the exchange to a user.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `withdrawal_id` | `string` | Stable identifier | No |
| `account_id` | `string` | Account requesting the withdrawal | No |
| `destination_address` | `string` | Address to send funds to | No |
| `amount_zatoshis` | `u64` | Amount to withdraw | Yes |
| `fee_zatoshis` | `u64` | Network fee deducted from exchange balance | Yes |
| `status` | `enum(requested, processing, broadcast, confirmed, failed)` | Lifecycle stage | No |
| `txid` | `Option<string>` | On-chain transaction ID once broadcast | Yes |
| `intent_id` | `Option<string>` | Links to TransactionIntent | No |
| `created_at` | `timestamp` | When the withdrawal was requested | No |
| `broadcast_at` | `Option<timestamp>` | When the transaction was sent to the node | No |

---

## Sweep

An exchange consolidation operation: moving funds from many deposit addresses into a
single hot-wallet address. Sweeps reduce the number of UTXOs the exchange manages.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `sweep_id` | `string` | Stable identifier | No |
| `source_addresses` | `Vec<string>` | Deposit addresses being swept | No |
| `destination_address` | `string` | Hot wallet address | No |
| `total_amount_zatoshis` | `u64` | Sum of all inputs, before fee | Yes |
| `fee_zatoshis` | `u64` | Network fee | Yes |
| `status` | `enum(pending, processing, broadcast, confirmed, failed)` | Lifecycle stage | No |
| `txid` | `Option<string>` | On-chain transaction ID | Yes |
| `intent_ids` | `Vec<string>` | One TransactionIntent per source address | No |
| `created_at` | `timestamp` | When the sweep was triggered | No |

**Note:** Sweep transactions with many inputs may have higher fees. The actual fee is
always auto-computed by Zallet via ZIP 317 and read from the operation result.

---

## RpcCall

A record of a single RPC call made by the simulator to a Z3 component. Every call is
recorded, whether it succeeds or fails. Written to `rpc_calls.jsonl` per run.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `call_id` | `string` | UUID for this call | No |
| `run_id` | `string` | Which simulator run | No |
| `method` | `string` | RPC method name (e.g. `getblockchaininfo`) | No |
| `backend` | `enum(Zebra, Zallet, Zaino, Unknown)` | Which backend served the call. For router calls, derived from the method's routing table; for calls to Zaino's JSON-RPC mirror, tagged `Zaino` via a client-side override | Yes — Z3-specific |
| `params_hash` | `Option<string>` | SHA-256 of serialised params (omit sensitive values) | No |
| `request_at` | `timestamp` | When the call was sent | No |
| `response_at` | `Option<timestamp>` | When the response arrived | No |
| `latency_ms` | `Option<u64>` | Round-trip time; null if call timed out | No |
| `success` | `bool` | Whether the call returned a result (not an error) | No |
| `error_code` | `Option<i64>` | JSON-RPC error code if the call failed | No |
| `error_message` | `Option<string>` | Error detail | No |

This is the primary data source for the RPC compatibility matrix and latency analysis.

---

## ScenarioConfig

The parsed and validated form of a scenario YAML file. Stored alongside run outputs
so results are always paired with their exact config.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `name` | `string` | Scenario name | No |
| `description` | `string` | Human-readable description | No |
| `seed` | `u64` | RNG seed for deterministic generation | No |
| `accounts_count` | `u64` | Number of synthetic accounts to provision | No |
| `accounts_active_fraction` | `f64` | Fraction of accounts that transact | No |
| `load_duration_seconds` | `u64` | Length of the load phase | No |
| `load_target_tps` | `f64` | Target transactions per second | No |
| `flows` | `FlowConfig` | Transparent/shielded ratio configuration | Yes — flow types are Zcash-specific |
| `confirmations_deposit_required` | `u64` | Block depth to credit a deposit | Yes |
| `observability` | `ObservabilityConfig` | What to record during the run | No |
| `config_hash` | `string` | SHA-256 of the raw YAML (for manifest recording) | No |
| `source_path` | `string` | Path to the YAML file | No |

---

## MetricSample

A single time-series data point, written to `metrics.jsonl`. The label schema is
designed to be compatible with Prometheus/OpenMetrics for future integration.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `run_id` | `string` | Which simulator run | No |
| `timestamp` | `timestamp` | When the sample was recorded | No |
| `metric_name` | `string` | e.g. `rpc_latency_ms`, `mempool_tx_count`, `confirmed_txs_total` | No |
| `value` | `f64` | Numeric value of the sample | No |
| `labels` | `Map<string, string>` | Dimensions, e.g. `{"component": "Zebra", "method": "getblockcount"}` | No |

---

## IntentRecord

The persisted outcome of a single dispatched `TransactionIntent`, written to
`intents.jsonl` once the load phase completes. One record per intent, regardless of
outcome — confirmed, failed, or timed out.

| Field | Type | Description | Zcash-specific? |
|---|---|---|---|
| `run_id` | `string` | Which simulator run | No |
| `intent_id` | `string` | Links back to the originating `TransactionIntent` | No |
| `flow_type` | `enum(t_to_t, t_to_z, z_to_t, z_to_z)` | Transparent/shielded classification | Yes — Zcash pool distinction |
| `outcome` | `enum(confirmed, failed, timed_out)` | Final state of the intent | No |
| `error` | `Option<string>` | Set when `outcome == failed`; the underlying error message | No |
| `timeout_context` | `Option<string>` | Set when `outcome == timed_out`; distinguishes an async-operation (ZK proving) wait from a confirmation-depth wait — see the two `ExchangeError::Timeout` sites in `src/scenarios/exchange.rs` | Yes — proving vs. confirmation are Zcash-specific wait phases |
| `recorded_at` | `timestamp` | Wall-clock time the outcome was recorded | No |

This is the primary data source for per-flow-type failure/timeout attribution in the
findings report — `RunStats`' aggregate confirmed/failed/timed-out counts alone cannot
distinguish, for example, whether Z→T flows fail more often than T→Z flows, or whether
a run's timeouts were dominated by ZK-proving stalls or by slow confirmation.
