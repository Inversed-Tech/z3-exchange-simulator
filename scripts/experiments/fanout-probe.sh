#!/usr/bin/env bash
# fanout-probe.sh: prove (or disprove), step by step, the funding flow the
# simulator needs: mine coinbase to ONE address we generated, then fan out
# from it to N of our addresses, then have those addresses spend.
#
# This is the exchange-shaped flow:
#
#   step 1   resolve/create the source account, reuse its existing UA  (wallet)
#   step 2   point Zebra's coinbase at the source's p2pkh receiver     (config)
#   step 3   mine past coinbase maturity                               (zebra)
#   step 4   wallet detects the funds                                  (zallet sync)
#   step 5   at least one UTXO is mature                               (consensus)
#   step 6   create N sink accounts, reuse their existing receivers    (wallet)
#   step 7   z_shieldcoinbase: transparent coinbase -> shielded        (wallet policy)
#   step 8   shielding op completes; mine anchor confirmations         (async op)
#   step 9   one z_sendmany fans out to N transparent receivers        (the fan-out)
#   step 10  the fan-out op completes with a txid                      (async op)
#   step 11  sinks hold non-coinbase transparent UTXOs                 (end to end)
#   step 12  a sink spends back to the source                          (exchange flow)
#
# Two findings this flow encodes (measured on Zallet v0.1.0-beta.1 + Zebra
# v6.0.0 + Zaino 0.6.0; see docs/regtest-funding-plan.md):
#
#   - Zallet enforces "coinbase must be spent to a single shielded output"
#     CLIENT-SIDE, even on regtest where Zebra's consensus waives that rule
#     (`should_allow_unshielded_coinbase_spends = true` is hardcoded for
#     Regtest). So transparent coinbase can only leave via z_shieldcoinbase,
#     on every Zallet version.
#   - A UA `from` draws SHIELDED funds only (zallet#644, by design), and the
#     shielded spend needs ~10 anchor confirmations on the shielding tx before
#     the proposal engine will select the notes.
#
# Every step prints `STEP <n> OK/FAIL` and the run continues as far as
# dependencies allow, so one run maps exactly which operations work on the
# stack under test. Run against different pins to compare:
#   scripts/experiments/fanout-probe.sh
#
# Environment:
#   Z3_PROBE_HOST    RPC host (default host.docker.internal)
#   Z3_PROBE_SINKS   number of fan-out accounts N (default 5)
#   Z3_PROBE_BLOCKS  warmup blocks (default 105 = 100 maturity + buffer)
#
# Expects an initialized regtest stack that is up. Repoints
# ZEBRA_MINING__MINER_ADDRESS in .env.regtest (restarts Zebra); does not wipe
# chain or wallet state. Exit code 0 = probe ran; verdicts are the output.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
Z3_DIR="$REPO_ROOT/external/z3"
ENV_FILE="$Z3_DIR/.env.regtest"

HOST="${Z3_PROBE_HOST:-host.docker.internal}"
ROUTER="http://$HOST:8181"
ZEBRA="http://$HOST:29232"
SINKS="${Z3_PROBE_SINKS:-5}"
MINE_BLOCKS="${Z3_PROBE_BLOCKS:-105}"
FAN_AMOUNT="1.0"
# Confirmations mined on top of the shielding tx before spending from the
# shielded pool. 3 was measured insufficient ("Insufficient balance (have 0)"),
# 10 sufficient — consistent with a 10-confirmation anchor policy.
ANCHOR_CONF=10

PASS=0
FAIL=0

log() { printf '%s\n' "$*"; }

rpc() {
    curl -s --max-time 120 -X POST -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$1\",\"params\":$2,\"id\":1}" "$ROUTER"
}

zrpc() {
    curl -s --max-time 120 -u zebra:zebra -X POST -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$1\",\"params\":$2,\"id\":1}" "$ZEBRA"
}

