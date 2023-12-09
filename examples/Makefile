FILES = erc20 greeting secp256k1 keccak256 poseidon rwasm panic state storage

all: $(FILES)

.PHONY: $(FILES)
$(FILES):
	cargo b --release --target=wasm32-unknown-unknown --features=$@
	mkdir -p bin
	cp ../target/wasm32-unknown-unknown/release/fluentbase_runtime_example.wasm ./bin/$@.wasm
	wasm2wat ./bin/$@.wasm > ./bin/$@.wat
