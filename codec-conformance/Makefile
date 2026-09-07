TARBALL_URL = https://registry.npmjs.org/@ethersproject/testcases/-/testcases-5.8.0.tgz
TARBALL_SHA = 68ef073bd48e9af008a62c0ed09fc8a510489a498f41ef4dd7eec39156da4686

.PHONY: vectors
vectors: vectors/contract-interface-abi2.json

vectors/contract-interface-abi2.json:
	mkdir -p vectors
	curl -sSL -o vectors/testcases.tgz $(TARBALL_URL)
	echo "$(TARBALL_SHA)  vectors/testcases.tgz" | shasum -a 256 -c -
	tar -xzf vectors/testcases.tgz -O package/testcases/contract-interface-abi2.json.gz \
	  | gunzip > vectors/contract-interface-abi2.json
	rm vectors/testcases.tgz

.PHONY: vectors-update
vectors-update:
	rm -rf vectors && $(MAKE) vectors

.PHONY: test
test: vectors
	cargo test --release --test corpus -- --exact the_corpus_encodes_as_solc_does

.PHONY: test-derive
test-derive: vectors
	cargo test --release --test corpus -- --exact the_corpus_encodes_as_solc_does_through_the_derive

.PHONY: fuzz
# The decoder suite builds the generated cases too, so it needs the corpus like the others.
fuzz: vectors
	PROPTEST_CASES=200000 cargo test --release --test decoder

.PHONY: mutants
mutants:
	cd .. && cargo mutants -p fluentbase-codec --test-tool=cargo

.PHONY: all
all: test test-derive fuzz
