# Global build arguments for version control
ARG RUST_VERSION=1.78.0
ARG SDK_VERSION=0.1.0-dev
ARG BINARYEN_VERSION=120
ARG WABT_VERSION=1.0.36
# The OS-specific part of the WABT release asset name
ARG WABT_OS=ubuntu-20.04

# ==============================================================================
# Stage 1: Base
# Purpose: Sets up the base OS and installs the specified Rust toolchain.
# ==============================================================================
FROM debian:bookworm-slim AS base

ARG RUST_VERSION

# Install essential dependencies for building and for rustup
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install Rust using rustup for version flexibility
ENV PATH="/root/.cargo/bin:${PATH}"
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain ${RUST_VERSION}
RUN rustup target add wasm32-unknown-unknown && \
    rustup component add rust-src

# ==============================================================================
# Stage 2: Tools
# Purpose: Downloads and unpacks external binary tools in an isolated stage.
# ==============================================================================
FROM alpine:3.19 AS tools

ARG BINARYEN_VERSION
ARG WABT_VERSION
ARG WABT_OS

RUN apk add --no-cache curl && \
    curl -L https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz | \
      tar xzf - -C /tmp && \
    curl -L https://github.com/WebAssembly/wabt/releases/download/${WABT_VERSION}/wabt-${WABT_VERSION}-${WABT_OS}.tar.gz | \
      tar xzf - -C /tmp

# ==============================================================================
# Stage 3: Builder
# Purpose: Compiles the project using cargo-chef and Docker BuildKit caching
# for maximum performance and optimal layer size.
# ==============================================================================
FROM base AS builder

# Install cargo-chef. The --mount cache will speed up this download on subsequent runs.
RUN --mount=type=cache,target=/root/.cargo/registry \
    cargo install cargo-chef --locked

WORKDIR /build

# 1. Prepare the recipe.
# For complex workspaces, we copy the entire directories of workspace members.
# This ensures all Cargo.toml files and their associated src folders are present,
# which is required for `cargo metadata` to parse the project structure correctly.
COPY Cargo.toml Cargo.lock ./
COPY bins/ ./bins/
COPY contracts/ ./contracts/
COPY crates/ ./crates/
COPY e2e/ ./e2e/
COPY examples/ ./examples/
COPY revm/ ./revm/

RUN cargo chef prepare --recipe-path recipe.json

# 2. Cook dependencies using the BuildKit cache mounts.
# This keeps the resulting image layer small and speeds up builds dramatically.
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/target \
    cargo chef cook --recipe-path recipe.json --target-dir /target

# 3. Build the application, re-using the same caches.
# The full source code is already present from the previous COPY steps.
# This final build step will be very fast.
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/target \
    cargo install --path ./bins/cli --target-dir /target

# ==============================================================================
# Stage 4: Final SDK Image
# Purpose: A distributable SDK image containing the Rust toolchain and all
# necessary binaries. Defaults to an interactive shell.
# ==============================================================================
FROM base AS final

ARG SDK_VERSION
ARG RUST_VERSION
ARG BINARYEN_VERSION
ARG WABT_VERSION

# Copy pre-built tools and the main application binary
COPY --from=tools /tmp/binaryen-version_*/bin/* /usr/local/bin/
COPY --from=tools /tmp/wabt-*/bin/* /usr/local/bin/
COPY --from=builder /root/.cargo/bin/fluentbase* /usr/local/bin/

RUN chmod +x /usr/local/bin/*

WORKDIR /workspace

LABEL maintainer="Fluent Labs" \
      org.opencontainers.image.title="Fluentbase SDK" \
      org.opencontainers.image.description="A complete SDK containing the Fluentbase CLI, Rust toolchain, and WASM tools" \
      org.opencontainers.image.source="https://github.com/fluentlabs-xyz/fluentbase" \
      org.opencontainers.image.version="${SDK_VERSION}" \
      io.fluentbase.sdk.version="${SDK_VERSION}" \
      io.fluentbase.rust.version="${RUST_VERSION}" \
      io.fluentbase.binaryen.version="${BINARYEN_VERSION}" \
      io.fluentbase.wabt.version="${WABT_VERSION}"

# Provide an interactive shell by default
CMD ["/bin/bash"]
