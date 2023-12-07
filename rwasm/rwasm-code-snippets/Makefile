FILES = bitwise_byte bitwise_shl bitwise_shr bitwise_and bitwise_or bitwise_xor bitwise_not bitwise_gt bitwise_lt bitwise_sgt bitwise_slt bitwise_eq bitwise_sar bitwise_iszero \
arithmetic_add arithmetic_sub arithmetic_sub_global arithmetic_mul arithmetic_div \
memory_mstore memory_mstore8

all: $(FILES)

TARGET=wasm32-unknown-unknown
PROFILE=release

.PHONY: $(FILES)
$(FILES):
	set -ex
	cargo b --profile ${PROFILE} --target=${TARGET} --features=$@
	mkdir -p bin
	wasm2wat ../../target/${TARGET}/${PROFILE}/fluentbase_rwasm_code_snippets.wasm > ./bin/$@.wat

test:
	 cargo test --features=testing --release all_tests -- --nocapture

clean:
	cargo clean