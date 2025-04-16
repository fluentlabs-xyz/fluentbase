all: build

.PHONY: build
build:
	# build examples & contracts by triggering "build.rs"
	cargo check --lib #--exclude fluentbase-genesis --workspace --lib
	# build genesis files
	#cd crates/genesis && $(MAKE) # build genesis

.PHONY: examples
examples:
	cd examples && $(MAKE)

.PHONY: clean
clean:
	if [ "$(SKIP_EXAMPLES)" = "n" ]; then cd examples && $(MAKE) clean; fi
	cargo clean
	find contracts/ -type f | grep -iP "lib\.wa(sm|t)" | grep -viP "/fairblock/" | xargs rm
	find examples/ -type f | grep -iP "lib\.wa(sm|t)"| xargs rm

.PHONY: test
test:
	cargo test --no-fail-fast -q