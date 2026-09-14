# Round-3 review — stand test `an_epoch_outside_this_nodes_read_window_costs_no_peer_its_channel`

Scope: `git diff HEAD` = `crates/dpos/consensus/src/testbed/{stand.rs,cert_inlet_tests.rs,tests.rs}`
only (verified by `git diff HEAD --name-only`: exactly those three; nothing else modified, nothing
untracked except the provided `dsh-input-round2-findings.md`). This is a **reading review**:
`cargo build`/`cargo test`/`cargo fmt` were not run (no build cache; forbidden by the task). Every
fact below is cited from a file I opened; comments/docstrings are treated as claims, not evidence.

Environment: commonware checkout `~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c`
(tag `v2026.4.0`, `Cargo.lock:3204`; `Cargo.toml:301-311`). There are two git checkouts
(`monorepo-9732103c47eb4665/3c4e02c` and `monorepo-27b478c9bb41d208/3c4e02c`), both at the same
rev; I read the second and spot-checked that the `peers_blocked` site/engine paths agree between them.

## Commands run (read-only)

```
git diff HEAD                      # the change under review
git diff HEAD --stat               # 3 files, +605/-61
git diff HEAD --name-only
git status --short
git show HEAD:crates/dpos/consensus/src/testbed/tests.rs
git ls-files .dpos-study | wc -l   # 152 files tracked
git cat-file -e HEAD:.dpos-study/history/E5-2-V.md   # fatal: does not exist
git check-ignore -v .dpos-study/history/E5-2-V.md    # .gitignore:49:.dpos-study
find . -iname '*E5-2-V*'           # (nothing)
```
plus `read`/`grep` on `stand.rs`, `cert_inlet_tests.rs`, `tests.rs`, `plane_upstream.rs`,
`fakes.rs`, `capture.rs`, `lib.rs`, `outer.rs`, `committee/store.rs`, `beacon/*.rs`, `.gitignore`,
`.dpos-study/history/E5-prompts/5.2-V-impl-1.md`, and the commonware checkout
(`resolver/src/p2p/{engine,fetcher,metrics}.rs`, `p2p/src/lib.rs`, `consensus/src/marshal/...`,
`consensus/src/simplex/...`, `cryptography/src/certificate.rs`, `runtime/src/deterministic.rs`,
`macros/src/lib.rs`).

## Assumptions (conservative readings)

1. The commonware checkout above is the one the workspace builds against.
2. The deterministic runner polls all tasks on the test thread (the stand has relied on this for
   `Outcome::logs` since long before this change; `capture.rs:1-10`). I did not run it to confirm.
3. The implementer's mutation report implies the new test is green today with the observed drop
   set exactly `{out_of_window}`. I could not reproduce it; the exact-set assertion is therefore
   the single riskiest assumption (see "Weakest points").
