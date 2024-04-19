all: build

.PHONY: build
build:
	clear
	cd crates/contracts && $(MAKE)
	cd examples && $(MAKE)
	cd crates/genesis && $(MAKE)

.PHONY: test
test:
	clear
	cargo test --no-fail-fast -q

CONTRACT_NAME:=greeting
.PHONE: deploy_example_contract
deploy_example_contract:
	node ./examples/deploy-contract.js --local ./examples/bin/$(CONTRACT_NAME).wasm

.PHONY:
build_contracts_and_reth_node:
	clear
	#$(MAKE)
	cd ../fluent/; $(MAKE) fluent_clean_datadir; $(MAKE) fluent_build
	(sleep 1; notify-send "ready to accept connections")&
	#(sleep 2; cd ../ErrorCase; make run_test)&
	cd ../fluent/; $(MAKE) fluent_run