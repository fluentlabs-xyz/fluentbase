all: build

.PHONY: build
build:
	cd crates/contracts && $(MAKE)
	cd examples && $(MAKE)
	cd crates/genesis && $(MAKE)
	#cd crates/code-snippets && $(MAKE)
