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

build:
	mkdir -p bin
	RUSTFLAGS='-C target-feature=+bulk-memory' cargo b --release --target=wasm32-unknown-unknown
	cp ../../target/wasm32-unknown-unknown/release/fluentbase_core.wasm ./bin/core.wasm
	wasm2wat ./bin/core.wasm > ./bin/core.wat