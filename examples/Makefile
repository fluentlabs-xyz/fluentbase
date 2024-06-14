FILES = cairo greeting hashing panic rwasm shakmaty router

all: $(FILES) $(MANUAL)

.PHONY: $(FILES)
$(FILES):
	cd $@ && $(MAKE)

clean:
	find . -type f -name "*.wasm" -delete
	find . -type f -name "*.wat" -delete