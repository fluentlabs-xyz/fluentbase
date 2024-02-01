.PHONY: build build2

#build2:
#	export RUSTFLAGS='-C target-feature=+bulk-memory'
#	mkdir -p bin
#	cargo b --release --target=wasm32-unknown-unknown
#	cp ../target/wasm32-unknown-unknown/release/libfluentbase_statedb.a ./bin/statedb.a
#	#clang asm/stack.s -c --target=wasm32-unknown-unknown -o ./bin/stack.o
#	#RUSTFLAGS='-C target-feature=+bulk-memory' clang asm/memory.s -c --target=wasm32-unknown-unknown -o ./bin/memory.o
#	wasm-ld --allow-undefined --static --no-entry --export=_evm_self_balance --export=_evm_balance --export=_evm_create --strip-all --stack-first ./bin/statedb.a -o ./bin/statedb.wasm
#	wasm2wat ./bin/statedb.wasm > ./bin/statedb.wat

build:
	mkdir -p bin
	RUSTFLAGS='-C target-feature=+bulk-memory' cargo b --release --target=wasm32-unknown-unknown
	cp ../target/wasm32-unknown-unknown/release/fluentbase_statedb.wasm ./bin/statedb.wasm
	wasm2wat ./bin/statedb.wasm > ./bin/statedb.wat