4. `.dpos-study` is the review workspace's state; a journal absent here is absent in the artifact
   under review (the task's `find` fact, re-run and confirmed).

---

# Part A — D-01…D-12, one verdict each

| id | verdict | file:lines (current) | why | what I tried to refute the verdict with | confidence |
|----|---------|---------------------|-----|------------------------------------------|------------|
| D-01 | FIXED | `stand.rs:1217-1227` (docstring "WHAT A ZERO PROVES"), `cert_inlet_tests.rs:1196,1303-1311` | The zero is now explicitly bounded to "as of that engine's LAST loop iteration" and the failure text says the same (`cert_inlet_tests.rs:1300-1301`). The windowless witness is the resolver's own `block!` WARN read out of `Outcome::logs` (`BLOCK_WARN`, `cert_inlet_tests.rs:1167-1169`, asserted at `:1303-1308`). `block!` emits `tracing::warn!` **before** `blocker.block`/`fetcher.block` (CW `p2p/src/lib.rs:435-442`, `resolver/src/p2p/engine.rs:437-438`), so an exclusion after the last `on_start` is still visible. | I re-derived the window: `select_loop!`'s `on_start` runs at the top of *every* iteration (CW `macros/src/lib.rs:70-76,133-161`), and `excluded` is never cleared (`fetcher.rs:401-407` `clear`, `:358-376` `retain`, `:500-505` `reconcile` all leave it; insert `:516`, read `:242`,`:567`) — so the window really is only engine-stop/last-iteration. I then tried to find a block path that skips `block!` (it does not: `handle_network_response` always calls `block!` before `fetcher.block`), and a WARN that capture cannot see (level WARN is captured by `capture.rs:50-52`; the message has no quotes because it is a `format_args!` recorded via `record_debug`). Could not break it. | confirmed by code |
| D-02 | FIXED | `cert_inlet_tests.rs:1267-1296`; chains verified `stand.rs:2238-2239`, `outer.rs:1262-1276` (the Hybrid arm this fixture takes; the Plane arm is `:1297-1311`) → CW `consensus/src/marshal/resolver/p2p.rs:69-70`, `beacon/plane.rs:326-327`, `beacon/dkg_engine.rs:332-333` + CW `simplex/engine.rs:104` | `!gauges.is_empty()` is gone. The test now requires, per node `0..5`, the exact families `node{i}_resolver_resolver_peers_blocked`, `node{i}_frontier_resolver_peers_blocked`, `node{i}_beacon_log_resolver_peers_blocked`, plus at least one family containing `dkg_simplex_resolver`. The runtime prefixes names with the label chain joined by `_` (`runtime/src/deterministic.rs:1322-1345,1253-1274`) and `ctx.encode()` emits the whole registry (`:1367-1372`), so these names are exactly what the stand's `node{i}`/`resolver`/`frontier_resolver`/`beacon_log_resolver`/`dkg_simplex` chains produce. | I tried to show a family is absent in this fixture: all 5 nodes are `Role::Honest` under `Beacon::Live` (`stand.rs:2712-2795`), so `beacon::build` → `open_artifact_seam` runs for every node; `outer.start(..., Some(upstream))` always takes the Hybrid arm and still calls `marshal_p2p::init` (`outer.rs:1240-1287`); the DKG agreement/`dkg_simplex` engine is spawned by the actor for the epoch's dealers (`dkg_engine.rs:308-333`), so at least one instance exists. Could not make a family legitimately absent. | confirmed by code |
| D-03 | FIXED | `cert_inlet_tests.rs:1231-1239` (`observed` closure), `:1253-1259` (rejected set), `:1335-1357` (drops set) | The coverage set is collected from `metrics_before_collect` (all `reason` label values of a family with `value > 0`) and compared as a `BTreeSet`; `SELF_INFLICTED` survives only as the printed table. `dpos_frontier_rejected_total`'s observed set must be empty; `dpos_frontier_dropped_total`'s must equal `{out_of_window}`. `metrics::counter!` is the only writer of each family (`plane_upstream.rs:354` and `:487`), and `drain_counters` resets on read (`stand.rs:796-822`), so a new firing label necessarily enters the observed set. | I tried to make an added `reason` invisible: the only way is if it does not fire in this run, which is inherent to any coverage test and is stated in the docstring. I checked whether the family could be registered with labels under a key the closure misses — no: the name is exact and `reason` is the only label (`plane_upstream.rs:354,487`). I could not produce a case where a firing label is absent from `fired`. | confirmed by code |
| D-04 | PARTIALLY FIXED | `cert_inlet_tests.rs:1202-1207` (`out.blocked.len() == N`), `:1313-1329` (`sites` == both labels); `stand.rs:441-489` | The length assertion is added, and `sites` proves `BlockerSpy::at` was called with both labels on every node (with the sorted-BTreeSet caveat handled at `:1319-1323`). Neither proves the value *returned* by `at` is the value the builder used; the docstring at `:1117-1123` says so honestly. | The suspected residual is real: `let _ = blocker_spy.at(BLOCKER_SITE_FRONTIER); frontier_plane(..., other_blocker, ...)` still yields `sites == [consensus, frontier]` and empty `calls`. I could **not** fully close it, but I narrowed it: if `other_blocker` is a `NoopBlocker`/different spy, the frontier resolver's `block!` still fires `tracing::warn!` *before* the blocker runs (`p2p/src/lib.rs:438`), so `block_warns.is_empty()` (`:1303-1308`) catches the mutation whenever `log_capture_live` is true (see E-01 for the residual branch). So the residual is narrower than "constructed and discarded": it needs capture not live **and** a stale gauge. | confirmed by code |
| D-05 | RECORDED, correctly | `cert_inlet_tests.rs:1359-1381` | The bound is retained and the comment now says in terms: "WEAK by construction … kept as a sanity rail and a printed number, not as the proof of anything." The misleading message ("something is retrying inside the resolver") is gone. | I re-checked the tautology claim: each step-(5) drop returns `true` (`plane_upstream.rs:486-490`), so it does not itself cause a resolver retry; `dropped <= plane_calls` therefore cannot detect a retry cycle. I could not find a wording that still overclaims. The *stated reason* it can never fail is itself wrong (E-03), but the rail is explicitly disclaimed. | confirmed by code |
| D-06 | RECORDED, correctly | `cert_inlet_tests.rs:1224-1259`; `plane_upstream.rs:486-509` | The "must drop the answer" half is still asserted only through the derived counter. A mutation that increments `FRONTIER_DROPPED` and then falls through to the admit path (`plane_upstream.rs:501-508`) leaves every assertion green (no block, `fired == {out_of_window}`). No `Outcome` field distinguishes admitted from dropped per node in a clean way. | I looked for a clean observable and found only a polluted one: `out.upstream[i].latest_delivered`/`finalized_delivered` are per-node admission counters (`fakes.rs:1626-1643`, `plane_upstream.rs:610-662`), but they also count pre-cut admissions while the victim was still in sync, so `assert_eq!(..., 0)` is not sound. `deliveries_decoded` (`fakes.rs:1808-1817`) counts drops *and* admits. So I could not construct a clean fix; the residual is real. | confirmed by code |
| D-07 | FIXED | `cert_inlet_tests.rs:1105-1110`; `fakes.rs:1517-1540`; `stand.rs:826-843` | The docstring now states the counters are process-wide, carry no node label, and that "nothing below attributes a drop to the victim"; it also names the per-node half (`defers`, the victim's own inlet). The old "the victim … 31 times" attribution is gone. | I checked the specific implementer claim that `UpstreamStats.deliveries_rejected` cannot attribute: correct — it counts `deliver → false` (`fakes.rs:1808-1817`) while a step-(5) drop returns `true`. I then tried to build a per-victim drop count from `upstream[VICTIM].deliveries_decoded` (which does count drops) and failed cleanly: it conflates admits, and admits can pre-date the cut. The doc narrowing is the best available fix; the *prose* at `:1086-1088` still ties "step (5) runs" to the victim (E-05). | confirmed by code |
| D-08 | FIXED | `cert_inlet_tests.rs:1088` | The literal is gone; the doc says the drop count "is a run fact the coverage assertion reads off the recorder, not a number in this comment." `grep -n "31"` over the file finds no such run fact. | I searched for any remaining hard-coded count in the new docstring/body (`3*` etc.) and found none; the only counts are the printed tables computed from the recorder. | confirmed by code |
| D-09 | RECORDED, but fixable | `cert_inlet_tests.rs:1099-1101`; `.gitignore:49` | The cited journal still does not exist (`find . -iname '*E5-2-V*'` → nothing; `git cat-file -e HEAD:.dpos-study/history/E5-2-V.md` → fatal, never in HEAD). The new wording ("the journal is untracked and lives beside the other `E5-*` records") is also wrong: `git ls-files .dpos-study` returns **152 tracked files** (`.gitignore` does not untrack already-tracked paths), and the sibling `E5-2-A*.md`/`E5-2-DOCS.md` files are tracked. The implementer's own brief (`E5-prompts/5.2-V-impl-1.md:39,58`) required this journal to be created; it is absent. | I tried to find the journal elsewhere (`find`, `grep -rln E5-2-V`, `git log --all`) and to make the "untracked" claim true (`git check-ignore` says ignored, but `git ls-files`/`git status --ignored` show the directory is tracked). Refutation failed. The measurement behind the mutation claim is still not inspectable, and the doc now contains a false statement. Fixable: cite the existing `E5-prompts/5.2-V-impl-1.md` (or `E5-2-A.md`) or write the journal. | confirmed by code |
| D-10 | FIXED | `cert_inlet_tests.rs:1052-1055` (header), `:1101-1103` ("does NOT close R-129") | The header now reads "the FRONTIER half of R-129; the marshal half is watched here, not exercised" and the docstring explicitly says the test does not close R-129. The factual chain is right: with `EpochSchemeProvider` not overriding `all()` (CW `cryptography/src/certificate.rs:417-419`; `outer.rs:283-290`), `get_scheme_certificate_verifier` is `all().or_else(scoped)` (CW `marshal/core/actor.rs:1189-1191`) and the missing-scheme branch sends `true` and returns without decoding (`:964-972`), so the resolver never blocks. | I read the register entry (`REGISTER.md:1013` is about the marshal resolver) and the marshal path in the checkout, and re-derived that the arm is unreachable in this fixture. I also checked the header cannot be read as closing R-129. Could not refute. | confirmed by code |
| D-11 | FIXED | `stand.rs:1231-1247` | `peers_blocked()` now anchors on the `node` prefix, parses the node index and returns it, requires the `_peers_blocked` suffix, and **panics** on an unparsable value instead of dropping the family through `?`. The test consumes the full key and compares it to `node{i}_<class>_peers_blocked` (`cert_inlet_tests.rs:1274-1276`), so the family is bound to the node. | I looked for remaining silent drops: a family whose node segment is not an integer is still dropped by `.parse().ok()?` (`stand.rs:1236`) — but that is a narrower, different residual (E-06), and no such family exists today (the only `peers_blocked` registration in the checkout is `resolver/src/p2p/metrics.rs:53`). I could not show a *value* silently vanishing. | confirmed by code |
| D-12 | FIXED | `cert_inlet_tests.rs:840-914` (fixture + premise helper), `:962-973` (neighbour now calls them), keyless test untouched | The held-lag fixture is one function `run_held_lag_over_donor_archive`, one set of `HELD_LAG_*` constants and one `assert_the_lag_is_held`; the neighbouring PeerArchive test calls it with `None`. I diffed the old inline body against the new helper: identical `StandConfig::live(5,1)`, `epoch_len = EPOCH_LEN`, `committees = drop_the_last_two_from_epoch_two`, `cert_inlet`, `partition([0,1,2,3],[4])`, `after_height(EPOCH_LEN+4).consensus_only().for_views(4096)`, `run_until(min_height_of([0,1,2]) >= 5*EPOCH_LEN, 400s)`, and the same three premise assertions. The only added statement is `cfg.metrics_snapshotter = snapshotter` (`:866`); the default is `None` (`stand.rs:561,587`), so the neighbour's behaviour is unchanged. The keyless `CertInletSource::Frontier` test's region is untouched by the diff (hunks are at 28, 836, 885, 1017). | I tried to find a behavioural drift in the extraction: constants, cut timing and premise strength are byte-equal (checked with `git show HEAD:` against the working tree); the only wording issue is the helper doc's "`None` leaves the config's default", which is imprecise but currently true (E-04). Could not find a real drift. | confirmed by code |

### Concrete falsifier edits for the `FIXED` rows

- **D-01** — In `plane_upstream.rs:486-490` change `return true` to `return false` (make `out_of_window`
  punish). Under the old gauge-only test a block taken after the engine's last `on_start` could leave
  the gauge at 0. Under the new form the resolver's `block!` records `("frontier", peer)` in
  `BlockerSpy::calls` (`stand.rs:476-478`) **and** emits the `BLOCK_WARN` line, so
  `blocks.is_empty()` (`cert_inlet_tests.rs:1215-1222`) and, when capture is live,
  `block_warns.is_empty()` (`:1303-1308`) go red.
- **D-02** — In `beacon/plane.rs:327` rename the label `"beacon_log_resolver"` → `"beacon_log_resolver_x"`.
  The old `!gauges.is_empty()` stays green (frontier/resolver families still exist); the new
  `node{i}_beacon_log_resolver_peers_blocked` assertion (`cert_inlet_tests.rs:1274-1284`) is red.
- **D-03** — Add at `plane_upstream.rs:487`, next to the existing increment,
  `metrics::counter!(FRONTIER_DROPPED, "reason" => "brand_new_arm").increment(1);`. The old
  `counter_of` over the hard-coded `SELF_INFLICTED` list stayed green; the new `observed` set at
  `cert_inlet_tests.rs:1348` gains `brand_new_arm`, so `fired == covered` (`:1350-1357`) is red.
- **D-08** — No counter assertion is involved; the edit is a fixture change that moves the drop count
  while leaving the old "31 times" prose silently wrong. The new text points at the recorder instead.
- **D-10** — Make the marshal's missing-scheme branch return `false` (CW `marshal/core/actor.rs:970`
  `send_lossy(true)` → `false`). R-129's marshal half becomes reachable; the new header/text say this
  test does not close it, so the reader is not misled (the test itself would not catch it, as stated).
- **D-11** — Register a family `nodeX_peers_blocked` with value `NaN` (or a non-integer node segment).
  The old matcher dropped it silently through the `?`; the new parser panics on the value
  (`stand.rs:1240-1243`).
- **D-12** — A single change to `HELD_LAG_CUT_AT` now moves both tests; before the extraction only one
  test's copy moved. (Behavioural, not a red/green falsifier.)
- **D-04** — (Partial) The residual mutation is `let _ = blocker_spy.at(BLOCKER_SITE_FRONTIER);
  frontier_plane(..., NoopBlocker, ...)`: `sites` stays `[consensus, frontier]` and `calls` empty.
  This survives `out.blocked.len()` and the `sites` assertion, so D-04 is not fully closed. It is
  caught by the `block!` WARN whenever capture is live, and not otherwise (E-01).

---

# Part B — fresh findings in the change as it now stands

| id | severity | file:lines | what is wrong | how I tried to refute it | confidence |
|----|----------|-----------|---------------|--------------------------|------------|
| E-01 | MINOR | `capture.rs:40-45,58-75`; `cert_inlet_tests.rs:1303-1311`; `stand.rs:1570,2103` | When `log_capture_live` is false the test **passes** and only prints an `eprintln!`. In that configuration the windowless WARN witness (`block_warns.is_empty()`) is vacuous (the sink is empty), and the only remaining guards for engines the spy is not wired into (beacon log's and DKG's internal `NoopBlocker`, `beacon/plane.rs:330`, `beacon/dkg_engine.rs:343`) are the stale-able `peers_blocked` gauges. So the D-01 window and the D-04 residual reopen silently. Concrete configuration: run in a process where `tracing::subscriber::set_global_default` was already taken (the `OnceLock` at `capture.rs:35,41` caches `false`). The test never asserts capture is live. | I tried to show capture must be live: in this crate nothing else calls `set_global_default` (`grep`), so in a plain `cargo test`/`nextest` run of this binary it is live. I also checked that tasks are polled on the thread that installed the sink (the runner polls inline, `runtime/src/deterministic.rs:560-640`; `capture.rs:1-10`). Refutation only proves "true today", not "guaranteed"; the silent downgrade remains. | confirmed by code |
| E-02 | MINOR | `cert_inlet_tests.rs:1167-1169`; CW `resolver/src/p2p/engine.rs:437`; `p2p/src/lib.rs:435-442` | The WARN witness is a hard-coded dependency string (`"commonware_resolver::p2p::engine: invalid data received"`). If the resolver changes the message, moves the call to another module, or downgrades the level, `logs_containing` returns nothing while the assertion `block_warns.is_empty()` stays green — a silent loss of the windowless witness. The same fragility applies to the D-01 fix as a whole. | I confirmed the string matches today: the macro default target is the expansion module (`engine`), the message is a `format_args!` (recorded without quotes via `record_debug`), and `capture.rs:50` accepts WARN. I could not make it robust to a dependency change without a production edit; this is inherent to matching logs. | confirmed by code |
| E-03 | MINOR | `cert_inlet_tests.rs:1359-1366`; `stand.rs:2247-2249`; `plane_upstream.rs:169`; CW `resolver/src/p2p/engine.rs:208-214,424-440` | The comment justifying the `dropped <= plane_calls` rail says "each step-(5) drop answers one counted fetch and returns `true`, so no resolver retry can multiply it." That is not structurally true: the stand's resolver timeout is 5 s while the client's `FRONTIER_FETCH_TIMEOUT` is 8 s, so an **active-request timeout** (`pop_active` → `add_retry`) can make the resolver fetch again; a subsequent response that takes step (5) increments `dropped` without a new `latest_calls`/`finalized_calls`. In this fixture the donor answers promptly so it does not bite, but the rail is not the invariant the comment states and could in principle red/flake. | I traced every retry route: a `deliver == false` (reject) retries but does not touch `dropped`; a step-(5) drop returns `true` and does **not** retry; only the timeout route can multiply attempts. I then checked the concrete ordering (5 s < 8 s) in `stand.rs:2247-2249` / `plane_upstream.rs:169`, which makes the route live. Refutation failed on the general claim; the rail is explicitly disclaimed, hence MINOR. | inferred |
| E-04 | NIT | `cert_inlet_tests.rs:857-866`; `stand.rs:561,587` | The helper docstring says "`None` leaves the config's default", but the code assigns `cfg.metrics_snapshotter = snapshotter` unconditionally. It happens to be behaviour-preserving because today's default is `None`; if `StandConfig::live`'s default ever becomes `Some(..)`, the neighbouring test silently changes and the doc is wrong. | I checked `StandConfig::live` → `Self::honest` → `metrics_snapshotter: None`, so the neighbour currently gets `None` either way. Refutation succeeds for today's behaviour but not for the wording. | confirmed by code |
| E-05 | MINOR | `cert_inlet_tests.rs:1086-1088,1105-1110,1348-1357`; `stand.rs:831-843` | The docstring says "The victim is cut … its frontier plane keeps answering, and step (5) runs", but the only attribution of the `out_of_window` drops is process-wide (`fired` over `dpos_frontier_dropped_total`, no node label) and the per-node premise is `f.defers`, which is the victim's **inlet over the donor's archive**, not its frontier plane. The test can be green with the victim's frontier plane idle and node 3 producing every drop. D-07 recorded the attribution gap, but this sentence still reads as if the victim's frontier plane were proven active. | I looked for a per-node frontier-plane observable wired into the assertions: `out.upstream[VICTIM].deliveries_decoded` counts the victim's `deliver → true` (drops + admits), so a non-zero value would at least show the victim's plane delivered something; it is printed in the `eprintln!` but never asserted. I could not refute that the assertion set does not pin the victim. | confirmed by code |
| E-06 | MINOR | `stand.rs:1231-1247` | D-11 made an unparsable **value** panic, but an unparsable **family** is still silently dropped: `key.strip_prefix("node")?.split_once('_')?.0.parse().ok()?` (`:1236`) turns `nodeX_..._peers_blocked` or `node_peers_blocked` into `None`. So a future resolver registered under a non-numeric node segment would be invisible to `peers_blocked()` and to the `excluded.is_empty()` assertion. | I checked whether any such family exists today: the only `peers_blocked` registration in the checkout is CW `resolver/src/p2p/metrics.rs:53`, always under the stand's `node{i}` chain, so today it cannot happen. Refutation fails for the latent case only. | confirmed by code |
| E-07 | NIT | `cert_inlet_tests.rs:1093-1096`; CW `marshal/core/actor.rs:954-983` | The docstring says the marshal "refuses at DECODE time and answers `true` ('ignoring stale delivery', …:965-971)". The cited branch is the **missing-scheme** check, which sends `true` and returns *before* any decode; the decode is at `:974-984`. The parenthetical makes the meaning clear, but "at DECODE time" contradicts the citation. | I read `actor.rs:954-984`: scheme lookup → `send_lossy(true); return false` at 964-972; decode at 974. So "before decode" is the correct phrasing. Could not reconcile the wording. | confirmed by code |
| E-08 | MINOR | `stand.rs:2867` (`blocker: blocker_spy.at(BLOCKER_SITE_CONSENSUS)`); `cert_inlet_tests.rs:1215-1222`; CW `p2p/src/lib.rs:502-514` (Bug A rationale), `consensus/src/simplex/actors/batcher/actor.rs:387,401,428,453,474,495,509`, `voter/actor.rs:298` | The consensus spy is also the simplex batcher's/voter's blocker. Production deliberately leaves those slots on `NoopBlocker` because those `block!` sites are "evidence-free" and can partition honest peers. The new test asserts `blocks.is_empty()` **globally**, so any future honest-run frame that trips one of those sites makes this test red for a reason unrelated to the frontier rule (a false red, not a missed property). | I checked the fixture is honest and that the current run was green, and read the batcher sites: each is a genuine fault site (decode error, epoch mismatch, invalid notarization/nullification/finalization, equivocation). I could not show one is reachable in this fixture, so this is latent fragility, not a current failure. | inferred |
| E-09 | NIT | `cert_inlet_tests.rs:1240-1259` | The failure message for the `rejected` assertion prints `rejected_table`, which is computed over `SELF_INFLICTED` (the non-punishing arms) on `dpos_frontier_rejected_total`; when a **punishing** reason fires, those four read 0 and the message's "the four self-inflicted arms read …" is misleading, although `rejected` itself lists the offending label. | I checked the message and the table sources; `rejected` is printed before the table, so the information is not lost, only noisy. Refutation partial, severity NIT. | confirmed by code |
| E-10 | NIT | `cert_inlet_tests.rs:1057-1070`; `plane_upstream.rs:486-490` | The docstring's falsifier says the coverage assertion catches "a reason that STOPS as well as one that STARTS", but it can only do so for reasons recorded under the same family name `dpos_frontier_dropped_total`. A new self-inflicted path that returns `true` without incrementing that family (or a new family) is invisible; the claim is true for labels, not for code paths. | I looked for a self-inflicted path in the current code that does not increment the counter: at `plane_upstream.rs:486-490` the increment is the first statement of the only self-inflicted branch, so today the claim holds. Refutation fails for today, succeeds as a scope caveat. | confirmed by code |

No production-code change exists in the diff (`git diff HEAD --name-only`), so no Part-B row is a
production-change BLOCKER. The only BLOCKER-class hole in the new test's reach is D-06's
"counted-but-admitted" mutation, which is an explicitly recorded residual, not a new finding.

---

# Re-asked round-2 questions, against the NEW code

1. **Can the test still be GREEN while the property is BROKEN?**
   - `blocks.is_empty()` (`:1215`): green if the spy value returned by `at` is discarded and the real
     blocker is `NoopBlocker`/another spy, **and** capture is not live (E-01); the frontier `block!`
     WARN would otherwise fire. Green also if the block is performed by a `Blocker` the spy never
     receives — only the beacon log's / DKG's internal `NoopBlocker`, whose `deliver == false` would
     be caught by the gauge/WARN, not the spy.
   - `rejected.is_empty()` (`:1254`): green under D-06's increment-but-admit mutation (property
     "drop the answer" broken); also green if `reject` is bypassed by a direct `false` return, but
     then the spy/WARN catch the resulting block.
   - `gauges` family presence (`:1268-1296`): not a property claim by itself.
   - `excluded.is_empty()` (`:1298`): green if an exclusion lands in the last iteration/stop window
     **and** the WARN needle misses (`E-02`) or capture is not live (E-01).
   - `block_warns.is_empty()` (`:1304`): vacuous when `log_capture_live == false` (E-01).
   - `sites == both` (`:1324`): green under the discarded-spy mutation.
   - `fired == {out_of_window}` (`:1350`): green while the victim's own frontier plane never ran
     (E-05/D-07); it only proves some node's plane took the arm.
   - `dropped <= plane_calls` (`:1377`): demonstrably weak (D-05/E-03).
   The remaining property-breaking configurations are exactly the recorded/flagged residuals (D-04,
   D-06, E-01, E-02, E-05). The new code closes the round-2 D-01 window in the normal (capture-live)
   configuration.