step_ok() {
    PASS=$((PASS + 1))
    log "STEP $1 OK   — $2"
}

step_fail() {
    FAIL=$((FAIL + 1))
    log "STEP $1 FAIL — $2"
}

abort_from() {
    local n="$1" why="$2"
    log ""
    log "Aborting: steps >= $n depend on the failed step ($why)."
    summary
    exit 0
}

summary() {
    log ""
    log "=== fan-out probe summary: $PASS ok, $FAIL failed ==="
    log "stack under test:"
    log "  zallet: $(grep -E '^Z3_ZALLET_IMAGE=' "$ENV_FILE" | cut -d= -f2- || true)"
    log "  zebra:  $(grep -E '^Z3_ZEBRA_IMAGE=' "$ENV_FILE" | cut -d= -f2- || true)"
    log "  zaino:  $(grep -E '^Z3_ZAINO_IMAGE=' "$ENV_FILE" | cut -d= -f2- || true)"
}

# Poll an async wallet operation to a terminal state. Sets the globals
# OP_STATE (success/failed/last-seen) and, on success, OP_TXID.
#
# Deliberately NOT invoked via $(...): command substitution would run this in a
# subshell and the globals would never reach the caller. The txid is taken from
# z_getoperationresult, which is consume-once (the wallet drops the entry after
# returning it) — so it is fetched exactly once, here, and callers must use
# OP_TXID rather than re-querying. Shape varies by method (`txid` for
# z_sendmany, `txids` for z_shieldcoinbase), so accept either.
OP_STATE=""
OP_TXID=""
OP_DETAIL=""
wait_op() {
    local opid="$1" reply=""
    OP_STATE=""
    OP_TXID=""
    OP_DETAIL=""
    for _ in $(seq 1 120); do
        OP_STATE="$(rpc z_getoperationstatus "[[\"$opid\"]]" | jq -r '.result[0].status // empty')"
        case "$OP_STATE" in success | failed) break ;; *) sleep 2 ;; esac
    done
    reply="$(rpc z_getoperationresult "[[\"$opid\"]]")"
    OP_TXID="$(jq -r '.result[0].result.txid // .result[0].result.txids[0] // empty' <<< "$reply")"
    OP_DETAIL="$(jq -c '.result[0].error // .result[0].result // .result' <<< "$reply")"
}

# Return the account's EXISTING lowest-diversifier UA from z_listaccounts.
#
# Never call z_getaddressforaccount to "get" an address: it always DERIVES a
# new one at the next Sapling-valid diversifier index, which skips indices in
# jumps — and on an account with no funded address the transparent gap window
# is indices 0..9 only, so two or three such calls exhaust it and every later
# call fails with "reached the transparent gap limit ... at index 10" (measured
# here on beta.1; the same mechanism explains the June smoke-run failures).
# Account creation already generated diversifier 0 with every receiver type,
# so the address we need always exists.
account_ua() {
    rpc z_listaccounts '[]' \
        | jq -r ".result[]? | select(.account_uuid == \"$1\") | .addresses | sort_by(.diversifier_index) | .[0].ua // empty"
}

for t in curl jq docker; do
    command -v "$t" > /dev/null 2>&1 || { log "$t is required" >&2; exit 2; }
done

log "=== fan-out probe: 1 source -> shield -> $SINKS sinks -> spend-back ==="
log ""

# ── STEP 1: source account + UA ──────────────────────────────────────────────
SRC_UUID="$(rpc z_listaccounts '[]' | jq -r '.result[]? | select(.name == "fanout_source") | .account_uuid' | head -n1)"
if [ -z "$SRC_UUID" ]; then
    SRC_UUID="$(rpc z_getnewaccount '["fanout_source"]' | jq -r '.result.account_uuid // .result.account // empty')"
