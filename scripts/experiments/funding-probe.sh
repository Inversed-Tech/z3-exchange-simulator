#!/usr/bin/env bash
# funding-probe.sh: determine which regtest funding routes actually yield
# spendable value in the Z3 stack.
#
# The simulator needs the hot wallet to hold funds it can spend to both
# transparent and shielded recipients. Coinbase is the only source of value on
# a regtest chain (no premine, no faucet RPC), but coinbase can be paid into
# three different pools, and each pool has to survive three separate layers:
# Zebra must build and accept the block, Zallet must detect the output, and
# Zallet's z_sendmany must select it as an input. This script probes one pool
# end to end and prints a verdict for each layer.
#
# Usage:
#   scripts/experiments/funding-probe.sh <pool>
#
#   pool = transparent | sapling | orchard
#
# Expects an initialized regtest stack (external/z3/scripts/regtest-init.sh)
# that is currently up. Mutates ZEBRA_MINING__MINER_ADDRESS in .env.regtest and
# restarts Zebra, so it is destructive to the miner configuration but not to the
# chain or the wallet.
#
# Exit status is 0 whether or not the pool works: the verdict is the output, not
# the exit code. A non-zero exit means the probe itself could not run.

set -euo pipefail

POOL="${1:-}"
case "$POOL" in
    transparent | sapling | orchard) ;;
    *)
        echo "usage: $0 <transparent|sapling|orchard>" >&2
        exit 2
        ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
Z3_DIR="$REPO_ROOT/external/z3"
ENV_FILE="$Z3_DIR/.env.regtest"

# Ports are published on the Docker host, which is not this container's
# localhost when the daemon is Docker Desktop, so address it as a peer.
HOST="${Z3_PROBE_HOST:-host.docker.internal}"
ROUTER="http://$HOST:8181"
ZEBRA="http://$HOST:29232"

# Blocks to mine after repointing the miner address. Shielded coinbase has no
# consensus maturity (ZIP 213), and regtest maturity for transparent coinbase is
# 100 blocks, so the transparent probe needs a longer run to prove spendability.
case "$POOL" in
    transparent) MINE_BLOCKS="${Z3_PROBE_BLOCKS:-105}" ;;
    *) MINE_BLOCKS="${Z3_PROBE_BLOCKS:-6}" ;;
esac

log() { printf '%s\n' "$*"; }

rpc() {
    curl -s --max-time 120 -X POST -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$1\",\"params\":$2,\"id\":1}" "$ROUTER"
}

zrpc() {
    curl -s --max-time 120 -u zebra:zebra -X POST -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$1\",\"params\":$2,\"id\":1}" "$ZEBRA"
}

# Report a JSON-RPC reply as either its result or its error, so a failing layer
# names its own reason instead of collapsing to "empty".
verdict() {
    local label="$1" reply="$2"
    if [ "$(jq -r 'has("error") and (.error != null)' <<< "$reply")" = "true" ]; then
        log "  $label: FAIL — $(jq -c '.error' <<< "$reply")"
        return 1
    fi
    log "  $label: OK — $(jq -c '.result' <<< "$reply" | cut -c1-160)"
    return 0
}

wait_for_zebra() {
    local i
    for i in $(seq 1 60); do
        if zrpc getblockchaininfo '[]' | jq -e '.result' > /dev/null 2>&1; then
            return 0
        fi
    done
    log "Zebra RPC did not come back after restart" >&2
    return 1
}

require_tools() {
    local t
    for t in curl jq docker; do
        command -v "$t" > /dev/null 2>&1 || {
            log "$t is required" >&2
            exit 2
        }
    done
}

require_tools

log "=== Probing the $POOL pool as a funding route ==="

# ── Layer 0: a hot wallet account that can receive in the probed pool ─────────
# The receiver set is requested explicitly, but the diversifier index is left to
# Zallet: only about half of all indices yield a valid Sapling diversifier, and
# index 0 for a fresh account generally does not, so pinning the index makes a
# Sapling receiver unobtainable ("diversifier index 0 cannot generate an address
# with the requested receivers").
HOT_UUID="$(rpc z_listaccounts '[]' | jq -r '.result[]? | select(.name == "hot_wallet") | .account_uuid' | head -n1)"
if [ -z "$HOT_UUID" ]; then
    HOT_UUID="$(rpc z_getnewaccount '["hot_wallet"]' | jq -r '.result.account_uuid // .result.account')"
    log "created hot_wallet account: $HOT_UUID"
else
    log "reusing hot_wallet account: $HOT_UUID"
fi
[ -n "$HOT_UUID" ] && [ "$HOT_UUID" != "null" ] || {
    log "could not resolve a hot_wallet account" >&2
    exit 1
}

HOT_ADDR_REPLY="$(rpc z_getaddressforaccount "[\"$HOT_UUID\",[\"orchard\",\"sapling\",\"p2pkh\"]]")"
HOT_UA="$(jq -r '.result.address // empty' <<< "$HOT_ADDR_REPLY")"
[ -n "$HOT_UA" ] || {
    log "could not derive a hot_wallet UA with all three receivers: $(jq -c '.error' <<< "$HOT_ADDR_REPLY")" >&2
    exit 1
}
log "hot_wallet UA at diversifier index $(jq -r '.result.diversifier_index' <<< "$HOT_ADDR_REPLY")"

