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
	$(MAKE) generate_genesis OPTIONS="--genesis-type=devnet --out-dir=$(OUT_DIR)"

.PHONY: build_wasm_main
build_wasm_main:
	mkdir -p $(OUT_DIR)
	touch "$(FILE_IN)"
	cargo b --release --target=${TARGET} --features="$(FEATURES)" --no-default-features
	clang stack.s -c --target=${TARGET}
	wasm-ld --allow-undefined --static --no-entry --export=main --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a stack.o -o "$(FILE_IN)"
	rm stack.o
