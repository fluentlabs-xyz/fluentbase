PACKAGE_VERSION=0.3.1

.PHONY: clone_state_tests
clone_state_tests:
	if [ ! -d tests ]; then \
  		git clone https://github.com/ethereum/tests; \
  	else \
  		cd tests; git pull; \
  	fi

.PHONY: run_state_tests
run_state_tests:
	cargo run -p fluentbase-revm-e2e@$(PACKAGE_VERSION) statetest tests/GeneralStateTests #-s
