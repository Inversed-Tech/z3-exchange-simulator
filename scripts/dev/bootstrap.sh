#!/usr/bin/env bash
# bootstrap.sh: one command to stand up the ACTUALLY-WORKING Z3 regtest stack.
#
# Why this exists
# ----------------
# The README quickstart historically pointed at the frozen Z3 pin
# (z3-commits.lock's top-level entries), whose Zallet v0.1.0-alpha.3 cannot
# spend from any pool — no scenario using it can ever confirm a transaction
# (docs/zallet-transparent-spending-bug.md). The stack that actually works is
# the override set (z3-commits.lock `overrides:`), which requires several
# steps beyond a fresh clone: applying config patches, building a Zallet image
# no upstream registry publishes, and initializing the wallet/miner. Until
# now those steps lived only as prose split across this directory's scripts
# and docs/regtest-funding-plan.md, with no single command sequencing them.
#
# What this does
# ---------------
#   1. Checks required local dependencies (Docker, Docker Compose >= 2.24.4,
#      rage-keygen, OpenSSL, curl, jq, Rust build dependencies, disk space;
#      pandoc is checked too but only ever advisory).
#   2. Clones the pinned Z3 stack (scripts/dev/clone-z3.sh) and applies the
#      working override set (scripts/dev/regtest-overrides/apply.sh),
#      building the local Zallet release image
#      (scripts/dev/zallet-release-image/build.sh) if it isn't already
#      present.
#   3. Builds the simulator binary and runs `z3sim print-versions`, which
#      brings the stack up, bootstraps the wallet and miner account (if this
#      environment hasn't been bootstrapped before — see
#      `Z3Config::ensure_wallet_bootstrapped`, src/z3/mod.rs), and prints the
#      exact image each component is running.
#
# `z3sim print-versions` resolves and caches this checkout's environment id
# the same way `z3sim run` does (see src/z3/env_id.rs), so the stack this
# script brings up and bootstraps is the SAME one a subsequent `z3sim run`
# reuses — nothing here is wasted or duplicated under a second, different
# Compose project.
#
# Every step above is independently idempotent, so re-running this script is
# safe and cheap: clone-z3.sh/apply.sh no-op on an already-configured
# checkout, the Zallet image build is skipped once the image already exists
# (Phase 2 below checks via `docker image inspect` before invoking build.sh),
# and print-versions' bootstrap step no-ops once the wallet/miner are already
# configured.
#
# Usage:
#   scripts/dev/bootstrap.sh              # full bootstrap
#   scripts/dev/bootstrap.sh --check-only # dependency check only (exit 2 on
#                                         # a missing/insufficient dependency)
#
# Exit codes:
#   0 — bootstrap completed; the stack is up and its wallet/miner configured.
#   2 — a required dependency is missing or insufficient (checked before
#       anything else in this script touches Docker or the filesystem).
#   1 — an existing script in the sequence failed (Docker/build/init error).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

log() { printf '%s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

# ── Phase 1: dependency check ────────────────────────────────────────────────
#
# Collects every failing check before exiting, rather than failing on the
# first missing tool — mirrors `validate_scenario`'s "report every violation"
# convention (src/scenarios/runner/config.rs) so a user fixes their host once
# per bootstrap attempt, not once per re-run.

MISSING=()

os_hint() {  # $1: apt package(s), $2: brew package(s) -> printed install hint
    case "$(uname -s)" in
        Linux) printf 'apt: sudo apt-get install -y %s' "$1" ;;
        Darwin) printf 'brew: brew install %s' "$2" ;;
        *) printf 'apt: sudo apt-get install -y %s / brew: brew install %s' "$1" "$2" ;;
    esac
}

