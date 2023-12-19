FILES = bitwise_byte bitwise_shl bitwise_shr bitwise_and bitwise_or bitwise_xor bitwise_not bitwise_gt bitwise_lt bitwise_sgt bitwise_slt bitwise_eq bitwise_sar bitwise_iszero \
arithmetic_add arithmetic_sub arithmetic_mul arithmetic_div arithmetic_sdiv arithmetic_mod arithmetic_smod arithmetic_exp arithmetic_addmod arithmetic_mulmod arithmetic_signextend \
memory_mstore memory_mstore8 \
stack_pop \
system_address system_caller system_callvalue \
host_sstore host_sload

all: $(FILES)

TARGET=wasm32-unknown-unknown
PROFILE=release
TEST_CASE=arithmetic_sdiv

.PHONY: $(FILES)
$(FILES):
	set -ex
	cargo b --profile ${PROFILE} --target=${TARGET} --features=$@
	mkdir -p bin
	#clang stack.s -c --target=wasm32-unknown-unknown
	#wasm-ld --stack-first --allow-undefined --no-entry --export=$@ ../../target/${TARGET}/${PROFILE}/libfluentbase_rwasm_code_snippets.a ./stack.o -o ./bin/$@.wasm
	#wasm2wat ./bin/$@.wasm > ./bin/$@.wat
	wasm2wat ../../target/${TARGET}/${PROFILE}/fluentbase_rwasm_code_snippets.wasm > ./bin/$@.wat

test:
	cargo test --features=${TEST_CASE} --release test_${TEST_CASE} -- --nocapture

clean:
	cargo clean