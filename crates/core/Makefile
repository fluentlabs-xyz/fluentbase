TARGET=wasm32-unknown-unknown
PROFILE=release
OUT_FOLDER=bin
SOURCE_NAME=fluentbase_core
SOURCE_LIB_NAME=lib${SOURCE_NAME}
ECL_CONTRACT_DEST_NAME=ecl_contract
ECL_CONTRACT_FEATURE_FLAG=ecl_contract_entry
EVM_LOADER_CONTRACT_DEST_NAME=evm_loader_contract
EVM_LOADER_CONTRACT_FEATURE_FLAG=evm_loader_contract_entry
WCL_CONTRACT_DEST_NAME=wcl_contract
WCL_CONTRACT_FEATURE_FLAG=wcl_contract_entry
WASM_LOADER_CONTRACT_DEST_NAME=wasm_loader_contract
WASM_LOADER_CONTRACT_FEATURE_FLAG=wasm_loader_contract_entry
CUR_DIR="$(shell pwd)"

.PHONY: build
all: build

.PHONY: build
build: build_all_contracts

.PHONY: build_wasm_main
build_wasm_main:
	mkdir -p ${OUT_FOLDER}
	touch "$(FILE_IN)"
	cargo b --release --target=${TARGET} --features="$(FEATURES)"
	clang stack.s -c --target=${TARGET}
	wasm-ld --allow-undefined --static --no-entry --export=main --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a stack.o -o "$(FILE_IN)"
	rm stack.o

.PHONY: build_wasm_deploy
build_wasm_deploy:
	mkdir -p ${OUT_FOLDER}
	touch "$(FILE_IN)"
	cargo b --release --target=${TARGET} --features="$(FEATURES)"
	wasm-ld --allow-undefined --static --no-entry --export=deploy --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a -o "$(FILE_IN)"

.PHONY: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm:
	$(MAKE) build_wasm_main FILE_IN="$@" FEATURES="${EVM_LOADER_CONTRACT_FEATURE_FLAG}"

.PHONY: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm
${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm:
	$(MAKE) build_wasm_deploy FILE_IN="$@" FEATURES="${EVM_LOADER_CONTRACT_FEATURE_FLAG}"

${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wat: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wat: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.rwasm: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
	cd ../../bin; $(MAKE) custom_file FILE_IN="${CUR_DIR}/$^" FILE_OUT="${CUR_DIR}/$@"

.PHONY: build_evm_loader_contract
build_evm_loader_contract: \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.rwasm \
	${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm

.PHONY: ${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.wasm
${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.wasm:
	$(MAKE) build_wasm_main FILE_IN="$@" FEATURES="${ECL_CONTRACT_FEATURE_FLAG}"

${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.rwasm: ${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.wasm
	cd ../../bin;  $(MAKE) custom_file FILE_IN="${CUR_DIR}/$^" FILE_OUT="${CUR_DIR}/$@"

.PHONY: ${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}-deploy.wasm
${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}-deploy.wasm:
	$(MAKE) build_wasm_deploy FILE_IN="$@" FEATURES="${ECL_CONTRACT_FEATURE_FLAG}"

${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.wat: ${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}-deploy.wat: ${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}-deploy.wasm
	wasm2wat $^ > $@

.PHONY: build_ecl_contract
build_ecl_contract: \
	${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.wasm \
	${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.rwasm \
	${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}-deploy.wasm

.PHONY: build_ecl_contract_wat
build_ecl_contract_wat: ${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}.wat ${OUT_FOLDER}/${ECL_CONTRACT_DEST_NAME}-deploy.wat

.PHONY: build_evm_loader_contract_wat
build_evm_loader_contract_wat: ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}.wat ${OUT_FOLDER}/${EVM_LOADER_CONTRACT_DEST_NAME}-deploy.wat

.PHONY: ${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.wasm
${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.wasm:
	$(MAKE) build_wasm_main FILE_IN="$@" FEATURES="${WASM_LOADER_CONTRACT_FEATURE_FLAG}"

.PHONY: ${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm
${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm:
	$(MAKE) build_wasm_deploy FILE_IN="$@" FEATURES="${WASM_LOADER_CONTRACT_FEATURE_FLAG}"

${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.wat: ${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}-deploy.wat: ${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.rwasm: ${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.wasm
	cd ../../bin; $(MAKE) custom_file FILE_IN="${CUR_DIR}/$^" FILE_OUT="${CUR_DIR}/$@"

#.PHONY: build_wasm_loader_contract
#build_wasm_loader_contract: \
#	${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.wasm \
#	${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.rwasm \
#	${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}-deploy.wasm

.PHONY: ${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.wasm
${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.wasm:
	$(MAKE) build_wasm_main FILE_IN="$@" FEATURES="${WCL_CONTRACT_FEATURE_FLAG}"

${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.rwasm: ${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.wasm
	cd ../../bin; $(MAKE) custom_file FILE_IN="${CUR_DIR}/$^" FILE_OUT="${CUR_DIR}/$@"

.PHONY: ${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}-deploy.wasm
${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}-deploy.wasm:
	$(MAKE) build_wasm_deploy FILE_IN="$@" FEATURES="${WCL_CONTRACT_FEATURE_FLAG}"

${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.wat: ${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}-deploy.wat: ${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}-deploy.wasm
	wasm2wat $^ > $@

.PHONY: build_wcl_contract
build_wcl_contract: \
	${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.wasm \
	${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.rwasm \
	${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}-deploy.wasm

.PHONY: build_wcl_contract_wat
build_wcl_contract_wat: ${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}.wat ${OUT_FOLDER}/${WCL_CONTRACT_DEST_NAME}-deploy.wat

#.PHONY: build_wasm_loader_contract_wat
#build_wasm_loader_contract_wat: ${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}.wat ${OUT_FOLDER}/${WASM_LOADER_CONTRACT_DEST_NAME}-deploy.wat

.PHONY: build_all_contracts_with_wats
build_all_contracts_with_wats: build_evm_loader_contract_wat build_ecl_contract_wat build_wcl_contract_wat

.PHONY: build_evm_contracts
build_evm_contracts: build_evm_loader_contract build_ecl_contract

.PHONY: build_wasm_contracts
build_wasm_contracts: build_wcl_contract

.PHONY: build_all_contracts
build_all_contracts: build_evm_contracts build_wasm_contracts

.PHONY: run_evm_call_from_wasm_example
run_evm_call_from_wasm_example:
	#cd ../../examples; $(MAKE) evm_call_from_wasm
	cd ../../e2e; cargo test test_call_evm_from_wasm -- --nocapture

.PHONY: run_test_wasm_create
run_test_wasm_create: build_wcl_contract
	cd ../../e2e; cargo test test_wasm_create -- --nocapture

.PHONY: run_test_wasm_create
run_test_wasm_create: build_wcl_contract
	cd ../../e2e; cargo test test_wasm_create -- --nocapture

.PHONY: run_test_wasm_call_after_create
run_test_wasm_call_after_create: build_wcl_contract
	cd ../../e2e; cargo test test_wasm_call_after_create -- --nocapture

.PHONY: test
test:
	cargo test -q

.PHONY: clean_tmp_files
clean_tmp_files:
	rm stack.o >/dev/null 2>&1 || true
	rm ${OUT_FOLDER}/*.wat >/dev/null 2>&1 || true

.PHONY: clean
clean:
	$(MAKE) clean_tmp_files
	cargo clean
