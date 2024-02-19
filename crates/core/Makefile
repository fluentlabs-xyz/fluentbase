.PHONY: build build2

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
TMP_FOLDER=bin
SOURCE_NAME=fluentbase_core
DEST_NAME=core

build:
	mkdir -p "${TMP_FOLDER}"
	RUSTFLAGS='-C target-feature=+bulk-memory' cargo b --release --target=wasm32-unknown-unknown
	cp ../../target/${TARGET}/${PROFILE}/${SOURCE_NAME}.wasm ./${TMP_FOLDER}/${DEST_NAME}.wasm
	wasm2wat ./${TMP_FOLDER}/${DEST_NAME}.wasm > ./${TMP_FOLDER}/${DEST_NAME}.wat
	ls -alh ${TMP_FOLDER}

test:
	cargo test create_correctness
