# Fluent Dev Oracle

This example demonstrates a developer identity registry with strict namespace separation to prevent storage collisions between Wasm and EVM storage slots.

The contract allows developers to register their identity for a given repository hash by calling the contract with a 32-byte input representing the repo hash. The caller's address is stored in a securely namespaced storage slot.

## Security Features

- **Namespace Separation**: Uses a unique prefix hashed with the input to create isolated storage keys, preventing collisions with EVM contracts.
- **Secure Hashing**: Employs keccak256 to generate deterministic, collision-resistant storage slots.

## Usage

Deploy the contract and call it with a 32-byte repository hash as input. The caller's address will be registered as the developer for that repository.