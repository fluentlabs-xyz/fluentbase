# BLAKE2F

BLAKE2f (EIP-152) precompile wrapper. Exposes the BLAKE2 F compression function with the same input/output and gas rules
as Ethereum.

- Entrypoint: main_entry (single entry). Reads full input, executes revm_precompile::blake2::run, syncs gas, writes
  output.
- Input (213 bytes): rounds(4) || h(64) || m(128) || t(16) || f(1). Any other size is rejected.
- Output: 64-byte state after applying F.
- Gas: Charged identically to EVM via sdk.sync_evm_gas(result.gas_used, 0).
- Host: Uses SharedAPI for I/O and fuel; deterministic and byte-for-byte compatible with EVM precompile.

Usage

- Link this crate and call its entrypoint as a precompile. Provide the exact 213-byte payload; the result buffer is the
  64-byte digest state.
