# Global build argument
ARG BINARYEN_VERSION=120

# Builder stage
FROM rust:1.87-slim AS builder

# Re-declare ARG in this stage
ARG BINARYEN_VERSION

# Install all dependencies including wabt from apt
RUN apt-get update && apt-get install -y \
    curl \
    git \
    pkg-config \
    libssl-dev \
    build-essential \
    python3 \
    cmake \
    clang \
    lld \
    ca-certificates \
    wabt \
    && rm -rf /var/lib/apt/lists/*

# Set environment variables for better network handling
ENV CARGO_NET_RETRY=10 \
    CARGO_HTTP_TIMEOUT=120

# Install Rust components
RUN rustup component add rust-src rustfmt clippy && \
    rustup target add wasm32-unknown-unknown

# Install wasm-opt from binaryen
RUN curl -L --retry 5 --retry-delay 5 \
    https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz | \
    tar xzf - -C /opt && \
    ln -s /opt/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt /usr/local/bin/

# Install cargo tools
RUN cargo install --locked \
    cargo-expand \
    cargo-udeps \
    cargo-audit \
    cargo-deny

# Runtime stage
FROM rust:1.87-slim

# Re-declare ARG for this stage
ARG BINARYEN_VERSION

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    git \
    pkg-config \
    libssl-dev \
    build-essential \
    clang \
    lld \
    ca-certificates \
    wabt \
    && rm -rf /var/lib/apt/lists/*

# Copy Rust toolchain and cargo from builder
COPY --from=builder /usr/local/rustup /usr/local/rustup
COPY --from=builder /usr/local/cargo /usr/local/cargo

# Copy wasm-opt from builder
COPY --from=builder /opt/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt /usr/local/bin/

# Set environment variables
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    RUST_BACKTRACE=1

# Create workspace directory
WORKDIR /workspace

# Configure cargo for better caching
RUN mkdir -p /usr/local/cargo && \
    echo '[build]' > /usr/local/cargo/config.toml && \
    echo 'target-dir = "/target"' >> /usr/local/cargo/config.toml && \
    echo '[net]' >> /usr/local/cargo/config.toml && \
    echo 'retry = 10' >> /usr/local/cargo/config.toml

# Verify installations
RUN wasm-opt --version && \
    wasm2wat --version && \
    wasm-strip --version && \
    rustc --version && \
    cargo --version

# Labels
LABEL maintainer="Fluent Labs" \
      description="Fluent smart contract build environment"

CMD ["/bin/bash"]
