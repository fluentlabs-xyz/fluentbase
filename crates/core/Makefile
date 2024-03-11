#build2:
#	export RUSTFLAGS='-C target-feature=+bulk-memory'
#	mkdir -p bin
#	cargo b --release --target=wasm32-unknown-unknown
#	cp ../../target/wasm32-unknown-unknown/release/libfluentbase_core.a ./bin/core.a
#	#clang asm/stack.s -c --target=wasm32-unknown-unknown -o ./bin/stack.o
#	#RUSTFLAGS='-C target-feature=+bulk-memory' clang asm/memory.s -c --target=wasm32-unknown-unknown -o ./bin/memory.o
#	wasm-ld --allow-undefined --static --no-entry --export=_evm_self_balance --export=_evm_balance --export=_evm_create --strip-all --stack-first ./bin/core.a -o ./bin/core.wasm
#	wasm2wat ./bin/core.wasm > ./bin/core.wat

TARGET=wasm32-unknown-unknown
PROFILE=release
OUT_FOLDER=bin
SOURCE_NAME=fluentbase_core
LIB_SOURCE_NAME=lib${SOURCE_NAME}
EVM_CONTRACT_DEST_NAME=evm_contract
EVM_LOADER_DEST_NAME=evm_loader
EVM_LOADER_FEATURE_FLAG=evm_loader_entry

.PHONY: build
all: build

.PHONY: build
build: build_evm_contract build_evm_loader

.PHONY: build_evm_loader
build_evm_loader:
	mkdir -p bin
	touch ./bin/${EVM_LOADER_DEST_NAME}.wasm ./bin/${EVM_LOADER_DEST_NAME}-deploy.wasm
	cargo b --release --target=${TARGET} --features=${EVM_LOADER_FEATURE_FLAG}
	clang stack.s -c --target=${TARGET}
	wasm-ld --allow-undefined --static --no-entry --export=main --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${LIB_SOURCE_NAME}.a ./stack.o -o ./bin/${EVM_LOADER_DEST_NAME}.wasm
	wasm2wat ./bin/${EVM_LOADER_DEST_NAME}.wasm > ./bin/${EVM_LOADER_DEST_NAME}.wat
	cargo b --release --target=${TARGET} --features=${EVM_LOADER_FEATURE_FLAG}
	wasm-ld --allow-undefined --static --no-entry --export=deploy --strip-all --stack-first ../../target/${TARGET}/${PROFILE}/${LIB_SOURCE_NAME}.a -o ./bin/${EVM_LOADER_DEST_NAME}-deploy.wasm
	wasm2wat ./bin/${EVM_LOADER_DEST_NAME}-deploy.wasm > ./bin/${EVM_LOADER_DEST_NAME}-deploy.wat
	rm stack.o
	ls -alh ${OUT_FOLDER}

.PHONY: build_evm_contract
build_evm_contract:
	mkdir -p "${OUT_FOLDER}"
	RUSTFLAGS='-C target-feature=+bulk-memory' cargo b --release --target=wasm32-unknown-unknown --features=evm_contract_entry
	cp ../../target/${TARGET}/${PROFILE}/${SOURCE_NAME}.wasm ./${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm
	wasm2wat ./${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wasm > ./${OUT_FOLDER}/${EVM_CONTRACT_DEST_NAME}.wat
	ls -alh ${OUT_FOLDER}

.PHONY: build_evm_call_from_wasm_example
build_evm_call_from_wasm_example:
	make build_evm_loader
	cd ../../examples; make evm_call_from_wasm

.PHONY: test
test:
	cargo test -q


.PHONY: clean
clean:
	rm stack.o || true
	rm bin/*.wat || true
	cargo clean
