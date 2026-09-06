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
# It also records this reset generation at
# configs/local/reset-epoch-<env_id> (an incrementing epoch counter plus the
# chain height observed right after this reset), gitignored alongside
# env-id and scoped per env_id the same way run-<env_id>.lock is — so a
# --fresh-env environment and the stable one never share reset provenance —
# read by the simulator into a run's manifest (StateIdentifier, Track 6) so
# a run can be told "freshly reset" apart from "reused since the last
# reset."
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

# This checkout's cached environment id (src/z3/env_id.rs's resolve_env_id
# cache), or empty if none has been cached yet (no z3sim run has happened
# here). Shared by default_project_name() below and by the reset-epoch
# filename further down, so both name the SAME environment consistently.
resolved_env_id() {
    local env_id_file="${REPO_ROOT}/configs/local/env-id" id
    if [ -f "$env_id_file" ]; then
        id="$(tr -d '[:space:]' < "$env_id_file")"
        case "$id" in
            [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f])
                printf '%s' "$id"
                return 0
                ;;
        esac
    fi
    printf ''
}

# Mirrors env_id::compose_project_for_env's "z3-sim-<env_id>" naming exactly
# (src/z3/env_id.rs) — the only part of that module duplicated here, since a
# single format string is a trivial, low-drift-risk duplication compared to
# re-deriving the full port/subnet formulas in shell.
default_project_name() {
    local id
    id="$(resolved_env_id)"
    if [ -n "$id" ]; then
        printf 'z3-sim-%s' "$id"
    else
        printf 'z3-regtest'
    fi
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
    # `-i.bak` (extension attached directly to `-i`, no space) is the one
    # `sed -i` spelling both BSD sed (macOS's default /usr/bin/sed — which
    # otherwise requires an extension argument and, without one, parses the
    # very next word as a file path instead of an extension, crashing with
    # "invalid command code") and GNU sed accept identically. Portable, so
    # this needs no GNU-sed dependency check the way
    # scripts/dev/regtest-overrides/apply.sh's own (genuinely GNU-only)
    # `sed -i` calls do (see scripts/dev/bootstrap.sh's dependency check).
    sed -i.bak "s|^COMPOSE_PROJECT_NAME=.*|COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME}|" "$ENV_FILE"
    rm -f "${ENV_FILE}.bak"
else
    printf 'COMPOSE_PROJECT_NAME=%s\n' "$COMPOSE_PROJECT_NAME" >> "$ENV_FILE"
fi

log "==> Re-initializing the regtest wallet from scratch..."
(cd "$Z3_DIR" && ./scripts/regtest-init.sh)

log "==> Re-pointing coinbase at the hot_wallet account..."
bash "${REPO_ROOT}/scripts/dev/regtest-miner-setup.sh"

log "==> Bringing the stack back up..."
(cd "$Z3_DIR" && $COMPOSE up -d)

# Record this reset generation for the manifest's state/snapshot provenance
# (StateIdentifier, Track 6): reset-epoch holds two whitespace-separated
# fields, "{epoch} {height_at_reset}" — incremented and re-queried on every
# reset so a run's manifest can tell "freshly reset" apart from "reused since
# the last reset" (see metrics::manifest::read_reset_state /
# StateFreshness::classify). Best-effort: a failure here does not fail the
# reset itself, since the chain/wallet are already usable at this point —
# it only means the next run's manifest falls back to reset_epoch 0.
#
# Scoped by env_id (reset-epoch-<env_id>, mirroring src/z3/env_id.rs's
# reset_epoch_path — the shell-side duplicate of that exact filename format,
# same low-drift-risk tradeoff as default_project_name() above), NOT a
# single checkout-wide file: two environments on this checkout (the stable
# one and a --fresh-env one) must never read or write each other's reset
# provenance.
#
# Derived from the AUTHORITATIVE ${COMPOSE_PROJECT_NAME} actually used above
# — NOT by re-reading resolved_env_id()/the env-id cache file a second time
# — so this stays correct even when a caller overrode COMPOSE_PROJECT_NAME
# directly rather than letting default_project_name() derive it from the
# cache: the reset-epoch file must always match whatever project this
# invocation actually reset, not whatever env_id happens to be cached right
# now. A project name that isn't the z3-sim-<env_id> shape (the pre-Track-2
# z3-regtest literal, or some other manual override) has no env_id-scoped
# identity to key this off, so it falls back to the shared legacy filename.
case "$COMPOSE_PROJECT_NAME" in
    z3-sim-????????)
        ENV_ID="${COMPOSE_PROJECT_NAME#z3-sim-}"
        ;;
    *)
        ENV_ID=""
        ;;
