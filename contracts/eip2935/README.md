contracts/eip2935

EIP-2935 system contract wrapper: access to historical block hashes (or related consensus data) via a host-provided interface.

- Entrypoint: main_entry. Decodes method selector and arguments, queries host, writes result.
- Methods: get_block_hash(number) -> B256; exact ABI follows crate implementation.
- Gas: Mirrors EVM rules for the corresponding op; charged via sdk.sync_evm_gas or final settlement.

Note: Behavior depends on host-maintained history length and pruning policy; ensure alignment with your network’s parameters.
