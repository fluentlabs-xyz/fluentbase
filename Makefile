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
	find . -type f | grep -iP "lib\.wa(sm|t)" | grep -viP "/fairblock/" | xargs rm || true
	cd examples/svm/solana-program && $(MAKE) clean
	cd examples/svm/solana-program-state-usage && $(MAKE) clean
	cd examples/svm/solana-program-transfer-with-cpi && $(MAKE) clean

.PHONY: test
test:
	cargo test --no-fail-fast #-q

.PHONY: custom_tests
custom_tests:
	cargo test --frozen --profile test --manifest-path crates/svm/Cargo.toml -- --exact --show-output --nocapture
	cargo test --frozen --lib svm_loader_v4::tests::test_svm_deploy_exec --profile test --manifest-path e2e/Cargo.toml -- --exact --show-output --nocapture

.PHONY: check
check:
	cargo check
