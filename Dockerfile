# syntax=docker/dockerfile:1.7-labs

ARG RUST_TOOLCHAIN=1.88
ARG PLATFORM=linux/amd64
ARG SDK_VERSION=0.4.8-dev
ARG BINARYEN_VERSION=120
ARG WABT_VERSION=1.0.36
ARG WABT_OS=ubuntu-20.04

#######################################
# Stage 1: Base (Rust)                #
#######################################
FROM --platform=${PLATFORM} debian:bookworm-slim AS base
ARG RUST_TOOLCHAIN

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl ca-certificates build-essential pkg-config libssl-dev git \
    mold clang \
    && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /root/.cargo && \
    printf '[build]\nrustflags = ["-C","link-arg=-fuse-ld=mold"]\n\
[target.x86_64-unknown-linux-gnu]\nlinker = "clang"\n' \
    > /root/.cargo/config.toml

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --no-modify-path --default-toolchain ${RUST_TOOLCHAIN} \
    && ${CARGO_HOME}/bin/rustup target add wasm32-unknown-unknown \
    && ${CARGO_HOME}/bin/rustup component add rust-src \
    && ${CARGO_HOME}/bin/cargo install sccache --locked

ENV PATH="${CARGO_HOME}/bin:${PATH}" \
    CARGO_NET_RETRY=3 \
    CARGO_NET_GIT_FETCH_WITH_CLI=true \
    CARGO_INCREMENTAL=0 \
    RUST_BACKTRACE=1

#######################################
# Stage 2: External WebAssembly Tools #
#######################################
FROM alpine:3.19 AS tools

ARG BINARYEN_VERSION
ARG WABT_VERSION
ARG WABT_OS

RUN apk add --no-cache curl tar

RUN curl -L https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz \
    | tar xz -C /tmp

RUN curl -L https://github.com/WebAssembly/wabt/releases/download/${WABT_VERSION}/wabt-${WABT_VERSION}-${WABT_OS}.tar.gz \
    | tar xz -C /tmp

#######################################
# Stage 3: CLI Builder                #
#######################################
FROM base AS cli-builder
ARG RUST_TOOLCHAIN

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY bins/cli ./bins/cli/
COPY e2e/ ./e2e/

# Simple build without cache mount
RUN cargo build --bin fluentbase --release

#######################################
# Stage 4: Contract Builder           #
#######################################
FROM base AS contract-builder
ARG RUST_TOOLCHAIN

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY e2e/ ./e2e/
COPY bins/ ./bins/
COPY docker/contract ./docker/contract

WORKDIR /build/docker/contract

# Enable sccache for cache warming
ENV RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/usr/local/cargo/sccache

# Simple build without cache mount
RUN cargo build --release --target wasm32-unknown-unknown --no-default-features

#######################################
# Stage 5: Final SDK                  #
#######################################
FROM base AS final

ARG SDK_VERSION
ARG RUST_TOOLCHAIN
ARG BINARYEN_VERSION
ARG WABT_VERSION

# Copy WASM tools
COPY --from=tools /tmp/binaryen-version_*/bin/* /usr/local/bin/
COPY --from=tools /tmp/wabt-*/bin/* /usr/local/bin/

# Copy CLI binary - now this simply works!
COPY --from=cli-builder /build/target/release/fluentbase /usr/local/bin/

# Copy sccache binary
COPY --from=contract-builder /usr/local/cargo/bin/sccache /usr/local/bin/sccache

# Copy pre-warmed sccache cache
COPY --from=contract-builder /usr/local/cargo/sccache /usr/local/cargo/sccache

# Copy cargo registry and git (for faster user builds)
COPY --from=contract-builder /usr/local/cargo/registry /usr/local/cargo/registry
COPY --from=contract-builder /usr/local/cargo/git /usr/local/cargo/git

# Copy pre-built target directory
COPY --from=contract-builder /build/docker/contract/target /target

# Enable sccache for user builds
ENV RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/usr/local/cargo/sccache

WORKDIR /workspace

LABEL maintainer="Fluent Labs" \
      org.opencontainers.image.title="Fluentbase SDK (Optimized)" \
      org.opencontainers.image.description="Fluentbase CLI, Rust toolchain, and pre-compiled contract caches for rapid project builds." \
      org.opencontainers.image.source="https://github.com/fluentlabs-xyz/fluentbase" \
      org.opencontainers.image.version="${SDK_VERSION}" \
      io.fluentbase.sdk.version="${SDK_VERSION}" \
      io.fluentbase.rust.toolchain="${RUST_TOOLCHAIN}" \
      io.fluentbase.binaryen.version="${BINARYEN_VERSION}" \
      io.fluentbase.wabt.version="${WABT_VERSION}"

CMD ["/bin/bash"]
