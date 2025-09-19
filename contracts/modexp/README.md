contracts/modexp

Modular exponentiation precompile wrapper (EIP-198, 0x05). Computes base^exp mod mod with EVM-compatible gas schedule.

- Entrypoint: main_entry. Reads input, executes revm_precompile::modexp::run, syncs gas, writes output.
- Input encoding (big-endian): len(base)(32) || len(exp)(32) || len(mod)(32) || base || exp || mod.
- Output: big-endian bytes representing (base^exp) mod mod, left-padded to len(mod).
- Gas: As per EIP-198 multiprecision formula; charged via sdk.sync_evm_gas.
- Validations: Zero modulus yields empty output.
