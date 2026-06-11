# Security Policy

This policy applies to Fluentbase, the Rust workspace for Fluent's blended execution stack.
Fluentbase includes execution runtimes, EVM/rWasm/SVM compatibility layers, system contracts,
genesis and runtime-upgrade tooling, SDK crates, examples, tests, and node integration code.

The project is security-sensitive because changes can affect consensus behavior, runtime
determinism, proof compatibility, account state, gas and fuel accounting, system precompile
semantics, and release artifacts used by node operators.

## Supported Versions

Security reports are accepted for the following branches:

| Branch | Support status | Notes |
| --- | --- | --- |
| `main` | Supported | Production-facing stable branch. |
| `devel` | Supported | Default development branch and the usual PR base. |
| `release/*` | Supported | Active release branches. Security fixes must preserve release-branch rules. |
| Other branches | Not supported | Use `main`, `devel`, or an active `release/*` branch unless a maintainer says otherwise. |

Tagged releases inherit support from the branch they were released from. Maintainers may narrow or
expand support in a release note, incident response thread, or issue comment.

## Reporting a Vulnerability

Report vulnerabilities publicly in GitHub by creating or commenting on a visible GitHub issue or
pull request and tagging `@dmitry123`.

Include enough information for maintainers to reproduce and triage the issue:

- Affected branch, commit, tag, or release artifact.
- Impacted component, such as `crates/runtime`, `crates/revm`, `contracts/evm`,
  `contracts/runtime-upgrade`, `crates/genesis`, `crates/node`, or CI/release automation.
- A minimal proof of concept, failing test, transaction, fixture, or command when possible.
- Expected behavior, actual behavior, and suspected security impact.
- Whether the issue affects consensus, state correctness, funds, availability, proof generation,
  release artifacts, or developer tooling only.

Because this reporting channel is visible, do not include private keys, live secrets, production
credentials, non-public user data, or access tokens. If a report needs sensitive exploit material,
post a minimal public report first and ask maintainers where to send the sensitive details.

Expected maintainer response:

1. A maintainer acknowledges the visible report.
2. The issue is labeled and scoped to the affected branches or releases.
3. A fix is prepared on a branch based on the relevant supported branch.
4. CI and security-relevant regression tests are reported before merge.
5. If the issue affects released artifacts or deployed networks, maintainers document the upgrade,
   mitigation, or release process.

## Disclosure, Safe Harbor, and Bounty Status

Good-faith security research is welcome when it avoids privacy violations, service disruption,
destructive testing, persistence, extortion, social engineering, and access to data or systems that
are not necessary to demonstrate the vulnerability.

The project does not currently advertise a public bug bounty. Do not assume monetary compensation
unless a maintainer explicitly publishes a bounty program or confirms compensation in writing.

When reporting a vulnerability, give maintainers reasonable time to investigate and prepare a fix
before broad public amplification. The initial report itself is visible by policy, but coordinated
follow-up may still be needed for high-impact issues.

## Assets and Security Goals

Critical assets:

- Runtime state transition correctness across EVM, rWasm, and other supported execution paths.
- Gas and fuel accounting, including conversion, charging order, and deterministic settlement.
- Runtime upgrade authority and host-side enforcement for upgrade syscalls.
- Genesis artifacts, runtime artifacts, release binaries, Docker images, and signatures.
- System contract behavior for EVM execution, native precompiles, runtime upgrades, and protocol
  contracts.
- Memory safety at guest/host boundaries, syscall handlers, ABI codecs, and rWasm integration.
- CI and release credentials, GitHub Actions configuration, and artifact upload paths.

High-value integrity goals:

- Valid transactions must produce deterministic state transitions on all supported execution paths.
- Runtime, genesis, and release artifacts must be reproducible enough for maintainers to audit.
- Guest-controlled offsets, lengths, gas, calldata, storage keys, and bytecode must be validated
  before arithmetic, allocation, host calls, or state changes.
- Runtime upgrades must remain owner-gated, auditable, and constrained to the intended execution
  path.

## Threat Model

Relevant threat actors include:

- Malicious contract authors submitting hostile bytecode, calldata, syscall inputs, or storage
  layouts.
- Malicious RPC users attempting denial of service through expensive execution, memory growth,
  pathological fixtures, or large payloads.
- Chain participants exploiting inconsistent EVM/rWasm semantics, gas/fuel mismatches, or runtime
  upgrade behavior.
- Supply-chain attackers targeting Rust dependencies, GitHub Actions, Docker images, release
  artifacts, or maintainer credentials.
