# WASM

Generic WASM contract harness. Provides a minimal entrypoint that wires the Fluentbase SharedAPI to a WASM module’s
exported logic.

- Entrypoint: main_entry (and optional deploy entry). Forwards input bytes to the module, writes returned bytes back to
  the host.
- ABI: Binary pass‑through; ABI encoding/decoding is the module’s responsibility.
- Gas/fuel: The module uses SharedAPI to charge/settle fuel where needed; this wrapper performs final settlement if
  applicable.

Use this crate as a template to host bespoke WASM contracts within the Fluentbase contracts workspace.
