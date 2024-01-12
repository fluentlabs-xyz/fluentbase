FILES = erc20 keccak256 greeting secp256k1 poseidon rwasm panic state
MANUAL = stack

all: $(FILES) $(MANUAL)

.PHONY: $(FILES)
$(FILES):
	cargo b --release --target=wasm32-unknown-unknown --features=$@
	mkdir -p bin
	cp ../target/wasm32-unknown-unknown/release/fluentbase_example.wasm ./bin/$@.wasm
	wasm2wat ./bin/$@.wasm > ./bin/$@.wat

.PHONY: $(MANUAL)
$(MANUAL):
	cargo b --release --target=wasm32-unknown-unknown --features=$@
	mkdir -p bin
	clang stack.s -c --target=wasm32-unknown-unknown
	wasm-ld --allow-undefined --no-entry --export=main --strip-all --export=deploy ../target/wasm32-unknown-unknown/release/libfluentbase_example.a ./stack.o -o ./bin/$@.wasm
	rm stack.o
	wasm2wat ./bin/$@.wasm > ./bin/$@.wat
