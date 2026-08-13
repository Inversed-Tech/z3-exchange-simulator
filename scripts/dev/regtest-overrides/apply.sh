#!/usr/bin/env bash
# apply.sh: apply the regtest override set to a fresh external/z3 clone.
#
# Why this exists
# ---------------
# The frozen Z3 pins cannot confirm a transaction (Zallet v0.1.0-alpha.3's
# z_sendmany cannot spend from any pool), so live runs use the upstream-coherent
# override set recorded in z3-commits.lock `overrides:` — Zebra 6.0.0 + Zaino
# 0.6.0 + Zallet v0.1.0-beta.1. Until now the edits that stack requires lived
# only as uncommitted local modifications inside external/z3 (a throwaway,
# pinned clone we deliberately do not fork). This script makes them
# reproducible: run it once after clone-z3.sh on any machine.
#
# What it applies (all idempotent, all guarded):
#   1. Generates the live config TOMLs from templates (delegates to the stack's
#      own setup-network.sh, which skips existing files).
#   2. zebra.toml: activates NU6.1 and NU6.2 at height 3. NU6.2 fixes the
#      Orchard Action circuit; without it Zebra builds Orchard coinbase with
#      the new circuit but verifies against the pre-NU6.2 key and rejects its
#      own blocks ("could not validate orchard proof"). Orchard coinbase is
#      the funding fast path: no 100-block maturity, no z_shieldcoinbase
#      round-trip (docs/regtest-funding-plan.md §4).
#   3. zallet.toml: mirrors the activation heights (Zallet >= beta.1 required —
#      alpha.3's zcash_protocol 0.7.2 cannot parse the NU6.2 branch id) and
#      bumps as_of_version to the beta.1 binary.
#   4. .env.regtest: sets Z3_{ZEBRA,ZAINO,ZALLET}_IMAGE to the override images
#      and appends docker-compose.regtest.override.yml to COMPOSE_FILE.
#   5. Copies docker-compose.regtest.override.yml (Zaino 0.6.0 private-bind
#      fix) into the stack directory.
#
# Run order on a fresh machine:
#   bash scripts/dev/clone-z3.sh
#   bash scripts/dev/regtest-overrides/apply.sh          # this script
#   bash scripts/dev/zallet-release-image/build.sh       # local beta.1 image
#   (cd external/z3 && ./scripts/regtest-init.sh)
#   bash scripts/dev/regtest-miner-setup.sh
#
# Requirements: the stack's own setup-network.sh requirements (rage-keygen,
# openssl) plus GNU sed.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
Z3_DIR="${Z3_DIR:-${REPO_ROOT}/external/z3}"
ENV_FILE="${Z3_DIR}/.env.regtest"
CONFIG_DIR="${Z3_DIR}/config/regtest"

