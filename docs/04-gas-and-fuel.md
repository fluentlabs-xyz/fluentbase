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

## Calldata surcharge

Large calldata gets extra quadratic surcharge above threshold.
Purpose is practical block-data pressure control.

This is part of block economics, not only runtime internals.

---

## Operational invariants

- never allocate large host buffers before validating/bounding lengths,
- charge before expensive host work where feasible,
- keep conversion/rounding unchanged unless explicitly coordinated,
- treat gas/fuel mapping changes as fork-level changes.
