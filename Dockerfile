# syntax=docker/dockerfile:1.7-labs
ARG RUST_TOOLCHAIN=1.88
ARG SDK_VERSION_BRANCH=""
ARG SDK_VERSION_TAG=""

#######################################
# Stage 0: Base with common dependencies
#######################################
FROM rust:${RUST_TOOLCHAIN}-slim AS base
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev git ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown

RUN curl -fsSL https://github.com/mozilla/sccache/releases/download/v0.12.0/sccache-v0.12.0-x86_64-unknown-linux-musl.tar.gz \
    | tar xz --strip-components=1 -C /usr/local/bin sccache-v0.12.0-x86_64-unknown-linux-musl/sccache && \
    chmod +x /usr/local/bin/sccache

ENV RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/sccache-cache \
    SCCACHE_CACHE_SIZE="5G"


#######################################
# Stage 1: Build Fluentbase CLI
#######################################
FROM base AS builder
WORKDIR /build
COPY . ./
RUN cargo build --bin fluentbase --release --locked

#######################################
# Stage 2: Cache Warmer
#######################################
FROM base AS cache-warmer
ARG SDK_VERSION_BRANCH
ARG SDK_VERSION_TAG
WORKDIR /warmup

COPY --from=builder /usr/local/cargo/git/db /usr/local/cargo/git/db
COPY --from=builder /usr/local/cargo/registry /usr/local/cargo/registry
COPY --from=builder /sccache-cache /sccache-cache

# Cargo.toml
RUN if [ -n "$SDK_VERSION_BRANCH" ]; then \
      SDK_VERSION="branch = \"$SDK_VERSION_BRANCH\""; \
    elif [ -n "$SDK_VERSION_TAG" ]; then \
      SDK_VERSION="tag = \"$SDK_VERSION_TAG\""; \
    else \
      echo "❌ Either SDK_VERSION_BRANCH or SDK_VERSION_TAG must be provided" && exit 1; \
    fi && \
    printf '[package]\nname = "warmer"\nversion = "0.1.0"\nedition = "2021"\n\n[lib]\ncrate-type = ["cdylib"]\npath = "lib.rs"\n\n[dependencies]\nfluentbase-sdk = { git = "https://github.com/fluentlabs-xyz/fluentbase", %s, default-features = false }\n\n[features]\ndefault = []\nstd = ["fluentbase-sdk/std"]\n\n[profile.release]\nopt-level = "z"\nlto = true\npanic = "abort"\ncodegen-units = 1\n' "$SDK_VERSION" > Cargo.toml

# lib.rs
RUN printf '#![cfg_attr(not(feature = "std"), no_std, no_main)]\n\nextern crate alloc;\nextern crate fluentbase_sdk;\n\nuse fluentbase_sdk::{\n    basic_entrypoint,\n    derive::{router, Contract},\n    SharedAPI,\n    U256,\n};\n\n#[derive(Contract, Default)]\nstruct Warmer<SDK> {\n    sdk: SDK,\n}\n\npub trait WarmerAPI {\n    fn warm(&self) -> U256;\n}\n\n#[router(mode = "solidity")]\nimpl<SDK: SharedAPI> WarmerAPI for Warmer<SDK> {\n    fn warm(&self) -> U256 {\n        U256::from(2)\n    }\n}\n\nimpl<SDK: SharedAPI> Warmer<SDK> {\n    pub fn deploy(&self) {}\n}\n\nbasic_entrypoint!(Warmer);\n' > lib.rs

RUN cargo fetch --target wasm32-unknown-unknown && \
    cargo build --release --target wasm32-unknown-unknown --lib

#######################################
# Stage 3: Final SDK
#######################################
FROM rust:${RUST_TOOLCHAIN}-slim AS final
WORKDIR /workspace
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown

RUN git config --global submodule.fetchJobs 8 && \
    git config --global fetch.parallel 8

COPY --from=builder /build/target/release/fluentbase /usr/local/bin/fluentbase
COPY --from=cache-warmer /usr/local/bin/sccache /usr/local/bin/sccache
COPY --from=cache-warmer /usr/local/cargo/git/db /usr/local/cargo/git/db
COPY --from=cache-warmer /usr/local/cargo/registry/cache /usr/local/cargo/registry/cache
COPY --from=cache-warmer /usr/local/cargo/registry/index /usr/local/cargo/registry/index
COPY --from=cache-warmer /sccache-cache /sccache-cache

ENV RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/sccache-cache \
    SCCACHE_CACHE_SIZE="5G" \
    CARGO_NET_GIT_FETCH_WITH_CLI=true

CMD ["/bin/bash"]
