FILES = erc20 greeting secp256k1 keccak256 poseidon rwasm panic state storage
MANUAL = stack

all: $(FILES) $(MANUAL)

.PHONY: $(FILES)
$(FILES):
	cargo b --release --target=wasm32-unknown-unknown --features=$@
	mkdir -p bin
	clang stack.s -c --target=wasm32-unknown-unknown
	wasm-ld --stack-first --allow-undefined --no-entry --export=main --export=deploy ../target/wasm32-unknown-unknown/release/libfluentbase_runtime_example.a ./stack.o -o ./bin/$@.wasm
	rm stack.o
	wasm2wat ./bin/$@.wasm > ./bin/$@.wat
