# Review — `git diff HEAD` (3 files, testbed only)

Change under review: the new stand test
`an_epoch_outside_this_nodes_read_window_costs_no_peer_its_channel`
(`crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:1090-1326`) plus the
test-only observable it needed, `BlockerSpy`/`SpyBlocker`/`BlockerFacts`
(`crates/dpos/consensus/src/testbed/stand.rs:411-489`, wired at
`stand.rs:2468`, `:2478`, `:2849`; outcome field `stand.rs:1027-1031`,
`stand.rs:1968`; accessor `stand.rs:1219-1229`) and the call-site fix in
`crates/dpos/consensus/src/testbed/tests.rs:64-84`.

Method: reading only. No `cargo build`/`cargo test` (per task), no file
modified except this artifact. `git diff HEAD`, `git status`, `git show` were
the only git commands. I read the production sites the test leans on and the
commonware checkout at
`~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c/` (rev
`3c4e02ceede03126f524216605a1195e1cee7d0e`, matching `Cargo.lock:3389`).

Scope confirmation (`git diff HEAD --name-only`): only
`cert_inlet_tests.rs`, `stand.rs`, `tests.rs`; `mod testbed;` is
`#[cfg(test)]` (`crates/dpos/consensus/src/lib.rs:73-74`), so there is **no
production change**. No new crate-visible `pub` item: everything added is
`pub(super)` or `pub` inside a `pub(super)` struct (`stand.rs:419,423,442,468,
483,1031,1219`).

---

## Findings

