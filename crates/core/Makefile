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
build: build_evm_contract build_evm_loader_contract # build_all_wat

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

.PHONY: build_evm_loader_contract_rwasm
build_evm_loader_contract_rwasm: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.rwasm

.PHONY: build_evm_loader_contract
build_evm_loader_contract: \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.rwasm

.PHONY: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm
${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm:
	mkdir -p ${OUT_FOLDER}
	touch $@
	cargo b --release --target=${TARGET} --features=${EVM_CONTRACT_FEATURE_FLAG}
	clang stack.s -c --target=${TARGET}
	wasm-ld --allow-undefined --static --no-entry --export=main --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a stack.o -o $@
	rm stack.o

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
build_evm_contract: ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm ${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}-deploy.wasm

.PHONY: build_evm_call_from_wasm_example
build_evm_call_from_wasm_example: build_evm_loader_contract
	cd ../../examples; make evm_call_from_wasm

build_all_wat: \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wat \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wat \
	${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wat \
	${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}-deploy.wat

.PHONY: test
test:
	cargo test -q

.PHONY: clean
clean:
	rm stack.o || true
	rm ${OUT_FOLDER}/*.wat || true
	cargo clean
