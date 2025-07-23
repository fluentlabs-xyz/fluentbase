all: build

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
	cd contracts/examples/svm/solana-program && $(MAKE) clean
	cd contracts/examples/svm/solana-program-state-usage && $(MAKE) clean
	cd contracts/examples/svm/solana-program-transfer-with-cpi && $(MAKE) clean
	cd revm/e2e && cargo clean

.PHONY: test
test:
	cargo test --no-fail-fast #-q

.PHONY: custom_tests
custom_tests:
	cargo test --frozen --profile test --manifest-path crates/svm/Cargo.toml -- --exact --show-output --nocapture
	cargo test --frozen --lib svm_loader_v4::tests::test_svm_deploy_exec --profile test --manifest-path e2e/Cargo.toml -- --exact --show-output --nocapture

.PHONY: wasm_contracts_sizes
wasm_contracts_sizes:
	ls -al target/target2/wasm32-unknown-unknown/release/*.wasm

CONTRACT_NAME=svm
.PHONY: wasm2wat
wasm2wat:
	mkdir -p tmp
	wasm2wat target/target2/wasm32-unknown-unknown/release/fluentbase_contracts_$(CONTRACT_NAME).wasm > tmp/$(CONTRACT_NAME).wat

.PHONY: check
check:
	cargo check
