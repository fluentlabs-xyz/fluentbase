all: build test

.PHONY: build
build:
	# build examples & contracts by triggering "build.rs"
	cargo check --lib #--exclude fluentbase-genesis --workspace --lib
	# build genesis files
	#cd crates/genesis && $(MAKE) # build genesis

.PHONY: update-deps
update-deps:
	cargo update --manifest-path=./contracts/Cargo.toml revm-rwasm rwasm
	cargo update --manifest-path=./examples/Cargo.toml revm-rwasm rwasm
	cargo update revm-rwasm rwasm
	cargo update --manifest-path=./evm-e2e/Cargo.toml revm-rwasm rwasm

.PHONY: examples
examples:
	cd examples && $(MAKE)

.PHONY: clean
clean:
	if [ "$(SKIP_EXAMPLES)" = "n" ]; then cd examples && $(MAKE) clean; fi
	cargo clean
	cd examples/svm/solana-program && $(MAKE) clean
	cd examples/svm/solana-program-state-usage && $(MAKE) clean
	cd examples/svm/solana-program-transfer-with-cpi && $(MAKE) clean
	cd revm/e2e && cargo clean
	cd examples/svm/solana-program && $(MAKE) clean
	cd examples/svm/solana-program-state-usage && $(MAKE) clean

.PHONY: test
test:
	cargo test --manifest-path=./contracts/Cargo.toml --release
	cargo test --manifest-path=./examples/Cargo.toml --release
	cargo test --release
	cargo test --manifest-path=./evm-e2e/Cargo.toml --package evm-e2e --bin evm-e2e short_tests::good_coverage_tests --release

.PHONY: testnet_test
testnet_test:
	cargo test --manifest-path=./contracts/Cargo.toml --release --features fluent-testnet
	cargo test --manifest-path=./examples/Cargo.toml --release --features fluent-testnet
	cargo test --release --features fluent-testnet
	cargo test --manifest-path=./evm-e2e/Cargo.toml --package evm-e2e --bin evm-e2e short_tests::good_coverage_tests --release

.PHONY: svm_tests
svm_tests:
	cargo test --frozen --profile test --manifest-path crates/svm/Cargo.toml --
	cargo test --frozen --lib svm::tests --profile test --manifest-path e2e/Cargo.toml --

.PHONY: wasm_contracts_sizes
wasm_contracts_sizes:
	ls -al target/contracts/wasm32-unknown-unknown/release/*.wasm

CONTRACT_NAME=svm
.PHONY: wasm2wat
wasm2wat:
	mkdir -p tmp
	wasm2wat target/contracts/wasm32-unknown-unknown/release/fluentbase_contracts_$(CONTRACT_NAME).wasm > tmp/$(CONTRACT_NAME).wat

.PHONY: check
check:
	cargo check
