all: build

.PHONY: build
build:
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