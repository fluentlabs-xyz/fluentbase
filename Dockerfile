# syntax=docker/dockerfile:1.7-labs

# Global build arguments
ARG RUST_TOOLCHAIN=nightly-2025-01-27
ARG PLATFORM=linux/amd64
ARG SDK_VERSION=0.3.6-dev
ARG BINARYEN_VERSION=120
ARG WABT_VERSION=1.0.36
ARG WABT_OS=ubuntu-20.04

#####################################
# Stage 1: Base with Rust Toolchain #
#####################################
FROM --platform=${PLATFORM} debian:bookworm-slim AS base

ENV DEBIAN_FRONTEND=noninteractive

# Install essential system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install Rust using rustup with specified toolchain
ARG RUST_TOOLCHAIN
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo

RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --no-modify-path --default-toolchain ${RUST_TOOLCHAIN} \
    && ${CARGO_HOME}/bin/rustup target add wasm32-unknown-unknown \
    && ${CARGO_HOME}/bin/rustup component add rust-src

# Update PATH to include Rust binaries
ENV PATH="${CARGO_HOME}/bin:${PATH}"

#############################################
# Stage 2: External WebAssembly Tools       #
#############################################
FROM alpine:3.19 AS tools

ARG BINARYEN_VERSION
ARG WABT_VERSION
ARG WABT_OS

# Install required dependencies
RUN apk add --no-cache curl tar

# Download and extract Binaryen
RUN curl -L https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz \
    | tar xz -C /tmp

# Download and extract WABT
RUN curl -L https://github.com/WebAssembly/wabt/releases/download/${WABT_VERSION}/wabt-${WABT_VERSION}-${WABT_OS}.tar.gz \
    | tar xz -C /tmp

######################################
# Stage 3: Builder (with cargo-chef) #
######################################
FROM base AS builder

WORKDIR /build
ENV CARGO_TARGET_DIR=/target

# Install cargo-chef to cache Rust dependencies
RUN cargo install cargo-chef --locked

# Copy workspace manifests first for optimal caching
COPY Cargo.toml Cargo.lock ./
COPY bins/ ./bins/
COPY contracts/ ./contracts/
COPY crates/ ./crates/
COPY examples/ ./examples/
COPY revm/ ./revm/
COPY e2e/ ./e2e/

# Generate and build dependencies cache
RUN cargo chef prepare --recipe-path recipe.json \
    && cargo chef cook --recipe-path recipe.json

# Copy entire source code to compile the binaries
COPY . .

# Build Fluentbase CLI
RUN cargo install --path ./bins/cli --profile release

# Compile Greeting example contract to pre-warm Fluentbase dependencies
RUN cargo build --release -p fluentbase-examples-greeting --target wasm32-unknown-unknown --no-default-features

######################################
# Stage 4: Final SDK (Pre-warmed)    #
######################################
FROM base AS final

ARG SDK_VERSION
ARG RUST_TOOLCHAIN
ARG BINARYEN_VERSION
ARG WABT_VERSION

# Copy Binaryen and WABT binaries from the tools stage
COPY --from=tools /tmp/binaryen-version_*/bin/* /usr/local/bin/
COPY --from=tools /tmp/wabt-*/bin/* /usr/local/bin/

# Copy Fluentbase CLI binary from the builder stage
COPY --from=builder /usr/local/cargo/bin/fluentbase* /usr/local/bin/

# Copy pre-warmed Cargo dependency caches from the builder stage
COPY --from=builder /usr/local/cargo/registry /usr/local/cargo/registry
COPY --from=builder /usr/local/cargo/git /usr/local/cargo/git
COPY --from=builder /target /target

# Ensure all binaries are executable
RUN chmod +x /usr/local/bin/*

# Set working directory
WORKDIR /workspace

# Add metadata labels for clarity and maintenance
LABEL maintainer="Fluent Labs" \
      org.opencontainers.image.title="Fluentbase SDK (Pre-warmed)" \
      org.opencontainers.image.description="Fluentbase CLI, Rust toolchain, and pre-compiled caches for rapid project builds." \
      org.opencontainers.image.source="https://github.com/fluentlabs-xyz/fluentbase" \
      org.opencontainers.image.version="${SDK_VERSION}" \
      io.fluentbase.sdk.version="${SDK_VERSION}" \
      io.fluentbase.rust.toolchain="${RUST_TOOLCHAIN}" \
      io.fluentbase.binaryen.version="${BINARYEN_VERSION}" \
      io.fluentbase.wabt.version="${WABT_VERSION}"

# Default entrypoint: interactive shell
CMD ["/bin/bash"]
