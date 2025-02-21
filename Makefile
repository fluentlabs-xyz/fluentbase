all: build

.PHONY: build
build:
	cargo build # build examples & contracts by triggering "build.rs"
	cd crates/genesis && $(MAKE) # build genesis

.PHONY: examples
examples:
	cd examples && $(MAKE)

.PHONY: clean
clean:
	if [ "$(SKIP_EXAMPLES)" = "n" ]; then cd examples && $(MAKE) clean; fi
	cargo clean

.PHONY: test
test:
	cargo test --no-fail-fast -q