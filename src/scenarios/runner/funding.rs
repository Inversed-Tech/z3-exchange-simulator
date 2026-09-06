//! Funding helper: turn mined coinbase into N spendable accounts.
//!
//! This is the common path every live test and scenario needs: coinbase is the
//! only source of value on a regtest chain (no premine, no faucet RPC), and it
//! lands in ONE account — the one whose receiver is Zebra's startup-only
//! `mining.miner_address`. Everything else must be funded by fanning out from
//! that account.
//!
//! The flow implemented here was established step by step in
//! `scripts/experiments/fanout-probe.sh` (12/12 on Zebra v6.0.0 + Zaino 0.6.0 +
//! Zallet v0.1.0-beta.1; see `docs/regtest-funding-plan.md`), and encodes these
//! measured rules:
//!
//! - **Never derive to "get" an address.** `z_getaddressforaccount` always
//!   derives a NEW address at the next Sapling-valid diversifier index; on an
//!   account with no funded address the transparent gap window is indices
//!   0..9, so a handful of calls exhausts it (`ReachedGapLimit` at index 10).
//!   Accounts are read back through `z_listaccounts`, which lists the
//!   addresses that already exist — including the one created with the account.
//! - **Coinbase should be mined to the hot wallet's Orchard receiver.**
//!   Shielded coinbase has no maturity (ZIP 213 limits the 100-block rule to
//!   transparent coinbase) and is spendable by `z_sendmany` directly. This
//!   needs NU6.2 active (fixed Orchard circuit) — with transparent coinbase
//!   the wallet insists on a `z_shieldcoinbase` round-trip even on regtest, so
//!   [`fund_accounts`] transparently falls back to shielding when it finds
//!   transparent-only funds.
//! - **A UA `from` spends shielded funds only; a bare t-addr spends its own
//!   transparent UTXOs** (zallet#644). The fan-out therefore sends from the
//!   source UA with `AllowRevealedRecipients`, and recipients that must hold
//!   *transparent* value are paid at their extracted p2pkh receiver, not their
//!   UA (paying a UA settles shielded).
//! - **Inputs need ~10 confirmations** before the proposal engine selects
//!   them, notes and transparent UTXOs alike. Younger funds produce error -4
//!   "Insufficient balance (have 0, …)" while the balance plainly shows them,
//!   so sends are retried while blocks are mined rather than failed fast.
//! - **`warmup_blocks` is sized for coinbase maturity, not funding value.**
//!   It produces a fixed amount of mined value regardless of a scenario's
//!   scale (`accounts_count` x TPS x duration), so a large enough scenario
//!   can need far more than warmup ever mined. [`fund_accounts`] tops up
//!   mining itself, on top of the existing shield-if-short step, until the
//!   source account's balance covers the plan — see [`ensure_funded`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::time::sleep;

use crate::data_model::Phase;
use crate::rpc::{Recipient, RpcClient, RpcError};
use crate::scenarios::runner::progress::ProgressLine;

/// Confirmations an output needs before Zallet's proposal engine will select
/// it as an input. Measured: refused at 3, accepted at >= 10 (consistent with
/// a 10-confirmation anchor policy).
pub const ANCHOR_CONFIRMATIONS: u32 = 10;

/// Zatoshis per ZEC, for converting scenario amounts (zatoshis) to the ZEC
/// floats the JSON-RPC surface expects.
pub const ZAT_PER_ZEC: f64 = 100_000_000.0;

/// Convert zatoshis to a ZEC f64 with exactly 8 decimal places, via integer
/// arithmetic. Never compute ZEC amounts with float multiplication: e.g.
/// `0.1_f64 * 1.5` is `0.15000000000000002`, which Zallet rejects with
/// `-3 Invalid amount` (amounts may not exceed 8 decimals).
pub fn zat_to_zec(zatoshis: u64) -> f64 {
    let whole = zatoshis / 100_000_000;
    let frac = zatoshis % 100_000_000;
    format!("{whole}.{frac:08}")
        .parse()
        .expect("generated ZEC decimal string is always a valid f64")
}

