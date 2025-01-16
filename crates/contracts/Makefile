PRECOMPILES=evm blake2 sha256 ripemd160 identity modexp ecrecover multicall
EXAMPLES=greeting fairblock
CUR_DIR="$(shell pwd)"
OUT_DIR="${CUR_DIR}/assets"
RUSTFLAGS='-C link-arg=-zstack-size=131072 -C target-feature=+bulk-memory'

all: $(PRECOMPILES) $(EXAMPLES)

.PHONY: $(PRECOMPILES)
$(PRECOMPILES):
	mkdir -p $(OUT_DIR)
	RUSTFLAGS=$(RUSTFLAGS) cargo b --release --target=wasm32-unknown-unknown --no-default-features --features=$@
	cp ../../target/wasm32-unknown-unknown/release/fluentbase_contracts.wasm ./assets/precompile_$@.wasm
	wasm2wat $(OUT_DIR)/precompile_$@.wasm > $(OUT_DIR)/precompile_$@.wat || true

.PHONY: $(EXAMPLES)
$(EXAMPLES):
	cp ../../examples/$@/lib.wasm ./assets/precompile_$@.wasm
	cp ../../examples/$@/lib.wat ./assets/precompile_$@.wat
