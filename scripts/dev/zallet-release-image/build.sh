#!/usr/bin/env bash
# build.sh: build a local container image from an official Zallet release tarball.
#
# Upstream publishes container images only through v0.1.0-alpha.3. Later releases
# ship source and prebuilt binaries but no image, and the simulator needs
# v0.1.0-beta.1 or later (the first release whose z_sendmany can select
# transparent inputs). This script fetches the release tarball for the host
# architecture, verifies its digest against the value recorded here, and
# builds a thin image around the binaries.
#
# Usage:
#   scripts/dev/zallet-release-image/build.sh [version] [image-tag]
#
# Defaults: version v0.1.0-beta.2, tag z3sim/zallet:<version>.
#
# Point the Z3 stack at the result by setting both of these in
# external/z3/.env.regtest:
#   Z3_ZALLET_IMAGE=z3sim/zallet:v0.1.0-beta.2
#   DOCKER_PLATFORM=linux/arm64      # or linux/amd64; the compose default is amd64
#
# Requires: docker, curl, tar, sha256sum. Uses gh for the download when present
# (it handles release asset redirects), otherwise curl.

set -euo pipefail

VERSION="${1:-v0.1.0-beta.2}"
IMAGE_TAG="${2:-z3sim/zallet:${VERSION}}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Default to the architecture the Z3 compose stack pins. Its `platform:` keys all
# read a single shared DOCKER_PLATFORM (default linux/amd64), and Zaino publishes
# amd64 only, so the whole stack has to agree: building a native arm64 Zallet
# would mean one native service among emulated ones, which makes latency
# measurements incomparable. Override with ZALLET_IMAGE_ARCH=arm64 for a native
# build when measurement parity does not matter.
ARCH="${ZALLET_IMAGE_ARCH:-amd64}"
case "$ARCH" in
    amd64 | arm64) ;;
    *)
        echo "unsupported ZALLET_IMAGE_ARCH: $ARCH (expected amd64 or arm64)" >&2
        exit 2
        ;;
esac

# Digests of the upstream release assets, so a tampered or truncated download
# fails loudly instead of producing a working-looking image. Extend this table
# when bumping VERSION; an unknown (version, arch) pair is a hard error rather
# than a silent skip, because an unverified wallet binary is the thing this
# check exists to prevent.
declare -A SHA256=(
    # Observed from the published assets on 2026-07-28. These pin what we built
    # and tested against; they are not a substitute for verifying upstream's
    # detached signature (`.asc`) or SLSA provenance on first adoption.
    [v0.1.0-beta.1-amd64]="d31c4d38ca27b620db5a7c7a40950786a877013ce68d5b5ed417691ac2db0922"
    [v0.1.0-beta.1-arm64]="230de200a4a4e6064945cf1609127e8d1a82f4434a049ddb1faf75ca3823b0be"
    # Observed from the published assets on 2026-07-31 (release published
    # 2026-07-28). beta.2 fixes the Zallet restart crash-loop documented in
    # docs/zallet-restart-sync-failure.md (upstream zcash/zallet#598/#599).
    [v0.1.0-beta.2-amd64]="c82d160f9e57905f481a8cba5ea26375856fa54de7c8571017c7c0e6767dbe2a"
    [v0.1.0-beta.2-arm64]="cee04b878f7d0dbaf3964164974975eac9c73d68b498cee07ea576282b08fa5e"
)

ASSET="zallet-${VERSION}-linux-${ARCH}.tar.gz"
EXPECTED="${SHA256[${VERSION}-${ARCH}]:-}"
if [ -z "$EXPECTED" ]; then
    echo "No recorded SHA-256 for $ASSET." >&2
    echo "Add one to the SHA256 table in $0 before building." >&2
    exit 2
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> Fetching $ASSET"
# gh handles release-asset redirects best, but an installed-yet-unauthenticated
# gh refuses even public downloads — fall back to curl in that case.
if command -v gh > /dev/null 2>&1 && gh auth status > /dev/null 2>&1; then
    gh release download "$VERSION" --repo zcash/zallet --pattern "$ASSET" --dir "$WORK"
else
    curl -fsSL -o "$WORK/$ASSET" \
        "https://github.com/zcash/zallet/releases/download/${VERSION}/${ASSET}"
fi

ACTUAL="$(sha256sum "$WORK/$ASSET" | awk '{print $1}')"
if [ "$EXPECTED" = "UNKNOWN" ]; then
    echo "No digest recorded for this asset yet. Observed SHA-256:" >&2
    echo "  $ACTUAL" >&2
    echo "Verify it against upstream's signature, then record it in $0." >&2
    exit 1
fi
if [ "$ACTUAL" != "$EXPECTED" ]; then
    echo "SHA-256 mismatch for $ASSET" >&2
    echo "  expected $EXPECTED" >&2
    echo "  actual   $ACTUAL" >&2
    exit 1
fi
echo "    digest OK"

echo "==> Unpacking"
tar xzf "$WORK/$ASSET" -C "$WORK"
mkdir -p "$WORK/ctx/bin"
cp "$WORK/zallet-${VERSION}-linux-${ARCH}"/zallet \
    "$WORK/zallet-${VERSION}-linux-${ARCH}"/zallet-zebra \
    "$WORK/zallet-${VERSION}-linux-${ARCH}"/zallet-zaino \
    "$WORK/ctx/bin/"
cp "$SCRIPT_DIR/Dockerfile" "$WORK/ctx/Dockerfile"

echo "==> Building $IMAGE_TAG (linux/$ARCH)"
docker build \
    --platform "linux/$ARCH" \
    --build-arg "ZALLET_VERSION=$VERSION" \
    -t "$IMAGE_TAG" \
    "$WORK/ctx"

echo ""
echo "Built $IMAGE_TAG"
echo "Point the stack at it by setting in external/z3/.env.regtest:"
echo "    Z3_ZALLET_IMAGE=$IMAGE_TAG"
echo "    DOCKER_PLATFORM=linux/$ARCH"