/// How long to keep polling an async wallet operation before giving up.
const OPERATION_TIMEOUT: Duration = Duration::from_secs(240);

/// `generate` must be chunked on this stack: an Orchard `miner_address` costs
/// ~2s of halo2 proving per block (emulated host), so one large `generate`
/// call can outlive the 30s HTTP timeout while Zebra keeps mining
/// server-side (see docs/regtest-funding-plan.md, "generate must be
/// chunked").
const GENERATE_CHUNK_BLOCKS: u64 = 5;

/// How many blocks [`ensure_funded`] mines per top-up round, before
/// re-measuring the balance. Deliberately not "mine exactly the computed
/// shortfall": only the oldest blocks in a batch have reached
/// [`ANCHOR_CONFIRMATIONS`] by the time the batch finishes, so a single
/// round under-counts newly mined value — the loop compensates by running
/// more, smaller rounds rather than trying to calculate an exact one-shot
/// amount from an assumed, possibly stale, per-block reward.
const FUND_TOPUP_BLOCKS_PER_ROUND: u64 = 30;

/// Cap on top-up rounds before [`ensure_funded`] gives up. Mining is cheap on
/// regtest (no real proof-of-work), so this is sized generously rather than
/// tightly: a large scenario's funding requirement can be a five-figure ZEC
/// amount (see docs/z3-regtest-bootstrap-nu5-miner-address.md's sibling
/// finding on funding scale), needing well over a thousand extra blocks.
const FUND_TOPUP_MAX_ROUNDS: u32 = 100;

/// Mine additional coinbase blocks, if necessary, until `source`'s spendable
/// balance (transparent + shielded, at [`ANCHOR_CONFIRMATIONS`]) covers
/// `needed` ZEC.
///
/// `warmup_blocks` mines a fixed number of blocks sized for coinbase
/// maturity, not for funding value — nothing else scales the amount mined to
/// a scenario's actual funding requirement, so a scenario large enough
/// (`accounts_count` x TPS x duration) can need far more value than a fixed
/// warmup ever produces. Without this, [`fund_accounts`]'s fan-out send
/// fails with "Insufficient balance (have 0, …)" and never recovers, no
/// matter how many times it retries — the retry loop is designed for a
/// confirmation-*depth* race (funds exist but are too young), not a
/// magnitude shortfall.
async fn ensure_funded(
    rpc: &Arc<RpcClient>,
    source: &FundedAccount,
    needed: f64,
) -> Result<(), FundingError> {
    for _ in 0..FUND_TOPUP_MAX_ROUNDS {
        if current_balance_zec(rpc, source).await? >= needed {
            return Ok(());
        }
        mine_chunked(rpc, FUND_TOPUP_BLOCKS_PER_ROUND).await?;
    }

    let balance = current_balance_zec(rpc, source).await?;
    if balance >= needed {
        return Ok(());
    }
    Err(FundingError::Failed {
        step: "ensure_funded",
        detail: format!(
            "source account {} holds {balance} ZEC after {FUND_TOPUP_MAX_ROUNDS} top-up \
             rounds ({} blocks), still short of the {needed} ZEC the funding plan requires",
            source.uuid,
            FUND_TOPUP_MAX_ROUNDS as u64 * FUND_TOPUP_BLOCKS_PER_ROUND
        ),
    })
}

/// `source`'s spendable balance (transparent + shielded, at
/// [`ANCHOR_CONFIRMATIONS`]), in ZEC.
async fn current_balance_zec(rpc: &RpcClient, source: &FundedAccount) -> Result<f64, FundingError> {
    let balance = rpc
        .z_get_balance_for_account(&source.uuid, Some(ANCHOR_CONFIRMATIONS))
        .await
        .map_err(|e| FundingError::Rpc {
            step: "z_getbalanceforaccount",
            source: e,
        })?;
    Ok(zat_to_zec(balance.shielded_zatoshis()) + zat_to_zec(balance.transparent_zatoshis()))
}

