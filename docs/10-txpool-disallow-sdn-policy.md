# txpool disallow policy for SDN-style address feeds

Fluent nodes can run a local transaction policy that prevents selected addresses from entering the txpool. This is intended for sanctions and compliance feeds where an operator must avoid locally mining or proposing transactions that involve listed addresses.

## Why this lives in Fluent docs

The `fluent` binary is built from Fluentbase and ships a patched reth runtime. Operational guidance for node operators belongs in Fluent docs, even when the implementation is in the reth subtree.

## Capability

Fluent-reth adds a txpool policy flag: `--txpool.disallow <PATH>`.

When configured, the node rejects txpool admission for any transaction that matches a disallowed address in one of these roles:

- transaction sender
- call recipient (`to` for call transactions)
- EIP-7702 authorization authority (recovered authority signer)

Rejected transactions are excluded from local txpool selection, so the local builder will not include them in blocks it builds.

## Input file format

The disallow file accepts either:

- a JSON array of address strings
- plain text tokens separated by newline, whitespace, or comma

This is intentionally tolerant for mixed SDN exports. Non-EVM tokens are ignored. Valid EVM addresses are parsed and deduplicated. If a token looks like an EVM address (`0x...`) but is malformed, startup fails with a parse error so the operator does not silently run with a bad policy file.

## Scope and consensus semantics

This is a local txpool policy, not a consensus rule.

- It controls what this node admits into its own pool and therefore what it can locally include.
- It does not redefine chain validity for blocks produced elsewhere.

This distinction is required to stay consensus-compatible with the network.

## Expected operator workflow

1. Maintain a local SDN source file.
2. Point `--txpool.disallow` to that file in the node service configuration.
3. Restart or roll the node after policy updates.
4. Confirm startup logs report a non-zero count of loaded disallowed addresses.

## Implementation reference

Patched reth change: `fluentlabs-xyz/reth` PR #1.