2. **Is the coverage narrowing honest now that the set is collected from the recorder?**
   Yes. `plane_upstream.rs:487` increments `dpos_frontier_dropped_total{reason}` as the first
   statement of the only self-inflicted branch, so a fire implies a non-zero sample and a non-zero
   sample implies a fire (the family has no other writer). `drain_counters` resets on read
   (`stand.rs:796-822`) and runs once per `drive` (`:2020-2028`), so the values are this run's; each
   test uses a fresh `DebuggingRecorder` (`cert_inlet_tests.rs:1171-1175`) and the `metrics` recorder
   is thread-local, so no cross-test bleed. The only limitation is scope (E-10): a new arm that does
   not use this family is invisible. The `reason`-label set is compared exactly (empty for
   `rejected`, `{out_of_window}` for `dropped`), so both "stops firing" and "starts firing" are red
   for that family.

3. **Does the D-12 extraction change either test's behaviour?** No. Constants, cut timing, config,
   premise assertions and window are equal (diffed old inline body vs new helper); the only added
   assignment is `cfg.metrics_snapshotter = snapshotter` and the default is already `None`
   (`stand.rs:561,587`), so the neighbour passing `None` is unchanged. The keyless `Frontier`-source
   test is untouched (diff hunks at `cert_inlet_tests.rs` 28/836/885/1017). The helper doc's "leaves
   the config's default" is imprecise (E-04) but not a behaviour change.

