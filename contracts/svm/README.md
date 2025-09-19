contracts/svm

Solana VM (SVM) adapter contract. Bridges Fluentbase contracts to Solana runtime primitives, enabling execution of Solana programs and Token‑2022 flows from within the Fluentbase environment.

- Entrypoint: main_entry (and optional deploy). Translates ABI calls into SVM instructions and account metas.
- Inputs: binary-encoded instruction set (program id, accounts, data). See crate types for exact layout.
- Outputs: program return data or error code mapped to ExitCode.
- Gas/fuel: Mirrors host SVM execution cost; final settlement performed by the entrypoint if applicable.

Notes
- Requires host to expose SVM execution syscalls and Solana-compatible account model.
- Used by contracts such as ERC‑20 to bridge to Token‑2022 processor.
