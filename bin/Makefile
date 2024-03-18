FILES = other_deploy_contract_test \
bitwise_byte bitwise_shl bitwise_shr bitwise_and bitwise_or bitwise_xor bitwise_not bitwise_gt bitwise_lt bitwise_sgt bitwise_slt bitwise_eq bitwise_sar bitwise_iszero \
arithmetic_add arithmetic_sub arithmetic_mul arithmetic_div arithmetic_sdiv arithmetic_mod arithmetic_smod arithmetic_exp arithmetic_addmod arithmetic_mulmod arithmetic_signextend \
memory_mstore memory_mstore8 memory_mload memory_msize \
stack_pop \
stack_dup1 stack_dup2 stack_dup3 stack_dup4 stack_dup5 stack_dup6 stack_dup7 stack_dup8 stack_dup9 stack_dup10 stack_dup11 stack_dup12 stack_dup13 stack_dup14 stack_dup15 stack_dup16 \
stack_swap1 stack_swap2 stack_swap3 stack_swap4 stack_swap5 stack_swap6 stack_swap7 stack_swap8 stack_swap9 stack_swap10 stack_swap11 stack_swap12 stack_swap13 stack_swap14 stack_swap15 stack_swap16 \
system_keccak256 system_address system_caller system_callvalue system_codesize system_gas system_calldataload system_calldatasize system_calldatacopy system_codecopy \
host_chainid host_basefee host_blockhash host_coinbase host_gaslimit host_number host_timestamp host_sload host_sstore host_tstore host_tload \
host_env_block_difficulty host_env_gasprice host_env_origin host_env_blobhash host_call host_staticcall host_delegatecall host_env_blobbasefee \
host_create host_create2 host_extcodesize host_extcodecopy host_extcodehash host_log0 host_log1 host_log2 host_log3 host_log4 host_balance host_selfbalance \
control_return control_revert control_pc

all: solid_file

#TARGET=wasm32-unknown-unknown
PROFILE=release

CARGO_OPTIONS=--entry-fn-name=$(ENTRY_FN_NAME) --no-magic-prefix --skip-type-check --no-router --retranslate-main --inject-init-bytecode

.PHONY: $(FILES)
$(FILES):
	cargo run -- --do-not-translate-sections ${CARGO_OPTIONS} $(OPTIONS) --file-in-path=../crates/code-snippets/bin/$@.wat

solid_file:
	cargo run -- ${CARGO_OPTIONS} --gen-source-map --file-in-path=../crates/code-snippets/bin/$@.wat --restricted-fn-names= --restricted-fn-name-prefixes=_evm

.PHONY: custom_file
custom_file:
	cargo run -- $(OPTIONS) --file-in-path="$(FILE_IN)" --rwasm-file-out-path="$(FILE_OUT)"
