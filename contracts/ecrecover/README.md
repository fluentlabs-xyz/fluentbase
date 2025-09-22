# ECRECOVER

secp256k1 ecrecover precompile wrapper (0x01). Recovers the Ethereum address from a message hash and ECDSA signature.

- Entrypoint: main_entry. Reads input, executes revm_precompile::secp256k1::ec_recover_run, syncs gas, writes output.
- Input: 128 bytes per the Ethereum precompile (hash(32) || v(32) || r(32) || s(32)). Non-standard sizes are rejected.
- Output: 32 bytes; the right-most 20 bytes hold the recovered address (left-padded with zeros) or empty on failure.
- Gas: EVM-compatible flat cost via sdk.sync_evm_gas.

Notes

- v must be 27/28 (or have low/high bits matching EVM rules). Invalid s or v yields empty output.
