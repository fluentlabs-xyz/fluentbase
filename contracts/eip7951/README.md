# SECP256R1

secp256r1 (P‑256) precompile-style wrapper. Provides signature verification and related operations over the NIST P‑256
curve.

- Entrypoint: main_entry. Reads input, executes host-backed P‑256 operation, syncs gas, writes output.
- Input: operation selector + payload (message hash, public key, signature). Encoding defined by this crate.
- Output: 32-byte 0/1 for verify, or encoded public key/result for other ops.
- Gas: Mirrors host schedule; charged via sdk.sync_evm_gas.

Note: Not part of Ethereum’s default precompile set; ensure your host exposes consistent P‑256 primitives.
