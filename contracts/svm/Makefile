all: build

build: lib.wasm

.PHONY: lib.wasm
lib.wasm:
	cargo build

.PHONY: build_release
build_release:
	cargo build --release