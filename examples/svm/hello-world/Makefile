all: build_cargo_powered_solana_program

SOLANA_ROOT=../../../../agave_copy
TOOLCHAIN=$(SOLANA_ROOT)/sdk/sbf/dependencies/platform-tools
RC_COMMON=$(TOOLCHAIN)/rust/bin/rustc --target sbf-solana-solana --crate-type lib -C panic=abort -C opt-level=2
RC=$(RC_COMMON) -C target_cpu=sbfv2
RC_V1=$(RC_COMMON) -C target_cpu=generic
LD_COMMON=$(TOOLCHAIN)/llvm/bin/ld.lld -z notext -shared --Bdynamic -entry entrypoint --script elf.ld
LD=$(LD_COMMON) --section-start=.text=0x100000000
LD_V1=$(LD_COMMON)

.PHONY: install_solana_tools
install_solana_tools:
	sh -c "$(curl -sSfL https://release.anza.xyz/v2.1.18/install)"

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

INPUT_NAME=hello_world
.PHONY: build_cargo_powered_solana_program
build_cargo_powered_solana_program:
	#cd $(SOLANA_ROOT) && git checkout v2.1.18 && ./cargo-build-sbf -- -Znext-lockfile-bump
	cargo build-sbf -j1 -- -Znext-lockfile-bump
	cp ../../../target/sbf-solana-solana/release/fluentbase_examples_svm_$(INPUT_NAME).so assets/hello_world.so