4. **Is the `block!` WARN observable sound?** Today, yes, in the normal run: `log_capture_live` is
   true whenever this crate's `capture::install` wins the process-global slot (`capture.rs:35-45`),
   which it does in this test binary (nothing else calls `set_global_default`); tasks are polled on
   the installing thread (`runtime/src/deterministic.rs:560-640`); the macro emits `warn!` with
   target = expansion module (`commonware_resolver::p2p::engine`) and a non-quoted `format_args!`
   message, which `capture.rs:58-75` records as `target: message`; `capture.rs:50-52` accepts WARN.
   When capture is not live the test only prints (E-01). The needle can be defeated by a
   target/level/message change in the dependency (E-02).

5. **Is the D-02 family list correct?** Yes for this fixture: all 5 nodes are honest `Beacon::Live`
   under `PeerSet::AllNodes`, so `node{i}_beacon_log_resolver` (`beacon/plane.rs:326-327`), the
   marshal's `node{i}_resolver_resolver` (`outer.rs:1262-1276` — the Hybrid arm this fixture takes
   via `outer.start(..., Some(upstream))`; the Plane arm is `:1297-1311` — plus
   CW `consensus/src/marshal/resolver/p2p.rs:69-70`) and the frontier's
   `node{i}_frontier_resolver` (`stand.rs:2238-2239`) are registered by every node; the DKG
   agreement's `..._dkg_simplex_resolver` (`beacon/dkg_engine.rs:332-333` + CW
   `simplex/engine.rs:104`) is registered for the epoch's dealers, so "at least one" holds because
   the run reaches epoch 5 with a live DKG. The "at least one DKG" premise is global, but it is a
   presence premise, not the property, and the `excluded.is_empty()` check covers every family
   regardless.

