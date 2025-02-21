TARGET=wasm32-unknown-unknown
PROFILE=release
OUT_DIR=assets
SOURCE_NAME=fluentbase_genesis
SOURCE_LIB_NAME=lib${SOURCE_NAME}

all: generate_devnet_genesis

PROFILE=release

.PHONY: generate_genesis
generate_genesis:
	cargo run --package fluentbase-genesis-bin --bin fluentbase-genesis-bin -- $(OPTIONS)

.PHONY: generate_devnet_genesis
generate_devnet_genesis:
	mkdir -p $(OUT_DIR)
	touch ./assets/genesis-devnet.json
	$(MAKE) generate_genesis OPTIONS="--genesis-type=devnet --out-dir=$(OUT_DIR)"
