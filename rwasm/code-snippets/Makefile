FILES = bitwise_byte bitwise_shl bitwise_shr bitwise_and bitwise_or bitwise_xor bitwise_not bitwise_gt bitwise_lt bitwise_sgt bitwise_slt bitwise_eq bitwise_sar bitwise_iszero \
arithmetic_add arithmetic_sub arithmetic_mul arithmetic_div arithmetic_sdiv arithmetic_mod arithmetic_smod arithmetic_exp arithmetic_addmod arithmetic_mulmod arithmetic_signextend \
memory_mstore memory_mstore8 memory_mload memory_msize \
stack_pop \
stack_dup1 stack_dup2 stack_dup3 stack_dup4 stack_dup5 stack_dup6 stack_dup7 stack_dup8 stack_dup9 stack_dup10 stack_dup11 stack_dup12 stack_dup13 stack_dup14 stack_dup15 stack_dup16 \
stack_swap1 stack_swap2 stack_swap3 stack_swap4 stack_swap5 stack_swap6 stack_swap7 stack_swap8 stack_swap9 stack_swap10 stack_swap11 stack_swap12 stack_swap13 stack_swap14 stack_swap15 stack_swap16 \
system_keccak256 system_address system_caller system_callvalue system_codesize system_gas system_calldataload system_calldatasize system_calldatacopy \
host_chainid host_basefee host_blockhash host_coinbase host_gaslimit host_number host_timestamp host_sload host_sstore host_tstore host_tload \
host_env_block_difficulty host_env_blobbasefee host_env_gasprice host_env_origin host_env_blobhash \
control_return control_revert

all: solid_file

TARGET=wasm32-unknown-unknown
PROFILE=release

.PHONY: $(FILES)
$(FILES):
	set -ex
	cargo b --profile ${PROFILE} --target=${TARGET} --features=$@
	mkdir -p bin
	#clang stack.s -c --target=wasm32-unknown-unknown
	#wasm-ld --stack-first --allow-undefined --no-entry --export=$@ ../../target/${TARGET}/${PROFILE}/libfluentbase_rwasm_code_snippets.a ./stack.o -o ./bin/$@.wasm
	#wasm2wat ./bin/$@.wasm > ./bin/$@.wat
	wasm2wat ../../target/${TARGET}/${PROFILE}/fluentbase_rwasm_code_snippets.wasm > ./bin/$@.wat

solid_file:
	set -ex
	cargo b --profile ${PROFILE} --target=${TARGET} --features="${FILES}"
	mkdir -p bin
	wasm2wat ../../target/${TARGET}/${PROFILE}/fluentbase_rwasm_code_snippets.wasm > ./bin/$@.wat
	(cd ../bin; make $@)

with_rwasm_translation:
	make $(FILE)
	(cd ../bin; make $(FILE) ENTRY_FN_NAME=$(ENTRY_FN_NAME))

clean:
	cargo clean