contracts/erc20

ERC‑20 contract implemented for Fluentbase with an SDK-first design and optional features (mintable/pausable). The crate exposes a single WASM entrypoint that routes calls by 4‑byte function selectors (Solidity‑style).

- Entrypoint: main_entry. Dispatches using known selectors; non-matching calls revert.
- Supported functions (subset):
  - name() -> string, symbol() -> string, decimals() -> uint8
  - totalSupply() -> uint256, balanceOf(address) -> uint256
  - transfer(address,uint256) -> bool, transferFrom(address,address,uint256) -> bool
  - approve(address,uint256) -> bool, allowance(address,address) -> uint256
  - Optional: mint(address,uint256), pause(), unpause() (guarded by settings)
- Events: Transfer, Approval, Pause/Unpause (topic layout compatible with EVM logs).
- Storage: Keyed by addresses; balances, allowances, and config live in contract metadata via SharedAPI storage helpers.
- Host integration: Uses SharedAPI for caller/context, storage, and I/O. On SVM targets, bindings bridge to Solana Token‑2022 via fluentbase‑svm.

Notes
- ABI is Solidity-like; selectors and encodings are defined in fluentbase_erc20. See lib.rs for offsets and exact packing.
- Gas/fuel is accounted by the host; state changes are deterministic and independent of the runtime backend.
