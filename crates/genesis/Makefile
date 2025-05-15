all:
	cargo build --package fluentbase-genesis --features fairblock
	cp assets/genesis-devnet.json assets/genesis-devnet.$(shell git rev-parse --short HEAD).json