TARGET=wasm32-unknown-unknown
PROFILE=release
OUT_FOLDER=bin
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
build: build_core_contracts build_precompiles

.PHONY: build_core_contracts
build_core_contracts:
	cd ../core; $(MAKE) build_all_contracts OUT_DIR=$(OUT_DIR)

.PHONY: build_precompiles
build_precompiles:
	cd ../core; $(MAKE) precompile_blake2 OUT_DIR=$(OUT_DIR)