fi
SRC_UA="$(account_ua "$SRC_UUID")"
SRC_T="$(rpc z_listunifiedreceivers "[\"$SRC_UA\"]" | jq -r '.result.p2pkh // empty')"
if [ -n "$SRC_UUID" ] && [ -n "$SRC_UA" ] && [ -n "$SRC_T" ]; then
    step_ok 1 "source account $SRC_UUID, p2pkh $SRC_T"
else
    step_fail 1 "account=$SRC_UUID ua=${SRC_UA:0:20} p2pkh=$SRC_T"
    abort_from 2 "no source address to mine to"
fi

# ── STEP 2: point coinbase at the source ─────────────────────────────────────
sed -i "s|^ZEBRA_MINING__MINER_ADDRESS=.*|ZEBRA_MINING__MINER_ADDRESS=${SRC_T}|" "$ENV_FILE"
(cd "$Z3_DIR" && docker compose --env-file .env.regtest up -d zebra > /dev/null 2>&1)
ZEBRA_UP=no
for _ in $(seq 1 60); do
    if zrpc getblockchaininfo '[]' | jq -e '.result' > /dev/null 2>&1; then ZEBRA_UP=yes; break; fi
done
if [ "$ZEBRA_UP" = yes ]; then
    step_ok 2 "Zebra restarted with miner_address=$SRC_T"
else
    step_fail 2 "Zebra RPC did not come back after restart"
    abort_from 3 "no validator"
fi

# ── STEP 3: mine past maturity ───────────────────────────────────────────────
MINE_REPLY="$(zrpc generate "[$MINE_BLOCKS]")"
if jq -e '.result' > /dev/null 2>&1 <<< "$MINE_REPLY"; then
    step_ok 3 "mined $MINE_BLOCKS blocks, height now $(zrpc getblockchaininfo '[]' | jq -r '.result.blocks')"
else
    step_fail 3 "generate: $(jq -c '.error' <<< "$MINE_REPLY")"
    abort_from 4 "no coinbase"
fi

# ── STEP 4: wallet detects the funds ─────────────────────────────────────────
DETECTED=""
for _ in $(seq 1 60); do
    DETECTED="$(rpc z_gettotalbalance '[1,true]' | jq -r '.result.transparent // empty')"
    case "$DETECTED" in "" | 0 | 0.00000000 | null) sleep 1 ;; *) break ;; esac
done
case "$DETECTED" in
    "" | 0 | 0.00000000 | null)
        step_fail 4 "wallet never credited the coinbase (z_gettotalbalance transparent=$DETECTED)"
        abort_from 5 "wallet does not see the funds"
        ;;
    *) step_ok 4 "wallet sees transparent=$DETECTED" ;;
esac

