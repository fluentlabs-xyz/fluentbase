contracts/bls12381

BLS12-381 precompile-style wrappers. Provides group operations and pairing checks over BLS12-381 for environments that expose these as host functions.

- Entrypoint: main_entry (router in-crate dispatches ops if present).
- Ops: G1/G2 add, G1/G2 mul, pairing check; exact encoding follows crate implementation (affine coordinates, big-endian).
- Output: points encoded in affine (x||y), pairing returns 32-byte 0/1.
- Gas: Mirrors configured host schedule; charged via sdk.sync_evm_gas.

Note: BLS12-381 is not part of Ethereum’s main precompile set; ensure your host enables and documents the encodings used here.
