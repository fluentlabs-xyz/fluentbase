.PHONY: sync_tests
sync_tests:
	if [ ! -d tests ]; then \
  		git clone https://github.com/ethereum/tests; \
  	fi
	cd tests; git pull; git checkout v16.0; \

.PHONY: run_general_state_tests
run_general_state_tests:
	cargo test --release --package evm-testing-suite --bin evm-testing-suite tests::good_coverage_tests

.PHONY: run_all_tests
run_all_tests:
	cargo test --release --package evm-testing-suite --bin evm-testing-suite

.PHONY: help
help:
	cargo run --color=always --bin evm-testing-suite -h
