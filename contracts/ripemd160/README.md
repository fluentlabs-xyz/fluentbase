# RIPEMD160

RIPEMD-160 precompile wrapper (0x03). Computes RIPEMD-160 over the input and returns a 20-byte digest.

- Entrypoint: main_entry. Reads input, executes revm_precompile::ripemd160::run, syncs gas, writes output.
- Input: arbitrary-length bytes.
- Output: 20-byte hash.
- Gas: Same schedule as EVM precompile (base + word cost); charged via sdk.sync_evm_gas.
