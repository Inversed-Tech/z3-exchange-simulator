#!/usr/bin/env bash
# regtest-reset.sh: wipe and reinitialize the regtest environment.
#
# Why this exists
# ----------------
# The regtest chain and Zallet's wallet database live in Docker volumes that
# survive `docker compose down` (no `-v`) — i.e. every plain stack restart —
# so they accumulate history across every scenario run ever attempted against
# them, indefinitely. Zallet's wallet-scan re-derives its view of that history
# from an on-disk checkpoint on every container start, at a fixed per-block
# cost; once enough history has piled up, a fresh container can no longer
# scan back to the live chain tip within any single scenario's time budget.
# Zallet computes new transactions' safety parameters (expiry included) from
# that lagging wallet-view rather than the live chain tip, so funding fails
# with a consensus rejection like:
#
#   transaction must not be mined at a block Height(N) greater than its
#   expiry Height(M)
#
# no matter how many times the send is retried — the wallet-view/chain-tip
# gap doesn't close by waiting a few seconds, only by not having accumulated
# in the first place. See docs/zallet-wallet-scan-lag.md for the full
# root-cause writeup.
#
# What this does
# ---------------
# Chains the exact recovery sequence already validated in
# docs/zallet-restart-sync-failure.md ("Recovery method used"): wipe the
# stack's Docker volumes, then re-run the same two init scripts the Quickstart
# uses on a fresh clone (regtest-init.sh, then regtest-miner-setup.sh — the
# latter re-points Zebra's coinbase at the hot_wallet account, and itself
# leaves the stack down), then bring the stack back up.
#
# This is destructive to the CURRENT regtest chain and wallet state (all
# accounts, balances, and transaction history are gone — regtest coinbase has
# no real value, so this is safe by design). Requires an explicit --yes.
#
# Usage:
#   scripts/dev/regtest-reset.sh --yes
#
# When to run it: before a scenario that failed with the expiry/consensus
# error above, and periodically during any extended run of repeated
# scenario attempts against the same environment (the accumulation is
# monotonic — it never resolves itself by running more scenarios).
#
# Environment:
#   Z3_RPC_HOST   Forwarded to regtest-miner-setup.sh (see that script).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
Z3_DIR="${Z3_DIR:-${REPO_ROOT}/external/z3}"
ENV_FILE="${Z3_DIR}/.env.regtest"

log() { printf '%s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

if [ "${1:-}" != "--yes" ]; then
    cat >&2 <<EOF
This wipes the regtest chain and Zallet wallet database entirely: all
accounts, balances, and transaction history in the current environment are
destroyed and reinitialized from scratch. Regtest coinbase has no real value,
so this is safe to do freely — but it is irreversible for whatever is
currently in the volumes.

Re-run with --yes to proceed:
    scripts/dev/regtest-reset.sh --yes
EOF
    exit 1
fi

[ -d "$Z3_DIR" ] || die "Z3 stack not found at $Z3_DIR — run scripts/dev/clone-z3.sh first."
[ -f "$ENV_FILE" ] || die "Missing $ENV_FILE — run external/z3/scripts/regtest-init.sh first."

COMPOSE="docker compose --env-file .env.regtest"
if ! docker info > /dev/null 2>&1; then
    COMPOSE="sudo -E $COMPOSE"
fi

log "==> Wiping the regtest stack (containers and volumes)..."
(cd "$Z3_DIR" && $COMPOSE down -v)

log "==> Re-initializing the regtest wallet from scratch..."
(cd "$Z3_DIR" && ./scripts/regtest-init.sh)

log "==> Re-pointing coinbase at the hot_wallet account..."
bash "${REPO_ROOT}/scripts/dev/regtest-miner-setup.sh"

log "==> Bringing the stack back up..."
(cd "$Z3_DIR" && $COMPOSE up -d)

log ""
log "Done. The regtest environment is now a fresh chain with a funded"
log "hot_wallet miner address — ready for a scenario run."
