# codec-conformance

`fluentbase-codec` against an external corpus of Solidity ABI vectors.

This crate is **not part of the workspace** and is never built by CI. It lives outside because its
build script turns 1880 corpus vectors into roughly two thousand monomorphised encode/decode paths,
and `cargo nextest run --workspace` would compile all of them on every run. The precedent is
`evm-e2e/`, which is excluded the same way.

## What the corpus is

`@ethersproject/testcases`, file `contract-interface-abi2.json.gz`. Its expected values were not
computed by another Rust implementation: a list of types was generated, random values chosen, the
whole thing written out as Solidity, compiled with **solc 0.4.18** and executed on a node, and the
returned bytes captured. So the oracle here is the reference implementation.

Each vector carries a list of Solidity types, the values, and the ABI encoding of the argument
tuple with no selector — which is exactly what `SolidityABI::encode_function_args` produces.

1880 vectors, 1425 distinct type signatures, nesting up to seven levels, every integer width from 8
to 256.

## What it does not cover

The corpus is broad over *shapes* and narrow over *values*: it contains no negative integers, no
explicit zeros, no empty strings and no empty byte strings. FLU-1111 was a zero-padding bug and this
corpus would not have caught it. The table-driven and randomised suites in
`crates/codec/tests/abi_conformance.rs` cover value space and stay in CI; the two are complementary,
not alternatives.

## Running it

```sh
make vectors        # fetch the corpus once, sha256-pinned, into ./vectors (gitignored)
make test           # the tuple path: all 1880 vectors
make test-derive    # the derive path: the 576 vectors containing a Solidity tuple
make fuzz           # the decoder against arbitrary bytes
make mutants        # cargo-mutants over fluentbase-codec (needs `cargo install cargo-mutants`)
make all            # test + test-derive + fuzz
```

A full `make test test-derive` is a few minutes, most of it compiling the generated cases.

## Why there are two paths

In Solidity a struct and a tuple encode identically. In this codec they are two independent bodies
of code — `impl_encoder_for_tuple!` in `crates/codec/src/tuple.rs`, written by hand, and the code
emitted by `crates/codec-derive`. They share no function, and they have been measured disagreeing.

`sol_to_rust` maps `tuple(...)` to an anonymous Rust tuple, so a corpus run reaches only the first.
`make test-derive` regenerates every tuple node as a named `#[derive(Codec)]` struct — 644 of them —
and runs the same vectors through the other. Both must produce the bytes solc produced.

The crate depends on `fluentbase-sdk` solely because of this: the `Codec` derive expands to
`::fluentbase_sdk::codec` for any crate not named `fluentbase-{codec,sdk,types,runtime}`
(`crates/codec-derive/src/lib.rs:55-64`), so the derive can only be exercised through the SDK. That
is also how a contract author reaches it.

## How the cases are generated

`build.rs` reads the corpus and writes Rust into `OUT_DIR`. A macro cannot do this: the type strings
arrive at runtime as JSON. The build script parses each type string into `SolType` — a parser the
workspace does not otherwise have, since `derive-core` only converts in the other direction — and
then reuses the SDK's own `sol_to_rust` for the Rust type. Values become calls to the small
constructors in `src/lib.rs` rather than literals, which keeps the generated file about ten times
smaller.

Vectors are grouped by signature so each type is monomorphised once.
