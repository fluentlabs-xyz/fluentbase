# syntax=docker.io/docker/dockerfile:1.7-labs
# Devnet-only image — DO NOT use for production validators.
# Builds both `fluent` (runtime) and `genesis-bootstrap` (one-shot
# init for local 5-node DPoS smoke). cargo-chef is intentionally NOT
# used here: `crates/genesis/build.rs` imports `fluentbase_evm`
# symbols at build-script time, but cargo-chef stubs every workspace
# member's `lib.rs` during cook so the build script's import fails
# (`unresolved import fluentbase_evm::EthereumMetadata`). Production
# `docker/Dockerfile` works around this via the FEATURES arg; devnet
# smoke is a hand-tool so we accept the ~15 min cold-cache penalty
# instead of feature-gating around the bug.

FROM rust:1 AS builder
WORKDIR /app

LABEL org.opencontainers.image.source=https://github.com/fluentlabs-xyz/fluent
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
LABEL fluent.image.kind="devnet-smoke"

RUN apt-get update && apt-get install -y clang libclang-dev lld llvm pkg-config && \
    rm -rf /var/lib/apt/lists/*
# `secp256k1-sys` and other C-deps in `fluentbase-contracts/build.rs`'s
# nested wasm32 cargo invocation use `cc-rs`, which needs a real
# wasm32-capable C compiler — gcc doesn't target wasm. `clang` (with
# `lld` for the linker) is what CI uses (.github/workflows/ci.yml).
ENV CC_wasm32_unknown_unknown=clang
ENV CFLAGS_wasm32_unknown_unknown="--target=wasm32-unknown-unknown"
ENV AR_wasm32_unknown_unknown=llvm-ar

# Install the pinned toolchain + wasm32 std from `rust-toolchain` BEFORE the full
# source COPY, so a source edit does NOT bust the ~108s rustup-install layer (it
# depends only on the rust-toolchain file). `cat rust-toolchain` installs the pinned
# version EXPLICITLY (not the base image's default 1.95), and the file is present in
# WORKDIR so cargo honours the override for the build below.
COPY rust-toolchain .
RUN rustup toolchain install $(cat rust-toolchain) --target wasm32-unknown-unknown
# Split the source COPY so devnet smoke-harness churn (scripts/*.sh, compose
# *.yml, vendored contracts/, docs) does NOT bust the build. The first COPY
# excludes ALL of devnet/; the second adds back ONLY the genesis-bootstrap
# crate — the sole build input under devnet/ (verified: it is the only Cargo.toml
# there, and its build-time include_str! targets crates/genesis/, not this dir).
# Its siblings are host-side: scripts run on the host, contracts/ is a runtime
# bind mount (./contracts:/contracts:ro), compose/Makefile/README never enter the
# image. Editing a smoke script now leaves BOTH COPY layer digests unchanged ⇒ the
# cargo-build layer below stays cached (no ~90s `fluent` recompile); only a
# genesis-bootstrap or workspace-crate source edit recompiles, as it should.
COPY --exclude=.git --exclude=dist --exclude=devnet . .
COPY devnet/local-dpos-smoke/genesis-bootstrap devnet/local-dpos-smoke/genesis-bootstrap

ARG BUILD_PROFILE=release
ENV BUILD_PROFILE=$BUILD_PROFILE

# BuildKit cache mounts persist the cargo registry/git caches and the
# `target/` dir across builds, so external deps (reth et al. — the bulk
# of cold-build time) compile once and only changed crates recompile on
# subsequent runs. Because a cache mount is NOT part of the image layer,
# the binaries MUST be copied out of `target/` in the SAME `RUN` while
# the mount is still active — a separate `cp` step would see an empty
# `target/`. `sharing=locked` serialises the concurrent per-service
# builds docker-compose kicks off so they don't corrupt the shared cache.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --profile $BUILD_PROFILE --locked \
        --bin fluent --bin genesis-bootstrap \
        --features fluent/dpos-devnet-metrics,fluent/dpos-devnet-byzantine && \
    cp /app/target/$BUILD_PROFILE/fluent /app/fluent && \
    cp /app/target/$BUILD_PROFILE/genesis-bootstrap /app/genesis-bootstrap

FROM ubuntu AS runtime
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends curl && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/fluent /usr/local/bin/
COPY --from=builder /app/genesis-bootstrap /usr/local/bin/
COPY LICENSE-* ./

EXPOSE 8545 8546 9000
ENTRYPOINT ["/usr/local/bin/fluent"]
