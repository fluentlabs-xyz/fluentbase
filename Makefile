all: check build

CARGO_LOCKED_FLAGS ?= --locked
COVERAGE_TARGET ?= $(shell rustc -vV | sed -n 's/^host: //p')
COVERAGE_IGNORE_FILENAME_REGEX ?= (^|/)(tests?|benches?|examples?|e2e|evm-e2e)(/|$$)|(^|/)crates/testing(/|$$)|/(tests|.*_tests)\.rs$$
COVERAGE_IGNORE_DEPENDENCY_REGEX ?= (^|/)(\.?cargo/)?(registry|git)(/|$$)|(^|/)(\.?rustup/)?toolchains(/|$$)|(^|/)rustc(/|$$)|(^|/)target(/|$$)
EXAMPLES_COVERAGE_DEPENDENCIES ?= fluentbase-crypto,fluentbase-evm,fluentbase-revm,fluentbase-runtime,fluentbase-sdk
EVM_E2E_COVERAGE_DEPENDENCIES ?= fluentbase-crypto,fluentbase-evm,fluentbase-genesis,fluentbase-revm,fluentbase-runtime,fluentbase-sdk
GUEST_COVERAGE_TARGET_DIR ?= $(abspath target/rwasm-guest-coverage)
GUEST_COVERAGE_PROFILE_DIR ?= $(GUEST_COVERAGE_TARGET_DIR)/profiles
GUEST_COVERAGE_OBJECT_DIR ?= $(GUEST_COVERAGE_TARGET_DIR)/objects
GUEST_COVERAGE_WASM ?= $(GUEST_COVERAGE_TARGET_DIR)/wasm32-unknown-unknown/guest-coverage/fluentbase_contracts_evm.wasm
GUEST_COVERAGE_CLANG ?= clang
GUEST_COVERAGE_CLANG_TARGET ?=
GUEST_COVERAGE_IGNORE_FILENAME_REGEX ?= (^|/)\.cargo/(registry|git)/
RUST_LLVM_TOOLS_DIR ?= $(abspath $(shell rustc --print target-libdir)/../bin)
LLVM_COV ?= $(RUST_LLVM_TOOLS_DIR)/llvm-cov
LLVM_PROFDATA ?= $(RUST_LLVM_TOOLS_DIR)/llvm-profdata

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
	cargo build $(CARGO_LOCKED_FLAGS) --all

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
	cargo nextest run --manifest-path=./evm-e2e/Cargo.toml $(TEST_PROFILE) --no-default-features --features "$(TEST_FEATURES)" --package evm-e2e --bin evm-e2e
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

.PHONY: coverage coverage-root coverage-contracts coverage-examples-deps coverage-evm-e2e-deps coverage-rwasm-guest
coverage: coverage-root coverage-contracts coverage-examples-deps coverage-evm-e2e-deps coverage-rwasm-guest

coverage-root:
	@test -n "$(COVERAGE_TARGET)"
	cargo llvm-cov clean --manifest-path=./Cargo.toml --workspace
	cargo llvm-cov nextest --manifest-path=./Cargo.toml --workspace --release \
		--no-default-features --features std --no-fail-fast --locked --no-report \
		--target "$(COVERAGE_TARGET)" --coverage-target-only
	cargo llvm-cov nextest --manifest-path=./Cargo.toml --workspace --release \
		--no-default-features --features std,wasmtime --no-fail-fast --locked --no-report \
		--target "$(COVERAGE_TARGET)" --coverage-target-only
	cargo llvm-cov report --manifest-path=./Cargo.toml --release \
		--target "$(COVERAGE_TARGET)" --coverage-target-only --lcov \
		--output-path coverage-root.lcov \
		--ignore-filename-regex "$(COVERAGE_IGNORE_FILENAME_REGEX)"

coverage-contracts:
	@test -n "$(COVERAGE_TARGET)"
	cargo llvm-cov clean --manifest-path=./contracts/Cargo.toml --workspace
	cargo llvm-cov nextest --manifest-path=./contracts/Cargo.toml --workspace --release \
		--no-default-features --features std --no-fail-fast --locked --no-report \
		--target "$(COVERAGE_TARGET)" --coverage-target-only
	cargo llvm-cov report --manifest-path=./contracts/Cargo.toml --release \
		--target "$(COVERAGE_TARGET)" --coverage-target-only --lcov \
		--output-path coverage-contracts.lcov \
		--ignore-filename-regex "$(COVERAGE_IGNORE_FILENAME_REGEX)"

