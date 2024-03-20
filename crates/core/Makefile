PRECOMPILES=precompile_blake2 precompile_bn128 precompile_identity precompile_kzg_point_evaluation \
precompile_modexp precompile_secp256k1
TARGET=wasm32-unknown-unknown
PROFILE=release
SOURCE_NAME=fluentbase_core
SOURCE_LIB_NAME=lib${SOURCE_NAME}
ECL_CONTRACT_DEST_NAME=ecl_contract
ECL_CONTRACT_FEATURE_FLAG=ecl
EVM_LOADER_CONTRACT_DEST_NAME=evm_loader_contract
EVM_LOADER_CONTRACT_FEATURE_FLAG=evm_loader
WCL_CONTRACT_DEST_NAME=wcl_contract
WCL_CONTRACT_FEATURE_FLAG=wcl
CUR_DIR="$(shell pwd)"
OUT_DIR="${CUR_DIR}/assets"

.PHONY: build
all: build

.PHONY: build
build: build_all_contracts

.PHONY: build_wasm_main
build_wasm_main:
	mkdir -p $(OUT_DIR)
	touch "$(FILE_IN)"
	cargo b --release --target=${TARGET} --features="$(FEATURES)" --no-default-features
	clang stack.s -c --target=${TARGET}
	wasm-ld --allow-undefined --static --no-entry --export=main --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${SOURCE_LIB_NAME}.a stack.o -o "$(FILE_IN)"
	rm stack.o

.PHONY: ${PRECOMPILES}
${PRECOMPILES}:
	$(MAKE) build_wasm_main FILE_IN="$(OUT_DIR)/$@.wasm" FEATURES="${PRECOMPILES}"
	cd ../../bin; $(MAKE) custom_file FILE_IN="$(OUT_DIR)/$@.wasm" FILE_OUT="$(OUT_DIR)/$@.rwasm"

.PHONY: $(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
$(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm:
	$(MAKE) build_wasm_main FILE_IN="$@" FEATURES="${EVM_LOADER_CONTRACT_FEATURE_FLAG}"

$(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.wat: $(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

$(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.rwasm: $(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm
	cd ../../bin; $(MAKE) custom_file FILE_IN="$^" FILE_OUT="$@"

.PHONY: build_evm_loader_contract
build_evm_loader_contract: \
	$(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.wasm \
	$(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.rwasm

.PHONY: $(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.wasm
$(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.wasm:
	$(MAKE) build_wasm_main FILE_IN="$@" FEATURES="${ECL_CONTRACT_FEATURE_FLAG}"

$(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.rwasm: $(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.wasm
	cd ../../bin;  $(MAKE) custom_file FILE_IN="$^" FILE_OUT="$@"

$(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.wat: $(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

.PHONY: build_ecl_contract
build_ecl_contract: \
	$(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.wasm \
	$(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.rwasm

.PHONY: build_ecl_contract_wat
build_ecl_contract_wat: $(OUT_DIR)/${ECL_CONTRACT_DEST_NAME}.wat

.PHONY: build_evm_loader_contract_wat
build_evm_loader_contract_wat: $(OUT_DIR)/${EVM_LOADER_CONTRACT_DEST_NAME}.wat

.PHONY: $(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.wasm
$(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.wasm:
	$(MAKE) build_wasm_main FILE_IN="$@" FEATURES="${WCL_CONTRACT_FEATURE_FLAG}"

$(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.rwasm: $(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.wasm
	cd ../../bin; $(MAKE) custom_file FILE_IN="$^" FILE_OUT="$@"

$(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.wat: $(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.wasm
	wasm2wat $^ > $@

$(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.rwasm: $(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.wasm
	cd ../../bin; $(MAKE) custom_file FILE_IN="$^" FILE_OUT="$@"

.PHONY: build_wcl_contract
build_wcl_contract: $(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.wasm $(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.rwasm

.PHONY: build_wcl_contract_wat
build_wcl_contract_wat: $(OUT_DIR)/${WCL_CONTRACT_DEST_NAME}.wat

.PHONY: build_evm_contracts
build_evm_contracts: build_evm_loader_contract build_ecl_contract

.PHONY: build_wasm_contracts
build_wasm_contracts: build_wcl_contract

.PHONY: build_all_contracts
build_all_contracts: build_evm_contracts build_wasm_contracts

.PHONY: build_all_contracts_wats
build_all_contracts_wats: build_evm_loader_contract_wat build_ecl_contract_wat build_wcl_contract_wat

.PHONY: run_test_evm_call_from_wasm
run_test_evm_call_from_wasm: build_evm_contracts
	cd ../../e2e; cargo test --lib core::tests_evm_runtime::test_evm_call_from_wasm -- --nocapture

.PHONY: test
test:
	cargo test -q

.PHONY: clean_tmp_files
clean_tmp_files:
	rm stack.o >/dev/null 2>&1 || true
	rm $(OUT_DIR)/*.wat >/dev/null 2>&1 || true

.PHONY: clean
clean:
	$(MAKE) clean_tmp_files
	cargo clean
