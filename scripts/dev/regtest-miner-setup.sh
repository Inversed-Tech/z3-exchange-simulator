#!/usr/bin/env bash
# regtest-miner-setup.sh: point Zebra's coinbase at a Zallet-managed account.
#
# Why this exists
# ---------------
# The scenario runner funds its hot wallet from coinbase: it mines warmup blocks
# and then checks the wallet balance. For that to be non-zero, Zebra must mine
# into a transparent (P2PKH) address that Zallet controls. The pinned Z3 stack
# (z3-commits.lock) ships a static placeholder `ZEBRA_MINING__MINER_ADDRESS`
# that no wallet owns, so coinbase is unspendable and warmup sees 0 ZEC.
#
# This script creates a dedicated Zallet "miner" account, derives its P2PKH
# receiver, and writes it into `external/z3/.env.regtest` so every subsequent
# `docker compose up -d` mines coinbase into the Zallet wallet.
#
# It is a *simulator-owned* post-clone step, deliberately kept out of the Z3
# stack repo: external/z3 is a throwaway clone pinned (and currently frozen) via
# z3-commits.lock, so we do not fork it. When the miner-account setup lands
# upstream in ZcashFoundation/z3 and the pin is bumped, this script becomes
# redundant and can be deleted.
#
# Run once, after cloning + initializing the stack:
#   bash scripts/dev/clone-z3.sh
#   (cd external/z3 && ./scripts/regtest-init.sh)
#   bash scripts/dev/regtest-miner-setup.sh
#   (cd external/z3 && docker compose --env-file .env.regtest up -d)
#
# Idempotent: re-running after the address is set is a no-op.
#
# Environment:
#   Z3_RPC_HOST   Host the RPC router is reachable on (default: 127.0.0.1).
#                 Set to `host.docker.internal` when running from inside a
#                 devcontainer against a Docker-Desktop-hosted stack.
#
# Requirements: docker compose v2, curl, jq.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
Z3_DIR="${Z3_DIR:-${REPO_ROOT}/external/z3}"
ENV_FILE="${Z3_DIR}/.env.regtest"

# The unowned placeholder shipped by the pinned stack. A value other than this
# means the miner address has already been configured — nothing to do.
PLACEHOLDER="tmSRd1r8gs77Ja67Fw1JcdoXytxsyrLTPJm"

RPC_HOST="${Z3_RPC_HOST:-127.0.0.1}"

log() { printf '%s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

for bin in curl jq; do
    command -v "$bin" > /dev/null 2>&1 || die "$bin is required but not installed."
done

[ -d "$Z3_DIR" ] || die "Z3 stack not found at $Z3_DIR — run scripts/dev/clone-z3.sh first."
[ -f "$ENV_FILE" ] || die "Missing $ENV_FILE — run external/z3/scripts/regtest-init.sh first."

# Read the router host port from the stack's env file (fallback to the default).
ROUTER_PORT="$(grep -E '^Z3_REGTEST_RPC_ROUTER_HOST_PORT=' "$ENV_FILE" | cut -d= -f2)"
ROUTER_PORT="${ROUTER_PORT:-8181}"
ROUTER_URL="http://${RPC_HOST}:${ROUTER_PORT}"

CURRENT_MINER="$(grep -E '^ZEBRA_MINING__MINER_ADDRESS=' "$ENV_FILE" | cut -d= -f2)"
if [ "$CURRENT_MINER" != "$PLACEHOLDER" ]; then
    log "==> Miner address already configured: ${CURRENT_MINER} — nothing to do."
    exit 0
fi

# docker compose invocation, with the same sudo fallback the stack scripts use.
COMPOSE="docker compose --env-file .env.regtest"
if ! docker info > /dev/null 2>&1; then
    COMPOSE="sudo -E $COMPOSE"
fi

rpc() {
    # rpc METHOD JSON_PARAMS -> raw JSON response on stdout
    curl -s -X POST -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"$1\",\"params\":$2,\"id\":1}" \
        "$ROUTER_URL"
}

cd "$Z3_DIR"

log "==> Bringing the stack up to create the miner account..."
$COMPOSE up -d

log "   Waiting for the RPC router..."
until rpc getblockchaininfo '[]' | grep -q '"result"'; do
    log "   Router not ready yet, retrying..."
    sleep 3
done
log "   Router is ready."

log "==> Creating Zallet 'miner' account..."
MINER_UUID="$(rpc z_getnewaccount '["miner"]' | jq -r '.result.account_uuid // .result.account')"
[ -n "$MINER_UUID" ] && [ "$MINER_UUID" != "null" ] || { $COMPOSE down; die "failed to create miner account"; }
log "   Miner account UUID: ${MINER_UUID}"

MINER_UA="$(rpc z_getaddressforaccount "[\"${MINER_UUID}\"]" | jq -r '.result.address')"
MINER_TADDR="$(rpc z_listunifiedreceivers "[\"${MINER_UA}\"]" | jq -r '.result.p2pkh')"
[ -n "$MINER_TADDR" ] && [ "$MINER_TADDR" != "null" ] || { $COMPOSE down; die "failed to derive transparent receiver for ${MINER_UUID}"; }
log "   Miner T-address: ${MINER_TADDR}"

# Persist so every future `docker compose up -d` uses the funded miner address.
tmp="$(mktemp "${TMPDIR:-/tmp}/env.regtest.XXXXXX")"
sed -E "s|^ZEBRA_MINING__MINER_ADDRESS=.*|ZEBRA_MINING__MINER_ADDRESS=${MINER_TADDR}|" "$ENV_FILE" > "$tmp"
mv "$tmp" "$ENV_FILE"
log "==> Updated ZEBRA_MINING__MINER_ADDRESS in ${ENV_FILE}."

log "==> Bringing the stack down (restart it to apply the new miner address)."
$COMPOSE down

log ""
log "Done. Start the stack with:"
log "   (cd ${Z3_DIR} && docker compose --env-file .env.regtest up -d)"