# ── STEP 5: maturity ─────────────────────────────────────────────────────────
# Poll: wallet UTXO enhancement lags the balance a little (measured: balance
# non-zero while z_listunspent was still empty for a few seconds).
MATURE=0
for _ in $(seq 1 30); do
    UNSPENT="$(rpc z_listunspent '[]')"
    jq -e '.error' > /dev/null 2>&1 <<< "$UNSPENT" && { sleep 1; continue; }
    MATURE="$(jq -r "[.result[]? | select(.account_uuid == \"$SRC_UUID\" and .confirmations >= 100)] | length" <<< "$UNSPENT")"
    [ "${MATURE:-0}" -gt 0 ] && break
    sleep 1
done
if [ "${MATURE:-0}" -gt 0 ]; then
    step_ok 5 "$MATURE mature (>=100 conf) coinbase UTXO(s) on the source account"
else
    step_fail 5 "no mature UTXO on the source account ($(jq -c '.error // (.result|length)' <<< "$UNSPENT") in z_listunspent)"
fi

# ── STEP 6: N sink accounts ──────────────────────────────────────────────────
SINK_UUIDS=()
SINK_TADDRS=()
SINK_UAS=()
SINK_ERR=""
for i in $(seq 1 "$SINKS"); do
    U="$(rpc z_listaccounts '[]' | jq -r ".result[]? | select(.name == \"fanout_sink_$i\") | .account_uuid" | head -n1)"
    [ -n "$U" ] || U="$(rpc z_getnewaccount "[\"fanout_sink_$i\"]" | jq -r '.result.account_uuid // .result.account // empty')"
    UA="$(account_ua "$U")"
    T="$(rpc z_listunifiedreceivers "[\"$UA\"]" | jq -r '.result.p2pkh // empty')"
    if [ -z "$U" ] || [ -z "$UA" ] || [ -z "$T" ]; then
        SINK_ERR="sink $i: uuid=$U ua=${UA:0:16} t=$T"
        break
    fi
    SINK_UUIDS+=("$U")
    SINK_UAS+=("$UA")
    SINK_TADDRS+=("$T")
done
if [ -z "$SINK_ERR" ]; then
    step_ok 6 "created/resolved $SINKS sink accounts with transparent receivers"
else
    step_fail 6 "$SINK_ERR"
    abort_from 7 "no sinks to fan out to"
fi

# ── STEP 7: shield the coinbase ──────────────────────────────────────────────
# Mandatory on every Zallet version: the proposal engine refuses to spend
# coinbase to transparent outputs even though regtest consensus allows it.
# `from` must be the account UUID or a t-addr; "*" is rejected by Zallet.
SHIELD_REPLY="$(rpc z_shieldcoinbase "[\"$SRC_UUID\",\"$SRC_UA\"]")"
SHIELD_OPID="$(jq -r '.result.opid // empty' <<< "$SHIELD_REPLY")"
if [ -n "$SHIELD_OPID" ]; then
    step_ok 7 "z_shieldcoinbase accepted: $(jq -c '{shieldingUTXOs: .result.shieldingUTXOs, shieldingValue: .result.shieldingValue}' <<< "$SHIELD_REPLY"), opid=$SHIELD_OPID"
else
    step_fail 7 "z_shieldcoinbase: $(jq -c '.error' <<< "$SHIELD_REPLY")"
    abort_from 8 "nothing was shielded"
fi

# ── STEP 8: shielding completes + anchor confirmations ───────────────────────
wait_op "$SHIELD_OPID"
if [ "$OP_STATE" = success ] && [ -n "$OP_TXID" ]; then
    zrpc generate "[$ANCHOR_CONF]" > /dev/null
    step_ok 8 "shielding txid=$OP_TXID, mined $ANCHOR_CONF anchor confirmations"
else
    step_fail 8 "shielding op state=$OP_STATE detail=$OP_DETAIL"
    abort_from 9 "no shielded funds to fan out"
fi

# ── STEP 9: the fan-out itself ───────────────────────────────────────────────
# Shielded source -> N transparent outputs: AllowRevealedRecipients. The source
# is the UA (a UA `from` draws shielded funds only). Retry while the wallet
# catches up to the anchor height — "Insufficient balance (have 0)" flips to
# success once the shielding tx is deep enough and scanned.
FAN_OPID=""
FAN_ERR=""
RECIPIENTS="$(for t in "${SINK_TADDRS[@]}"; do printf '{"address":"%s","amount":%s}\n' "$t" "$FAN_AMOUNT"; done | jq -sc '.')"
for _ in $(seq 1 12); do
    FAN_REPLY="$(rpc z_sendmany "[\"$SRC_UA\",$RECIPIENTS,null,null,\"AllowRevealedRecipients\"]")"
    FAN_OPID="$(jq -r '.result // empty' <<< "$FAN_REPLY")"
    [ -n "$FAN_OPID" ] && break
    FAN_ERR="$(jq -c '.error' <<< "$FAN_REPLY")"
    sleep 5
done
if [ -n "$FAN_OPID" ]; then
    step_ok 9 "z_sendmany accepted the $SINKS-output fan-out, opid=$FAN_OPID"
else
    step_fail 9 "z_sendmany: $FAN_ERR"
    abort_from 10 "the fan-out transaction was refused"
fi

# ── STEP 10: fan-out completes ───────────────────────────────────────────────
wait_op "$FAN_OPID"
if [ "$OP_STATE" = success ] && [ -n "$OP_TXID" ]; then
    step_ok 10 "fan-out txid=$OP_TXID"
else
    step_fail 10 "fan-out op state=$OP_STATE detail=$OP_DETAIL"
    abort_from 11 "fan-out never produced a transaction"
fi

# ── STEP 11: sinks see the money ─────────────────────────────────────────────
# Mine ANCHOR_CONF (not fewer): step 12 spends these outputs, and transparent
# inputs need the same ~10 confirmations as shielded notes before the proposal
# engine selects them (measured: refused at 3 confirmations, accepted at 13).
zrpc generate "[$ANCHOR_CONF]" > /dev/null
SINKS_FUNDED=0
for _ in $(seq 1 60); do
    UNSPENT="$(rpc z_listunspent '[]')"
    SINKS_FUNDED=0
    for u in "${SINK_UUIDS[@]}"; do
        n="$(jq -r "[.result[]? | select(.account_uuid == \"$u\")] | length" <<< "$UNSPENT")"
        [ "${n:-0}" -gt 0 ] && SINKS_FUNDED=$((SINKS_FUNDED + 1))
    done
    [ "$SINKS_FUNDED" -eq "$SINKS" ] && break
    sleep 1
done
if [ "$SINKS_FUNDED" -eq "$SINKS" ]; then
    step_ok 11 "all $SINKS sinks hold a non-coinbase transparent UTXO"
else
    step_fail 11 "only $SINKS_FUNDED/$SINKS sinks show an unspent output"
fi

# ── STEP 12: a sink can spend (the exchange withdrawal shape) ────────────────
# The sink's funds are transparent, so a UA source cannot draw them; the `from`
# must be the sink's own t-addr. This is the form the simulator's TToT/TToZ
# flows must use.
#
# Retry with a block mined per attempt: step 11 returns as soon as the outputs
# are visible, not when all ANCHOR_CONF blocks are scanned, so the wallet's
# view of the confirmations can trail the chain by several blocks here.
BACK_OPID=""
BACK_ERR=""
BACK_FROM="t-addr"
for _ in $(seq 1 12); do
    BACK_REPLY="$(rpc z_sendmany "[\"${SINK_TADDRS[0]}\",[{\"address\":\"$SRC_T\",\"amount\":0.1}],null,null,\"AllowFullyTransparent\"]")"
    BACK_OPID="$(jq -r '.result // empty' <<< "$BACK_REPLY")"
    [ -n "$BACK_OPID" ] && break
    BACK_ERR="$(jq -c '.error' <<< "$BACK_REPLY")"
    zrpc generate '[1]' > /dev/null
    sleep 5
done
if [ -z "$BACK_OPID" ]; then
    # Fall back to the UA source to record which form works on this stack.
    BACK_FROM="UA (t-addr from failed: $BACK_ERR)"
    BACK_REPLY="$(rpc z_sendmany "[\"${SINK_UAS[0]}\",[{\"address\":\"$SRC_T\",\"amount\":0.1}],null,null,\"AllowFullyTransparent\"]")"
    BACK_OPID="$(jq -r '.result // empty' <<< "$BACK_REPLY")"
fi
if [ -z "$BACK_OPID" ]; then
    step_fail 12 "sink z_sendmany refused from both t-addr and UA: $(jq -c '.error' <<< "$BACK_REPLY")"
else
    wait_op "$BACK_OPID"
    if [ "$OP_STATE" = success ]; then
        step_ok 12 "sink 1 spent back to the source via from=$BACK_FROM (txid=$OP_TXID)"
    else
        step_fail 12 "sink spend-back via from=$BACK_FROM state=$OP_STATE detail=$OP_DETAIL"
    fi
fi

summary
