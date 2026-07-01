# Runtime Upgrade

Runtime upgrade is a privileged control plane.
It exists so chain operators can replace system runtime bytecode, but it must stay tightly constrained.

## High-level flow

1. Governance owner calls runtime-upgrade contract.
2. Input wasm is validated and compiled to rWasm.
3. Contract invokes native upgrade syscall with target address + serialized rWasm module.
4. Host side verifies caller path and installs new code at target.
5. Upgrade emits target/genesis refs/code hash; recompile emits target/code hash.

---

## Contract-level controls

Upgrade contract exposes:

- `upgradeTo(...)`
- `recompile(...)`
- `planUpgrade(...)`
- `upgradeToPlanned(...)`
- `changeOwner(...)`
- `owner()`
- `renounceOwnership()`

Key behavior:
- only owner can upgrade,
- `recompile(address)` loads the target account bytecode with `code_size`/`code_copy`, requires it to be
  original WASM bytes, recompiles it to rWasm, and then uses the same upgrade syscall path as
  `upgradeTo(...)`,
- `upgradeTo(...)` emits `RuntimeUpgraded`, while `recompile(...)` emits `ContractRecompiled`,
- `planUpgrade(...)` lets the owner pre-authorize a release batch as exact `(target, raw WASM
  hash)` pairs plus release metadata and an authorized upgrador,
- `upgradeToPlanned(...)` can be called only by that upgrador and only for a stored target/hash
  pair; the pair is removed after successful installation to prevent replay,
- planned upgrades bind hashes to target addresses so a delegated upgrader cannot reuse approved
  bytecode against the wrong system contract,
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

## Runtime-upgrade CLI

The `runtime-upgrade` CLI prepares upgrade transactions from a Fluentbase release genesis file.
It downloads `genesis-<release-tag>.json.gz`, extracts the target WASM bytecode, compares it with
the current chain state, and builds calls to the runtime-upgrade precompile.

The CLI always uses the runtime-upgrade precompile path:

- transaction `to` is `PRECOMPILE_RUNTIME_UPGRADE`,
- calldata is `UPDATE_GENESIS_PREFIX` followed by ABI-encoded
  `(target, genesisHash, genesisVersion, wasmBytecode)`,
- legacy direct-to-target upgrade calldata is not supported.

Build the CLI before use:

```bash
cargo build -p fluentbase-runtime-upgrade --bin runtime-upgrade
```

Generate a Safe Transaction Builder bundle without signing or broadcasting:

```bash
cargo run -p fluentbase-runtime-upgrade --bin runtime-upgrade -- \
  --genesis <release-tag> \
  --test \
  --contract PRECOMPILE_EVM_RUNTIME \
  --safe-bundle /tmp/runtime-upgrade.json
```

Use `--dev` for devnet, `--test` for testnet, `--local` for `http://localhost:8545`, or
`--rpc <url>` for a custom endpoint. To prepare every known contract, omit `--contract` and confirm
the prompt:

```bash
cargo run -p fluentbase-runtime-upgrade --bin runtime-upgrade -- \
  --genesis <release-tag> \
  --test \
  --safe-bundle /tmp/runtime-upgrade-all.json
```

Print a signed raw transaction instead of broadcasting. The signer comes from `--private-key`,
`PRIVATE_KEY`, or an interactive hidden prompt:

```bash
PRIVATE_KEY=<hex-private-key> \
cargo run -p fluentbase-runtime-upgrade --bin runtime-upgrade -- \
  --genesis <release-tag> \
  --test \
  --contract PRECOMPILE_EVM_RUNTIME \
  --print-raw-tx
```

Broadcast directly only after validating the generated transaction data and signer:

```bash
PRIVATE_KEY=<hex-private-key> \
cargo run -p fluentbase-runtime-upgrade --bin runtime-upgrade -- \
  --genesis <release-tag> \
  --test \
  --contract PRECOMPILE_EVM_RUNTIME
```

Use `--gas-limit <gas>` when the target network requires an explicit gas limit.

---

## Operational expectations

For real upgrades:

- produce deterministic build artifacts,
- publish target-address-to-WASM-hash pairs for planned releases,
- use multisig/operator quorum controls,
- roll out to all nodes coherently,
- verify post-upgrade code hash and runtime behavior,
- keep audit trail of who upgraded what and when.

Runtime upgrade changes consensus behavior; treat every upgrade as fork-critical change management.
