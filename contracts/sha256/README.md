# SHA256

SHA-256 precompile wrapper (Byzantium 0x02). Computes SHA-256 over the input and returns a 32-byte digest.

- Entrypoint: main_entry. Streams input from the host, executes revm_precompile::sha256::run, syncs gas, writes output.
- Input: arbitrary-length bytes.
- Output: 32-byte hash.
- Gas: Same schedule as EVM precompile (base + word cost); charged via sdk.sync_evm_gas.

Usage

- Call the entrypoint with the payload to hash. The output buffer contains the 32-byte digest.
