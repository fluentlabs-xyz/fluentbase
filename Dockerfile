# Global build arguments for version control
ARG RUST_VERSION=1.78.0
ARG SDK_VERSION=0.1.0-dev
ARG BINARYEN_VERSION=120
ARG WABT_VERSION=1.0.36
ARG WABT_OS=ubuntu-20.04

# ==============================================================================
# Stage 1: Base
# Purpose: Sets up a clean OS with the specified Rust toolchain.
# ==============================================================================
FROM debian:bookworm-slim AS base

ARG RUST_VERSION

# Install essential system dependencies for Rust and native code compilation.
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install Rust using rustup for version flexibility.
ENV PATH="/root/.cargo/bin:${PATH}"
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain ${RUST_VERSION}
RUN rustup target add wasm32-unknown-unknown && rustup component add rust-src

# ==============================================================================
# Stage 2: Tools
# Purpose: Downloads and unpacks external binary tools in an isolated stage.
# ==============================================================================
FROM alpine:3.19 AS tools

ARG BINARYEN_VERSION
ARG WABT_VERSION
ARG WABT_OS

RUN apk add --no-cache curl && \
    curl -L https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz | tar xzf - -C /tmp && \
    # ИСПРАВЛЕНИЕ ЗДЕСЬ: было ${WBT_OS}, стало ${WABT_OS}
    curl -L https://github.com/WebAssembly/wabt/releases/download/${WABT_VERSION}/wabt-${WABT_VERSION}-${WABT_OS}.tar.gz | tar xzf - -C /tmp

# ==============================================================================
# Stage 3: Builder
# Purpose: Compiles the project and all its dependencies. The results will be
# "baked" into the final image to create a pre-warmed SDK for fast first-time builds.
# ==============================================================================
FROM base AS builder

RUN cargo install cargo-chef --locked

WORKDIR /build
ENV CARGO_TARGET_DIR=/target

# 1. Copy workspace manifests and source code.
COPY Cargo.toml Cargo.lock ./
COPY bins/ ./bins/
COPY contracts/ ./contracts/
COPY crates/ ./crates/
COPY e2e/ ./e2e/
COPY examples/ ./examples/
COPY revm/ ./revm/

# 2. Prepare and cook dependencies.
RUN cargo chef prepare --recipe-path recipe.json
RUN cargo chef cook --recipe-path recipe.json

# 3. Build the CLI tool.
COPY . .
RUN cargo install --path ./bins/cli

# ==============================================================================
# Stage 4: Final SDK Image
# Purpose: A distributable, pre-warmed SDK image. It contains the toolchain,
# binaries, and all pre-compiled dependency caches for an optimal user experience.
# ==============================================================================
FROM base AS final

ARG SDK_VERSION
ARG RUST_VERSION
ARG BINARYEN_VERSION
ARG WABT_VERSION

# Copy pre-built tools and the main application binary from previous stages.
COPY --from=tools /tmp/binaryen-version_*/bin/* /usr/local/bin/
COPY --from=tools /tmp/wabt-*/bin/* /usr/local/bin/
COPY --from=builder /root/.cargo/bin/fluentbase* /usr/local/bin/

# --- This is the core optimization for fast first-time builds ---
# Copy the pre-warmed dependency caches into the final image.
COPY --from=builder /root/.cargo/registry /root/.cargo/registry
COPY --from=builder /root/.cargo/git /root/.cargo/git

ENV CARGO_TARGET_DIR=/target
COPY --from=builder /target ${CARGO_TARGET_DIR}
# --- End of optimization section ---

RUN chmod +x /usr/local/bin/*

WORKDIR /workspace

LABEL maintainer="Fluent Labs" \
      org.opencontainers.image.title="Fluentbase SDK (Pre-warmed)" \
      org.opencontainers.image.description="A complete SDK containing the Fluentbase CLI, Rust toolchain, and all pre-compiled dependency caches for a fast development experience." \
      org.opencontainers.image.source="https://github.com/fluentlabs-xyz/fluentbase" \
      org.opencontainers.image.version="${SDK_VERSION}" \
      io.fluentbase.sdk.version="${SDK_VERSION}" \
      io.fluentbase.rust.version="${RUST_VERSION}" \
      io.fluentbase.binaryen.version="${BINARYEN_VERSION}" \
      io.fluentbase.wabt.version="${WABT_VERSION}"

# Provide an interactive shell by default for the best user experience.
CMD ["/bin/bash"]
