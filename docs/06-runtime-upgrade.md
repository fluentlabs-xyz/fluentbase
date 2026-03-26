# Runtime Upgrade

Runtime upgrade is a privileged control plane.
It exists so chain operators can replace system runtime bytecode, but it must stay tightly constrained.

## High-level flow

1. Governance owner calls runtime-upgrade contract.
2. Input wasm is validated and compiled to rWasm.
3. Contract invokes native upgrade syscall with target address + serialized rWasm module.
4. Host side verifies caller path and installs new code at target.
5. Upgrade event is emitted with metadata (target/genesis refs/code hash).

---

## Contract-level controls

Upgrade contract exposes:

- `upgradeTo(...)`
- `changeOwner(...)`
- `owner()`
- `renounceOwnership()`

Key behavior:
- only owner can upgrade,
- zero owner assignment is rejected by `changeOwner`,
- renounce sets owner to system address,
- default owner fallback is defined for unset state.

---

## Host-side enforcement

Upgrade syscall handler enforces:

- not callable in static context,
- allowed only via runtime-upgrade precompile execution path,
- payload must decode correctly,
- bytecode must be valid rWasm payload,
- target account is loaded and code is replaced deterministically.

This enforcement is the final security boundary; contract-side checks alone are not enough.

---

## Legacy testnet-only hook

There is an explicit temporary testnet hook path for legacy upgrade behavior.
It is chain-id gated and documented as temporary.

Treat it as compatibility debt, not target architecture.

---

## Operational expectations

For real upgrades:

- produce deterministic build artifacts,
- use multisig/operator quorum controls,
- roll out to all nodes coherently,
- verify post-upgrade code hash and runtime behavior,
- keep audit trail of who upgraded what and when.

Runtime upgrade changes consensus behavior; treat every upgrade as fork-critical change management.
