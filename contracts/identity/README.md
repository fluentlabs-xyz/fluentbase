contracts/identity

Identity precompile wrapper (0x04). Returns the input unchanged, following EVM gas rules.

- Entrypoint: main_entry. Reads input, executes revm_precompile::identity::run, syncs gas, writes output.
- Input: arbitrary bytes.
- Output: identical bytes.
- Gas: EVM-compatible (base + word copy) via sdk.sync_evm_gas.

Use this for efficient memory copy within the precompile address space.
