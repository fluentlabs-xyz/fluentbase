# AGENTS.md - Fluentbase Development Guide

This file is for coding agents working in this repository. Follow it unless a more specific user instruction or a nested `AGENTS.md` overrides it.

## Project Snapshot

Fluentbase is a Rust workspace for the Fluent L2 execution stack. The core idea is blended execution: EVM/SVM/WASM/UST compatibility layers converge into rWasm IR and a single proof-friendly runtime/STF.

Important areas:

- `bins/` - binary entrypoints, especially the `fluent` CLI.
- `crates/` - core Rust crates (`runtime`, `revm`, `evm`, `sdk`, `codec`, `node`, `genesis`, etc.).
- `contracts/` - system contracts and genesis/runtime upgrade contract artifacts.
- `examples/` - example contracts/apps.
- `e2e/` - end-to-end tests and benchmarks.
- `evm-e2e/` - separate EVM state-test/fixture runner crate, intentionally excluded from the root workspace.
- `flips/`, `docs/` - design and documentation.

SVM-related crates are currently unstable and excluded from the top-level workspace unless explicitly requested.

## Working Rules

- Protect local work. Do not overwrite, reset, rebase, or delete user changes unless explicitly asked.
- Check `git status --short` before editing and again before reporting completion.
- Keep changes focused and minimal. Avoid opportunistic refactors.
- Prefer fixing root causes over broad compatibility shims.
- Do not vendor generated output or large artifacts unless the task explicitly requires it.
- Preserve `no_std` constraints where crates are configured for it.
- Be careful with genesis/runtime changes: they may be chain-breaking and require release/upgrade planning.

## Branch and Git Standards

- Default remote base branch is `origin/devel` in this repo. Rebase/start branch work from latest remote base before implementation unless the issue/PR targets another branch.
- Use Conventional Commits for commits and PR titles:
  - `feat: ...`
  - `fix: ...`
  - `docs: ...`
  - `refactor: ...`
  - `test: ...`
  - `chore: ...`
- Branch names should be short and typed, for example:
  - `fix/evm-gas-accounting`
  - `feat/fixture-tx-export`
  - `docs/runtime-upgrade-notes`
- After opening/updating a PR, check CI until it is green or clearly report pending/failing checks as a blocker.

## Rust Style

- Rust edition: 2021.
- Workspace rust version in `Cargo.toml`: `1.92.0`; CI currently installs stable.
- Formatting is controlled by `.rustfmt.toml`:
  - max width 100
  - crate-level import granularity
  - grouped imports
  - Unix newlines
- Run `cargo fmt`/`cargo fmt --check` for touched Rust code.
- Clippy warnings are errors in CI (`-D warnings`).
- Prefer explicit, deterministic behavior. Fluentbase code often runs in VM/proving/runtime-sensitive contexts.
- Avoid hidden allocation or `std` dependencies in runtime/SDK paths unless the crate feature model already allows it.

## Verification Ladder

Pick the smallest meaningful gate first, then expand if the change is broad.

Common quick checks:

```bash
cargo fmt --check
cargo check --all
cargo check -p <crate>
cargo test -p <crate> <test_name>
```

Full local checks from the Makefile:

```bash
make check       # cargo check --all
make clippy      # root/contracts/examples clippy with -D warnings
make test        # release nextest suites for contracts/examples/root/evm-e2e
make pr          # clippy + test
```

CI-representative commands:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --manifest-path=./contracts/Cargo.toml --workspace --all-targets -- -D warnings
cargo clippy --manifest-path=./examples/Cargo.toml --workspace --all-targets -- -D warnings

cargo nextest run --manifest-path=./Cargo.toml --workspace --release --no-default-features --features std,wasmtime --no-fail-fast --locked
cargo nextest run --manifest-path=./Cargo.toml --workspace --release --no-default-features --features std --no-fail-fast --locked
cargo nextest run --manifest-path=./contracts/Cargo.toml --workspace --release --no-default-features --features std --no-fail-fast --locked
cargo nextest run --manifest-path=./examples/Cargo.toml --workspace --release --no-default-features --features std --no-fail-fast --locked
```

For `evm-e2e`:

```bash
make -C evm-e2e sync_tests
cargo nextest run --manifest-path=./evm-e2e/Cargo.toml --release --no-default-features --features std --package evm-e2e --bin evm-e2e tests::good_coverage_tests
cargo nextest run --manifest-path=./evm-e2e/Cargo.toml --release --no-default-features --features std,wasmtime --package evm-e2e --bin evm-e2e tests::good_coverage_tests
cargo nextest run --manifest-path=./evm-e2e/Cargo.toml --release --no-default-features --features std --package evm-e2e --bin evm-e2e fixture
cargo nextest run --manifest-path=./evm-e2e/Cargo.toml --release --no-default-features --features std,wasmtime --package evm-e2e --bin evm-e2e fixture
```

Use targeted versions of these commands when full suites are too expensive, and clearly state what was and was not run.

## EVM / Fixture Work

- `evm-e2e` is a separate crate. Do not assume root workspace commands include it.
- Reuse existing fixture plumbing instead of duplicating parsing logic:
  - `resolve_externalized_bytecodes`
  - `prepare_env`
  - `fill_tx_env`
  - `execute_fluent_test_suite`
  - `execute_evm_test_suite`
- Many fixture transaction fields are computed after environment preparation and post-index selection. If exporting/replaying transactions, derive from the final `TxEnv`, not raw JSON alone.
- Ethereum state-test `post` cases usually start from the same prestate independently; do not treat all post entries as one sequential blockchain script unless explicitly modeled that way.
- For reproducibility, include chain id, fork/spec, block env, prestate assumptions, raw signed txs if available, and expected state/log/output roots.

## Node / Reth Integration Work

- Be conservative around shutdown, payload building, FCU/block processing, and background task lifecycle.
- Favor clean cancellation/drain semantics over abrupt task termination when in-flight block work may exist.
- For node changes, at minimum run focused checks such as:

```bash
cargo fmt --check --package fluentbase-node
cargo check -p fluentbase-node
```

Expand to tests/clippy if behavior or public interfaces changed.

## Dependencies and Generated Artifacts

- Use `cargo update` only when dependency updates are the task. The Makefile has an `update-deps` target for revm/rwasm across root, contracts, examples, and evm-e2e.
- Do not edit lockfiles casually.
- Be cautious with generated contract/genesis artifacts. If a build script or checked-in generated file changes, explain why and how it was regenerated.

## PR Reporting Checklist

When finishing work, report:

- What changed, in 1-3 bullets.
- Files/areas touched.
- Tests/checks run, exact commands and outcomes.
- Any checks skipped and why.
- PR link, if opened.
- CI status: green, pending, failing, or blocked.

If blocked, say exactly what input, credential, environment, or failing check is blocking progress.
