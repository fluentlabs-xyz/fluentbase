RUSTFLAGS='-C link-arg=-zstack-size=262144 -C target-feature=+bulk-memory'

.PHONY: lib.wasm
lib.wasm: lib.rs Cargo.toml
	RUSTFLAGS=$(RUSTFLAGS) cargo build --release --target=wasm32-unknown-unknown --no-default-features --features=""
	cp ./target/wasm32-unknown-unknown/release/*.wasm ./lib.wasm
	wasm2wat ./lib.wasm > ./lib.wat || true