RECEIVERS="$(rpc z_listunifiedreceivers "[\"$HOT_UA\"]" | jq -c '.result')"
HOT_T="$(jq -r '.p2pkh' <<< "$RECEIVERS")"
case "$POOL" in
    transparent) MINER_ADDR="$HOT_T" ;;
    sapling) MINER_ADDR="$(jq -r '.sapling' <<< "$RECEIVERS")" ;;
    orchard) MINER_ADDR="$(jq -r '.orchard' <<< "$RECEIVERS")" ;;
esac
[ -n "$MINER_ADDR" ] && [ "$MINER_ADDR" != "null" ] || {
    log "hot_wallet UA has no $POOL receiver" >&2
    exit 1
}
log "miner address ($POOL): $MINER_ADDR"

# ── Layer 1: does Zebra build and accept a block paying this address? ────────
sed -i "s|^ZEBRA_MINING__MINER_ADDRESS=.*|ZEBRA_MINING__MINER_ADDRESS=${MINER_ADDR}|" "$ENV_FILE"
(cd "$Z3_DIR" && docker compose --env-file .env.regtest up -d zebra > /dev/null 2>&1)
wait_for_zebra

log "Layer 1 — Zebra mines $MINE_BLOCKS blocks to a $POOL address:"
MINED="$(zrpc generate "[$MINE_BLOCKS]")"
if verdict "mine" "$MINED"; then
    log "    height now $(zrpc getblockchaininfo '[]' | jq -r '.result.blocks')"
else
    log ""
    log "VERDICT: $POOL is not a viable funding pool — Zebra cannot mine to it."
    log "Check 'docker compose logs zebra' for the rejection reason."
    exit 0
fi

# ── Layer 2: does Zallet detect the coinbase output? ─────────────────────────
# Poll rather than sleep: detection is asynchronous behind Zaino's indexer.
log "Layer 2 — Zallet detects the coinbase output:"
DETECTED=no
for _ in $(seq 1 40); do
    BAL="$(rpc z_gettotalbalance '[1,true]')"
    T="$(jq -r '.result.transparent // "0"' <<< "$BAL")"
    P="$(jq -r '.result.private // "0"' <<< "$BAL")"
    case "$POOL" in
        transparent) OBSERVED="$T" ;;
        *) OBSERVED="$P" ;;
    esac
    if [ -n "$OBSERVED" ] && [ "$OBSERVED" != "0" ] && [ "$OBSERVED" != "0.00000000" ] && [ "$OBSERVED" != "null" ]; then
        DETECTED=yes
        break
    fi
done
log "  balance: transparent=$T private=$P"
log "  notes:   $(rpc z_getnotescount '[]' | jq -c '.result')"
if [ "$DETECTED" != yes ]; then
    log ""
    log "VERDICT: Zebra mines to $POOL but Zallet never credits the balance."
    exit 0
fi
log "  detected: OK"

# z_listunspent is not required for funding, but a failure here is a real defect
# worth surfacing: the simulator uses it for balance verification.
log "Layer 2b — z_listunspent (diagnostic, not required for funding):"
verdict "z_listunspent" "$(rpc z_listunspent '[]')" || true

# ── Layer 3: can z_sendmany select it as an input? ───────────────────────────
# A second account gives the send a destination the source account does not
# already own, so a success cannot be a no-op self-transfer.
SINK_UUID="$(rpc z_listaccounts '[]' | jq -r '.result[]? | select(.name == "probe_sink") | .account_uuid' | head -n1)"
if [ -z "$SINK_UUID" ]; then
    SINK_UUID="$(rpc z_getnewaccount '["probe_sink"]' | jq -r '.result.account_uuid // .result.account')"
fi
# As above, let Zallet choose the diversifier. Creating an account already
# generates its index-0 address with all three receiver types, so asking for a
# narrower set at index 0 always fails with "already generated with different
# receiver types".
SINK_ADDR_REPLY="$(rpc z_getaddressforaccount "[\"$SINK_UUID\"]")"
SINK_UA="$(jq -r '.result.address // empty' <<< "$SINK_ADDR_REPLY")"
[ -n "$SINK_UA" ] || {
    log "could not derive a probe_sink address: $(jq -c '.error' <<< "$SINK_ADDR_REPLY")" >&2
    exit 1
}
SINK_T="$(rpc z_listunifiedreceivers "[\"$SINK_UA\"]" | jq -r '.result.p2pkh')"

log "Layer 3 — z_sendmany spends from the $POOL balance:"
SHIELDED_OK=1
TRANSPARENT_OK=1
verdict "-> shielded UA (FullPrivacy)" \
    "$(rpc z_sendmany "[\"$HOT_UA\",[{\"address\":\"$SINK_UA\",\"amount\":0.01}],null,null,\"FullPrivacy\"]")" || SHIELDED_OK=0
verdict "-> transparent taddr (AllowFullyTransparent)" \
    "$(rpc z_sendmany "[\"$HOT_UA\",[{\"address\":\"$SINK_T\",\"amount\":0.01}],null,null,\"AllowFullyTransparent\"]")" || TRANSPARENT_OK=0

log ""
if [ "$SHIELDED_OK" = 1 ] || [ "$TRANSPARENT_OK" = 1 ]; then
    log "VERDICT: $POOL is a viable funding pool (shielded_send=$SHIELDED_OK transparent_send=$TRANSPARENT_OK)."
else
    log "VERDICT: $POOL is detected but unspendable — z_sendmany selects no inputs from it."
fi
