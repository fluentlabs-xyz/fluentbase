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
	cargo test --no-fail-fast -q

.PHONY: custom_tests_2_check
custom_tests_2_check:
	cargo test --profile test --manifest-path /home/bfday/github/fluentlabs-xyz/fluentbase/crates/svm/Cargo.toml
	cargo test svm_loader_v4 --profile test --manifest-path /home/bfday/github/fluentlabs-xyz/fluentbase/e2e/Cargo.toml
