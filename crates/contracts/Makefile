OUT_FOLDER=assets
CUR_DIR="$(shell pwd)"
OUT_DIR="${CUR_DIR}/${OUT_FOLDER}"

.PHONY: build
all: build

.PHONY: build
build: build_core_contracts build_precompiles

.PHONY: build_core_contracts
build_core_contracts:
	cd ../core; $(MAKE) build_all_contracts OUT_DIR="$(OUT_DIR)"

.PHONY: build_all_contracts_wats
build_all_contracts_wats:
	cd ../core; $(MAKE) build_all_contracts_wats OUT_DIR="$(OUT_DIR)"

.PHONY: build_precompiles
build_precompiles:
	cd ../core; $(MAKE) precompile_blake2 OUT_DIR="$(OUT_DIR)"
	#cd ../core; $(MAKE) precompile_bn128 OUT_DIR="$(OUT_DIR)"
	cd ../core; $(MAKE) precompile_identity OUT_DIR="$(OUT_DIR)"
	#cd ../core; $(MAKE) precompile_kzg_point_evaluation OUT_DIR="$(OUT_DIR)"
	cd ../core; $(MAKE) precompile_modexp OUT_DIR="$(OUT_DIR)"
	cd ../core; $(MAKE) precompile_secp256k1 OUT_DIR="$(OUT_DIR)"
