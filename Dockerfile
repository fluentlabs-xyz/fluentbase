# syntax=docker/dockerfile:1.7-labs

ARG RUST_TOOLCHAIN=1.88
ARG PLATFORM=linux/amd64
ARG SDK_VERSION=0.4.10-dev

#######################################
# Stage 1: Build Fluentbase CLI
#######################################
FROM --platform=${PLATFORM} rust:${RUST_TOOLCHAIN}-slim AS builder

WORKDIR /build
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY . ./
RUN cargo build --bin fluentbase --release --locked

#######################################
# Stage 2: Cache Warmer
#######################################
FROM --platform=${PLATFORM} rust:${RUST_TOOLCHAIN}-slim AS cache-warmer
ARG SDK_VERSION

WORKDIR /warmup
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown && \
    cargo install sccache --locked

ENV RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/sccache-cache \
    SCCACHE_CACHE_SIZE="5G"

# Create minimal dummy contract
RUN printf '[package]\nname = "warmer"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\nfluentbase-sdk = { git = "https://github.com/fluentlabs-xyz/fluentbase", tag = "v%s", default-features = false }\n\n[lib]\ncrate-type = ["cdylib"]\npath = "lib.rs"' "${SDK_VERSION}" > Cargo.toml

RUN printf '#![cfg_attr(target_arch = "wasm32", no_std, no_main)]\nextern crate fluentbase_sdk;\nuse fluentbase_sdk::{entrypoint, SharedAPI};\npub fn main_entry(_: impl SharedAPI) {}\nentrypoint!(main_entry);' > lib.rs

RUN cargo build --release --target wasm32-unknown-unknown --no-default-features

#######################################
# Stage 3: Final SDK
#######################################
FROM --platform=${PLATFORM} rust:${RUST_TOOLCHAIN}-slim AS final
ARG SDK_VERSION
ARG RUST_TOOLCHAIN

WORKDIR /workspace

RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown

RUN git config --global fetch.depth 1 && \
    git config --global submodule.fetchDepth 1 && \
    git config --global submodule.fetchJobs 8 && \
    git config --global fetch.parallel 8

COPY --from=builder /build/target/release/fluentbase /usr/local/bin/fluentbase
COPY --from=cache-warmer /usr/local/cargo/bin/sccache /usr/local/bin/sccache
COPY --from=cache-warmer /usr/local/cargo/git/db /usr/local/cargo/git/db
COPY --from=cache-warmer /usr/local/cargo/registry/cache /usr/local/cargo/registry/cache
COPY --from=cache-warmer /usr/local/cargo/registry/index /usr/local/cargo/registry/index
COPY --from=cache-warmer /sccache-cache /sccache-cache

ENV RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/sccache-cache \
    SCCACHE_CACHE_SIZE="5G" \
    CARGO_NET_GIT_FETCH_WITH_CLI=true

LABEL maintainer="Fluent Labs" \
      org.opencontainers.image.title="Fluentbase SDK" \
      org.opencontainers.image.description="Fluentbase CLI with cached dependencies for v${SDK_VERSION}" \
      org.opencontainers.image.source="https://github.com/fluentlabs-xyz/fluentbase" \
      org.opencontainers.image.version="${SDK_VERSION}" \
      io.fluentbase.sdk.version="${SDK_VERSION}" \
      io.fluentbase.rust.toolchain="${RUST_TOOLCHAIN}"

CMD ["/bin/bash"]