/// Mine `blocks` blocks in chunks of [`GENERATE_CHUNK_BLOCKS`], retrying a
/// transport error a few times per chunk — see
/// [`RpcClient::generate_in_chunks`] for why this is necessary rather than
/// treating a chunk's transport error as fatal.
async fn mine_chunked(rpc: &RpcClient, blocks: u64) -> Result<(), FundingError> {
    rpc.generate_in_chunks(blocks, GENERATE_CHUNK_BLOCKS as u32)
        .await
        .map_err(|e| FundingError::Rpc {
            step: "generate",
            source: e,
        })
}

/// How many times a send is retried while the wallet catches up to freshly
/// mined anchor blocks (one block is mined between attempts). Sized to
/// comfortably cover the round-0 fan-out send, which combines every sink's
/// shielded and first transparent output into one multi-recipient
/// transaction — larger than any single load-phase send, and so needing more
/// margin against confirmation-depth delay.
const SEND_RETRIES: u32 = 24;

/// Errors from the funding pipeline. Wraps the failing step so a probe-style
/// "which operation broke" reading survives into the error message.
#[derive(Debug)]
pub enum FundingError {
    Rpc {
        step: &'static str,
        source: RpcError,
    },
    Failed {
        step: &'static str,
        detail: String,
    },
}

impl std::fmt::Display for FundingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FundingError::Rpc { step, source } => write!(f, "funding step `{step}`: {source}"),
            FundingError::Failed { step, detail } => write!(f, "funding step `{step}`: {detail}"),
        }
    }
}

impl std::error::Error for FundingError {}

/// A funded (or fundable) account: its UUID, its pre-existing UA, and the
/// receivers extracted from that UA.
#[derive(Debug, Clone)]
pub struct FundedAccount {
    pub uuid: String,
    /// The account's primary UA (lowest diversifier index — the address
    /// created with the account). Use as `z_sendmany` `from` for shielded
    /// spends.
    pub address: String,
    /// The UA's transparent receiver. Pay this (not the UA) when the account
    /// must hold transparent value; use it as `from` for transparent spends.
    pub transparent_receiver: Option<String>,
    /// The UA's Orchard receiver, as a single-receiver UA. This is what
    /// Zebra's `mining.miner_address` should be set to for the hot wallet.
    pub orchard_receiver: Option<String>,
}

/// Resolve an account by name, creating it if absent, and return it with its
/// existing primary address (never deriving a new one — see module docs).
pub async fn resolve_account(rpc: &RpcClient, name: &str) -> Result<FundedAccount, FundingError> {
    let accounts = rpc.z_list_accounts().await.map_err(|e| FundingError::Rpc {
        step: "z_listaccounts",
        source: e,
    })?;

    let existing = accounts
        .into_iter()
        .find(|a| a.name.as_deref() == Some(name));

    let info = match existing {
        Some(info) => info,
        None => {
            // z_getnewaccount's response has no addresses; re-list to pick up
            // the diversifier-0 address that account creation generated.
            let created = rpc
                .z_get_new_account(name)
                .await
                .map_err(|e| FundingError::Rpc {
                    step: "z_getnewaccount",
                    source: e,
                })?;
            rpc.z_list_accounts()
                .await
                .map_err(|e| FundingError::Rpc {
                    step: "z_listaccounts",
                    source: e,
                })?
                .into_iter()
                .find(|a| a.account == created.account)
                .ok_or_else(|| FundingError::Failed {
                    step: "z_listaccounts",
                    detail: format!("account {} not listed after creation", created.account),
                })?
        }
    };

    resolve_receivers(rpc, info).await
}

/// Resolve an account that is known to exist by UUID (e.g. from
/// `RunOptions::hot_wallet_uuid`). Unlike [`resolve_account`], never creates.
pub async fn resolve_account_by_uuid(
    rpc: &RpcClient,
    uuid: &str,
) -> Result<FundedAccount, FundingError> {
    let info = rpc
        .z_list_accounts()
        .await
        .map_err(|e| FundingError::Rpc {
            step: "z_listaccounts",
            source: e,
        })?
        .into_iter()
        .find(|a| a.account == uuid)
        .ok_or_else(|| FundingError::Failed {
            step: "resolve_account_by_uuid",
            detail: format!("no account with UUID {uuid}"),
        })?;
    resolve_receivers(rpc, info).await
}

