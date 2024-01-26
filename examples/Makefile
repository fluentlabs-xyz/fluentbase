FILES = keccak256 greeting secp256k1 poseidon rwasm panic stack

all: $(FILES) $(MANUAL)

#.PHONY: $(FILES)
#$(FILES):
#	cargo b --release --target=wasm32-unknown-unknown --features=$@
#	mkdir -p bin
#	cp ../target/wasm32-unknown-unknown/release/fluentbase_example.wasm ./bin/$@.wasm
#	wasm2wat ./bin/$@.wasm > ./bin/$@.wat

.PHONY: $(FILES)
$(FILES):
	mkdir -p bin
	touch ./bin/$@.wasm ./bin/$@-deploy.wasm
	# RUSTFLAGS='-C link-arg=-zstack-size=65536 -C target-feature=+bulk-memory'
	cargo b --release --target=wasm32-unknown-unknown --features=$@
	clang stack.s -c --target=wasm32-unknown-unknown
	wasm-ld --allow-undefined --static --no-entry --export=main --strip-all --stack-first ../target/wasm32-unknown-unknown/release/libfluentbase_example.a ./stack.o -o ./bin/$@.wasm
	wasm2wat ./bin/$@.wasm > ./bin/$@.wat
	cargo b --release --target=wasm32-unknown-unknown --features=$@
	wasm-ld --allow-undefined --static --no-entry --export=deploy --strip-all --stack-first ../target/wasm32-unknown-unknown/release/libfluentbase_example.a -o ./bin/$@-deploy.wasm
	wasm2wat ./bin/$@-deploy.wasm > ./bin/$@-deploy.wat
	rm stack.o