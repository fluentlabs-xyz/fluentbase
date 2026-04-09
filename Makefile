all: check build

.PHONY: check
check:
	cargo check --all

.PHONY: clippy
clippy:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy --manifest-path=./contracts/Cargo.toml --workspace --all-targets -- -D warnings
	cargo clippy --manifest-path=./examples/Cargo.toml --workspace --all-targets -- -D warnings

.PHONY: pr
pr: clippy test

.PHONY: build
build:
	cargo build --all

.PHONY: update-deps
update-deps:
	cargo update --manifest-path=./contracts/Cargo.toml revm
	cargo update --manifest-path=./examples/Cargo.toml revm
	cargo update revm
	cargo update --manifest-path=./evm-e2e/Cargo.toml revm
	cargo update --manifest-path=./contracts/Cargo.toml rwasm
	cargo update --manifest-path=./examples/Cargo.toml rwasm
	cargo update rwasm
	cargo update --manifest-path=./evm-e2e/Cargo.toml rwasm

.PHONY: clean
clean:
	cargo clean --manifest-path=./contracts/Cargo.toml
	cargo clean --manifest-path=./examples/Cargo.toml
	cargo clean
	cargo clean --manifest-path=./evm-e2e/Cargo.toml

TEST_PROFILE ?=
TEST_FEATURES ?=

.PHONY: run-e2e-tests
run-e2e-tests:
	cargo nextest run --manifest-path=./Cargo.toml --workspace $(TEST_PROFILE) --no-default-features --features $(TEST_FEATURES)
	$(MAKE) -C evm-e2e sync_tests
	cargo nextest run --manifest-path=./evm-e2e/Cargo.toml $(TEST_PROFILE) --no-default-features --features "$(TEST_FEATURES)" --package evm-e2e --bin evm-e2e tests
.PHONY: run-contracts-tests
run-contracts-tests:
	cargo nextest run --manifest-path=./contracts/Cargo.toml --workspace $(TEST_PROFILE) --no-default-features --features "$(TEST_FEATURES)"
	cargo nextest run --manifest-path=./examples/Cargo.toml --workspace $(TEST_PROFILE) --no-default-features --features "$(TEST_FEATURES)"

.PHONY: test
test:
	# devnet/mainnet: contracts unit tests
	$(MAKE) run-contracts-tests TEST_FEATURES=std TEST_PROFILE=--release
	# devnet/mainnet: wasmtime case
	$(MAKE) run-e2e-tests TEST_FEATURES=std,wasmtime TEST_PROFILE=--release
	# devnet/mainnet: rwasm case
	$(MAKE) run-e2e-tests TEST_FEATURES=std TEST_PROFILE=--release
.PHONY: test-debug
test-debug:
	# devnet/mainnet: contracts unit tests
	$(MAKE) run-contracts-tests TEST_FEATURES=std TEST_PROFILE=
	# devnet/mainnet: wasmtime case
	$(MAKE) run-e2e-tests TEST_FEATURES=std,wasmtime TEST_PROFILE=
	# devnet/mainnet: rwasm case
	$(MAKE) run-e2e-tests TEST_FEATURES=std TEST_PROFILE=

#.PHONY: svm_tests
#svm_tests:
#	cargo test --frozen --profile test --manifest-path crates/svm/Cargo.toml --
#	cargo test --frozen --lib svm::tests --profile test --manifest-path e2e/Cargo.toml --

