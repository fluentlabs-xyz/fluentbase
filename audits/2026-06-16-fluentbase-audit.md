# Fluentbase Security Audit — 2026-06-16

- **Date:** 2026-06-16 (fixes landed 2026-06-17)
- **Repository:** `fluentlabs-xyz/fluentbase`
- **Audited commit:** not recorded in the ticket (branch `devel`)
- **Linear:** `FLU-856`
- **Fix PRs:** #440
- **Focus:** Offensive re-audit along three axes — value security (mint/steal in
  system contracts), node liveness (crafted tx/block halting the node), and common
  Rust attack classes + dependency advisories.

## Scope

The token, bridge, fee-manager, storage, and upgrade-authority value paths; host
panic/liveness surfaces (module factory, deploy path, decoders, genesis loading);
and workspace dependency advisories. Two headline results: (1) no
mint/steal/double-spend/upgrade-bypass exists — the value layer is well-built and
needed no code changes; (2) the real new risk is a panic-amplifier that can turn
any single host panic into a persistent chain halt.

## Result

| Crit | High | Medium | Low/Info |
| --- | --- | --- | --- |
| 0 | 2 | 3 | several |

All value-theft attacks were **blocked** (see Notes). The findings below are
liveness, allocation, and process/operational hardening items.

## Findings

### High

#### N1 — Panic amplifier: any host panic can permanently brick the node

- **Severity:** High · **Status:** Recommended (tracked) · **Linear:** `FLU-856` · **Fix:** —
- **Where:** `Cargo.toml` `panic = "unwind"` with **zero** `catch_unwind` in the
  codebase; `crates/runtime/src/module_factory.rs` holds execution state behind a
  global `Arc<Mutex>` accessed via `.lock().unwrap()`.
- **Impact:** If any host panic fires while the lock is held, the `Mutex` is
  **poisoned**, so every subsequent block's `.lock().unwrap()` panics forever — a
  persistent halt, not a one-block failure. This converts every latent/gated host
  panic into a much sharper liveness risk.
- **Remediation:** Wrap block/tx execution in `catch_unwind` (or `panic = "abort"`
  + supervised restart), and replace `.lock().unwrap()` with poison recovery
  (`lock().unwrap_or_else(|e| e.into_inner())`).

#### N2 — Host panic on malformed module at deploy

- **Severity:** High · **Status:** Fixed · **Linear:** `FLU-856` · **Fix:** #440
- **Where:** `crates/revm/src/executor.rs` — `RwasmModule::new(...)` reaches
  `unreachable!("rwasm: malformed rwasm binary")` on bad bytes, running natively
  during CREATE.
- **Impact:** Gated today to delegated-runtime-owned accounts, but it sits directly
  on the runtime-upgrade / recompile path. Any translator bug emitting a
  `0xEF`-prefixed-but-undecodable output is an instant halt.
- **Remediation:** Return `ExitCode::MalformedBuiltinParams`; never `unreachable!`
  on bytes crossing the VM boundary.

#### Genesis signature verification stub

- **Severity:** High (devnet/testnet) / Low (mainnet) · **Status:** Fixed · **Linear:** `FLU-856` · **Fix:** #440
- **Where:** `crates/node/src/utils.rs` — signature verification is a no-op stub.
- **Impact:** Devnet/testnet have no hash pin, so a substituted genesis is
  accepted; mainnet is hash-pinned.
- **Remediation:** Implement real fail-closed detached-signature verification.

### Medium

#### N3 — Unbounded host allocation (OOM) in bincode decode

- **Severity:** Medium (latent) · **Status:** Recommended (tracked) · **Linear:** `FLU-856` · **Fix:** —
- **Where:** `crates/types/src/bincode.rs` and `bincode::config::legacy()` callers
  set no decode limit; host-side `Vec::with_capacity(len)` in
  `system/new_frame_input.rs` and `system/journal_storage.rs` trusts a length
  prefix.
- **Impact:** OOM primitive; gated today because the lengths come from the trusted
  system runtime.
- **Remediation:** Apply `.with_limit::<N>()` on every host decode.

#### Genesis zip-bomb

