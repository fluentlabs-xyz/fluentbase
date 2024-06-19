all: build

SKIP_CONTRACTS=n
SKIP_EXAMPLES=n
SKIP_GENESIS=n
.PHONY: build
build:
	clear
	if [ "$(SKIP_CONTRACTS)" = "n" ]; then cd crates/contracts && $(MAKE); fi
	if [ "$(SKIP_EXAMPLES)" = "n" ]; then cd examples && $(MAKE); fi
	if [ "$(SKIP_GENESIS)" = "n" ]; then cd crates/genesis && $(MAKE); fi
	if [ -f /usr/bin/notify-send ]; then notify-send "fluentbase" "build finished"; fi || true

.PHONY: test
test:
	clear
	cargo test --no-fail-fast -q

CONTRACT_NAME:=greeting
.PHONE: deploy_example_contract
deploy_example_contract:
	node ./examples/deploy-contract.js --local ./examples/bin/$(CONTRACT_NAME).wasm

.PHONY: run_fluent_node
run_fluent_node:
	clear
	cd ../fluent/; $(MAKE) fluent_clean_datadir; $(MAKE) fluent_run | tee -i ../fluentbase/tmp/log.txt

.PHONY: build_contracts_and_run_fluent_node
build_contracts_and_run_fluent_node:
	clear
	$(MAKE)
	cd ../fluent/; $(MAKE) fluent_clean_datadir; $(MAKE) fluent_build
	(sleep 1; notify-send "fluent" "ready to process requests" || true)&
	mkdir -p tmp
	$(MAKE) run_fluent_node