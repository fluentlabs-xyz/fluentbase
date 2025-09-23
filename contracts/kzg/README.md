# KZG

KZG commitment precompile-style wrapper (EIP-4844). Exposes blob KZG operations (e.g., point evaluation verification) as
host-backed calls.

- Entrypoint: main_entry (routes to the specific op implemented in this crate).
- Input: binary encoding for the requested operation (commitment, proof, point/value). See crate implementation for
  exact layout.
- Output: 32-byte 0/1 or computed value depending on op.
- Gas: Mirrors host’s EIP-4844 schedule; charged via sdk.sync_evm_gas.

Note: Requires a host that provides EIP-4844 primitives and trusted setup parameters consistent with the network.
