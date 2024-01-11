FILES = bitwise_byte bitwise_shl bitwise_shr bitwise_and bitwise_or bitwise_xor bitwise_not bitwise_gt bitwise_lt bitwise_sgt bitwise_slt bitwise_eq bitwise_sar bitwise_iszero \
arithmetic_add arithmetic_sub arithmetic_mul arithmetic_div arithmetic_sdiv arithmetic_mod arithmetic_smod arithmetic_exp arithmetic_addmod arithmetic_mulmod arithmetic_signextend \
memory_mstore memory_mstore8 memory_mload memory_msize \
stack_pop stack_dup1 stack_dup2 stack_swap1 stack_swap2 \
system_keccak256 system_address system_caller system_callvalue system_codesize system_gas system_calldataload system_calldatasize system_calldatacopy \
host_chainid host_basefee host_blockhash host_coinbase host_gaslimit host_number host_timestamp host_sload host_sstore \
host_env_block_difficulty host_env_blobbasefee host_env_gasprice host_env_origin host_env_blobhash \
control_return control_revert

all: $(FILES)

TARGET=wasm32-unknown-unknown
PROFILE=release
TEST_CASE=arithmetic_sdiv

.PHONY: $(FILES)
$(FILES):
	cargo run -- --skip-translate-sections --skip-type-check --entry-fn-name-matches-file-in-name --do-not-inject-fuel --use-subroutine-router --file-in-path=../../rwasm/rwasm-code-snippets/bin/$@.wat

solid_file:
	cargo run -- --entry-fn-name=$(ENTRY_FN_NAME) --entry-fn-name-beginnings-for="$(FILES)" --skip-translate-sections --skip-type-check --do-not-inject-fuel --use-subroutine-router --file-in-path=../../rwasm/rwasm-code-snippets/bin/$@.wat
