all: build_cargo_powered_solana_program

SOLANA_ROOT=../../../../agave_copy

.PHONY: install_solana_tools
install_solana_tools:
	sh -c "`curl -sSfL https://release.anza.xyz/v2.1.18/install`"

.PHONY: clean
clean:
	cargo clean

.PHONY: prepare
prepare:
	#cd $(SOLANA_ROOT) && git checkout v1.18.26 && ./cargo build
	$(MAKE) install_solana_tools

.PHONY: build_hello_world
build_hello_world:
	set -ex
	./build.sh hello_world

.PHONY: build_solana_program
build_solana_program:
	NAME=solana_program
	$(RC_V1) -o assets/solana_program.o src/solana_program.rs
	$(LD_V1) -o assets/solana_program.so assets/solana_program.o
	rm assets/*.o

INPUT_NAME=solana_program_state_usage
OUTPUT_NAME=solana_program
.PHONY: build_cargo_powered_solana_program
build_cargo_powered_solana_program:
	#cd $(SOLANA_ROOT) && git checkout v2.1.18 && ./cargo-build-sbf -- -Znext-lockfile-bump
	cargo build-sbf -j1 -- -Znext-lockfile-bump
	cp ../../../../target/target2/sbf-solana-solana/release/fluentbase_examples_svm_$(INPUT_NAME).so assets/$(OUTPUT_NAME).so
	ls -alh assets/$(OUTPUT_NAME).so
