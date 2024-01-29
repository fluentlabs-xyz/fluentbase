FILES = bitwise_byte bitwise_shl bitwise_shr bitwise_and bitwise_or bitwise_xor bitwise_not bitwise_gt bitwise_lt bitwise_sgt bitwise_slt bitwise_eq bitwise_sar bitwise_iszero \
arithmetic_add arithmetic_sub arithmetic_mul arithmetic_div arithmetic_sdiv arithmetic_mod arithmetic_smod arithmetic_exp arithmetic_addmod arithmetic_mulmod arithmetic_signextend \
memory_mstore memory_mstore8 memory_mload memory_msize \
stack_pop \
stack_dup1 stack_dup2 stack_dup3 stack_dup4 stack_dup5 stack_dup6 stack_dup7 stack_dup8 stack_dup9 stack_dup10 stack_dup11 stack_dup12 stack_dup13 stack_dup14 stack_dup15 stack_dup16 \
stack_swap1 stack_swap2 stack_swap3 stack_swap4 stack_swap5 stack_swap6 stack_swap7 stack_swap8 stack_swap9 stack_swap10 stack_swap11 stack_swap12 stack_swap13 stack_swap14 stack_swap15 stack_swap16 \
system_keccak256 system_address system_caller system_callvalue system_codesize system_gas system_calldataload system_calldatasize system_calldatacopy \
host_chainid host_basefee host_blockhash host_coinbase host_gaslimit host_number host_timestamp host_sload host_sstore host_tstore host_tload \
host_env_block_difficulty host_env_blobbasefee host_env_gasprice host_env_origin host_env_blobhash host_call host_staticcall host_delegatecall host_create host_create2 \
control_return control_revert

all: solid_file

TARGET=wasm32-unknown-unknown
PROFILE=release
TEST_CASE=arithmetic_sdiv

OPTIONS=--entry-fn-name=$(ENTRY_FN_NAME) --do-not-inject-fuel --skip-type-check --no-router #--skip-translate-sections

.PHONY: $(FILES)
$(FILES):
	cargo run -- --skip-translate-sections ${OPTIONS} --skip-source-map --file-in-path=../../rwasm/code-snippets/bin/$@.wat

solid_file:
	cargo run -- ${OPTIONS} --file-in-path=../../rwasm/code-snippets/bin/$@.wat
