Fluentbase
==========

Fluentbase is a framework that brings an SDK and proving system for Fluent STF (state transition function).

Currently, Fluentbase supports the following runtimes:
- crypto (keccak256, poseidon)
- ecc (secp256k1 verify & recover)
- evm (sload/sstore opcode simulation)
- rwasm (transact, compile)
- sys (read, write, state, halt)
- zktrie

Developers must use only functions from our SDK because when we compute proofs, we replace these functions with circuits that are much more optimized for these computations.

Fluentbase runs using the rWasm VM (reduced WebAssembly). It's 100% compatible with the WebAssembly binary representation and is more friendly for ZK operations. We also reduce the instruction set and embed sections inside the binary to simplify the proving process.

We don't support floating point operations for now but plan to bring them in the future.

## Structure

Here is a folder structure of the repository:
- `circuits` - circuits for rWasm and Fluentbase proving
- `examples` - examples of apps that can be run by Fluent network and are compatible with Fluentbase SDK
- `poseidon` - zktrie compatible poseidon adapter
- `runtime` - execution runtime that defines host functions
- `rwasm` - virtual machine and AOT compilers for rWasm
- `sdk` - SDK for creating host and guest apps

## Examples

In the examples folder, you can find sample apps that can be run in the Fluent network. Currently, we don't provide access to context ops, like block/tx info, because we haven't standardized the codec for passing these parameters.

You can find more info about how to deploy apps in the examples readme file.
