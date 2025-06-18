# Global build argument
ARG BINARYEN_VERSION=120

# Stage 1: Download and extract tools
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

# Stage 2: Final image
FROM rust:1.87-slim

# Install only essential runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    ca-certificates \
    # Required for building C dependencies
    build-essential \
    # Required for cargo dependencies with native code
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Copy only the tools we need from the tools stage
COPY --from=tools /tmp/binaryen-version_*/bin/wasm-opt /usr/local/bin/
COPY --from=tools /tmp/wabt-*/bin/wasm2wat /usr/local/bin/
COPY --from=tools /tmp/wabt-*/bin/wasm-strip /usr/local/bin/

# Make binaries executable
RUN chmod +x /usr/local/bin/wasm*

# Install Rust target for WASM and rust-src component
RUN rustup target add wasm32-unknown-unknown && \
    rustup component add rust-src

# Set environment variables
ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_TIMEOUT=120 \
    RUST_BACKTRACE=1

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
    rustup component list --installed

# Labels
LABEL maintainer="Fluent Labs" \
      description="Fluent smart contract build environment" \
      org.opencontainers.image.source="https://github.com/fluentlabs-xyz/fluentbase"

CMD ["/bin/bash"]
