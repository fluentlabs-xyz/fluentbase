RUSTFLAGS='-C link-arg=-zstack-size=262144 -C target-feature=+bulk-memory'
OUT_DIR="$(shell pwd)"

.PHONY: lib.wasm
lib.wasm: lib.rs Cargo.toml
	RUSTFLAGS=$(RUSTFLAGS) cargo build --release --target=wasm32-unknown-unknown --target-dir=./target --no-default-features
	cp ./target/wasm32-unknown-unknown/release/*.wasm ./lib.wasm
	wasm2wat ./lib.wasm > ./lib.wat || true
	cd ../../bin; $(MAKE) custom_file FILE_IN="$(OUT_DIR)/lib.wasm" FILE_OUT="$(OUT_DIR)/lib.rwasm"
