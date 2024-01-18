FEATURES =

TARGET=wasm32-unknown-unknown
PROFILE=release

all: solid_file

solid_file:
	set -ex
	cargo b --profile ${PROFILE} --target=${TARGET} --features="${FILES}"
	mkdir -p bin
	wasm2wat ../../target/${TARGET}/${PROFILE}/fluentbase_rwasm_evm_translator.wasm > ../../tmp/$@.wat

clean:
	cargo clean

dependency_tree:
	OPTIONS="--manifest-path Cargo.toml" #--no-default-features
	cargo tree ${OPTIONS} > ../../tmp/cargo-dep-tree.txt