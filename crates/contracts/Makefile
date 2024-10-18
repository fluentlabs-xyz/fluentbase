PRECOMPILES=evm blake2 sha256 ripemd160 identity modexp ecrecover
CUR_DIR="$(shell pwd)"
OUT_DIR="${CUR_DIR}/assets"
RUSTFLAGS='-C link-arg=-zstack-size=131072 -C target-feature=+bulk-memory'
GITHUB_REPO := git@github.com:fluentlabs-xyz/DistributedIBE.git
PROJECT_DIR := "${CUR_DIR}/fireblock"

all: $(PRECOMPILES) fireblock

.PHONY: $(PRECOMPILES)
$(PRECOMPILES):
	mkdir -p $(OUT_DIR)
	RUSTFLAGS=$(RUSTFLAGS) cargo b --release --target=wasm32-unknown-unknown --no-default-features --features=$@
	cp ../../target/wasm32-unknown-unknown/release/fluentbase_contracts.wasm ./assets/precompile_$@.wasm
	wasm2wat $(OUT_DIR)/precompile_$@.wasm > $(OUT_DIR)/precompile_$@.wat || true
	cd ../../bin; $(MAKE) custom_file FILE_IN="$(OUT_DIR)/precompile_$@.wasm" FILE_OUT="$(OUT_DIR)/precompile_$@.rwasm"


fireblock:
	mkdir -p $(OUT_DIR)
	rm -rf $(PROJECT_DIR)
	git clone $(GITHUB_REPO) $(PROJECT_DIR)
	cd $(PROJECT_DIR); tinygo build -o $(OUT_DIR)/precompile_fireblock.wasm --target wasm-unknown github.com/FairBlock/DistributedIBE
	rm -rf $(PROJECT_DIR)
	wasm2wat $(OUT_DIR)/precompile_fireblock.wasm > $(OUT_DIR)/precompile_fireblock.wat || true
	cd ../../bin; $(MAKE) custom_file FILE_IN="$(OUT_DIR)/precompile_fireblock.wasm" FILE_OUT="$(OUT_DIR)/precompile_fireblock.rwasm"

