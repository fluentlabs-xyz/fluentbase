# Fluentbase Quickstart

Goal: Go from a fresh clone to building, (optionally) deploying, and verifying a sample Fluentbase contract artifact.

> This guide is intentionally concise. If anything is unclear while you follow it, open an issue and reference the step number.

---

## 1. Prerequisites

Install / have available:
- Rust (stable) — check: `rustc --version`
- Cargo — comes with Rust
- Make — `make --version`
- Docker — for deterministic (reproducible) builds
- WebAssembly target:  
  ```bash
  rustup target add wasm32-unknown-unknown
  ```

Optional:
- `wasm-tools` (for inspecting output): `cargo install wasm-tools`

---

## 2. Clone the Repository

```bash
git clone https://github.com/fluentlabs-xyz/fluentbase.git
cd fluentbase
```

(Optional) Add upstream (if you later fork):
```bash
git remote add upstream https://github.com/fluentlabs-xyz/fluentbase.git
```

---

## 3. Workspace Bootstrap & Sanity Checks

```bash
# Build everything (crates + examples + genesis artifacts)
make

# Lints / formatting / basic checks
make check

# Run tests (unit + any configured e2e)
make test
```

If any step fails, capture:
- Commit hash: `git rev-parse HEAD`
- Rust version: `rustc --version`
- Full command + output
…then open/augment an issue.

---

## 4. Build an Example (Makefile Route)

Examples are grouped in a single crate with Cargo feature flags.

List available example feature names (quick heuristic):
```bash
grep 'features' -n contracts/examples/Cargo.toml
# then inspect below the [features] table
```

Build all examples:
```bash
make examples
```

Build one example (replace NAME with the feature / example target if present in the Makefile):
```bash
make NAME
```

Resulting optimized WASM usually lands at:
```
target/wasm32-unknown-unknown/release/<example>.wasm
```

---

## 5. Build with the CLI (Deterministic Artifacts)

Install the CLI locally (temporary path install):
```bash
cargo install --path bins/cli
```
(or run without installing: `cargo run -p fluentbase-cli -- <command>`)

Common build forms:

```bash
# Recommended: Docker-enabled reproducible build (default)
fluentbase-cli build . --generate rwasm,abi,metadata

# Explicitly disable Docker (faster, but NOT reproducible)
fluentbase-cli build . --no-docker --generate rwasm

# With custom feature set (comma separated)
fluentbase-cli build . --features my-feature,another-feature --generate rwasm,abi
```

Artifacts (default output dir = `./out` unless changed with `--output`):
- `<name>.rwasm`  (reduced WebAssembly deployable)
- `<name>.wasm`
- `<name>.abi.json`
- `<name>.metadata.json`
- (optionally) `<name>.wat`, `<name>.sol` depending on flags/support

Using cargo run instead of install:
```bash
cargo run -p fluentbase-cli -- build . --generate rwasm,abi,metadata
```

---

## 6. (Optional) Deploy an Artifact

Deployment tooling may live in a separate CLI (e.g., `gblend`) or forthcoming Fluentbase extensions. If you already have a deploy flow that accepts `.rwasm`:

Example (pseudo-flow):
```bash
# Set environment (example values)
export FB_RPC_URL=http://localhost:8545
export FB_PRIVATE_KEY=0xYOUR_LOCAL_DEV_PRIVATE_KEY
# Then use your deployment tool, e.g.:
some-deploy-cli deploy out/example.rwasm --rpc $FB_RPC_URL --private-key $FB_PRIVATE_KEY
```

If you are using the `gblend` tool (see examples README) follow its `deploy` instructions instead.

Keep track of the deployed address for verification.

---

## 7. Verify a Deployment

Assuming you deployed and know the address:

```bash
fluentbase-cli verify . \
  --address 0xYourDeployedAddressHere \
  --rpc https://your.rpc.endpoint \
  --chain-id 1337
```

Add any feature flags used during build:
```bash
fluentbase-cli verify . \
  --address 0xYourDeployedAddressHere \
  --rpc https://your.rpc.endpoint \
  --chain-id 1337 \
  --features my-feature
```

Sample output (truncated):
```json
{
  "verified": true,
  "expected_hash": "0x...",
  "actual_hash": "0x...",
  "rustc_version": "rustc 1.xx.x (...)",
  "sdk_version": "0.x.y-dev-<commit>",
  "build_platform": "docker:linux-x86_64"
}
```

If `verified` is false, check:
1. Docker usage mismatch (used `--no-docker` locally but original used Docker)
2. Feature flags / default-features mismatch
3. Different rustc versions
4. Local modifications vs deployed code

---

## 8. Troubleshooting Matrix

| Symptom | Likely Cause | Suggested Fix |
|---------|--------------|---------------|
| `wasm32-unknown-unknown` target not found | Target not installed | `rustup target add wasm32-unknown-unknown` |
| Verification hash mismatch | Non-deterministic build (no Docker) | Rebuild with Docker; re-run verify |
| Missing artifact file | Wrong output dir or feature not built | Use `--output` flag or confirm feature name |
| Build very slow repeatedly | No incremental caching (always docker) | Iterate locally with `--no-docker`, final build with Docker |
| CLI not found after install | `$CARGO_HOME/bin` not in PATH | Add `~/.cargo/bin` to PATH |

---

## 9. Suggested Next Steps

- Read `docs/ARCHITECTURE.md` (if present) for system overview
- Explore `contracts/examples/` for patterns
- Inspect `crates/sdk` to author a custom contract
- Open a small issue or PR (docs/tests) to begin contributing

---

## 10. Fast Self-Check Before Opening a Verification Issue

Provide all of:
- Commit hash: `git rev-parse HEAD`
- `rustc --version`
- CLI command used (exact)
- Whether Docker was enabled
- Features list
- Expected vs actual hash (from verify JSON)

---

## 11. FAQ (Seed)

Q: Do I need Docker every time?  
A: For production / verification-critical builds yes; for rapid iteration no (`--no-docker`).

Q: Why both `.wasm` and `.rwasm`?  
A: `.rwasm` is the reduced, metadata-embedded form optimized for the Fluent proving pipeline; `.wasm` is the conventional output.

Q: Which file do I deploy?  
A: Unless otherwise specified, deploy the `.rwasm` artifact.

---

Happy building! If you spot a gap in this Quickstart, please submit a PR improving it.