coverage-examples-deps:
	@test -n "$(COVERAGE_TARGET)"
	cargo llvm-cov clean --manifest-path=./examples/Cargo.toml --workspace
	cargo llvm-cov nextest --manifest-path=./examples/Cargo.toml --workspace --release \
		--no-default-features --features std --no-fail-fast --locked --no-report \
		--dep-coverage "$(EXAMPLES_COVERAGE_DEPENDENCIES)" \
		--target "$(COVERAGE_TARGET)" --coverage-target-only
	cargo llvm-cov report --manifest-path=./examples/Cargo.toml --release \
		--dep-coverage "$(EXAMPLES_COVERAGE_DEPENDENCIES)" \
		--target "$(COVERAGE_TARGET)" --coverage-target-only --lcov \
		--output-path coverage-examples-deps.lcov \
		--no-default-ignore-filename-regex \
		--ignore-filename-regex "$(COVERAGE_IGNORE_FILENAME_REGEX)|$(COVERAGE_IGNORE_DEPENDENCY_REGEX)"
	@test -s coverage-examples-deps.lcov

coverage-evm-e2e-deps:
	@test -n "$(COVERAGE_TARGET)"
	$(MAKE) -C evm-e2e sync_tests
	cargo llvm-cov clean --manifest-path=./evm-e2e/Cargo.toml --workspace
	cargo llvm-cov nextest --manifest-path=./evm-e2e/Cargo.toml --release \
		--no-default-features --features std --package evm-e2e --bin evm-e2e \
		--no-fail-fast --locked --no-report \
		--dep-coverage "$(EVM_E2E_COVERAGE_DEPENDENCIES)" \
		--target "$(COVERAGE_TARGET)" --coverage-target-only tests::good_coverage_tests
	cargo llvm-cov nextest --manifest-path=./evm-e2e/Cargo.toml --release \
		--no-default-features --features std,wasmtime --package evm-e2e --bin evm-e2e \
		--no-fail-fast --locked --no-report \
		--dep-coverage "$(EVM_E2E_COVERAGE_DEPENDENCIES)" \
		--target "$(COVERAGE_TARGET)" --coverage-target-only tests::good_coverage_tests
	cargo llvm-cov nextest --manifest-path=./evm-e2e/Cargo.toml --release \
		--no-default-features --features std --package evm-e2e --bin evm-e2e \
		--no-fail-fast --locked --no-report \
		--dep-coverage "$(EVM_E2E_COVERAGE_DEPENDENCIES)" \
		--target "$(COVERAGE_TARGET)" --coverage-target-only fixture
	cargo llvm-cov nextest --manifest-path=./evm-e2e/Cargo.toml --release \
		--no-default-features --features std,wasmtime --package evm-e2e --bin evm-e2e \
		--no-fail-fast --locked --no-report \
		--dep-coverage "$(EVM_E2E_COVERAGE_DEPENDENCIES)" \
		--target "$(COVERAGE_TARGET)" --coverage-target-only fixture
	cargo llvm-cov report --manifest-path=./evm-e2e/Cargo.toml --release \
		--dep-coverage "$(EVM_E2E_COVERAGE_DEPENDENCIES)" \
		--target "$(COVERAGE_TARGET)" --coverage-target-only --lcov \
		--output-path coverage-evm-e2e-deps.lcov \
		--no-default-ignore-filename-regex \
		--ignore-filename-regex "$(COVERAGE_IGNORE_FILENAME_REGEX)|$(COVERAGE_IGNORE_DEPENDENCY_REGEX)"
	@test -s coverage-evm-e2e-deps.lcov

