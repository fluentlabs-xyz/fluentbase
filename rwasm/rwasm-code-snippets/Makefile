FILES = bitwise_byte bitwise_shl bitwise_shr bitwise_and bitwise_or bitwise_xor bitwise_not bitwise_gt bitwise_lt bitwise_sgt bitwise_slt bitwise_eq bitwise_sar bitwise_iszero \
arithmetic_add arithmetic_sub arithmetic_mul arithmetic_div arithmetic_sdiv arithmetic_mod arithmetic_smod arithmetic_exp arithmetic_addmod arithmetic_mulmod arithmetic_signextend \
memory_mstore memory_mstore8 memory_mload \
stack_pop stack_dup1 stack_dup2 stack_swap1 stack_swap2 \
system_keccak system_address system_caller system_callvalue system_codesize system_gas system_calldataload system_calldatasize system_calldatacopy \
host_chainid host_basefee host_blockhash host_coinbase host_gaslimit host_number host_timestamp host_sload host_sstore \
host_env_block_difficulty host_env_blobbasefee host_env_gasprice host_env_origin \
control_return

all: $(FILES)

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

with_rwasm_translation:
	make $(FILE)
	(cd ../bin; make $(FILE))

clean:
	cargo clean