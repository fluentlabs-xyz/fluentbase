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

.PHONY: test_success
test_success:
	cargo test --package fluentbase-svm --lib fluentbase::helpers_v2_tests::tests::test_create_fill_deploy_exec_with_state -- --exact --nocapture > tmp/test_success.txt

.PHONY: test_error
test_error:
	cargo test --package fluentbase-e2e --lib svm_loader_v4::tests::test_svm_deploy_exec -- --exact --nocapture > tmp/test_error.txt

.PHONY: test_both
test_both:
	$(MAKE) test_success
	$(MAKE) test_error