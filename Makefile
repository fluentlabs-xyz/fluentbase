all: build

SKIP_CONTRACTS=n
SKIP_EXAMPLES=y
SKIP_GENESIS=n

.PHONY: build
build:
	if [ "$(SKIP_EXAMPLES)" = "n" ]; then cd examples && $(MAKE); fi
	if [ "$(SKIP_CONTRACTS)" = "n" ]; then cd crates/contracts && $(MAKE); fi
	if [ "$(SKIP_GENESIS)" = "n" ]; then cd crates/genesis && $(MAKE); fi

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