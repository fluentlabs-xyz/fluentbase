# Gas and Fuel

Fluentbase uses two accounting units:

- **gas** for EVM-visible execution economics,
- **fuel** for runtime (rWasm/engine) execution.

They are linked by a fixed conversion ratio.

---

## Conversion model

Current ratio:

- `FUEL_DENOM_RATE = 20`

Operationally:

- runtime fuel limit is derived from available gas,
- consumed/refunded fuel is converted back into gas settlement,
- conversion and rounding behavior must stay deterministic.

If this ratio changes, execution economics and charging paths change everywhere.

---

## Where settlement happens

During runtime calls and resumes, host side:

1. computes runtime fuel limit from remaining gas,
2. executes runtime step,
3. converts returned fuel consumption/refund back to gas,
4. applies gas deltas to interpreter state.

This is critical for keeping EVM-visible gas usage consistent with runtime work.

---

## Internal EVM runtime sync

Delegated EVM runtime keeps its own committed-gas tracking.
Before interruption/final return, it synchronizes committed delta to host fuel.

This prevents drift between local interpreter gas and host-side charged fuel.

---

## Import-level fuel schedules

Runtime imports have explicit fuel formulas (const/linear/quadratic) attached to syscall indexes.
Examples:

- copy/hash/log operations: linear by data size,
- `exec`: quadratic policy,
- some state/control calls: constant.

These formulas are part of runtime ABI behavior, not optional heuristics.

---

## Engine-metered vs self-metered system runtimes

Not all system runtimes meter fuel the same way.

- **self-metered**: runtime code charges fuel explicitly,
- **engine-metered**: execution engine automatically meters configured precompiles.

Universal Token runtime is currently in engine-metered set.

---

## EIP-8037 state gas

[EIP-8037](https://eips.ethereum.org/EIPS/eip-8037) (Amsterdam) adds a state-gas component to
operations that create state. The active spec is Osaka, so it is inert today, but the syscall
handlers already mirror the EVM interpreter so a fork that schedules it does not open a gap
between EVM bytecode and rWasm contracts: `STORAGE_WRITE` and `METADATA_STORAGE_WRITE` charge the
slot-creation state gas next to the dynamic SSTORE cost, `DESTROY_ACCOUNT` charges the
account-creation state gas when the beneficiary is created by the top-up, and the CALL family
charges it when a value transfer creates the callee. All of it is gated on
`is_amsterdam_eip8037_enabled()`.

---

## Buffered system-runtime effects

The buffered `RuntimeExecutionOutcomeV1` commit path has accepted gas-pricing limitations:

- Storage writes pay the SLOAD-priced preload but omit dynamic SSTORE write costs (FLU-1160).
- Buffered logs omit the LOG base/topic/data gas charged by the direct `EMIT_LOG` syscall (FLU-1306).
- Buffered native transfers omit CALL-style account-access, value-transfer, and new-account gas
  (FLU-1306).

Preserve this policy for existing networks. Repricing affects calls to already-deployed contracts
and can change gas usage, transaction success, and resulting state. It requires a coordinated
network fork that retains the previous execution rules before activation and during historical replay.

Metadata deposit is already charged by the EVM and Universal Token constructors before they emit
`new_metadata`. EVM charges canonical runtime bytecode; Universal Token charges its metadata payload.
The host must not charge that deposit again.

---

## Ethereum compatibility: transaction gas and calldata

[EIP-7825](https://eips.ethereum.org/EIPS/eip-7825) caps the gas limit declared by an Ethereum
transaction at `2^24` gas (16,777,216), independently of the block gas limit. Ethereum clients
reject transactions above that cap from the transaction pool and reject blocks containing them.

Fluentbase does not enforce this fixed per-transaction cap. WASM smart contracts can require more
than `2^24` gas, so limiting every transaction to Ethereum's cap would prevent valid WASM execution.
This does not make transaction execution literally unlimited: the transaction gas limit must still
fit within the current block gas limit and pass the usual intrinsic-gas, balance, and fee checks.

Fluentbase instead controls large transaction input through gas pricing. Input up to 128 KiB
(131,072 bytes) receives no Fluent-specific surcharge beyond the normal intrinsic calldata gas.
For larger input, only the excess is divided into 32-byte words and charged using:

```text
words = ceil((input_length - 128 KiB) / 32)
surcharge = 3 * words + floor(words^2 / 30)
```

The transaction is rejected if its declared gas limit cannot cover the intrinsic gas plus this
surcharge. This policy allows transactions above 128 KiB rather than imposing a protocol-level
input-size limit, while the quadratic term constrains block-data pressure.

Consequently, Ethereum tooling must not assume that Fluentbase applies EIP-7825: a transaction with
a gas limit above `2^24` can be valid on Fluentbase if it satisfies the block-level and economic
constraints above.

---

## Operational invariants

- never allocate large host buffers before validating/bounding lengths,
- charge before expensive host work where feasible,
- keep conversion/rounding unchanged unless explicitly coordinated,
- treat gas/fuel mapping changes as fork-level changes.
