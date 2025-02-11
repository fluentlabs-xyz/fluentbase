FILES = greeting erc20 hashing panic client-fluent client-solidity router-fluent router-solidity rwasm chess-validator shakmaty storage keccak rwasm-jit fairblock simple-storage echo constructor-params json nitro-verifier

all: $(FILES) $(MANUAL)

.PHONY: $(FILES)
$(FILES):
	cd $@ && $(MAKE)

.PHONY: prepare
prepare:
	yarn

.PHONY: clean
clean:
	find . -type f -name "lib.wasm" -delete
	find . -type f -name "lib.wat" -delete
	find . -type d -name "target" -delete

TARGET=wasm32-unknown-unknown
RPC_ENDPOINT_TYPE=local # local|dev - see deploy-contract.js impl
EXAMPLE_NAME=greeting
.PHONY: deploy_example_contract
deploy_example_contract:
	node deploy-contract.js --$(RPC_ENDPOINT_TYPE) $(EXAMPLE_NAME)/lib.wasm
