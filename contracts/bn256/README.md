contracts/bn256

alt_bn128 (BN256) precompile wrappers (EIP-196/197). Implements G1 add, G1 mul, and pairing check with EVM-compatible I/O and gas.

- Entrypoint: main_entry (router inside the crate dispatches to the selected op if applicable).
- Ops and Inputs:
  - ADD (0x06): two points in affine (x,y) over Fq; 64 bytes per point; output is 64-byte point.
  - MUL (0x07): point (64 bytes) and scalar (32 bytes); output is 64-byte point.
  - PAIRING (0x08): sequence of (G1,G2) tuples; input multiple of 192 bytes; output 32-byte 0/1.
- Gas: Matches EVM precompile schedule; charged via sdk.sync_evm_gas.
- Validation: Points/scalars must be in field/subgroup; invalid encodings yield failure per spec.
