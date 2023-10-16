FILES = greeting zktrie_open_test panic rwasm evm wasi

all: $(FILES)

.PHONY: $(FILES)
$(FILES):
	cargo b --release --target=wasm32-wasi --features=$@
	mkdir -p bin
	cp ../../target/wasm32-wasi/release/fluentbase_runtime_example.wasm ./bin/$@.wasm
	wasm2wat ./bin/$@.wasm > ./bin/$@.wat

