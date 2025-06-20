# Global build arguments
ARG BINARYEN_VERSION=120

# Stage 1: Base dependencies (most stable)
FROM rust:1.87-slim AS base-deps

# Install only essential runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    ca-certificates \
    # Required for building C dependencies
    build-essential \
    # Required for cargo dependencies with native code
    pkg-config \
    libssl-dev \
    # Tool for downloading
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Install Rust target for WASM and rust-src component
RUN rustup target add wasm32-unknown-unknown && \
    rustup component add rust-src

# Stage 2: Tools (changes infrequently)
FROM alpine:3.19 AS tools

ARG BINARYEN_VERSION

# Install curl and download tools
RUN apk add --no-cache curl && \
    # Download binaryen
    curl -L https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz | \
    tar xzf - -C /tmp && \
    # Download wabt
    curl -L https://github.com/WebAssembly/wabt/releases/download/1.0.36/wabt-1.0.36-ubuntu-20.04.tar.gz | \
    tar xzf - -C /tmp

# Stage 3: Build and cache dependencies
FROM base-deps AS builder

# Copy the entire workspace
WORKDIR /build
COPY . .

# Fetch all workspace dependencies and build only the CLI
RUN cargo fetch && \
    # Build and install the CLI (it's small and needed)
    cargo install --path ./bins/cli

# Stage 4: Final image
FROM base-deps AS final

# Copy tools from tools stage
COPY --from=tools /tmp/binaryen-version_*/bin/wasm-opt /usr/local/bin/
COPY --from=tools /tmp/wabt-*/bin/wasm2wat /usr/local/bin/
COPY --from=tools /tmp/wabt-*/bin/wasm-strip /usr/local/bin/

# Copy CLI from builder stage
COPY --from=builder /usr/local/cargo/bin/fluentbase* /usr/local/bin/

# Copy cached dependencies from builder stage
COPY --from=builder /usr/local/cargo/registry /usr/local/cargo/registry
COPY --from=builder /usr/local/cargo/git /usr/local/cargo/git

# Make binaries executable
RUN chmod +x /usr/local/bin/wasm* && \
    chmod +x /usr/local/bin/fluentbase*

# Set environment variables
ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_TIMEOUT=120 \
    RUST_BACKTRACE=1 \
    CARGO_TARGET_DIR=/target

# Create workspace directory
WORKDIR /workspace

# Configure cargo for better caching
RUN mkdir -p $CARGO_HOME && \
    echo '[build]' > $CARGO_HOME/config.toml && \
    echo 'target-dir = "/target"' >> $CARGO_HOME/config.toml && \
    echo '' >> $CARGO_HOME/config.toml && \
    echo '[net]' >> $CARGO_HOME/config.toml && \
    echo 'retry = 10' >> $CARGO_HOME/config.toml && \
    echo '' >> $CARGO_HOME/config.toml && \
    echo '[registries.crates-io]' >> $CARGO_HOME/config.toml && \
    echo 'protocol = "sparse"' >> $CARGO_HOME/config.toml

# Verify installations
RUN rustc --version && \
    cargo --version && \
    wasm-opt --version && \
    wasm2wat --version && \
    fluentbase --version && \
    rustup component list --installed

# Labels
LABEL maintainer="Fluent Labs" \
      description="Fluent smart contract build environment" \
      org.opencontainers.image.source="https://github.com/fluentlabs-xyz/fluentbase"

CMD ["/bin/bash"]
