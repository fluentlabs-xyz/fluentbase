all: build_hello_world

SOLANA_ROOT=../../../../agave_copy

.PHONY: install_solana_tools
install_solana_tools:
	sh -c "`curl -sSfL https://release.anza.xyz/v2.1.18/install`"

.PHONY: clean
clean:
	cargo clean

.PHONY: build_hello_world
build_hello_world:
	set -ex
	./build.sh hello_world
