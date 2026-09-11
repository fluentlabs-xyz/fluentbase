# Ethereum execution tests

The upstream corpus is pinned in `ethereum-tests.json` to the official
[`ethereum/execution-specs` mainnet fixture release `tests@v20.0.2`](https://github.com/ethereum/execution-specs/releases/tag/tests%40v20.0.2).
The retired `ethereum/tests` checkout is no longer used. Downloads are checked against
the release asset's SHA256 before extraction, and stored under a versioned, ignored
directory. Existing legacy checkouts are left untouched.

From this directory:

```sh
make sync_tests
node gen_tests.js --check
cargo nextest run --release --no-default-features --features std --locked \
  --success-output final -E 'test(good_coverage_tests) | test(selection_tests) | test(state::tests)'
cargo nextest run --release --no-default-features --features std,wasmtime --locked \
  --success-output final -E 'test(good_coverage_tests) | test(selection_tests) | test(state::tests)'
```

CI covers 34 Osaka fixture families selected in `ci-tests.json`, containing 235
transaction cases. They use the upstream ports of the previous CI cases, including
three separate CREATE-result cases and the current EXTCODEHASH, CODECOPY and ECADD
equivalents. Upstream provides `create_large_result` only for Prague, so it has no
Osaka registration.
Every file must contain at least one eligible post case and execute transactions.
Passing test output reports the executed and skipped counts.

The complete registration contains 2,359 fixture files / 14,614 transaction cases
for Osaka. Run it by omitting the `-E` filter. General Ethereum fixtures
compare native execution against the upstream expected state/log roots, then compare
Fluent execution against native execution. Fluent's historical transaction fixtures
remain separate and keep their existing checks. A passing CI subset is not a claim
that the entire upstream corpus passes or that Fluent has no intentional differences
from Ethereum.

The differential suite uses Osaka because the delegated EVM instruction set
(`crates/revm/src/evm.rs`) and built-in precompiles such as MODEXP
(`contracts/modexp/src/lib.rs`) are pinned to Osaka independently of the host chain's
fork schedule. Comparing Prague reference results against those artifacts would
compare different rules, for example the 200 versus 500 minimum MODEXP gas cost.
Historical host fork activation belongs to separate compatibility tests. The runner
rejects suites containing only earlier forks rather than reporting them as passed.

To update the corpus, edit the release URL, SHA256 and versioned directory in
`ethereum-tests.json`, run `make sync_tests`, then `node gen_tests.js` to regenerate
`src/tests.rs` and `src/short_tests.rs`. The generator rejects missing CI files,
duplicate Rust names and files with no cases for their declared fork. Do not rewrite
fork labels or expected results to make an old corpus appear current. Run both
backends and review actual failures before changing the selection.

`python3 sync_tests.py --archive /path/to/fixtures.tar.gz` accepts a pre-downloaded
archive with the same mandatory checksum check. Python 3, Node.js and curl are the
only additional fixture-management requirements; no fixture archive is committed.

Fixture-tool regression checks:

```sh
python3 -m unittest discover -s tests_tools -v
```
