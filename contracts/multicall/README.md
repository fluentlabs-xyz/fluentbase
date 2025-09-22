# MULTICALL

Multicall/aggregate helper contract. Batches multiple calls into a single entrypoint invocation and returns compact
results.

- Entrypoint: main_entry. Decodes a list of call targets and payloads; executes them in-order; assembles return data.
- Inputs: array of (to, value?, data) tuples; exact encoding depends on crate implementation (Solidity-like ABI).
- Outputs: aggregated success flags and return blobs.
- Gas/fuel: Each inner call is charged according to host rules; the entrypoint settles final deltas.

Notes

- Fails fast or continues-on-error based on flags in the input. See crate for exact semantics.
