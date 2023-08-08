
bin/greeting.wasm: greeting/src/lib.rs
	cd greeting
	cargo b --release --target=wasm32-unknown-unknown
	cd ..
	mkdir -p bin
	cp ../../target/wasm32-unknown-unknown/release/greeting.wasm ./bin/greeting.wasm
	wasm2wat ./bin/greeting.wasm > ./bin/greeting.wat