- **Severity:** Medium · **Status:** Fixed · **Linear:** `FLU-856` · **Fix:** #440
- **Where:** `crates/node/src/utils.rs` — `read_to_string` on a `GzDecoder` with no
  size cap → startup OOM.
- **Remediation:** `.take(MAX)` on the decompression stream.

#### Shared default authority for two roles (operational)

- **Severity:** Medium (operational) · **Status:** Recommended (tracked) · **Linear:** `FLU-856` · **Fix:** —
- **Where:** `crates/types/src/genesis.rs` sets the same default authority for both
  runtime-upgrade **and** fee-manager.
- **Impact:** Whoever holds that key controls arbitrary-code-install on any account
  **and** the fee treasury until owners are reassigned.
- **Remediation:** Use distinct keys per role; confirm the post-genesis runbook
  reassigns both to multisigs.

### Low / Informational

- **No `cargo-deny`/`cargo-audit` gate** (process gap). Live advisory hits were all
  transitive/non-consensus (`ring 0.16.20`, `rustls 0.21`, `tungstenite 0.20`,
  `spin 0.5.2`); `curve25519-dalek 4.1.3` is the patched version. Dead
  `libsecp256k1` declared in workspace deps — remove.
- **`unsafe impl Send/Sync for EthVM`** (`crates/evm/src/evm.rs`) — unsound over
  revm's `Rc`-backed `SharedMemory`; no cross-thread use today (latent).
- **`overflow-checks = false` in release** — not exploitable (value/gas/fuel paths
  use `checked_*`/`saturating_*`); enable once N1 is fixed.
- **Corrected ratings from the prior round:** revm `panic!("revm: fatal external
  error")` is a real DB/`ContextError`, not calldata → Info; `crypto.rs`
  `unwrap_exit_code` panics run guest-side → Low; `prevrandao().unwrap()` and
  `unreachable!` syscall-param guards are host-constructed/header-gated → Low.

## Notes

**Value-theft matrix (all BLOCKED).** Every attack attempted against the value
layer was blocked; no code change was needed there.

| Attack | Blocked by |
| --- | --- |
| Unauthorized mint to self | `caller == minter` gate, `universal-token/src/lib.rs` |
| Burn someone else's tokens | minter-only guard |
| `transferFrom` over/without allowance | `checked_sub` allowance written before balances |
| Balance over/underflow (overflow-checks OFF) | every balance/supply/allowance op uses `checked_add`/`checked_sub` |
| Permit signature replay | nonce increment + chainId/contract-bound domain + low-s, `erc2612.rs` |
| Storage-key collision | `keccak256(key‖slot)` + ERC-7201 namespaces, `crates/sdk/src/storage/map.rs` |
| Direct `UPGRADE_RUNTIME` syscall | `target == PRECOMPILE_RUNTIME_UPGRADE` assert, `crates/revm/src/syscall.rs` |
| Bridge mint with no L1 deposit / forged event | post-hook requires one log from the bridge address, `crates/revm/src/bridge.rs` |
| Bridge double-mint via nested calls / replay | per-frame log window, journaled mint reverts on revert |
| Wrapped-token withdraw drain | reconciliation rejects `balance < Σtransfers`, `crates/revm/src/executor.rs` |
| fee-manager non-owner withdraw | `only_owner` on every mutator, `contracts/fee-manager/src/lib.rs` |

**Fix priority (as filed):** (1) N1 panic amplifier — highest leverage,
de-risks every other host panic; (2) genesis signature stub + zip-bomb cap;
(3) N2; (4) N3 bincode limits + cargo-deny gate + dep dedup + remove `EthVM`
Send/Sync + remove dead `libsecp256k1`.

## Re-review checklist for future audits

- Confirm block/tx execution is panic-contained and the module-factory mutex
  recovers from poisoning (N1).
- Confirm no `unreachable!`/`unwrap` on bytes crossing the VM boundary on the
  CREATE / runtime-upgrade path (N2).
- Confirm every host-side bincode decode has an explicit limit (N3).
- Confirm genesis signature verification is fail-closed and gz decoding is size
  capped.