6. **Hygiene.** No `#[allow]` was added (the only ones on the touched functions are pre-existing:
   `stand.rs:2214`, `:2258`, `:1561`). All new code lives under `#[cfg(test)] mod testbed`
   (`lib.rs:73-74`), so no new `pub` item escapes the crate and all `unwrap`/`panic!` are test-only.
   The diff touches only the three testbed files. The new test uses `unwrap_or_else(panic!)` for the
   gauge value and `lock().unwrap()` in the spy — consistent with the module. No new dependency.

7. **Where the review is weakest (ranked).**
   1. No run: I cannot confirm the test is green, nor that the observed drop set is exactly
      `{out_of_window}`. If `not_readable` also fires in this fixture, the exact-set assertion
      (`cert_inlet_tests.rs:1350`) is red — a real risk I could only bound by reading the window
      arithmetic (`committee/store.rs:526-549`), not by executing.
   2. The thread-locality of both `metrics::with_local_recorder` and `capture`'s sink is verified by
      reading the runtime and by sibling-test usage, not by running.
   3. I did not enumerate every resolver engine the full stand can construct; I verified the named
      families and the two NoopBlocker engines, not the main simplex ordering resolver's family.
   4. D-06/D-07 fixability is an argument about available observables, not a demonstrated fix.

