FILES = bitwise_byte bitwise_shl

all: $(FILES)

TARGET=wasm32-unknown-unknown
PROFILE=release

.PHONY: $(FILES)
$(FILES):
	set -ex
	cargo b --profile ${PROFILE} --target=${TARGET} --features=$@
	mkdir -p bin
	wasm2wat ../../target/${TARGET}/${PROFILE}/fluentbase_rwasm_code_snippets.wasm > ./bin/$@.wat
	#cp ../../target/${TARGET}/release/fluentbase_rwasm_code_snippets.wasm ./bin/$@.wasm
	#wasm2wat ./bin/$@.wasm > ./bin/$@.wat

test:
	 cargo test --features=testing --release all_tests -- --nocapture

clean:
	cargo clean