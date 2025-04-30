set -ex

SOLANA_ROOT=../../../../agave_copy
TOOLCHAIN=$SOLANA_ROOT/sdk/sbf/dependencies/platform-tools
RC_COMMON="$TOOLCHAIN/rust/bin/rustc --target sbf-solana-solana --crate-type lib -C panic=abort -C opt-level=2"
RC="$RC_COMMON -C target_cpu=sbfv2"
RC_V1="$RC_COMMON -C target_cpu=generic"
LD_COMMON="$TOOLCHAIN/llvm/bin/ld.lld -z notext -shared --Bdynamic -entry entrypoint --script elf.ld"
LD="$LD_COMMON --section-start=.text=0x100000000"
LD_V1=$LD_COMMON

FILENAME=$1
$RC_V1 -o assets/$FILENAME.o src/$FILENAME.rs
$LD_V1 -o assets/$FILENAME.so assets/$FILENAME.o
rm assets/$FILENAME.o
