# Fluentbase Interoperability Guide

Fluentbase is designed to support multiple virtual machines (VMs) — EVM, WASM, and SVM — under one unified framework.  
This guide explains how developers can write portable contracts across these environments.

---

## 1. Why Interoperability Matters
- **EVM**: Huge ecosystem, Solidity compatibility.  
- **WASM**: Flexible, safe, and optimized for Rust.  
- **SVM**: Specialized VM support for custom workflows.  

Fluentbase provides shared SDKs and entrypoints so developers can move logic between these runtimes with minimal changes.

---

## 2. Common Patterns Across VMs
| Feature       | EVM                           | WASM (Rust)                             | SVM (Rust)                         |
|---------------|-------------------------------|------------------------------------------|-------------------------------------|
| Entrypoint    | `constructor`, `function`     | `entrypoint!(main_entry, deploy_entry)`  | `entrypoint!(main_entry, deploy_entry)` |
| Storage       | Mappings, variables           | `SharedAPI::write` / `read`              | `SharedAPI::write` / `read`         |
| Events        | `emit Event(...)`             | Logging via SDK                          | Logging via SDK                     |
| Compilation   | Solidity → Bytecode           | Rust → WASM                              | Rust → SVM bytecode                 |

---

## 3. Example: Counter Contract

### EVM (Solidity)
```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Counter {
    uint256 public count;

    function increment() public {
        count += 1;
    }
}
