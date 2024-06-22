PRECOMPILES=blake2 sha256 ripemd160 identity modexp ecrecover evm_loader evm_deployer evm wasm_deployer wasm_loader wasm blended
CUR_DIR="$(shell pwd)"
OUT_DIR="${CUR_DIR}/assets"
RUSTFLAGS='-C link-arg=-zstack-size=262144 -C target-feature=+bulk-memory'

all: $(PRECOMPILES)

.PHONY: $(PRECOMPILES)
$(PRECOMPILES):
	mkdir -p $(OUT_DIR)
	RUSTFLAGS=$(RUSTFLAGS) cargo b --release --target=wasm32-unknown-unknown --no-default-features --features=$@
	cp ../../target/wasm32-unknown-unknown/release/fluentbase_contracts.wasm ./assets/precompile_$@.wasm
	wasm2wat $(OUT_DIR)/precompile_$@.wasm > $(OUT_DIR)/precompile_$@.wat || true
	cd ../../bin; $(MAKE) custom_file FILE_IN="$(OUT_DIR)/precompile_$@.wasm" FILE_OUT="$(OUT_DIR)/precompile_$@.rwasm"