coverage-rwasm-guest:
	@test -x "$(LLVM_COV)"
	@test -x "$(LLVM_PROFDATA)"
	@command -v "$(GUEST_COVERAGE_CLANG)" >/dev/null
	@rust_llvm_major=$$(rustc -vV | sed -n 's/^LLVM version: \([0-9]*\).*/\1/p'); \
		clang_llvm_major=$$("$(GUEST_COVERAGE_CLANG)" --version | sed -n '1s/[^0-9]*\([0-9][0-9]*\).*/\1/p'); \
		test -n "$$rust_llvm_major"; \
		test "$$rust_llvm_major" = "$$clang_llvm_major" || { \
			echo "guest coverage requires clang $$rust_llvm_major.x; found $$clang_llvm_major.x" >&2; \
			exit 1; \
		}
	RUSTC_WRAPPER= cargo clean --manifest-path=./contracts/Cargo.toml \
		--target-dir "$(GUEST_COVERAGE_TARGET_DIR)"
	RUSTC_WRAPPER= RUSTC_BOOTSTRAP=1 \
		RUSTFLAGS='-C instrument-coverage -Zno-profiler-runtime --emit=llvm-ir -C link-arg=-zstack-size=1048576 -C target-feature=+bulk-memory,+tail-call --remap-path-prefix=$(CURDIR)=.' \
		cargo build --manifest-path=./contracts/Cargo.toml \
		--package fluentbase-contracts-evm \
		--target wasm32-unknown-unknown \
		--target-dir "$(GUEST_COVERAGE_TARGET_DIR)" \
		--profile guest-coverage --no-default-features --features guest-coverage --locked
	@mkdir -p "$(GUEST_COVERAGE_PROFILE_DIR)" "$(GUEST_COVERAGE_OBJECT_DIR)"
	@contract_ir=$$(find "$(GUEST_COVERAGE_TARGET_DIR)/wasm32-unknown-unknown/guest-coverage/deps" -name 'fluentbase_contracts_evm.ll' -print -quit); \
		test -n "$$contract_ir"; \
		"$(GUEST_COVERAGE_CLANG)" $(GUEST_COVERAGE_CLANG_TARGET) "$$contract_ir" -Wno-override-module -c \
			-o "$(GUEST_COVERAGE_OBJECT_DIR)/fluentbase_contracts_evm.o"
	RUSTC_WRAPPER= \
		FLUENTBASE_EVM_WASM_PATH="$(GUEST_COVERAGE_WASM)" \
		FLUENTBASE_GUEST_PROFILE_DIR="$(GUEST_COVERAGE_PROFILE_DIR)" \
		cargo nextest run --manifest-path=./Cargo.toml --release \
		--no-default-features --features std,wasmtime,guest-coverage \
		--package fluentbase-e2e --no-fail-fast --locked evm::
	@find "$(GUEST_COVERAGE_PROFILE_DIR)" -maxdepth 1 -name '*.profraw' -print -quit | grep -q .
	"$(LLVM_PROFDATA)" merge -sparse "$(GUEST_COVERAGE_PROFILE_DIR)"/*.profraw \
		-o "$(GUEST_COVERAGE_TARGET_DIR)/guest.profdata"
	"$(LLVM_COV)" export --format=lcov \
		"$(GUEST_COVERAGE_OBJECT_DIR)/fluentbase_contracts_evm.o" \
		--instr-profile="$(GUEST_COVERAGE_TARGET_DIR)/guest.profdata" \
		--ignore-filename-regex="$(GUEST_COVERAGE_IGNORE_FILENAME_REGEX)" \
		| sed 's#^SF:contracts/crates/#SF:crates/#' > coverage-rwasm-guest.lcov
	@test -s coverage-rwasm-guest.lcov
	@awk 'BEGIN { in_file = 0; covered = 0 } /^SF:.*contracts\/evm\/src\/lib.rs$$/ { in_file = 1; next } /^SF:/ { in_file = 0 } in_file && /^DA:/ { split($$0, fields, ","); if (fields[2] > 0) covered = 1 } END { exit !covered }' coverage-rwasm-guest.lcov
	@awk 'BEGIN { in_file = 0; covered = 0 } /^SF:.*crates\/evm\/src\/opcodes.rs$$/ { in_file = 1; next } /^SF:/ { in_file = 0 } in_file && /^DA:/ { split($$0, fields, ","); if (fields[2] > 0) covered = 1 } END { exit !covered }' coverage-rwasm-guest.lcov
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
	cargo build $(CARGO_LOCKED_FLAGS) --bin fluent --features "$(FEATURES)" --profile "$(PROFILE)"

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
	cargo build $(CARGO_LOCKED_FLAGS) --bin fluent --features "$(FEATURES)"

# Builds the fluent binary natively.
build-native-%:
	cargo build $(CARGO_LOCKED_FLAGS) --bin fluent --target $* --features "$(FEATURES)" --profile "$(PROFILE)"

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
		cross build $(CARGO_LOCKED_FLAGS) --bin fluent --target $* $(NO_DEFAULT_FEATURES) --features "$(FEATURES)" --profile "$(PROFILE)"

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
	@./.github/scripts/check-release-tag.sh "$(GIT_TAG)" | grep -qx "channel=stable" || { \
		echo "Refusing to move 'latest': '$(GIT_TAG)' is not the canonical stable release tag" >&2; exit 1; }
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

	@set -eu; for arch in amd64 arm64; do \
		image="fluent-smoke-$$arch:$(GIT_SHA)"; \
		trap 'docker image rm -f "$$image" >/dev/null 2>&1 || true' 0; \
		docker buildx build --file ./docker/Dockerfile.cross . \
			--platform "linux/$$arch" \
			--tag "$$image" \
			--load; \
		docker run --rm --platform "linux/$$arch" "$$image" --version; \
		docker image rm "$$image"; \
		trap - 0; \
	done

	docker buildx build --file ./docker/Dockerfile.cross . \
		--platform linux/amd64,linux/arm64 \
		--tag $(DOCKER_IMAGE_NAME):$(1) \
		--tag $(DOCKER_IMAGE_NAME):$(2) \
		--provenance=true \
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
