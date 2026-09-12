# Release environment and Testnet replay notes

The existing tag-triggered workflows remain active. Genesis generation waits for the
`fluentbase-build` image and then verifies its provenance. The `release` environment controls
approval of the builder job.

## GitHub environment configuration

In **Settings → Environments**, create or edit the environment named exactly `release`:

1. Add release maintainers under **Required reviewers** and enable **Prevent self-review**.
2. Under **Deployment branches and tags**, allow approved tags matching `v*`. The workflows
   separately validate canonical `v<workspace-version>` and `v<workspace-version>-rc.N` tags.
   Allow a specific branch only if you also use standalone builder dispatches from that branch.
3. No environment secrets or variables are required. Keep `GPG_SIGNING_KEY`, `GPG_PASSPHRASE`
   and `CARGO_REGISTRY_TOKEN` in their existing repository/organization Actions secret scope.
   Signing and publishing jobs do not declare this environment. GitHub supplies `GITHUB_TOKEN`.

Approve the builder and let the release workflow wait. Its current poll makes 40 attempts,
30 seconds apart, plus Docker pull time. If approval exceeds that window, or provenance is
not available when verification runs, rerun the failed release job after the builder job has
completed. Routine CI, Docker publication and crates publication keep their existing triggers.

## Runtime and SDK changes

CALLCODE and FP2 corrections retain the existing syscall IDs and imports. Their compatibility
has been checked by the maintainers; this PR adds no versioned interfaces or activation gates.

- The EVM system runtime must be upgraded to activate its GASPRICE and empty-bytecode fixes.
- New compiler fuel instrumentation activates through rebuilt artifacts. A node update does
  not replace instrumentation already embedded in deployed code.
- The CompactABI vector fix restores v1.4.2's aligned element slots. Application-specific
  recovery is needed if the broken interim encoder was deployed and used to persist values.
- ABI generation rejects pinned types with incompatible representations or value domains.
  `bytes32`/`uint256` aliases remain supported; `bytes4`/`uint32` does not have matching padding.
- The rebuilt runtime-upgrade contract removes `recompile(address)`; use the supported methods
  and matching ABI described in [Runtime upgrades](06-runtime-upgrade.md).

## Historical Testnet rules and replay limits

Chain `20994` has two separate historical boundaries:

| Rule | Historical behavior | Current behavior |
| --- | --- | --- |
| Beneficiary validation | Before block `21,755,352`, accept historical non-fee-manager beneficiaries. | Require `PRECOMPILE_FEE_MANAGER` from block `21,755,352`. Other chains always retain this requirement. |
| Fee credit | Before block `21,781,417`, credit only the priority fee at London+ blocks. | Credit the full effective gas price from block `21,781,417`. Mainnet's full-fee rule is unchanged. |

The first boundary is confirmed by consecutive canonical headers: block `21,755,351`, hash
`0x30aac77c8bfc53b5f6e7bd80fa1c023f4a7df6ded0a22bbc51a97bfdb87b2354`, has beneficiary
`0x6659090f873cf50ea000e89ec5616150a6d7f6e4`; the next block has the fee manager.

[Fee vectors](../crates/revm/testdata/testnet-fee-history.json) retain RPC-derived block hashes,
receipts and beneficiary balances fetched from `https://rpc.testnet.fluent.xyz` on 2026-09-12:

- Block `21,781,414` credits `47,314,000,000,000` wei: 47,314 gas at a priority price of
  1,000,000,000 wei. Its effective price is 1,000,000,007 wei.
- Block `21,781,417` credits `55,510,800,323,813` wei: 46,259 gas at the full effective price of
  1,200,000,007 wei.
- Blocks `21,781,415` and `21,781,416` are empty. The implementation uses the first observed
  full-fee transaction block; choosing this boundary does not affect fee credit in the empty blocks.
  It is not evidence of the exact operator deployment instant within that empty interval.
- Block `21,845,831` already credits full fees, contradicting the old unmerged branch's
  proposed `21,845,842` cutoff. Do not use that old cutoff.

The fee-reward tests consume recorded balances independently of the handler's formula. To
refresh a vector, fetch its block with `eth_getBlockByNumber`, verify its hash, fetch each
`eth_getTransactionReceipt`, and compare `eth_getBalance` for the beneficiary at `n-1` and `n`.
Check transaction traces for transfers involving the beneficiary before interpreting the delta
as fee credit. These tests validate fee accounting, not execution of the recorded transactions.
Do not treat a historical debug tracer's computed post-balance as canonical: on block
`21,781,414`, the RPC debug re-execution credits an extra `331,198` wei (the base fee), while
`eth_getBalance` returns the recorded tip-only balance. Whole-block balances can serve as a
single-transaction oracle only after excluding other beneficiary transfers.

Full canonical state-root replay from the supported Testnet snapshot is still required for
FLU-1395. Existing partial transaction fixtures whose expected state hash is zero do not prove
that property. Preserve fixed RPC-derived beneficiary balances in historical fixtures when
available; computed fee expectations are only fallback checks.

The pinned Testnet v0.3.4-dev genesis contains system runtimes with a legacy import/entrypoint
ABI (including `_charge_fuel_manually`) that this node cannot execute. This predates the current
changes. Testnet history before the `21,300,000` fork is snapshot-served; operators must supply
and validate the supported snapshot and its installed runtimes before claiming replay support.
Do not add guessed import aliases or change the pinned genesis to make admission tests pass.
Mainnet/Devnet genesis admission and fixture-runtime admission do not establish compatibility
for every currently deployed runtime or prove complete historical execution.