| id | severity | file:lines | what is wrong | how I tried to refute it | confidence |
|----|----------|-----------|----------------|--------------------------|------------|
| D-01 | MODERATE | `stand.rs:1216-1229`, `cert_inlet_tests.rs:1228-1239` | The `peers_blocked` assertion is treated as proof that nobody was excluded, but the gauge is only written in the resolver select-loop's `on_start` arm (CW `resolver/src/p2p/engine.rs:166-178`) as `fetcher.len_blocked()`; the exposition read at `stand.rs:2044` is the *last written* value, not `fetcher.excluded.len()` at read time. An exclusion taken after the last `on_start` of an engine (engine stopped/aborted, or the run's final iteration) leaves the gauge at 0 even though `excluded` is non-empty (append-only, CW `resolver/src/p2p/fetcher.rs:510-517`, read at `:242`, `:567`). The failure text "a resolver excluded a peer … for the life of the engine" therefore overclaims. | I tried to close the window: after `handle_network_response` calls `fetcher.block` (CW `engine.rs:437-438`) the loop's next iteration runs `on_start` before any other arm, so a *live* engine refreshes promptly; I also confirmed `reconcile` (`fetcher.rs:500-505`) does not clear `excluded`. That leaves only the engine-stop/last-iteration window — narrow, but real, and the docstring itself admits it (`stand.rs:1216-1218`). I could not refute it, I could only narrow it. | confirmed by code |
| D-02 | MINOR | `cert_inlet_tests.rs:1228-1233`, doc claim `stand.rs:1209-1214` | `assert!(!gauges.is_empty())` only proves *some* `peers_blocked` family exists. The docstring claims the gauge "covers every resolver engine the run registered, the beacon plane's and the DKG log's included" (`stand.rs:1209-1212`, `cert_inlet_tests.rs:1225-1227`), but the test never asserts the family count or that the beacon/DKG engine families are present. A resolver whose family is absent (engine not constructed) is invisible while the assertion still passes on another family. | I tried to show the fixture always starts those engines and that their families must therefore appear; the code does build a beacon plane/DKG engine under `StandConfig::live` (`stand.rs:2448-2457` uses the real `CommitteeStore`), but `peers_blocked()` returns every matching family without identifying which engine it belongs to (`stand.rs:1219-1229`), so the test still does not verify the claimed set. | confirmed by code |
| D-03 | MINOR | `cert_inlet_tests.rs:1102-1114`, `:1258-1286` | The "exact set" coverage is computed only over the hard-coded `SELF_INFLICTED` list; a future `dpos_frontier_dropped_total{reason="<new>"}` is invisible to `counter_of` (it filters on an exact label pair, `stand.rs:831-843`) and the assertion stays green. The falsifier text ("a change in the SET of step-(5) reasons this fixture produces … then the narrowing in the name is no longer the truth", `cert_inlet_tests.rs:1086-1088`) overstates: an *added* drop reason does not turn the test red. | I checked whether a new reason could break the property while staying green: a new reason that *punishes* returns `false`, so the resolver calls `block!`/`fetcher.block` and D-01's gauge or the spy catches it. So this is a coverage-honesty gap, not a property hole. I could not construct a punishing path that both stays green and breaks the property without also editing the external resolver. | confirmed by code |
| D-04 | MINOR | `stand.rs:447-455`, `cert_inlet_tests.rs:1241-1256` | `sites` records that `BlockerSpy::at` was *called*; it does not prove the resolver slot actually received the `SpyBlocker`. An edit that keeps `blocker_spy.at(BLOCKER_SITE_FRONTIER)` for its side effect but passes a different blocker into `frontier_plane`/`OuterBuilder` would keep `sites == [consensus, frontier]` while both assertions in (1)/(3) go vacuous. The test also never asserts `out.blocked.len()` equals the node count, so a truncated vector would silently skip nodes in the `for`. | I tried to argue the type system forbids it: `at` is the only constructor of `SpyBlocker` (`stand.rs:449-455`, fields private `:468-471`), so a slot can only receive the spy through `at`. That proves the *value* came from `at`, not that the production wiring at `stand.rs:2478`/`:2849` still uses it — the assertion tests the stand's call, not the resolver's use. | confirmed by code |
| D-05 | MINOR | `cert_inlet_tests.rs:1288-1305` | `dropped <= plane_calls` is structurally near-tautological and cannot detect what its message claims ("something is retrying inside the resolver, which is an unbounded cycle"). Each step-(5) drop is triggered by one delivered answer; a step-(5) drop returns `true` (`plane_upstream.rs:486-490`), so the resolver records success and does **not** retry (`engine.rs:425-440`), and each fetch was counted once by `CountingUpstream` at `get_finalization`/`get_latest` (`stand.rs:2470-2500`, `fakes.rs:1626-1643`). A resolver retry loop would still consume one counted call per request, so the bound would not move. | I looked for a legitimate way the bound could fail (multi-peer fan-out, uncounted handle): the fetcher sends to one eligible peer per key per attempt (`fetcher.rs:264-279`), `PlaneUpstreamHandle` is only reached through the `CountingUpstream` wrapper (`stand.rs:2470-2500`), and `CertInletSource::PeerArchive` does not touch the plane at all (`stand.rs:3046-3057`). I could not produce a case where a retry cycle makes `dropped > plane_calls`, which is exactly why the assert forbids nothing. | confirmed by code |
| D-06 | MINOR | `cert_inlet_tests.rs:1258-1286`, `plane_upstream.rs:486-490` | The "node must drop the answer" half of the property is asserted only through the plane's own counter (`dpos_frontier_dropped_total{out_of_window} > 0`). A mutation that increments the counter but still reaches the admit path at `plane_upstream.rs:501-509` (or increments it without the `return true`) keeps the test green while an unverifiable certificate is forwarded. | I read `plane_upstream.rs:486-490`: in the current code the increment, the waiter removal and the `return true` are one straight-line block before the admit path, so the counter does imply the drop. But the test only reads the counter, it never observes the absence of the admitted value (no waiter/`fetch_one` result assertion). I could not find a runtime observable in `Outcome` that would distinguish "counted and dropped" from "counted and admitted", so the gap is real though mutation-only. | confirmed by code |
| D-07 | MINOR | `cert_inlet_tests.rs:1165-1167`, `:1197-1216`, `:1258-1273`; `fakes.rs:1808-1817` | The drop/reject numbers are process-wide (`counter_of` sums the shared recorder, `stand.rs:824-843`) while the docstring attributes the exercised frontier drops to the victim ("the victim is cut … its frontier plane keeps answering, and step (5) is taken 31 times", `cert_inlet_tests.rs:1050-1052`). Node 3 shares the same short window and can contribute the same labels (the keyless test says so explicitly at `cert_inlet_tests.rs:654-664`). The per-node `UpstreamStats.deliveries_rejected` (`fakes.rs:1808-1817`) is available and unused. | I checked whether the victim is the only possible producer: node 4 cannot finalize on the consensus plane and the donor serves epochs above its window, so it almost certainly does drop; but the assertion does not establish that, and the counter cannot be attributed after the fact. Refutation failed; the claim is unverified, not false. | inferred (attribution) / confirmed (process-wide counter) |
| D-08 | NIT | `cert_inlet_tests.rs:1050-1052` | "step (5) is taken 31 times" is a hard-coded run fact that no assertion pins; the test only asserts the *set* of reasons and `> 0`. It will silently rot. | I searched the test body for any `31`/count assertion — none. `drops` is only used for the exact-set test and the `dropped <= plane_calls` bound. | confirmed by code |
| D-09 | NIT | `cert_inlet_tests.rs:1061-1063` | "journal `E5-2-V.md` §0(4), mutation 2" cites a file that does not exist in the repository (`find . -iname '*E5-2*'` returns `E5-2-A*.md`/`E5-2-DOCS.md` only). The measurement behind the claim is therefore not inspectable from the change. | I tried to locate the journal or an equivalent record of the mutation under `.dpos-study/`; no `E5-2-V.md` and no file matching the `mutation 2` wording for this fixture. | confirmed by code (absence) |
| D-10 | NIT | `cert_inlet_tests.rs:1020`, `:1036-1063` | The test is headed "(5.2 заход В, R-129)", but R-129 as written is about the **marshal** resolver (`REGISTER.md:1013-1017`: "На marshal-резолвере … `deliver == false` … правило доведено только до FRONTIER"). This fixture exercises the FRONTIER rule and only *observes* the consensus slot; the docstring says so, but the header tag can still be read as closing R-129. | I read `REGISTER.md:1013-1017` and the marshal `Consumer::deliver` path (`CW consensus/src/marshal/resolver/handler.rs:63-77` returning the actor's `send_lossy` value; `CW consensus/src/marshal/core/actor.rs:957-971` sending `true` on a missing scheme). The docstring's narrowing is accurate; only the tag is loose. | confirmed by code |
| D-11 | NIT | `stand.rs:1219-1229` | `peers_blocked()` uses an unanchored `key.ends_with("peers_blocked")` and silently drops any family whose value fails `parse()` (`?` inside `then`). Today only the resolver registers this name (`grep -rn peers_blocked` over the checkout hits `resolver/src/p2p/metrics.rs:53` only), so it is correct; but the matcher does not bind the family to `node{i}_…` and a future `NaN`/`+Inf` value would vanish rather than fail. | I searched the workspace and the whole commonware checkout for other `peers_blocked` registrations; none. So the unanchored match is harmless *today*; I could not refute the latent fragility. | confirmed by code |
| D-12 | NIT | `cert_inlet_tests.rs:886-1018` vs `:1090-1326` | The new test duplicates ~200 lines of the preceding archive test's fixture (same config, cut, premises, `inlet()` assertions). The two now share the same lag fixture; a change to the fixture must be made twice. | I compared the two functions: identical `StandConfig`, `partition`, `CUT_AT`, `NEVER`, premise block and `defers/rotations` assertions; only the added observables differ. Refutation failed — the duplication is real, though it is a maintainability cost rather than a correctness bug. | confirmed by code |

No BLOCKER and no SERIOUS finding. In particular I could not construct a
configuration in which all of (1), (2), (3), (4), (5) hold while the
frontier `out_of_window` arm punishes the peer, or in which a production file
was touched.

---

## Answers to the required questions

### (1) Can the test still be GREEN while the property is BROKEN?

For the **arm the fixture exercises (`out_of_window`), no.** Enumerating the
assertions against the possible breakages:

* `out_of_window` returns `false` via `Self::reject(REASON_OUT_OF_WINDOW, …)`
  → `dpos_frontier_rejected_total{reason="out_of_window"}` increments
  (`plane_upstream.rs:353-362`), assertion (2) (`cert_inlet_tests.rs:1204-1221`)
  goes red.
* `out_of_window` returns `false` **without** counting → the resolver runs
  `block!` + `fetcher.block` (`CW engine.rs:435-440`); the spy counts the
  `block!` (assertion (1), `cert_inlet_tests.rs:1183-1195`) and/or the gauge
  goes non-zero (assertion (3), `:1228-1239`). To stay green an edit would have
  to defeat *both* independent observables.
* The fixture stops producing `out_of_window` → assertion (5) requires
  `fired == ["out_of_window"]` (`:1279-1286`), so the run goes red rather than
  vacuously green.

The places where green can coexist with *some* broken thing are the ones
listed above and they are all narrow:
* a `peers_blocked` exclusion that lands after the engine's last `on_start`
  (D-01) — the counter/`block!` observables still cover the frontier arm, so
  this cannot hide an `out_of_window` punishment, only an exclusion in a
  resolver that has no other witness;
* a mutation that counts a drop but still admits the certificate (D-06);
* a future self-inflicted reason whose *drop* label is outside
  `SELF_INFLICTED` (D-03) — but a *punishing* new reason is still caught by
  the spy.

The consensus/marshal `verify_delivered ⇒ false` arm that R-129 is actually
about is **not exercised at all** (`cert_inlet_tests.rs:1053-1063`), so this
test cannot go red for a regression there; it is honest about that in the
docstring, but it means the test does not close R-129 (D-10).

### (2) Is the coverage narrowing honest?

The **name** (`an_epoch_outside_this_nodes_read_window_…`), the **docstring**
("this fixture executes exactly ONE of them — `out_of_window`"), and the
**`COVERED` set** (`["out_of_window"]`, `cert_inlet_tests.rs:1114`) agree.
The fixture really should produce only `out_of_window`: the victim's anchor
epoch is 1 (`heights[VICTIM] < 64`, `:1152-1157`), the window is
`anchor_epoch + MAX_COMMITTEE_LOOKAHEAD_EPOCHS` (`committee/store.rs:218-224`,
`MAX_COMMITTEE_LOOKAHEAD_EPOCHS = 2`), so epochs 4+ are refused by the window
check that runs **first** (`store.rs:531-540`), while epochs 2–3 are readable
(`commit_height` = 1 / 32 with `DPOS_ACTIVATION_BLOCK = 0`, `store.rs:616-622`,
`fakes.rs:51`) and the geometry is frozen from the start
(`stand.rs:2454`). So the naming is honest.

Two claims are **not** exercised: the docstring says the consensus slot is
"UNDER OBSERVATION", but no call is asserted to occur on it — only that the
spy was wired there (D-04); and it says the gauge "covers every resolver …
the beacon plane's and the DKG log's included", which the non-emptiness check
does not verify (D-02). The falsifier line overstates the exact-set guarantee
for an *added* drop reason (D-03).

### (3) Does `BlockerSpy` change stand behaviour other than counting?

No scheduling, ordering or network effect: `SpyBlocker::block`
(`stand.rs:473-479`) has no `.await`, so it completes in one poll like
`NoopBlocker::block` (`crates/dpos/p2p/src/lib.rs:537-541`), the simulated
network is `disconnect_on_block: false` (`stand.rs:1602`), and both carry
`PublicKey = PeerPubkey = ed25519::PublicKey` (`crates/dpos/bls/src/lib.rs:54`).
The only added work is a `std::sync::Mutex` lock plus a `Vec::push` per
`block` call, none of which changes virtual time. `Clone` is correct: both
slot values share the same `calls` Arc (`stand.rs:451-454`) and each keeps its
own `site`, so the two slots are distinguishable; `sites` lives on the
`BlockerSpy` and is a `BTreeSet` (`stand.rs:444`), so its order is
deterministic and the test's re-sort (`cert_inlet_tests.rs:1246-1250`) is
redundant but harmless. One nit: the docstring's "the run is byte-identical to
a `NoopBlocker` run" (`stand.rs:431-432`) is an overstatement only in that a
poisoned `Mutex` would panic where `NoopBlocker` never could — I did not raise
this as a separate finding because the mutex cannot be poisoned in this
single-threaded, no-panic-while-held usage.

### (4) Is the `peers_blocked` assertion sound?

It reads the right family (`peers_blocked` is the resolver's gauge,
`CW resolver/src/p2p/metrics.rs:51-56`, keyed by the engine context), and for
the frontier resolver it is a genuine second witness of `fetcher.block`. It is
**not** sound as an absolute "nobody was excluded": zero can also mean the
last `on_start` did not run after an exclusion (D-01), or the family for the
relevant engine is not present while another family satisfies
`!gauges.is_empty()` (D-02). Both are code-level facts, not speculation. The
docstring already flags the boundary undercount (`stand.rs:1216-1218`), but the
assertion message treats zero as proof.

### (5) Is `dropped <= plane_calls` a real bound?

Effectively tautological (D-05). Every step-(5) drop answers a fetch this
process issued; each `get_latest`/`get_finalization` is counted exactly once
by the `CountingUpstream` wrapper (`fakes.rs:1626-1643`), the resolver sends
to one peer per attempt, and a step-(5) drop returns `true` so it never
triggers the retry that could multiply drops per call. It forbids essentially
nothing and cannot detect the "unbounded resolver cycle" its message names.

### (6) Hygiene

* `#[allow]`: none added. `frontier_plane`'s existing
  `#[allow(clippy::too_many_arguments)]` (`stand.rs:2196`) now covers one more
  argument.
* `unwrap`/`expect` outside `#[cfg(test)]`: none. The new `unwrap`s
  (`stand.rs:450,459,460,477`) are inside `mod testbed`, which is
  `#[cfg(test)]` (`lib.rs:73-74`).
* new `pub` visible outside the crate: none; all additions are `pub(super)` or
  `pub` fields of a `pub(super)` struct (`stand.rs:419,423,442,468,483,1031,1219`).
* changes outside the three testbed files: none (`git diff HEAD --name-only`).
* `tests.rs:64-84` only threads the new argument in the correct position; it
  uses a throwaway `BlockerSpy` (`tests.rs:77`) and its comment is accurate.

### (7) Where this review is weakest (ranked)

1. **No execution.** Cargo is forbidden, so "the fixture produces only
   `out_of_window`", "the test is green at HEAD", and the `31` are derived from
   code, not observed. This is the single biggest gap.
2. **The consensus/marshal path.** I verified the scheme-missing `deliver`
   returns `true` at `CW marshal/resolver/handler.rs:63-77` +
   `marshal/core/actor.rs:957-971`, but I did not trace every per-epoch
   marshal resolver arm in the stand run; I relied on the orchestrator's
   established facts there.
3. **The exposition key format for the gauge.** I inferred the nested label
   prefix from `Outcome::metric`/`marshal_tip_of` (`stand.rs:1231-1243`,
   `:2103-2109`); I did not see an actual captured exposition for
   `…_peers_blocked`.
4. **Whether every resolver engine in this fixture registers a
   `peers_blocked` family** (beacon/DKG) — I did not enumerate the engines
   created in `drive`.
5. **Compile/clippy.** Not run; I note only that `Some(("reason", reason))`
   with `reason: &&str` compiles via deref coercion in the same pattern as the
   pre-existing keyless test (`cert_inlet_tests.rs:644-653`).

---

## "Leave as is"

* **`SpyBlocker` not actually blocking.** Intended and identical in effect to
  `NoopBlocker` (`stand.rs:431-433`, `crates/dpos/p2p/src/lib.rs:537-541`);
  changing it would change the fixture's meaning.
* **`std::sync::Mutex` + `.unwrap()` inside the spy.** Test-only, single
  threaded runner, no `.await` held across the lock (`stand.rs:476-478`).
* **`BTreeSet` for `sites` and the test's second `sort_unstable`.** Redundant
  but harmless and defensive against the constants' spelling
  (`cert_inlet_tests.rs:1241-1250`).
* **`metrics_before_collect` drain split.** Correct and necessary: production
  `Committee::committee` calls during collect would otherwise pollute the same
  counters (`stand.rs:113-124`, `:2002-2010`).
* **`dpos_frontier_dropped_total` / `_rejected_total` names keeping `_total`
  through the `metrics` facade.** Matches the existing tests
  (`cert_inlet_tests.rs:644-653`) and the recorder records the name verbatim
  (`stand.rs:804-822`).
* **`frontier_plane` keeping `#[allow(clippy::too_many_arguments)]`.** Pre-existing.
* **New argument in `tests.rs`.** Correct position and dummy spy with an
  accurate comment (`tests.rs:74-77`).
* **`assert_lockstep_except(&[3, VICTIM])` and `only_these_ran_inlets`.**
  Inherited unchanged from the fixture's sibling test (`cert_inlet_tests.rs:1307-1309`).
* **The `fetcher.excluded` append-only claim and the `block!`+`fetcher.block`
  pairing.** Verified against CW `engine.rs:435-441`, `fetcher.rs:242,510-517,565-568`.

---

## Commands run (all read-only)

```
git status --short
git diff HEAD --stat
git diff HEAD --name-only
git log --oneline -5
grep -rn ... (workspace + cargo checkout)   # symbol/line discovery
sed -n / read tool on the files cited above
find . -iname '*E5-2*'                       # D-09
```

Assumption recorded for the ambiguous points: I treated the standing
"facts already established" (previous review round, gauge semantics,
marshal`verify_delivered` unreachability) as *hypotheses to spot-check*, and
only raised findings where I could point at code; where the hypothesis held I
did not re-litigate it.

Open question for a human: whether the intended acceptance for row 5.2 is
"I the FRONTIER arm is pinned and the marshal arm is documented unreachable"
or "the marshal arm must be made reachable and pinned". This test satisfies
the former; R-129 as registered asks for the latter (`REGISTER.md:1013-1017`).
