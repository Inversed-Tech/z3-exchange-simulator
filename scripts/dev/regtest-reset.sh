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
# no real value, so this is safe by design). Before asking for confirmation,
# it previews exactly which Compose project/network/volumes it is about to
# remove (Track 2 — isolated environments can otherwise only be identified by
# a name a reader has to reconstruct by hand).
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
#   Z3_RPC_HOST            Forwarded to regtest-miner-setup.sh (see that script).
#   COMPOSE_PROJECT_NAME   Compose project to reset. Defaults to this
#                          checkout's own env_id-derived project (read from
#                          configs/local/env-id, the same file and naming
#                          scheme src/z3/env_id.rs uses — see
#                          default_project_name() below), so an unqualified
#                          `make regtest-reset` targets the project a real
#                          `z3sim run` on this checkout actually created,
#                          not a stale literal. Falls back to the pre-Track-2
#                          literal z3-regtest only if no env_id has been
#                          cached yet (no z3sim run has happened here).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
Z3_DIR="${Z3_DIR:-${REPO_ROOT}/external/z3}"
ENV_FILE="${Z3_DIR}/.env.regtest"

log() { printf '%s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

[ -d "$Z3_DIR" ] || die "Z3 stack not found at $Z3_DIR — run scripts/dev/clone-z3.sh first."
[ -f "$ENV_FILE" ] || die "Missing $ENV_FILE — run external/z3/scripts/regtest-init.sh first."

# Mirrors env_id::compose_project_for_env's "z3-sim-<env_id>" naming exactly
# (src/z3/env_id.rs) — the only part of that module duplicated here, since a
# single format string is a trivial, low-drift-risk duplication compared to
# re-deriving the full port/subnet formulas in shell.
default_project_name() {
    local env_id_file="${REPO_ROOT}/configs/local/env-id" id
    if [ -f "$env_id_file" ]; then
        id="$(tr -d '[:space:]' < "$env_id_file")"
        case "$id" in
            [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f])
                printf 'z3-sim-%s' "$id"
                return 0
                ;;
        esac
    fi
    printf 'z3-regtest'
}

COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-$(default_project_name)}"
COMPOSE="docker compose --env-file .env.regtest -p ${COMPOSE_PROJECT_NAME}"
if ! docker info > /dev/null 2>&1; then
    COMPOSE="sudo -E $COMPOSE"
fi

# Preview exactly what is about to be destroyed, BEFORE the --yes gate below —
# a reader should never have to reconstruct the project/network/volume names
# by hand to know what this is about to remove. Read each resource's resolved
# `.name` field rather than reconstructing `${project}_${key}` ourselves:
# Z3's own compose files give volumes and the default network explicit
# `name:` overrides (a `${COMPOSE_PROJECT_NAME}-<suffix>` hyphenated scheme,
# not Compose's implicit underscore-joined default), so `.name` is the only
# way to show the name Docker will actually use.
command -v jq > /dev/null 2>&1 \
    || die "jq is required to preview what this reset would remove (install jq, or see docs/infra/gcp-test-machine.md)."
CONFIG_JSON="$(cd "$Z3_DIR" && $COMPOSE config --format json)" \
    || die "\`docker compose config\` failed — is $ENV_FILE valid?"
PROJECT_NAME="$(printf '%s' "$CONFIG_JSON" | jq -r '.name')"

log "About to reset environment: ${PROJECT_NAME}"
printf '%s' "$CONFIG_JSON" | jq -r '.networks[].name' | while read -r net; do
    log "  network: ${net}"
done
printf '%s' "$CONFIG_JSON" | jq -r '.volumes[].name' | while read -r vol; do
    log "  volumes: ${vol}"
done
log ""

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

log "==> Wiping the regtest stack (containers and volumes)..."
(cd "$Z3_DIR" && $COMPOSE down -v)

# regtest-init.sh sources .env.regtest itself (`set -a; . "$ENV_FILE"`) to
# resolve its own COMPOSE_PROJECT_NAME — that assignment unconditionally
# overwrites whatever value we exported into its environment, so passing
# COMPOSE_PROJECT_NAME as a process env var here would silently be ignored.
# Write it into the file instead, matching what src/z3/mod.rs's
# Z3Config::sync_bootstrap_env_file does for the same reason on the Rust
# side — otherwise regtest-init.sh always (re-)initializes z3-regtest
# regardless of which project this script just wiped.
if grep -q '^COMPOSE_PROJECT_NAME=' "$ENV_FILE"; then
    sed -i "s|^COMPOSE_PROJECT_NAME=.*|COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME}|" "$ENV_FILE"
else
    printf 'COMPOSE_PROJECT_NAME=%s\n' "$COMPOSE_PROJECT_NAME" >> "$ENV_FILE"
fi

log "==> Re-initializing the regtest wallet from scratch..."
(cd "$Z3_DIR" && ./scripts/regtest-init.sh)

log "==> Re-pointing coinbase at the hot_wallet account..."
bash "${REPO_ROOT}/scripts/dev/regtest-miner-setup.sh"

log "==> Bringing the stack back up..."
(cd "$Z3_DIR" && $COMPOSE up -d)

log ""
log "Done. The regtest environment is now a fresh chain with a funded"
log "hot_wallet miner address — ready for a scenario run."
