TARGET=wasm32-unknown-unknown
PROFILE=release
OUT_FOLDER=bin
SOURCE_NAME=fluentbase_core
SOURCE_LIB_NAME=lib${SOURCE_NAME}
EVM_CONTRACT_DEST_NAME=evm_contract
EVM_CONTRACT_FEATURE_FLAG=evm_contract_entry
EVM_LOADER_CONTRACT_DEST_NAME=evm_loader_contract
EVM_LOADER_CONTRACT_FEATURE_FLAG=evm_loader_contract_entry
CUR_DIR="$(shell pwd)"

.PHONY: build
all: build

.PHONY: build
build: build_all_contracts

.PHONY: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm:
	mkdir -p ${OUT_FOLDER}
	touch $@
	cargo b --release --target=${TARGET} --features=${EVM_LOADER_CONTRACT_FEATURE_FLAG}
	clang stack.s -c --target=${TARGET}
	wasm-ld --allow-undefined --static --no-entry --export=main --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a stack.o -o $@
	rm stack.o

.PHONY: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm
${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm:
	mkdir -p ${OUT_FOLDER}
	touch $@
	cargo b --release --target=${TARGET} --features=${EVM_LOADER_CONTRACT_FEATURE_FLAG}
	wasm-ld --allow-undefined --static --no-entry --export=deploy --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a -o $@

${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wat: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wat: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.rwasm: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
	cd ../../bin; FILE_IN="${CUR_DIR}/$^" FILE_OUT="${CUR_DIR}/$@" make custom_file

.PHONY: build_evm_loader
build_evm_loader: \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.rwasm \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm

.PHONY: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm
${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm:
	mkdir -p ${OUT_FOLDER}
	touch $@
	cargo b --release --target=${TARGET} --features=${EVM_CONTRACT_FEATURE_FLAG}
	clang stack.s -c --target=${TARGET}
	wasm-ld --allow-undefined --static --no-entry --export=main --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a stack.o -o $@
	rm stack.o

.PHONY: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.rwasm
${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.rwasm: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm
	cd ../../bin; FILE_IN="${CUR_DIR}/$^" FILE_OUT="${CUR_DIR}/$@" make custom_file

.PHONY: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}-deploy.wasm
${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}-deploy.wasm:
	mkdir -p ${OUT_FOLDER}
	touch $@
	cargo b --release --target=${TARGET} --features=${EVM_CONTRACT_FEATURE_FLAG}
	wasm-ld --allow-undefined --static --no-entry --export=deploy --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a -o $@

${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wat: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}-deploy.wat: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}-deploy.wasm
	wasm2wat $^ > $@

.PHONY: build_evm_contract
build_evm_contract: \
	${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm \
	${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.rwasm \
	${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}-deploy.wasm

.PHONY: build_all_contracts
build_all_contracts: build_evm_loader build_evm_contract

.PHONY: build_evm_contract_wat
build_evm_contract_wat: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wat ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}-deploy.wat

.PHONY: build_evm_loader_contract_wat
build_evm_loader_contract_wat: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wat ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wat

.PHONY: build_all_contracts_with_wats
build_all_contracts_with_wats: build_evm_loader build_evm_loader_contract_wat build_evm_contract_wat

.PHONY: build_evm_call_from_wasm_example
#build_evm_call_from_wasm_example: build_all_contracts_with_wats
build_evm_call_from_wasm_example: build_all_contracts
	cd ../../examples; make evm_call_from_wasm

.PHONY: run_evm_call_from_wasm_example
run_evm_call_from_wasm_example: build_evm_call_from_wasm_example
	cd ../../e2e; cargo test test_call_evm_from_wasm

.PHONY: test
test:
	cargo test -q

.PHONY: clean
clean:
	rm stack.o || true
	rm ${OUT_FOLDER}/*.wat || true
	cargo clean
