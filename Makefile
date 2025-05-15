all: build

.PHONY: help
help:
	@echo "Available targets:"
	@echo "  help             - Display this help message"
	@echo "  all              - Same as 'build'"
	@echo "  build            - Build the library"
	@echo "  examples         - Build examples"
	@echo "  clean            - Clean build artifacts (including examples if SKIP_EXAMPLES=n)"
	@echo "  test             - Run all tests (unit tests, doc tests, and evm-e2e tests)"
	@echo "  test-fluentbase  - Run unit and e2e tests for fluentbase"
	@echo "  doc-tests        - Run only documentation tests"
	@echo "  evm-e2e          - Run only the EVM end-to-end tests"

.PHONY: build
build:
	# build examples & contracts by triggering "build.rs"
	cargo check --lib #--exclude fluentbase-genesis --workspace --lib
	# build genesis files
	#cd crates/genesis && $(MAKE) # build genesis

.PHONY: examples
examples:
	cd examples && $(MAKE)

.PHONY: clean
clean:
	if [ "$(SKIP_EXAMPLES)" = "n" ]; then cd examples && $(MAKE) clean; fi
	cargo clean


.PHONY: test-fluentbase
test-fluentbase:
	@echo "Running unit tests..."
	@if command -v cargo-nextest >/dev/null 2>&1; then \
		echo "Using nextest for testing..."; \
		cargo nextest run --no-fail-fast \
			--status-level=none \
			--final-status-level=fail \
			--failure-output=immediate; \
	else \
		echo "Using standard cargo test..."; \
		cargo test --no-fail-fast -q; \
	fi

.PHONY: doc-tests
doc-tests:
	@echo "Running doc tests..."
	@cargo test --doc --workspace -q

# EVM tests use custom initialization output that confuses nextest
# so we'll use regular cargo test for these
.PHONY: evm-e2e
evm-e2e:
	@echo "Running EVM e2e tests..."
	cd revm/e2e && cargo test --release --package revm-rwasm-e2e --bin revm-rwasm-e2e -q

.PHONY: test
test: test-fluentbase doc-tests evm-e2e