check_dependencies() {
    log "==> Checking dependencies..."

    command -v docker > /dev/null 2>&1 \
        || MISSING+=("docker: not found — install from https://docs.docker.com/get-docker/")

    local need="2.24.4" ver
    if ver="$(docker compose version --short 2>/dev/null)"; then
        ver="${ver#v}"
        # Matches the version floor external/z3/scripts/regtest-init.sh itself
        # enforces (!override/COMPOSE_FILE merge support) — see that script's
        # own require_compose_v2 for the identical sort -V idiom.
        if [ -n "$ver" ] && [ "$(printf '%s\n%s\n' "$need" "$ver" | sort -V | head -n1)" != "$need" ]; then
            MISSING+=("docker compose >= ${need} required, found ${ver}")
        fi
    else
        MISSING+=("docker compose v2 plugin: not found — install/upgrade Docker (compose-plugin package or Docker Desktop)")
    fi

    command -v rage-keygen > /dev/null 2>&1 \
        || MISSING+=("rage-keygen: not found — 'cargo install rage', or see https://github.com/str4d/rage/releases (not apt/brew-installable under this name)")
    command -v openssl > /dev/null 2>&1 \
        || MISSING+=("openssl: not found — $(os_hint openssl openssl)")
    command -v curl > /dev/null 2>&1 \
        || MISSING+=("curl: not found — $(os_hint curl curl)")
    command -v jq > /dev/null 2>&1 \
        || MISSING+=("jq: not found — $(os_hint jq jq)")
    # scripts/dev/regtest-overrides/apply.sh's own header documents a GNU sed
    # requirement: its `sed -i 's|...|...|' file` calls are GNU syntax and
    # fail outright against macOS's default BSD `/usr/bin/sed` (which requires
    # an explicit -i backup-extension argument) — caught here, before Phase 2
    # reaches that script, rather than as a confusing mid-sequence failure.
    if sed --version > /dev/null 2>&1; then
        : # GNU sed prints a version banner and exits 0; BSD sed does not.
    else
        MISSING+=("GNU sed: 'sed' on PATH is not GNU sed (required by scripts/dev/regtest-overrides/apply.sh) — on macOS: brew install gnu-sed, then put \"\$(brew --prefix gnu-sed)/libexec/gnubin\" ahead of /usr/bin on PATH so 'sed' resolves to it")
    fi
    command -v cargo > /dev/null 2>&1 \
        || MISSING+=("cargo: not found — install Rust via https://rustup.rs")
    command -v cc > /dev/null 2>&1 \
        || MISSING+=("a C linker (cc): not found — $(os_hint build-essential "Xcode Command Line Tools (xcode-select --install)")")

    # libfontconfig1-dev/libssl-dev: pulled in transitively by plotters' `ttf`
    # feature and openssl-sys respectively — see docs/infra/gcp-test-machine.md.
    pkg-config --exists openssl 2>/dev/null \
        || MISSING+=("libssl-dev (openssl pkg-config file): not found — $(os_hint libssl-dev openssl)")
    pkg-config --exists fontconfig 2>/dev/null \
        || MISSING+=("libfontconfig1-dev (fontconfig pkg-config file): not found — $(os_hint libfontconfig1-dev fontconfig)")

    # ~20GB: sized to a full regtest chain history plus the Zebra/Zaino/Zallet
    # images, per the Track 2 isolation work's own measurements. Overridable
    # so a test can isolate the "every binary present" case from this
    # process's real, incidental free disk space.
    local required_kb="${Z3_BOOTSTRAP_MIN_DISK_KB:-20000000}" avail_kb
    avail_kb="$(df -Pk "$REPO_ROOT" | awk 'NR==2{print $4}')"
    if [ -z "${avail_kb:-}" ] || [ "$avail_kb" -lt "$required_kb" ]; then
        MISSING+=("disk space: $(( ${avail_kb:-0} / 1048576 ))GB available at ${REPO_ROOT}, ~$(( required_kb / 1048576 ))GB required")
    fi

    if command -v pandoc > /dev/null 2>&1; then
        log "    pandoc found — PDF report output (z3sim report --pdf) is available."
    else
        log "    pandoc not found — PDF report output (z3sim report --pdf) will be unavailable; Markdown reports are unaffected."
    fi

    if [ "${#MISSING[@]}" -gt 0 ]; then
        log ""
        log "Missing or insufficient dependencies:"
        for m in "${MISSING[@]}"; do
            log "  - $m"
        done
        exit 2
    fi
    log "    all required dependencies present."
}

check_dependencies

if [ "${1:-}" = "--check-only" ]; then
    exit 0
fi

# ── Phase 2: clone + apply the working override set ─────────────────────────
#
# `run_step` normalizes ANY non-zero exit from here on to exactly 1, per the
# exit-code contract documented above — several of these scripts use their
# own exit codes for their own purposes (e.g. zallet-release-image/build.sh
# exits 2 for an unsupported arch/unrecorded digest, a meaning specific to
# that script, not "missing dependency"), and `set -e` would otherwise
# propagate whatever raw code they happen to exit with verbatim as
# bootstrap.sh's own exit code.
run_step() {
    "$@" || exit 1
}

log ""
log "==> Cloning/checking out the pinned Z3 stack..."
run_step bash scripts/dev/clone-z3.sh

log ""
log "==> Applying the working (override) component set..."
run_step bash scripts/dev/regtest-overrides/apply.sh

# build.sh (unlike the other four scripts this sequences) does not itself
# check for an already-built image before re-fetching and re-building — guard
# it here so a bootstrap re-run doesn't needlessly re-download the release
# tarball. The lock file's `overrides:` section is the single source of truth
# for which version/tag that is (mirrors apply.sh's own lock_override, a
# small, low-drift-risk duplication of one awk snippet rather than sourcing
# shell functions across scripts).
LOCK_FILE="$REPO_ROOT/z3-commits.lock"
lock_override() {  # component field -> value
    awk -v comp="  $1:" -v field="$2:" '
        /^overrides:/ { o = 1 }
        o && $0 == comp { c = 1; next }
        o && c && /^  [a-z]+:$/ { c = 0 }
        o && c && $1 == field { print $2; exit }
    ' "$LOCK_FILE"
}
ZALLET_VERSION="$(lock_override zallet version)"
ZALLET_IMAGE="$(lock_override zallet image)"
[ -n "$ZALLET_VERSION" ] && [ -n "$ZALLET_IMAGE" ] || die "could not parse the zallet overrides section of $LOCK_FILE"

log ""
if docker image inspect "$ZALLET_IMAGE" > /dev/null 2>&1; then
    log "==> Zallet release image ${ZALLET_IMAGE} already built — skipping."
else
    log "==> Building the local Zallet release image (${ZALLET_VERSION} — no upstream image past alpha.3)..."
    run_step bash scripts/dev/zallet-release-image/build.sh "$ZALLET_VERSION"
fi

# ── Phase 3: build the simulator, bring the stack up, print versions ────────

log ""
log "==> Building the simulator binary..."
run_step cargo build

log ""
log "==> Starting the stack, preparing the wallet and miner, and confirming health..."
run_step ./target/debug/z3sim print-versions

log ""
log "Bootstrap complete. Run a scenario with:"
log "  ./target/debug/z3sim run --scenario configs/scenarios/smoke.yaml"
