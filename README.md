Fluentbase
==========

Fluentbase is a framework that brings SDK and proving system for Fluent STF (state transition function).

Right now Fluentbase supports following runtimes:
- crypto (keccak256, poseidon)
- ecc (secp256k1 verify & recover)
- evm (sload/sstore opcode simulation)
- rwasm (transact, compile)
- sys (read, write, state, halt)
- zktrie

Developer must use only functions from our SDK because when we compute proof we replace it with circuits that is much more optimized for these computations.

Fluentbase runs using rWASM VM (reduced WebAssembly).
It's 100% compatible WebAssembly binary representation that is more friendly for ZK operations.
We also reduce instruction set and embed sections inside binary for simplify proving process.

We don't support floating point operations for now, but going to bring them in the future.

## Structure

Here is a folder structure of the repository:
- `circuits` - circuits for rWASM and Fluentbase proving
- `examples` - examples of apps that can be run by Fluent network and are compatible with Fluentbase SDK
- `poseidon` - zktrie compatible poseidon adapter
- `runtime` - execution runtime that defines host functions
- `rwasm` - virtual machine and AOT compilers for rWASM
- `sdk` - SDK for creating host and guest apps

## Examples

In the example folder you can find sample apps that can be run in the Fluent network.
Currently, we don't provide access to the context ops, like block/tx info, because we haven't standardized codec for passing these params.

You can find more info about how to deploy apps in the examples readme file.