esac
if [ -n "$ENV_ID" ]; then
    RESET_EPOCH_FILE="${REPO_ROOT}/configs/local/reset-epoch-${ENV_ID}"
else
    RESET_EPOCH_FILE="${REPO_ROOT}/configs/local/reset-epoch"
fi
PREV_EPOCH="0"
if [ -f "$RESET_EPOCH_FILE" ]; then
    PREV_EPOCH="$(awk '{print $1}' "$RESET_EPOCH_FILE" 2>/dev/null || true)"
    case "$PREV_EPOCH" in
        ''|*[!0-9]*) PREV_EPOCH="0" ;;
    esac
fi
NEXT_EPOCH=$((PREV_EPOCH + 1))

ROUTER_PORT="$(grep -E '^Z3_REGTEST_RPC_ROUTER_HOST_PORT=' "$ENV_FILE" | cut -d= -f2)"
ROUTER_PORT="${ROUTER_PORT:-8181}"
ROUTER_URL="http://${Z3_RPC_HOST:-127.0.0.1}:${ROUTER_PORT}"

# `docker compose up -d` above returns once containers report "started," not
# once a freshly-(re)started container's own process is actually accepting
# connections — the same race regtest-miner-setup.sh already retries around
# after its own `up -d` (see that script's "Waiting for the RPC router..."
# loop, which this mirrors). Bounded here, unlike that script's unbounded
# wait, because this step is best-effort evidence, not a prerequisite the
# rest of the reset depends on — a router that never comes up within the
# budget below falls through to "leaving reset-epoch unwritten," not a
# script failure. Every step is explicitly `|| true`-guarded (or run outside
# a pipeline) so a connection-refused/non-2xx/malformed response degrades to
# an empty result under `set -e`/`pipefail` instead of aborting this
# already-successful reset outright.
HEIGHT_AT_RESET=""
ROUTER_WAIT_ATTEMPTS=20
attempt=0
while [ "$attempt" -lt "$ROUTER_WAIT_ATTEMPTS" ]; do
    attempt=$((attempt + 1))
    RESPONSE="$(curl -sf -u zebra:zebra -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"getblockcount","params":[],"id":1}' \
        "$ROUTER_URL" 2>/dev/null || true)"
    HEIGHT_AT_RESET="$(printf '%s' "$RESPONSE" | jq -r '.result // empty' 2>/dev/null || true)"
    case "$HEIGHT_AT_RESET" in
        ''|*[!0-9]*)
            HEIGHT_AT_RESET=""
            [ "$attempt" -lt "$ROUTER_WAIT_ATTEMPTS" ] && sleep 3
            ;;
        *)
            break
            ;;
    esac
done

case "$HEIGHT_AT_RESET" in
    ''|*[!0-9]*)
        log "==> Router did not report a chain height after ${ROUTER_WAIT_ATTEMPTS} attempts — leaving reset-epoch unwritten."
        ;;
    *)
        mkdir -p "$(dirname "$RESET_EPOCH_FILE")"
        printf '%s %s\n' "$NEXT_EPOCH" "$HEIGHT_AT_RESET" > "$RESET_EPOCH_FILE"
        log "==> Recorded reset epoch ${NEXT_EPOCH} at chain height ${HEIGHT_AT_RESET} (${RESET_EPOCH_FILE##*/})."
        ;;
esac

log ""
log "Done. The regtest environment is now a fresh chain with a funded"
log "hot_wallet miner address — ready for a scenario run."
