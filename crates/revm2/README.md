Fluentbase Revm
===============

This crates defines wrapper over REVM and simulates its methods but uses Fluentbase SDK inside instead of EVM
interpreter.
To achieve EVM/WASM compatibility it forwards call/create opcodes into ECL/WCL WASM-based system smart contracts.

This repository is based on REVM v34 tag version.