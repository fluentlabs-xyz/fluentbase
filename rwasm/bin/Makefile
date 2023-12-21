FILES = bitwise_byte bitwise_shl bitwise_shr bitwise_and bitwise_or bitwise_xor bitwise_not bitwise_gt bitwise_lt bitwise_sgt bitwise_slt bitwise_eq bitwise_sar bitwise_iszero \
arithmetic_add arithmetic_sub arithmetic_mul arithmetic_div arithmetic_sdiv arithmetic_mod arithmetic_smod arithmetic_exp arithmetic_addmod arithmetic_mulmod arithmetic_signextend \
memory_mstore memory_mstore8 \
stack_pop \
system_keccak system_address system_caller system_callvalue \
host_sstore host_sload

all: $(FILES)

TARGET=wasm32-unknown-unknown
PROFILE=release
TEST_CASE=arithmetic_sdiv

.PHONY: $(FILES)
$(FILES):
	cargo run -- --skip-translate-sections --skip-type-check --entry-fn-name-matches-file-in-name --do-not-inject-fuel --file-in-path=/home/bfday/gitANKR/wasm0/fluentbase/rwasm/rwasm-code-snippets/bin/$@.wat