---

# Leave as is (considered, deliberately not flagged)

- **`BlockerSpy` as a whole / replacing `NoopBlocker` in the test stand.** It is test-only and the
  `block` body is side-effect-identical to `NoopBlocker::block` (`crates/dpos/p2p/src/lib.rs:537-541`),
  so the run is unchanged. The anti-vacuity purpose is real and the residual is disclosed.
- **`Outcome::blocked` and `peers_blocked()` being `pub(super)` rather than private.** They are
  crate-test-only (`lib.rs:73`), and the sibling test needs `frontier_plane`'s new parameter
  (`tests.rs:77`), so this is the minimal visibility.
- **The `tests.rs` throwaway spy.** `BlockerSpy::default().at(FRONTIER)` is dropped after the call
  and never read; the timeout test asserts no blocking, and the spy only counts. No behaviour change.
- **The 152 tracked files under `.dpos-study` despite `.gitignore:49`.** Pre-existing repo state, not
  part of this change; only the *false claim* about it is flagged (D-09).
- **The `reason` string duplication between `SELF_INFLICTED` and `plane_upstream.rs:140-143`.** The
  private constants are not reachable from the test crate, the list is printed only, and the exact-set
  assertion makes a stale name self-correcting when the arm fires.
- **`SELF_INFLICTED` not including the punishing reasons.** The task of the table is to show the
  self-inflicted arms; punishing reasons are covered by the empty `rejected` set.
- **`assert!(out.blocked.len() == N)` instead of `assert_eq!`.** Style only.
- **Not asserting `out.errors().is_empty()`.** The sibling tests in this file (`cert_inlet_tests.rs:677-679,1037-1039`)
  also do not; consistency, not a new gap.
- **`peers_blocked()` allocating `key.to_string()` per line.** Test-only, exposition is small.
- **The `dkg_simplex_resolver` global `contains` check.** It is an anti-vacuity premise, not the
  property, and the `excluded.is_empty()` assertion already covers the family itself.
- **The D-06/D-05 residuals themselves.** They are the explicitly recorded/accepted items; I flag only
  the supporting wording (E-03) and the attribution prose (E-05).
