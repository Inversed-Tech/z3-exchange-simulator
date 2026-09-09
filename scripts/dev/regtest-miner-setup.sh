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
# This script points Zebra's coinbase at the **Orchard receiver** of the
# **hot_wallet** account — the same account the scenario runner spends from — and
# writes it into `external/z3/.env.regtest` so every subsequent
# `docker compose up -d` mines coinbase into that account.
#
# Orchard (not P2PKH) because shielded coinbase is strictly better on regtest
# (measured; see docs/regtest-funding-plan.md §4): no 100-block maturity
# (ZIP 213 limits that rule to transparent coinbase) and no z_shieldcoinbase
# round-trip (Zallet refuses to spend TRANSPARENT coinbase to transparent
# outputs on every version, even though regtest consensus allows it). Requires
# NU6.2 active in the regtest params (fixed Orchard circuit) and Zallet >=
# v0.1.0-beta.1 — both part of the override stack in z3-commits.lock. Set
# Z3_MINER_POOL=p2pkh to fall back to transparent coinbase mining.
#
# It deliberately does NOT create a separate "miner" account, which is what this
# script and the stack's own regtest-init.sh both used to do. Coinbase landed in
# `miner` while the runner spent from `hot_wallet`, so every send failed with
# "Insufficient balance (have 0)" — a correct answer to the wrong question, and
# a confound that made a genuine Zallet defect much harder to diagnose. Funding
# the account we spend from removes a whole class of false diagnosis.
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

log "==> Bringing the stack up to resolve the hot_wallet account..."
$COMPOSE up -d

log "   Waiting for the RPC router..."
until rpc getblockchaininfo '[]' | grep -q '"result"'; do
    log "   Router not ready yet, retrying..."
    sleep 3
done
log "   Router is ready."

# Reuse the hot_wallet account if a previous run (or the runner itself) already
# created it, so re-running this script cannot split coinbase across two accounts.
log "==> Resolving the Zallet 'hot_wallet' account..."
MINER_UUID="$(rpc z_listaccounts '[]' | jq -r '.result[]? | select(.name == "hot_wallet") | .account_uuid' | head -n1)"
if [ -n "$MINER_UUID" ]; then
    log "   Reusing existing hot_wallet account: ${MINER_UUID}"
else
    MINER_UUID="$(rpc z_getnewaccount '["hot_wallet"]' | jq -r '.result.account_uuid // .result.account')"
    log "   Created hot_wallet account: ${MINER_UUID}"
fi
[ -n "$MINER_UUID" ] && [ "$MINER_UUID" != "null" ] || { $COMPOSE down; die "failed to resolve or create the hot_wallet account"; }

# Read the account's EXISTING diversifier-0 address from z_listaccounts instead of
# deriving one: every z_getaddressforaccount call derives a NEW address at the next
# Sapling-valid diversifier index, and on an unfunded account the transparent gap
# window is indices 0..9 — a few derivations exhaust it (ReachedGapLimit at index
# 10). Account creation already generated the address we need.
MINER_UA="$(rpc z_listaccounts '[]' | jq -r ".result[] | select(.account_uuid == \"${MINER_UUID}\") | .addresses | sort_by(.diversifier_index) | .[0].ua")"
MINER_POOL="${Z3_MINER_POOL:-orchard}"
MINER_TADDR="$(rpc z_listunifiedreceivers "[\"${MINER_UA}\"]" | jq -r ".result.${MINER_POOL}")"
[ -n "$MINER_TADDR" ] && [ "$MINER_TADDR" != "null" ] || { $COMPOSE down; die "failed to derive ${MINER_POOL} receiver for ${MINER_UUID}"; }
log "   hot_wallet ${MINER_POOL} receiver: ${MINER_TADDR}"

# Zebra keeps the most recent ~100 blocks (its non-finalized state) in memory
# only, and regtest stack restarts routinely lose it — the chain rolls back to
# the finalized floor (measured on the GCP runner: a chain at height 2 came
# back at height 0 after the reset flow's own restarts;
# docs/regtest-funding-plan.md records the same for SIGKILL). Once the miner
# address below is an Orchard UA, a rollback under the NU5 activation height
# (2) wedges the stack permanently: Zebra cannot mine ANY block to an Orchard
# address pre-NU5 ("Cannot create Orchard transactions … before NU5
# activation" panics its RPC worker), so nothing can ever advance the chain
# again. Mine past the finalization depth with the still-configured
# placeholder transparent address FIRST, so every activation height is
# finalized on disk before coinbase switches to the UA. Transparent blocks
# carry no proving cost — this takes seconds; chunked generate calls stay well
# inside HTTP timeouts. Direct Zebra RPC (not the router): mirrors
# regtest-init.sh's own mining step.
ZEBRA_HOST_RPC="$(grep -E '^Z3_ZEBRA_HOST_RPC_PORT=' "$ENV_FILE" | cut -d= -f2 || true)"
ZEBRA_HOST_RPC="${ZEBRA_HOST_RPC:-29232}"
log "==> Finalizing the activation heights (105 placeholder-mined blocks)..."
for _ in 1 2 3 4 5; do
    curl -s -u zebra:zebra -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"generate","params":[21],"id":1}' \
        "http://${RPC_HOST}:${ZEBRA_HOST_RPC}" | grep -q '"result"' \
        || { $COMPOSE down; die "placeholder-address mining failed — check the zebra container logs"; }
done

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
