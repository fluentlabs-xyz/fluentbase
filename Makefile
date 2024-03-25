all: build

.PHONY: build
build:
	cd crates/contracts && $(MAKE)
	cd examples && $(MAKE)
	cd crates/genesis && $(MAKE)

.PHONY: test
test:
	clear
	cargo test --no-fail-fast -q
