FILES = greeting erc20 panic client-solidity router-solidity rwasm chess-validator storage keccak256 \
	simple-storage identity constructor-params json nitro-verifier abi-solidity

all: $(FILES)

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
