.PHONY: lib.wasm
lib.wasm:
	gblend build rust
	wasm2wat ./lib.wasm > ./lib.wat || true