log() { printf '%s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

[ -d "$Z3_DIR" ] || die "Z3 stack not found at $Z3_DIR — run scripts/dev/clone-z3.sh first."
[ -f "$ENV_FILE" ] || die "Missing $ENV_FILE — is $Z3_DIR a Z3 checkout?"

# 1. Generate live configs from templates (skips files that already exist).
"$Z3_DIR/scripts/setup-network.sh" regtest

ZEBRA_TOML="$CONFIG_DIR/zebra.toml"
ZALLET_TOML="$CONFIG_DIR/zallet.toml"
[ -f "$ZEBRA_TOML" ] || die "setup-network.sh did not produce $ZEBRA_TOML"
[ -f "$ZALLET_TOML" ] || die "setup-network.sh did not produce $ZALLET_TOML"

# 2. zebra.toml: NU6.1 + NU6.2 at height 3.
if grep -q '^"NU6.2"' "$ZEBRA_TOML"; then
    log "==> zebra.toml: NU6.2 already configured."
else
    grep -q '^"NU6.1" = ' "$ZEBRA_TOML" || die "no NU6.1 entry in $ZEBRA_TOML to patch"
    sed -i 's|^"NU6.1" = .*|"NU6.1" = 3\n"NU6.2" = 3|' "$ZEBRA_TOML"
    log "==> zebra.toml: activated NU6.1 and NU6.2 at height 3."
fi

# 3. zallet.toml: mirror the heights and bump as_of_version.
if grep -q '5437f330' "$ZALLET_TOML"; then
    log "==> zallet.toml: NU6.2 branch already configured."
else
    grep -q '"4dec4df0:' "$ZALLET_TOML" || die "no NU6.1 branch entry in $ZALLET_TOML to patch"
    sed -i 's|"4dec4df0:[0-9]*",.*|"4dec4df0:3",      # NU6.1\n    "5437f330:3",      # NU6.2 (fixed Orchard circuit; required for Orchard coinbase proofs to verify)|' "$ZALLET_TOML"
    log "==> zallet.toml: activated NU6.1 and NU6.2 at height 3."
fi
if grep -q '^as_of_version = "0.1.0-beta.1"' "$ZALLET_TOML"; then
    log "==> zallet.toml: as_of_version already 0.1.0-beta.1."
else
    sed -i 's|^as_of_version = ".*"|as_of_version = "0.1.0-beta.1"|' "$ZALLET_TOML"
    log "==> zallet.toml: as_of_version bumped to 0.1.0-beta.1."
fi

# 4. .env.regtest: override images + third compose layer.
if grep -q '^Z3_ZEBRA_IMAGE=' "$ENV_FILE"; then
    log "==> .env.regtest: image overrides already present."
else
    cat >> "$ENV_FILE" << 'EOF'

# ── Regtest override set (z3-commits.lock `overrides:`) ───────────────────────
# Upstream-coherent combo: Zallet beta.1 embeds zaino@8048bf8 and locks
# zebra-state 10.1 (= Zebra v6.0.0); standalone Zaino 0.6.0 locks the same
# zebra-state. The Zallet image is built locally by
# scripts/dev/zallet-release-image/build.sh (no upstream image past alpha.3).
# Evidence: docs/regtest-funding-plan.md. Applied by
# scripts/dev/regtest-overrides/apply.sh.
Z3_ZEBRA_IMAGE=zfnd/zebra:6.0.0
Z3_ZAINO_IMAGE=zingodevops/zainod:0.6.0-no-tls
Z3_ZALLET_IMAGE=z3sim/zallet:v0.1.0-beta.1
EOF
    log "==> .env.regtest: added Z3_{ZEBRA,ZAINO,ZALLET}_IMAGE overrides."
fi

if grep -q '^COMPOSE_FILE=.*docker-compose.regtest.override.yml' "$ENV_FILE"; then
    log "==> .env.regtest: COMPOSE_FILE already includes the override layer."
else
    sed -i 's|^COMPOSE_FILE=docker-compose.yml:docker-compose.regtest.yml$|&:docker-compose.regtest.override.yml|' "$ENV_FILE"
    grep -q '^COMPOSE_FILE=.*docker-compose.regtest.override.yml' "$ENV_FILE" \
        || die "COMPOSE_FILE line in $ENV_FILE did not match the expected upstream value; append :docker-compose.regtest.override.yml manually."
    log "==> .env.regtest: appended override layer to COMPOSE_FILE."
fi

# 5. Drop the Zaino bind-fix compose layer into the stack directory.
cp "$SCRIPT_DIR/docker-compose.regtest.override.yml" "$Z3_DIR/docker-compose.regtest.override.yml"
log "==> Copied docker-compose.regtest.override.yml into $Z3_DIR."

log ""
log "Override set applied. Next steps:"
log "  bash scripts/dev/zallet-release-image/build.sh"
log "  (cd ${Z3_DIR} && ./scripts/regtest-init.sh)"
log "  bash scripts/dev/regtest-miner-setup.sh"