- Contributors or AI coding agents introducing unsafe code, hidden allocation, nondeterminism, or
  release-breaking behavior.

Out of scope for this repository unless explicitly tied to Fluentbase code:

- Vulnerabilities in unrelated downstream applications.
- Social engineering against maintainers.
- Attacks requiring compromised private keys, unless Fluentbase mishandles the key material.
- Network-level or infrastructure issues outside this repository's node, release, or artifact code.

## Attack Surface

Primary attack surfaces:

- EVM execution and syscall routing in `crates/revm`.
- Runtime syscall handlers, memory reads and writes, hashing, host execution, resume, logging, and
  output handling in `crates/runtime`.
- rWasm integration and runtime interoperability paths documented in `docs/07-rwasm-integration.md`.
- System contracts under `contracts/`, especially `contracts/evm`, `contracts/runtime-upgrade`, and
  precompile-style wrappers.
- ABI, codec, SDK, and derive crates under `crates/codec`, `crates/sdk`, and `crates/sdk-derive`.
- Genesis and runtime artifact generation under `crates/genesis`, `bins/runtime-upgrade`, and
  release workflows.
- Node execution and integration code under `crates/node` and `bins/fluent`.
- CI, release, Docker, benchmark, and artifact workflows under `.github/workflows`.
- EVM state tests, fixtures, examples, and e2e harnesses under `evm-e2e`, `e2e`, and `examples`.

Untrusted inputs include transaction calldata, deployed bytecode, rWasm modules, EVM state tests,
guest memory offsets and lengths, syscall parameters, JSON fixtures, CLI arguments, environment
configuration, and GitHub Actions event inputs.

## Auditor's Checklist

These vulnerability classes recur in execution and codec code and are the highest-yield. Each is a
reachable-from-untrusted-input class, not a hypothetical. An auditor should check these first before
reasoning from scratch. AI agents in audit mode: see `AGENTS.md` → Security Audit Mode for how to run.

Decoders / codec (`crates/codec`, `crates/codec-derive`, `crates/sdk`, `crates/types`):

- **Header validated, body not.** The dominant pattern: a length/offset word is bounds-checked when
  read (e.g. `read_u32_aligned` validates its own 4-byte read), then the resulting offset/length is
  used in `buf.chunk()[a..b]` or `with_capacity(n)` with no check that the described body is inside
  the buffer. Every decode that reads an offset/length from input and then slices or allocates MUST
  re-validate `offset (+ headroom) + len <= buf.len()` with checked arithmetic. Slice-out-of-range
  panics the decoder — a remote DoS from any contract entrypoint that decodes `bytes`/`string`/`Vec`/
  `HashMap`/derived structs.
- **Unbounded allocation.** `Vec::with_capacity(n)` / `HashMap::with_capacity(n)` / `vec![0; n]`
  where `n` is an attacker-controlled count → OOM abort. Bound `n` by remaining-bytes / element size
  before allocating.
- **Arithmetic on offsets/lengths.** `as u32` / `as usize` truncation on length fields; `offset + 32`
  (or `align_up`) wrapping, then used as an index. Use checked arithmetic on every decode path.
- **Swallowed decode errors.** `unwrap_or_default()` on a per-element decode hides corruption and
  makes two distinct inputs decode to the same value — a determinism/correctness risk. Propagate with
  `?`.
- **LEB128 / varint** (`crates/sdk/src/leb128.rs`): bound byte count and shift. An unterminated or
  oversized varint must return an error, not loop unbounded or overflow the target integer.

Runtime / syscalls (`crates/runtime`):

- Guest-supplied offset/length used to read or write guest memory without a bounds check.
- Fuel/gas charged AFTER an expensive operation instead of before it.
- Fixed-size buffer reads in the crypto/hashing/curve syscall handlers without validating the
  guest-provided length first.

Precompiles (`contracts/*`, `crates/crypto`):

- Must accept ANY input length (empty / short / huge). Slicing a fixed-size chunk without a length
  guard panics on crafted calldata. `modexp`: overflow in the gas formula and absurd base/exp/mod
  length fields. `blake2f`: rounds parameter and final-block flag validation per EIP-152. Point
  decompression (`bn254`, `bls12381`, edwards/weierstrass): subgroup and curve-membership checks.
  `ecrecover` / `secp256r1` / `ed25519` / `webauthn`: signature malleability and wrong-length slices.

EVM / consensus (`crates/evm`, `crates/revm`):

- Gas arithmetic without checked/saturating ops; memory-expansion cost overflow.
- Any divergence from canonical EVM semantics, or dependence on map-iteration order, is a consensus
  split.