.PHONY: wasm_contracts_sizes
wasm_contracts_sizes:
	du -sch target/contracts/wasm32-unknown-unknown/release/*.wasm

CONTRACTS_DIR := target/contracts/wasm32-unknown-unknown
WAT_OUT_DIR       := target/wats

.PHONY: wasm2wat
wasm2wat:
	mkdir -p $(WAT_OUT_DIR)
	for mode in debug release; do \
		for f in $(CONTRACTS_DIR)/$$mode/*.wasm; do \
			[ -e "$$f" ] || continue; \
			name=$$(basename $$f .wasm); \
			echo "Converting $$f -> $(WAT_OUT_DIR)/$$name.$$mode.wat"; \
			wasm2wat "$$f" > "$(WAT_OUT_DIR)/$$name.$$mode.wat"; \
		done; \
	done

# Heavily inspired by Lighthouse: https://github.com/sigp/lighthouse/blob/693886b94176faa4cb450f024696cb69cda2fe58/Makefile
# Gratefully stolen from Reth: https://github.com/fluentlabs-xyz/reth/blob/v1.10-patched/Makefile
GIT_SHA ?= $(shell git rev-parse HEAD)
GIT_TAG ?= $(shell git describe --tags --abbrev=0)
BIN_DIR = "dist/bin"

CARGO_TARGET_DIR ?= target

# List of features to use when building. Can be overridden via the environment.
# No jemalloc on Windows
ifeq ($(OS),Windows_NT)
    FEATURES ?= asm-keccak min-debug-logs
else
    FEATURES ?= jemalloc asm-keccak min-debug-logs
endif

NO_DEFAULT_FEATURES ?=

# Cargo profile for builds. Default is for local builds, CI uses an override.
PROFILE ?= release

# Extra flags for Cargo
CARGO_INSTALL_EXTRA_FLAGS ?=

# The docker image name
DOCKER_IMAGE_NAME ?= ghcr.io/fluentlabs-xyz/fluent

##@ Help

.PHONY: help
help: ## Display this help.
	@awk 'BEGIN {FS = ":.*##"; printf "Usage:\n  make \033[36m<target>\033[0m\n"} /^[a-zA-Z_0-9-]+:.*?##/ { printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)

##@ Build

.PHONY: install
install: ## Build and install the fluent binary under `$(CARGO_HOME)/bin`.
	cargo install --path bins/fluent --bin fluent --force --locked \
		--features "$(FEATURES)" \
		--profile "$(PROFILE)" \
		$(CARGO_INSTALL_EXTRA_FLAGS)

.PHONY: build-fluent
build-fluent: ## Build the fluent binary into `target` directory.
	cargo build --bin fluent --features "$(FEATURES)" --profile "$(PROFILE)"

# Environment variables for reproducible builds
# Set timestamp from last git commit for reproducible builds
SOURCE_DATE ?= $(shell git log -1 --pretty=%ct)

# Extra RUSTFLAGS for reproducible builds. Can be overridden via the environment.
RUSTFLAGS_REPRODUCIBLE_EXTRA ?=

# `reproducible` only supports fluent on x86_64-unknown-linux-gnu
build-%-reproducible:
	@if [ "$*" != "fluent" ]; then \
		echo "Error: Reproducible builds are only supported for fluent, not $*"; \
		exit 1; \
	fi
	SOURCE_DATE_EPOCH=$(SOURCE_DATE) \
	RUSTFLAGS="-C symbol-mangling-version=v0 -C strip=none -C link-arg=-Wl,--build-id=none -C metadata='' --remap-path-prefix $$(pwd)=. $(RUSTFLAGS_REPRODUCIBLE_EXTRA)" \
	LC_ALL=C \
	TZ=UTC \
	JEMALLOC_OVERRIDE=/usr/lib/x86_64-linux-gnu/libjemalloc.a \
	cargo build --bin fluent --features "$(FEATURES) jemalloc-unprefixed" --profile "reproducible" --locked --target x86_64-unknown-linux-gnu

.PHONY: build-debug
build-debug: ## Build the fluent binary into `target/debug` directory.
	cargo build --bin fluent --features "$(FEATURES)"

.PHONY: reproduce-genesis-build
reproduce-genesis-build: ## Reproduce genesis build with local fluentbase-build image and compare against release hashes.
	@set -euo pipefail; \
	TAG="$${TAG:-}"; \
	if [ -z "$$TAG" ]; then \
		VER="$$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' Cargo.toml | head -n1)"; \
		if [ -z "$$VER" ]; then \
			echo "Cannot extract version from Cargo.toml"; \
			exit 1; \
		fi; \
		TAG="v$$VER"; \
		echo "TAG not provided; using $$TAG from Cargo.toml"; \
		if git rev-parse -q --verify "refs/tags/$$TAG" >/dev/null; then \
			HEAD_SHA="$$(git rev-parse HEAD)"; \
			TAG_SHA="$$(git rev-list -n1 "$$TAG")"; \
			if [ "$$HEAD_SHA" != "$$TAG_SHA" ]; then \
				echo "WARNING: HEAD ($$HEAD_SHA) does not match $$TAG ($$TAG_SHA)"; \
				if [ "$${CONTINUE_ON_TAG_MISMATCH:-}" != "1" ]; then \
					printf "Continue anyway? [y/N] "; \
					read -r ans; \
					case "$$ans" in y|Y|yes|YES) ;; *) exit 1;; esac; \
				fi; \
			fi; \
		else \
			echo "WARNING: local git tag $$TAG not found"; \
		fi; \
	fi; \
	echo "Using TAG=$$TAG"; \
	docker build --platform=linux/amd64 -t fluentbase-build:local \
		--build-arg SDK_VERSION_TAG="$$TAG" \
		--build-arg RUST_TOOLCHAIN=1.92.0 \
		.; \
	FLUENTBASE_CONTRACTS_DOCKER=true \
	FLUENTBASE_BUILD_DOCKER_IMAGE=fluentbase-build \
	FLUENTBASE_BUILD_DOCKER_TAG=local \
	FLUENTBASE_CONTRACTS_IGNORE_DEFAULT_RUST_FLAGS=true \
	cargo b --release; \
	TMP_DIR="$$(mktemp -d)"; \
	trap 'rm -rf "$$TMP_DIR"' EXIT; \
	BASE_URL="https://github.com/fluentlabs-xyz/fluentbase/releases/download/$$TAG"; \
	curl -fL "$$BASE_URL/genesis-$$TAG.json.gz" -o "$$TMP_DIR/remote-devnet.json.gz"; \
	curl -fL "$$BASE_URL/genesis-mainnet-$$TAG.json.gz" -o "$$TMP_DIR/remote-mainnet.json.gz"; \
	gunzip -c "$$TMP_DIR/remote-devnet.json.gz" > "$$TMP_DIR/remote-devnet.json"; \
	gunzip -c "$$TMP_DIR/remote-mainnet.json.gz" > "$$TMP_DIR/remote-mainnet.json"; \
	REMOTE_DEVNET_SHA="$$(sha256sum "$$TMP_DIR/remote-devnet.json" | awk '{print $$1}')"; \
	REMOTE_MAINNET_SHA="$$(sha256sum "$$TMP_DIR/remote-mainnet.json" | awk '{print $$1}')"; \
	LOCAL_DEVNET_SHA="$$(sha256sum target/release/genesis-devnet.json | awk '{print $$1}')"; \
	LOCAL_MAINNET_SHA="$$(sha256sum target/release/genesis-mainnet.json | awk '{print $$1}')"; \
	echo "remote devnet : $$REMOTE_DEVNET_SHA"; \
	echo "local  devnet : $$LOCAL_DEVNET_SHA"; \
	echo "remote mainnet: $$REMOTE_MAINNET_SHA"; \
	echo "local  mainnet: $$LOCAL_MAINNET_SHA"; \
	test "$$REMOTE_DEVNET_SHA" = "$$LOCAL_DEVNET_SHA"; \
	test "$$REMOTE_MAINNET_SHA" = "$$LOCAL_MAINNET_SHA"; \
	echo "OK: remote genesis hashes match local reproduced hashes";

# Builds the fluent binary natively.
build-native-%:
	cargo build --bin fluent --target $* --features "$(FEATURES)" --profile "$(PROFILE)"

# The following commands use `cross` to build a cross-compile.
#
# These commands require that:
#
# - `cross` is installed (`cargo install cross`).
# - Docker is running.
# - The current user is in the `docker` group.
#
# The resulting binaries will be created in the `target/` directory.

# For aarch64, set the page size for jemalloc.
# When cross compiling, we must compile jemalloc with a large page size,
# otherwise it will use the current system's page size which may not work
# on other systems. JEMALLOC_SYS_WITH_LG_PAGE=16 tells jemalloc to use 64-KiB
# pages. See: https://github.com/paradigmxyz/reth/issues/6742
build-aarch64-unknown-linux-gnu: export JEMALLOC_SYS_WITH_LG_PAGE=16

# No jemalloc on Windows
build-x86_64-pc-windows-gnu: FEATURES := $(filter-out jemalloc jemalloc-prof,$(FEATURES))

# Note: The additional rustc compiler flags are for intrinsics needed by MDBX.
# See: https://github.com/cross-rs/cross/wiki/FAQ#undefined-reference-with-build-std
build-%:
	RUSTFLAGS="-C link-arg=-lgcc -Clink-arg=-static-libgcc" \
		cross build --bin fluent --target $* $(NO_DEFAULT_FEATURES) --features "$(FEATURES)" --profile "$(PROFILE)"

# Unfortunately we can't easily use cross to build for Darwin because of licensing issues.
# If we wanted to, we would need to build a custom Docker image with the SDK available.
#
# Note: You must set `SDKROOT` and `MACOSX_DEPLOYMENT_TARGET`. These can be found using `xcrun`.
#
# `SDKROOT=$(xcrun -sdk macosx --show-sdk-path) MACOSX_DEPLOYMENT_TARGET=$(xcrun -sdk macosx --show-sdk-platform-version)`
build-x86_64-apple-darwin:
	$(MAKE) build-native-x86_64-apple-darwin
build-aarch64-apple-darwin:
	$(MAKE) build-native-aarch64-apple-darwin

##@ Docker

# Note: This requires a buildx builder with emulation support. For example:
#
# `docker run --privileged --rm tonistiigi/binfmt --install amd64,arm64`
# `docker buildx create --use --driver docker-container --name cross-builder`
.PHONY: docker-build-push
docker-build-push: ## Build and push a cross-arch Docker image tagged with the latest git tag.
	$(call docker_build_push,$(GIT_TAG),$(GIT_TAG))

# Note: This requires a buildx builder with emulation support. For example:
#
# `docker run --privileged --rm tonistiigi/binfmt --install amd64,arm64`
# `docker buildx create --use --driver docker-container --name cross-builder`
.PHONY: docker-build-push-git-sha
docker-build-push-git-sha: ## Build and push a cross-arch Docker image tagged with the latest git sha.
	$(call docker_build_push,$(GIT_SHA),$(GIT_SHA))

# Note: This requires a buildx builder with emulation support. For example:
#
# `docker run --privileged --rm tonistiigi/binfmt --install amd64,arm64`
# `docker buildx create --use --driver docker-container --name cross-builder`
.PHONY: docker-build-push-latest
docker-build-push-latest: ## Build and push a cross-arch Docker image tagged with the latest git tag and `latest`.
	$(call docker_build_push,$(GIT_TAG),latest)

# Note: This requires a buildx builder with emulation support. For example:
#
# `docker run --privileged --rm tonistiigi/binfmt --install amd64,arm64`
# `docker buildx create --use --name cross-builder`
.PHONY: docker-build-push-nightly
docker-build-push-nightly: ## Build and push cross-arch Docker image tagged with the latest git tag with a `-nightly` suffix, and `latest-nightly`.
	$(call docker_build_push,nightly,nightly)

.PHONY: docker-build-push-nightly-edge-profiling
docker-build-push-nightly-edge-profiling: FEATURES := $(FEATURES) edge
docker-build-push-nightly-edge-profiling: ## Build and push cross-arch Docker image with edge features tagged with `nightly-edge-profiling`.
	$(call docker_build_push,nightly-edge-profiling,nightly-edge-profiling)

# Create a cross-arch Docker image with the given tags and push it
define docker_build_push
	rustup target add wasm32-unknown-unknown

	$(MAKE) FEATURES="$(FEATURES)" build-x86_64-unknown-linux-gnu
	mkdir -p $(BIN_DIR)/amd64
	cp $(CARGO_TARGET_DIR)/x86_64-unknown-linux-gnu/$(PROFILE)/fluent $(BIN_DIR)/amd64/fluent

	$(MAKE) FEATURES="$(FEATURES)" build-aarch64-unknown-linux-gnu
	mkdir -p $(BIN_DIR)/arm64
	cp $(CARGO_TARGET_DIR)/aarch64-unknown-linux-gnu/$(PROFILE)/fluent $(BIN_DIR)/arm64/fluent

	docker buildx build --file ./docker/Dockerfile.cross . \
		--platform linux/amd64,linux/arm64 \
		--tag $(DOCKER_IMAGE_NAME):$(1) \
		--tag $(DOCKER_IMAGE_NAME):$(2) \
		--provenance=false \
		--push
endef

# Note: This requires a buildx builder with emulation support. For example:
#
# `docker run --privileged --rm tonistiigi/binfmt --install amd64,arm64`
# `docker buildx create --use --name cross-builder`
.PHONY: docker-build-push-nightly-profiling
docker-build-push-nightly-profiling: ## Build and push cross-arch Docker image with profiling profile tagged with nightly-profiling.
	$(call docker_build_push,nightly-profiling,nightly-profiling)

##@ Other

#.PHONY: clean
#clean: ## Perform a `cargo` clean and remove the binary and test vectors directories.
	#cargo clean
	#rm -rf $(BIN_DIR)

.PHONY: profiling
profiling: ## Builds `fluent` with optimisations, but also symbols.
	RUSTFLAGS="-C target-cpu=native" cargo build --profile profiling --features jemalloc,asm-keccak

.PHONY: maxperf
maxperf: ## Builds `fluent` with the most aggressive optimisations.
	RUSTFLAGS="-C target-cpu=native" cargo build --profile maxperf --features jemalloc,asm-keccak

.PHONY: maxperf-no-asm
maxperf-no-asm: ## Builds `fluent` with the most aggressive optimisations, minus the "asm-keccak" feature.
	RUSTFLAGS="-C target-cpu=native" cargo build --profile maxperf --features jemalloc