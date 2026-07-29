# A thin runtime image around an official Zallet release tarball.
#
# Upstream publishes container images only up to v0.1.0-alpha.3; every release
# after that (alpha.4, beta.1) ships source and prebuilt binaries but no image.
# The simulator needs beta.1 to spend transparent funds at all, so this wraps
# the published binaries rather than building Zallet from source.
#
# The release binaries are statically linked, so the base image only has to
# supply a filesystem, a shell for debugging, and the uid the Z3 compose stack
# runs Zallet as (1000:1000).
#
# Build via scripts/dev/zallet-release-image/build.sh, which fetches and
# verifies the tarball; it is not buildable from this directory alone.

FROM debian:bookworm-slim

ARG ZALLET_VERSION

# Which backend binary to run. beta.1 splits Zallet into `zallet-zebra` (reads
# Zebra's state directly) and `zallet-zaino` (talks to a Zaino indexer), with a
# `zallet` dispatcher that defaults to the zebra-state backend. The Z3 stack
# runs a Zaino indexer and configures Zallet with `[indexer] validator_address`,
# so it needs the zaino backend; the dispatcher's default fails at startup with
# "the zebra-state backend requires an [indexer.read_state_service] config
# section".
ARG ZALLET_BACKEND=zallet-zaino

# Zallet's embedded Zaino client builds a reqwest client at startup, and reqwest
# reads the system trust store even when every request is plain HTTP to a
# container on the same network. Without this the wallet dies with
# "No CA certificates were loaded from the system".
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates; \
    rm -rf /var/lib/apt/lists/*

# The tarball unpacks to zallet-<version>-linux-<arch>/{zallet,zallet-zebra,zallet-zaino}.
# `zallet` is a small dispatcher that execs the backend-specific binary, so all
# three must land side by side on PATH.
COPY bin/ /usr/local/bin/

# The compose stack mounts a named volume at /var/lib/zallet and runs as
# 1000:1000, matching the upstream image's user and working directory so this
# image is a drop-in replacement.
#
# `zallet-backend` is the entrypoint: the Z3 compose service supplies only
# arguments via `command:`, so it cannot select a backend itself. The indirection
# lets one Dockerfile serve either backend, and `zallet` stays on PATH so
# dispatcher-style invocations still work.
#
# Runs as root: the symlink and chown need write access to paths uid 1000 does
# not own. USER drops privileges below, before the entrypoint runs.
RUN set -eux; \
    chmod 0755 /usr/local/bin/zallet /usr/local/bin/zallet-zebra /usr/local/bin/zallet-zaino; \
    ln -sf "/usr/local/bin/${ZALLET_BACKEND}" /usr/local/bin/zallet-backend; \
    mkdir -p /var/lib/zallet; \
    chown 1000:1000 /var/lib/zallet; \
    /usr/local/bin/zallet-backend --version

LABEL org.opencontainers.image.title="zallet (unofficial release-tarball image)" \
      org.opencontainers.image.version="${ZALLET_VERSION}" \
      org.opencontainers.image.source="https://github.com/zcash/zallet" \
      org.opencontainers.image.description="Official Zallet release binaries in a minimal image. Not published by the Zallet team."

USER 1000:1000
WORKDIR /var/lib/zallet
ENTRYPOINT ["zallet-backend"]