Build / CI (`crates/build`, `.github/workflows`):

- Untrusted WASM translated to rWasm without validation; path traversal or command execution at build
  time.
- `pull_request_target` combined with `${{ github.event.* }}` interpolated into `run:` steps (script
  injection); secrets exposed to fork PRs; unpinned third-party actions used at a mutable ref.

## Existing Security Controls

Important controls already present in the repository:

- `docs/05-security-invariants.md` documents deterministic routing, memory, gas/fuel, storage,
  runtime-upgrade, and release invariants.
- `docs/06-runtime-upgrade.md` documents the privileged runtime-upgrade path and host enforcement.
- CI runs on `main`, `devel`, and `release/*` branches.
- Rust workspace checks, clippy, tests, and nextest suites are documented in `AGENTS.md`.
- The release workflow builds artifacts and signs mainnet genesis outputs.
- EVM system precompile handling contains explicit compatibility comments and branch-sensitive
  behavior.
- Runtime and syscall code has targeted handling for guest memory, host execution, gas/fuel
  accounting, and interruption/resume behavior.

These controls do not remove the need for review. Security-sensitive changes still require focused
tests, explicit reasoning, and CI status reporting.

## Secure Coding Guidelines

Follow these rules for all Fluentbase contributions:

- Preserve deterministic behavior. Do not introduce wall-clock time, host randomness, map iteration
  order dependencies, platform-specific behavior, or nondeterministic serialization in runtime paths.
- Validate guest-controlled offsets, lengths, counts, gas values, calldata, and bytecode before
  arithmetic, allocation, memory reads, memory writes, or host calls.
- Use checked arithmetic for address, offset, length, gas, fuel, and allocation calculations.
- Return deterministic runtime errors instead of panicking or relying on host OOM behavior for
  invalid guest input.
- Treat `unsafe` as security-critical. Keep it minimal, document the invariant it relies on, and add
  tests around the boundary when behavior can regress.
- Preserve `no_std` constraints for runtime, SDK, contract, and wasm-targeted code unless the crate
  feature model explicitly allows `std`.
- Keep runtime-upgrade changes tightly scoped. Changes to upgrade authority, code hash handling,
  genesis wiring, or host enforcement require extra review and release notes.
- Preserve EVM compatibility when handling precompiles, gas, storage, revert behavior, logs, account
  warmth, and code size semantics.
- Do not silently change genesis, runtime, or release artifacts. Explain how generated artifacts were
  produced and why they changed.
- Do not commit secrets, private keys, credentials, local datadirs, generated wallets, or production
  config.
- Do not add broad dependencies to runtime-sensitive crates unless the dependency is necessary,
  audited for the use case, and compatible with the crate's feature model.
- Keep tests close to the changed behavior. Security fixes should include regression coverage for
  the failure mode when feasible.

## AI Coding Agent Rules

AI coding agents working in this repository must:

- Read this file and `AGENTS.md` before security-sensitive edits.
- Check `git status --short` before editing and before reporting completion.
- Base branch work on the latest `origin/devel` unless the issue targets `main` or `release/*`.
- Avoid rewriting, deleting, or rebasing user work without explicit instruction.
- Avoid inventing security contacts, bounty terms, release support, or disclosure timelines.
- Do not mark a security task complete without reporting local checks and PR/CI status.
- Escalate when a requested change would weaken runtime determinism, upgrade authority, release
  integrity, or public vulnerability handling.

## Security-Related Files and Areas

- `SECURITY.md` - this policy.
- `AGENTS.md` - repository workflow and agent rules.
- `docs/05-security-invariants.md` - protocol and runtime invariants.
- `docs/06-runtime-upgrade.md` - runtime-upgrade control plane.
- `docs/07-rwasm-integration.md` - rWasm integration and upgrade process.
- `.github/workflows/ci.yml` - CI coverage for `main`, `devel`, and `release/*`.
- `.github/workflows/release.yml` - release artifact build, signing, and upload path.
- `crates/runtime` - runtime syscall handling.
- `crates/revm` - EVM execution and syscall integration.
- `contracts` - system contracts and precompile wrappers.
- `crates/genesis` - genesis data and artifact handling.
- `bins/fluent` and `crates/node` - node binary and node integration.

## Revision History

| Date | Change |
| --- | --- |
| 2026-05-28 | Initial repository-specific security policy. |
| 2026-06-10 | Added Auditor's Checklist (known vulnerability classes); paired with AGENTS.md Security Audit Mode. |