/// Shared tail of the resolve functions: take the account's pre-existing
/// primary address and split out its receivers.
pub async fn resolve_receivers(
    rpc: &RpcClient,
    info: crate::rpc::AccountInfo,
) -> Result<FundedAccount, FundingError> {
    let address = info
        .primary_address()
        .ok_or_else(|| FundingError::Failed {
            step: "resolve_receivers",
            detail: format!("account {} has no addresses", info.account),
        })?
        .to_string();

    let receivers =
        rpc.z_list_unified_receivers(&address)
            .await
            .map_err(|e| FundingError::Rpc {
                step: "z_listunifiedreceivers",
                source: e,
            })?;

    Ok(FundedAccount {
        uuid: info.account,
        address,
        transparent_receiver: receivers.p2pkh,
        orchard_receiver: receivers.orchard,
    })
}

/// Poll an async wallet operation to completion and return its txid.
pub async fn wait_operation(rpc: &RpcClient, opid: &str) -> Result<String, FundingError> {
    let deadline = tokio::time::Instant::now() + OPERATION_TIMEOUT;
    loop {
        let statuses =
            rpc.z_get_operation_status(&[opid])
                .await
                .map_err(|e| FundingError::Rpc {
                    step: "z_getoperationstatus",
                    source: e,
                })?;
        if let Some(status) = statuses.first() {
            if status.is_complete() {
                if status.status == "success" {
                    if let Some(txid) = status.txid() {
                        return Ok(txid.to_string());
                    }
                }
                // Terminal but not a clean success: fetch the (consume-once)
                // result for the error detail.
                let results =
                    rpc.z_get_operation_result(&[opid])
                        .await
                        .map_err(|e| FundingError::Rpc {
                            step: "z_getoperationresult",
                            source: e,
                        })?;
                let mut detail = results
                    .first()
                    .and_then(|r| r.error.as_ref())
                    .map(|e| format!("code {}: {}", e.code, e.message))
                    .unwrap_or_else(|| format!("operation ended as `{}`", status.status));
                if is_expiry_race(&detail) {
                    // Not a transient race: Zallet computed this operation's
                    // expiry from its own wallet-scan height, which can trail
                    // the real chain tip by hundreds of blocks once the
                    // regtest volumes have accumulated enough history that a
                    // fresh container's rescan can no longer catch up inside
                    // one run's budget. Resubmitting doesn't help — it only
                    // costs an operation timeout's worth of retries with the
                    // identical result (see docs/zallet-wallet-scan-lag.md).
                    // The environment needs a reset, not a retry.
                    detail.push_str(
                        " [likely cause: Zallet's wallet-scan is far behind the chain tip, \
                         probably because the regtest environment has accumulated too much \
                         history across repeated runs — reset it with \
                         scripts/dev/regtest-reset.sh before retrying]",
                    );
                }
                return match results.first().and_then(|r| r.result.as_ref()) {
                    Some(detail) => Ok(detail.txid.clone()),
                    None => Err(FundingError::Failed {
                        step: "wait_operation",
                        detail,
                    }),
                };
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(FundingError::Failed {
                step: "wait_operation",
                detail: format!("operation {opid} did not complete within {OPERATION_TIMEOUT:?}"),
            });
        }
        sleep(Duration::from_secs(2)).await;
    }
}

/// Send with retries while the wallet catches up to freshly mined blocks.
///
/// "Insufficient balance (have 0, …)" from a wallet that demonstrably holds
/// the funds means the inputs are younger than [`ANCHOR_CONFIRMATIONS`] *in
/// the wallet's view*, which trails the chain during scanning. One block is
/// mined per retry so waiting always converges.
async fn send_with_anchor_retries(
    rpc: &RpcClient,
    from: &str,
    recipients: &[Recipient],
    policy: &str,
) -> Result<String, FundingError> {
    let mut last_err: Option<RpcError> = None;
    for _ in 0..SEND_RETRIES {
        match rpc.z_send_many_with_policy(from, recipients, policy).await {
            Ok(opid) => return Ok(opid),
            Err(e) => {
                let retryable = matches!(
                    &e,
                    RpcError::JsonRpc { message, .. } if message.contains("Insufficient balance")
                );
                last_err = Some(e);
                if !retryable {
                    break;
                }
            }
        }
        let _ = rpc.generate(1).await;
        sleep(Duration::from_secs(5)).await;
    }
    Err(FundingError::Rpc {
        step: "z_sendmany",
        source: last_err.expect("at least one attempt was made"),
    })
}

/// True if an operation's terminal error is Zallet rejecting its own
/// transaction for being mined past its expiry height — e.g. "...
/// transaction must not be mined at a block Height(1697) greater than its
/// expiry Height(1199) ...". Matched on the stable middle clause rather than
/// the variable heights either side of it.
fn is_expiry_race(detail: &str) -> bool {
    detail.contains("greater than its expiry Height")
}

/// How each sink account is funded by [`fund_accounts`].
#[derive(Debug, Clone, Copy)]
pub struct FundingPlan {
    /// Number of separate transparent UTXOs to give each sink. One per
    /// expected transparent spend: a transparent spend consumes its whole
    /// UTXO and the change returns to the account's SHIELDED pool (the
    /// wallet's change strategy), so an account holding a single t-UTXO can
    /// source exactly one TToT/TToZ intent no matter how large the UTXO was.
    pub transparent_outputs: u32,
    /// ZEC per transparent UTXO. Must cover one intent's amount plus fee.
    pub transparent_zec_each: f64,
    /// ZEC paid to the sink's UA. One output suffices for any number of
    /// shielded spends: shielded change stays in the account's own pool.
    pub shielded_zec: f64,
}

impl FundingPlan {
    fn per_sink_total(&self) -> f64 {
        self.transparent_outputs as f64 * self.transparent_zec_each + self.shielded_zec
    }
}

/// Fund `sinks` from `source` in one fan-out transaction, per `plan`.
///
/// Both pools matter for flow fidelity: TToT/TToZ intents spend the account's
/// *transparent* funds (via its t-addr as `from`), while ZToT sweeps and ZToZ
/// sends spend its *shielded* funds (via its UA as `from`). An account funded
/// in only one pool can only source half the flow types.
///
/// Precondition: coinbase has been mined to one of `source`'s receivers (the
/// Orchard receiver, ideally — see module docs) and the wallet has had time to
/// detect it. If the source's spendable funds are transparent coinbase, they
/// are routed through `z_shieldcoinbase` first, because the wallet will not
/// spend coinbase to transparent outputs on any Zallet version.
///
/// Returns the fan-out txid. The caller must keep mining (the runner's
/// background miner does) or mine [`ANCHOR_CONFIRMATIONS`] blocks before the
/// sinks can spend what they received.
pub async fn fund_accounts(
    rpc: &Arc<RpcClient>,
    source: &FundedAccount,
    sinks: &[FundedAccount],
    plan: FundingPlan,
    progress: &ProgressLine,
) -> Result<String, FundingError> {
    // 1. If the shielded pool is empty but transparent coinbase is present,
    //    shield it. With Orchard-coinbase mining this is a no-op.
    //
    // Scoped to `source` specifically via z_getbalanceforaccount, NOT
    // z_gettotalbalance: the latter is wallet-wide, so on a wallet with prior
    // run history (other accounts already holding shielded balance) it would
    // report plenty of funds regardless of whether `source` itself has
    // anything spendable — the subsequent z_sendmany is scoped to `source`
    // alone and would fail with "Insufficient balance (have 0, ...)" no
    // matter how much other accounts hold.
    let balance = rpc
        .z_get_balance_for_account(&source.uuid, Some(ANCHOR_CONFIRMATIONS))
        .await
        .map_err(|e| FundingError::Rpc {
            step: "z_getbalanceforaccount",
            source: e,
        })?;
    let private = zat_to_zec(balance.shielded_zatoshis());
    let transparent = zat_to_zec(balance.transparent_zatoshis());
    let needed = plan.per_sink_total() * sinks.len() as f64;

    if private < needed && transparent > 0.0 {
        let shield = rpc
            .z_shield_coinbase(&source.uuid, &source.address)
            .await
            .map_err(|e| FundingError::Rpc {
                step: "z_shieldcoinbase",
                source: e,
            })?;
        wait_operation(rpc, &shield.opid).await?;
        // The shielding tx itself needs anchor confirmations before the notes
        // it created are spendable.
        mine_chunked(rpc, ANCHOR_CONFIRMATIONS as u64).await?;
    }

    // Shielding only redistributes existing coinbase between pools — it
    // cannot make the account hold more value than warmup already mined
    // into it. If that still isn't enough for this plan, mine more.
    ensure_funded(rpc, source, needed).await?;

    // 2. Fan out in rounds. Each sink needs `plan.transparent_outputs`
    //    separate UTXOs at ONE address, but z_sendmany rejects duplicate
    //    recipient addresses within a transaction (-8 "duplicated recipient
    //    address", zcashd-compatible) — so each round is one transaction that
    //    pays every sink's t-addr once, and the rounds repeat until every sink
    //    holds the planned UTXO count. Shielded outputs ride along in the
    //    first round (each UA appears once). Rounds run sequentially: the hot
    //    wallet's note selection must not race itself.
    //
    //    Transparent value goes to the extracted p2pkh receivers so the sinks
    //    end up with genuinely transparent funds (paying their UAs would
    //    settle shielded and TToT/TToZ flows would have no transparent inputs
    //    to spend); shielded value goes to the UAs.
    let rounds = plan
        .transparent_outputs
        .max(u32::from(plan.shielded_zec > 0.0));
    let rounds_start = Instant::now();
    let mut last_txid: Option<String> = None;
    for round in 0..rounds {
        progress.update(
            Phase::Funding,
            &format!("funding round {}/{rounds}", round + 1),
            rounds_start.elapsed(),
            None,
        );
        let mut recipients: Vec<Recipient> = Vec::with_capacity(sinks.len() * 2);
        for s in sinks {
            if round < plan.transparent_outputs && plan.transparent_zec_each > 0.0 {
                let address =
                    s.transparent_receiver
                        .clone()
                        .ok_or_else(|| FundingError::Failed {
                            step: "fund_accounts",
                            detail: format!("sink account {} has no transparent receiver", s.uuid),
                        })?;
                recipients.push(Recipient {
                    address,
                    amount: plan.transparent_zec_each,
                    memo: None,
                });
            }
            if round == 0 && plan.shielded_zec > 0.0 {
                recipients.push(Recipient {
                    address: s.address.clone(),
                    amount: plan.shielded_zec,
                    memo: None,
                });
            }
        }
        if recipients.is_empty() {
            continue;
        }
        let opid =
            send_with_anchor_retries(rpc, &source.address, &recipients, "AllowRevealedRecipients")
                .await?;
        last_txid = Some(wait_operation(rpc, &opid).await?);
    }
    progress.finish();

    last_txid.ok_or_else(|| FundingError::Failed {
        step: "fund_accounts",
        detail: "nothing to fund: the plan produces no outputs (or no sinks)".into(),
    })
}

#[cfg(test)]
mod tests {
    use wiremock::{matchers, Mock, MockServer, ResponseTemplate};

    use super::{is_expiry_race, wait_operation, FundingError};
    use crate::rpc::{AccountInfo, RpcClient};

    fn account(uuid: &str, addrs: &[(u64, &str)]) -> AccountInfo {
        let json = serde_json::json!({
            "account_uuid": uuid,
            "name": "x",
            "addresses": addrs
                .iter()
                .map(|(i, ua)| serde_json::json!({"diversifier_index": i, "ua": ua}))
                .collect::<Vec<_>>(),
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn primary_address_is_lowest_diversifier() {
        let info = account("u", &[(4, "ua-four"), (0, "ua-zero"), (9, "ua-nine")]);
        assert_eq!(info.primary_address(), Some("ua-zero"));
    }

    #[test]
    fn primary_address_empty_when_no_addresses() {
        let info = account("u", &[]);
        assert_eq!(info.primary_address(), None);
    }

    #[test]
    fn account_info_tolerates_missing_addresses_field() {
        // z_getnewaccount responses have no `addresses` array.
        let info: AccountInfo =
            serde_json::from_value(serde_json::json!({"account_uuid": "u"})).unwrap();
        assert!(info.addresses.is_empty());
        assert_eq!(info.primary_address(), None);
    }

    // ── is_expiry_race / wait_operation diagnostics ─────────────────────────

    const EXPIRY_RACE_MESSAGE: &str = "SendTransaction: Transaction commit failed:: chain \
         backend error: unexpected error response from server: RPC Error (code: -25): failed \
         to validate tx: WtxId(\"private\"), error: transaction did not pass consensus \
         validation: transaction must not be mined at a block Height(1697) greater than its \
         expiry Height(1199), failing transaction transaction::Hash(\"deb3ebf3\")";

    #[test]
    fn is_expiry_race_matches_the_observed_zallet_message() {
        assert!(is_expiry_race(EXPIRY_RACE_MESSAGE));
        assert!(!is_expiry_race("code -6: Insufficient funds"));
    }

    /// Mount both `z_getoperationstatus` and `z_getoperationresult` for one
    /// `opid`, terminal from the first status poll (matching real Zallet
    /// behavior for a fast-failing or fast-succeeding operation).
    async fn mount_operation(
        server: &MockServer,
        opid: &'static str,
        status: &str,
        result: serde_json::Value,
        error: serde_json::Value,
    ) {
        let body = serde_json::json!({
            "result": [{"id": opid, "status": status, "result": result, "error": error}],
            "error": null,
            "id": 1
        });
        for method in ["z_getoperationstatus", "z_getoperationresult"] {
            Mock::given(matchers::method("POST"))
                .and(matchers::body_partial_json(serde_json::json!({
                    "method": method,
                    "params": [[opid]],
                })))
                .respond_with(ResponseTemplate::new(200).set_body_json(body.clone()))
                .mount(server)
                .await;
        }
    }

    #[tokio::test]
    async fn wait_operation_annotates_an_expiry_race_with_the_likely_cause_and_fix() {
        let server = MockServer::start().await;
        mount_operation(
            &server,
            "op-1",
            "failed",
            serde_json::Value::Null,
            serde_json::json!({"code": -4, "message": EXPIRY_RACE_MESSAGE}),
        )
        .await;

        let rpc = RpcClient::new(&server.uri(), "test-run", None, None);
        let err = wait_operation(&rpc, "op-1").await.unwrap_err();

        let FundingError::Failed { step, detail } = err else {
            panic!("expected FundingError::Failed, got {err:?}");
        };
        assert_eq!(step, "wait_operation");
        assert!(detail.contains(EXPIRY_RACE_MESSAGE));
        assert!(
            detail.contains("regtest-reset.sh"),
            "expected the diagnostic to point at the reset script, got: {detail}"
        );
    }

    #[tokio::test]
    async fn wait_operation_leaves_unrelated_failures_unannotated() {
        let server = MockServer::start().await;
        mount_operation(
            &server,
            "op-1",
            "failed",
            serde_json::Value::Null,
            serde_json::json!({"code": -6, "message": "Insufficient funds"}),
        )
        .await;

        let rpc = RpcClient::new(&server.uri(), "test-run", None, None);
        let err = wait_operation(&rpc, "op-1").await.unwrap_err();

        let FundingError::Failed { detail, .. } = err else {
            panic!("expected FundingError::Failed, got {err:?}");
        };
        assert_eq!(detail, "code -6: Insufficient funds");
    }
}
