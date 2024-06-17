FILES = cairo greeting hashing panic rwasm shakmaty router erc20

all: $(FILES) $(MANUAL)

.PHONY: $(FILES)
$(FILES):
	cd $@ && $(MAKE)

.PHONY: clean
clean:
	find . -type f -name "lib.wasm" -delete
	find . -type f -name "lib.wat" -delete