# Independent review — DKG beacon actor clock: one `watch` from the marshal tip (5.4-А)

Reviewer: independent code review, one-shot. Base = `HEAD 08764870`, tree = working tree
(`git diff HEAD`, 14 files under `crates/`). No `cargo` was run (task constraint); every
claim below is from reading the code and the pinned dependency sources, tagged `[KNOWN]`
/ `[LIKELY]` / `[GUESS]`. `file:line` refer to the working tree as it stands.

Scope of the change (from the task + the diff): the `DkgActor` clock becomes one
`tokio::sync::watch::Receiver<u64>` of the marshal's ordering tip. Removed:
`FluentApp.dkg_height_tx`, `with_dkg_heights`, `OuterBuilder.dkg_height_tx`,
`SharedBeaconPlane.dkg_height_tx`, `ValidatorInputs.heights`, `CertInlet::with_tee` /
`LiveFrontierTee`, the node poller's `fin + K` send, `PlaneClock::note_height_drop` and
`dpos_dkg_height_drops_total`. Added: `FluentApp.beacon_tip` + `with_beacon_tip`,
`SharedBeaconPlane.beacon_tip`, `ValidatorInputs.clock`, `clock.changed()` +
`borrow_and_update()`, four partition names in `beacon/mod.rs`, rewritten
`testbed/cert_inlet_tests.rs`, one new stand test.

## Assumptions

1. The orchestrator's gates (730/0, 57/0, 64/0, 16/0, clippy 0 own, fmt 0) and the
   "5/5 md5 equal to base" determinism measurement are accepted as given; I did not
   rerun them (no `cargo`).
2. The journal's measured numbers (`heal=[lag 8 vs majority 36]`, `ordering=96
   dkg_clock=96`, M1/M2/M3 transcripts) are the implementer's claims; I only checked
   that the code can produce them and that the mutations would be caught.
3. "Verifier/isolated mode" is read as: a validator that is verify-only because the
   `SafetyHalt` latch engaged, or `Role::AbsentBeacon` / `Beacon::Static` in the stand.
   There is no separate binary entrypoint for a verifier in this tree.
4. The `watch` semantics used below are from the pinned `tokio-1.52.3` source in
   `~/.cargo/registry/src/…/tokio-1.52.3/src/sync/watch.rs`.

## Findings

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| D-01 | **BLOCKER** (candidate; condition-dependent) | `crates/dpos/consensus/src/beacon/actor.rs:1239-1250` (coalescing comment + `changed()`/`borrow_and_update`), `:628-633` (`decidable_epochs`), `:2913-2927` (`decide_window`), `:3232-3233` (`past_seal`), `:2261` (seal threshold) | The clock is now a coalescing `watch`: "a burst of tips costs one `on_height` at the highest of them" (`:1242-1243`). `decide_window` starts the ceremony for epoch `E` only when the actor observes `now = epoch_of(height) = E-1` (`:2916`, `:628-633`). If a burst jumps over all of epoch `E-1`, the actor's first `now` for `E` is `>= E`, `recover(E)` computes `past_seal = height >= epoch_start(E) - DKG_MARGIN_BLOCKS` (`:3232-3233`) as true and returns `SatOut` (`:3253-3261`) instead of `Dealing`. The base app feeder `tx.try_send(height.get())` (removed `application.rs:1081-1085`) queued every tip into a 256-slot `mpsc`, so the actor processed `E-1` (dropping only on a full buffer, and then counting it). The deal window for `E` is therefore skippable by construction; a jump of one full epoch (interval) is the trigger. This is exactly the failure the removed `fin + K`/tee feeders were there to prevent for a catching-up validator. | I cannot disprove reachability: the actor's `on_height` awaits `broadcast_all` and does committee reads, so it can be busy across many `send_replace` calls; the marshal can store a backfill batch quickly. Counter-arguments: (a) in steady state the tip advances one block at a time and the actor keeps up, so no epoch is skipped; (b) a node a full epoch behind is arguably already past dealing for that epoch; (c) the base `mpsc` also dropped on overflow (the deleted `dpos_dkg_height_drops_total` existed for that). But the base had a *lossless* feeder under capacity, and the new channel is lossy by construction, not by load. Severity is BLOCKER under the task's literal criterion; I could not exhibit a production trace without running. | mechanism confirmed by code; reachability `[LIKELY]` |
| D-02 | **SERIOUS** | `crates/dpos/consensus/src/beacon/actor.rs:2350-2357` (`epoch_clock.send_if_modified`), `crates/dpos/consensus/src/beacon/dkg_engine.rs:332-389` (`prune_agreements`, band `cutoff-span..cutoff`, `swept_to = max(swept_to, cutoff)`) | The agreement launcher's journal-partition sweep is edge-driven off the actor's `now`. Under `mpsc` the actor published every epoch change; under the coalescing watch a jump of more than `AGREEMENT_SWEEP_SPAN = SCHEME_RETENTION_EPOCHS = 8` epochs moves `cutoff` directly from `C1` to `C2`, so the band `[C2-8, C2)` misses `[C1-8, C2-8)`, and `swept_to` then names `C2`, so no later band re-covers it. Partitions left by aborted instances in the missed band are never reclaimed (disk leak). The `stale` abort pass (`:340-360`) still aborts live instances, so no live instance leaks. | Could a production jump be >8 epochs? Only if the actor is >8 epochs behind, which is a pathological lag. The leak is bounded by the number of agreement instances that ran in the missed band, and partitions are pruned on the next genesis in devnet. Still a real behavior change from the lossless feeder. | confirmed by code |
| D-03 | **MODERATE** | `crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:1509-1541` (setup), `:1579-1610` (assertion 2), `:1640-1652` (assertion 5) | The mandatory stand test is a *necessary* but not *sufficient* witness that the inlet/plane-upstream moves the clock. Node 3 is cut from the consensus plane too (`Stand::partition(&[0,1,2],[3])`, `:1539-1541`), and after `heal_above(36)` both planes are restored, so node 3's own marshal gap repair and its own engine can advance its tip as well as the inlet. The test asserts `delivered` is non-empty in the catch-up window and that the artifact pins the laggard's seat, but it has no no-inlet control, so it cannot show the node would have missed the deal without the inlet. The test's own doc admits both producers (`:1483-1486` in the diff, "its own marshal repair and its inlet's by-height walk, both landing in the marshal"). | The plan's intent was "the replacement feeds the catch-up"; the test does show the replacement path is *live and delivers* (`delivered` non-empty, `dkg_clock >= EPOCH_2_START`), and M1/M2 reds show the clock itself is load-bearing. A missing-inlet control would be a perf comparison, which the journal explicitly declined (`§0.9.1`). So this is a test-strength gap, not a code bug. | confirmed by reading the setup and assertions |
| D-04 | **MODERATE** | `crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:88-96` (`clock_pair`), `:1624-1638`, `:130-138` (assertion in the first test) | `dkg_clock == ordering` "at rest" is a single end-of-run snapshot of two gauges written by two different tasks. Nothing synchronizes the actor's `on_height` with the metric scrape, so the assertion is a scheduling race (deterministic per seed/select order, but still a race). There is no `dpos_dkg_clock_height` time series (`marshal_tip_series` exists, no dkg series), so the invariant "the clock is never above the tip" is enforced only by construction (one writer), not observed. | The gauges are registered on the same context and read after `run_until` stops; in practice the actor drains before the predicate is met, and the orchestrator's gate is green. But `§3.7` records residual `select!` nondeterminism in this very test, so the equality can in principle move. The construction argument does prove there is no second writer. | confirmed by code; flake potential `[LIKELY]` |
| D-05 | **MINOR** | `crates/dpos/consensus/src/beacon/actor.rs:2590-2615` (`on_confirm` window uses `last_height`), `:172-174` (`within_ingress_window`), `application.rs` / `confirmations.rs` (mint is width-growth edge-triggered, per the code comment `:2603-2609`) | A coalesced jump moves `last_height` forward by more than `INGRESS_LOOKAHEAD_EPOCHS` in one step; a share-confirmation for a target epoch that was `now+1` before the jump is then `< now` and is refused (`confirm_window`), and the comment says such a confirmation "is never re-issued". | A lagging `mpsc` actor could also refuse the confirmation from the other side (`target > now+2`), because `select!` may process `on_message` before draining queued heights; so this is not a clean regression. The code comment argues a confirmation for a far epoch decides nothing at this node. | inferred |
| D-06 | **MINOR** (pre-existing, not introduced) | `crates/dpos/consensus/src/sync_metrics.rs:419` (`engaged: watch::Sender<bool>`), `:595-604` (`engaged_edge` = `subscribe().wait_for`), `crates/dpos/consensus/src/epoch_manager.rs:601`, `crates/dpos/consensus/src/beacon/dkg_engine.rs:757` | The round-3 fix's invariant "one parked receiver per `watch` channel" is not global: the `SafetyHalt.engaged` watch has **two** parked `wait_for` waiters (the epoch manager's engine-abort arm and the agreement launcher's abort arm), woken through the same `big_notify` + `thread_rng_n(8)` shard order that caused the round-3 nondeterminism. | Pre-existing and untouched by this change; the edge fires at most once and both waiters do independent aborts, so the impact is far smaller than the per-tip `ordering_tip` case. The round-3 journal explicitly lists both waiters as intended. Not this change's defect, but it is the exact pattern the change's own docs claim to have eliminated. | confirmed by code |
| D-07 | **MINOR** | `crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:1509+` (the new test); `dsh-input-journal.md` §3.7 | The new catch-up test is not fully deterministic even after the two-channel fix: the journal measures ~2–3% divergence (2/65 before the doc edit, 0/40 after) in the printed `in_window` field, attributed by measurement to `tokio::select!` random branch choice, and records this as a STOP. The orchestrator's "bit-identical 5/5 md5" claim therefore holds for the other stand tests, not provably for this one. No assertion depends on the varying field. | The variation is in a `eprintln!` value, not an assertion; the journal says 0/40 after the doc edit. It is a flake in the recorded run digest, not in the verdict. | journal claim `[KNOWN]`, code inference `[LIKELY]` |
| D-08 | **MINOR** | `crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:1606-1610` | `pinned_seats.len() == N` asserts all four dealers made the seal; the claim under test only needs the laggard's seat (`:1596-1602`). Any one of the other three missing the window reds the test for a reason unrelated to the property. | The measured run has `pinned=[0,1,2,3]`; the extra assertion guards against a run where the laggard's seat is present only because everyone was late, which is a fair fixture guard. Still tighter than the claim and a pacing-sensitive flake surface. | confirmed by code |
| D-09 | **NIT** | `devnet/local-dpos-smoke/dpos_harness/cases/smoke/verdicts_fault.py:624-628` | The comment still says the beacon's clock is `finalized + K` fed by "the finalized-height poller (`crates/node/src/dpos.rs`, the finalized-height poller)". That feeder is deleted; the clock is the marshal tip. The derived `DKG_CLOCK_LEAD = RESULT_LAG_K` arithmetic stays valid because the tip equals `fin + K` once the cold-start floor is inactive, so no test change is needed. Also `test_smoke_fault_verdicts.py:499` still asserts `DKG_CLOCK_LEAD == 3`. | Purely a comment; "verdicts_fault.py" logic uses the ordering-scale edge converted to the finalized scale, which is unchanged. The devnet smoke was not run by the implementer (`§3.7.4`). | confirmed by code |
| D-10 | **NIT** | `crates/dpos/consensus/src/beacon/mod.rs:160,172,177,190` (new private `const`s) vs removed `crates/dpos/consensus/src/dpos.rs` (`ARTIFACT_JOURNAL_PARTITION` was `pub`) | One public item (`ARTIFACT_JOURNAL_PARTITION`) was removed from the crate's public `dpos` surface and all four names are now private to `beacon/`. No in-tree consumer exists (`git grep` over `crates bins devnet` is empty outside `beacon/`), so no build break; an out-of-tree consumer would break. | The old doc's rationale ("Public because the store is opened by the always-on beacon plane in the node crate") was already false: the node opens the store through `beacon::build`, and `git grep ARTIFACT_JOURNAL_PARTITION -- crates bins` had no non-beacon reader at base either. `dpos.rs` needs no re-export. | confirmed by code |
| D-11 | **NIT** | `crates/dpos/consensus/src/cert_inlet.rs:1121`, `crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:23,1454` | Historical `LiveFrontierTee` / `tee` references remain in doc comments after the type was deleted. | Deliberate per journal §0.8 ("records of what was true on their date"); the neighboring new text names the replacement. Not code. | confirmed by code |
| D-12 | **NIT** | `crates/dpos/consensus/src/cert_inlet.rs:730-736`, `crates/dpos/consensus/src/application.rs:349-381,1092-1111`, `crates/node/src/cert_inlet.rs:29-34`, `crates/dpos/consensus/src/testbed/cert_inlet_tests.rs:1454-1508` | New comments narrate history and cite the row/review process ("5.4-А", "Д-5.4Б-3", "review B1-14", "M1 of the row's map", "the deleted `:781-785`"). `AGENTS.md` bans change notes and review pointers. | The repo's human baseline is dense with the same style (`R-121`, `§0.7(а)`, `FLU-1173`); these comments match the file they live in. No fix requested; reported under the hygiene question. | confirmed by code |
| D-13 | **NIT** | `crates/dpos/consensus/src/sync_metrics.rs:264-269` | The new doc says "Both halves are the SAME tip". The DKG half is the actor's clamped clock, which legitimately lags the tip while the actor is behind (that is what the subsequent lag sentence relies on). `record_dkg_clock` (`:345-350`) is written only from `on_height` (`actor.rs:2228`). | The next sentence ("a growing lag with a moving ordering half is a beacon actor that is not running") is correct and the gauges are correct; only the first sentence overstates. | confirmed by code |
| D-14 | **NIT** (observability) | `crates/dpos/consensus/src/sync_metrics.rs:318-333` (removed `dpos_dkg_height_drops_total`; only the height + lag gauges remain) | The base had a counter whose documented meaning was "ticks dropped / actor not draining". The watch cannot "drop", but it *coalesces* silently: the number of publishes the actor skipped is now unobservable. The pair `dpos_ordering_finalized_height` vs `dpos_dkg_clock_height` (lag gauge) still shows a stopped actor, so the loss is partial. | The lag gauge is the intended replacement and is arguably the better signal ("a growing lag under a moving ordering half is an actor that has stopped taking the tip"); no dashboard/script referenced the deleted counter (`git grep dpos_dkg_height_drops` is empty under `crates devnet`). | confirmed by code |

## Question-by-question

### (1) CLOCK LIVENESS
`[KNOWN]` `FluentApp::report` writes `ordering_tip.send_replace(h)` and then, in the same
synchronous arm, `beacon_tip.send_replace(h)` (`application.rs:1097-1111`). The beacon
sender is created by the plane *before* the app: node `crates/node/src/dpos.rs:1323`
(`Arc::new(watch::Sender::new(0u64))`), the actor's receiver taken at
`crates/node/src/dpos.rs:1899` (`clock: beacon_tip.subscribe()`), the sender then moved
into `SharedBeaconPlane` (`:1962`) → `DposLayerConfig` → `DposLayer::launch`
(`crates/dpos/consensus/src/dpos.rs:2079`) → `OuterBuilder.beacon_tip: Some(beacon_tip)`
(`:2936`) → `FluentApp::with_beacon_tip` (`outer.rs:1002-1005`). The stand is identical
(`testbed/stand.rs:2785`, `:2914`, `:2957`, `:2979`).

* `git grep with_beacon_tip -- crates`: only `application.rs` (definition + tests),
  `outer.rs:1003` (the one production call), plus doc references. There is exactly one
  production construction site, and it is on the validator path.
* `FluentApp::new` call sites: `outer.rs:989` (production/stand) and three inside
  `application.rs` tests. `with_beacon_tip` is applied at `outer.rs:1002-1005` **before**
  `marshal_reporter_app = app.clone()` (`:1006`), so every clone — the marshal's reporter
  half and the epoch manager's copy — publishes on the same sender.
* Follower path: `launch_follower` builds `OuterBuilder { beacon_tip: None, … }`
  (`dpos.rs:4116`) and `launch_follower` runs `beacon::build_follower`, which spawns no
  `DkgActor`. No receiver, no clock.
* `Beacon::Static` stand: `beacon_tip = None` (`stand.rs:2957`), no actor. `Beacon::Live`
  + `Role::AbsentBeacon`: sender is `Some` with zero receivers; `send_replace` →
  `send_modify` does not fail or panic with zero receivers (tokio `watch.rs:1105-1204`,
  `:1229-1234`), so this is safe.
* `watch::Sender::subscribe()` sets the receiver's version to the current version
  (tokio `watch.rs:1387-1394`), so `changed()` ignores a value published *before*
  subscribe. In both node and stand, subscribe happens before any `FluentApp` exists, so
  the "a tip published before the actor started is seen on the first pass" claim
  (`actor.rs:1239-1242`) is correct: the receiver's version is fixed at subscribe, and
  every subsequent publish bumps it.
* Restart: the marshal reports its highest stored finalization at startup (CW pinned
  checkout `consensus/src/marshal/core/actor.rs:396-401`), which is a publish after
  subscribe; the clock needs no seed.

**Conclusion:** I found no validator construction path where the actor's sender is never
written. `[KNOWN]` for the call graph; `[LIKELY]` that no untracked path exists (I grepped
`with_beacon_tip`, `FluentApp::new`, `OuterBuilder`, `SharedBeaconPlane`, `beacon_tip`).

### (2) LOSS from `watch` coalescing
Consumers of the height value in `on_height` (`actor.rs:2217-2399`):
1. `last_height = clamp(prev, height)` (`:2222-2223`) — feeds `epoch_of`/`height_now`
   everywhere.
2. `plane_clock.record_dkg_clock(height)` (`:2228`) — gauge only.
3. `now = epoch_of(height)`; `decide_window(now)` (`:2251`) — starts/recovers epochs
   `[now-8 ..= now] + {now+1}` (`decidable_epochs`, `:628-633`).
4. Seal: `height >= epoch_start(e) - DKG_MARGIN_BLOCKS` (`:2256-2264`) — a threshold, so a
   later height still seals; skipping the exact deadline height does **not** skip the seal.
5. `pending.retain` (`:2310-2319`), `drive_finalization` (`:2325`, also driven by
   `on_message`), `sweep_epoch_state(now)` (`:2341`), `epoch_clock.send_if_modified` (`:2350`),
   `retransmit` (`:2376`), `broadcast_all`, `drive_acquisition(now)`, `fetch_missing_logs`.

* Skipping an epoch boundary: **D-01** — a jump over all of `E-1` starts `E` only at
  `now >= E`, and `past_seal` (`:3232-3233`) then yields `SatOut` (`:3253-3261`). Under the
  removed `mpsc` app feeder the `E-1` tick was queued and processed. This is the one
  consumer where coalescing can skip an action.
* Sweep/cutoff: **D-02** — the launcher's band sweep can miss a band on a >8-epoch jump.
* Deadline height passed without observation: not a skip, because the seal test is `>=`
  and `decide_window` re-recovers the trailing window; the *timing* of the deal is the
  issue (D-01), not the seal.
* Confirmations: `on_confirm` reads `last_height` (`:2610-2615`), see **D-05**; both a
  coalescing jump and an `mpsc` backlog can put the target outside `[now, now+2]`, so it is
  not a clean regression.

### (3) TEE REMOVAL
The tee used to `try_send` the verified upstream height *before* `marshal.verify_block`
(the deleted block sat where `cert_inlet.rs:730-736` now explains). Today the only clock
move is `report_finalization` → CW actor `Message::Finalization` → `store_finalization`,
which reports `Update::Tip` only for `height > self.tip` (CW `marshal/core/actor.rs:585-593`
and `:1404`, `:1454-1456`); the block must first be in the verified cache via
`verify_block` (`cert_inlet.rs:754-767`), which is why the order in `ingest` is
`verify_block` then `report_finalization`.

* Exact deal-deadline computation: `actor.rs:2261`,
  `height >= self.epoch_start(e).saturating_sub(DKG_MARGIN_BLOCKS)`; `epoch_start` at
  `:1169`, `DKG_MARGIN_BLOCKS = 20` at `:123`. For the mandatory test, the epoch-2
  ceremony is started on entering epoch 1 and seals at `epoch_start(2) - 20 = 44`
  (`cert_inlet_tests.rs:1513-1520`).
* Exact height the clock reaches before it: the test requires the laggard's clock to leave
  8 and reach at least `epoch_start(1) = 32` before the seal at 44; the evidence is the
  artifact pinning the laggard's seat (`cert_inlet_tests.rs:1596-1602`), not a direct
  deadline observation.
* Is the mandatory test really exercising the inlet/plane-upstream? Setup lines:
  `cert_inlet_tests.rs:1509-1541` — `StandConfig::live(4,1)`, `Committees::All`,
  `cert_inlet = { nodes: [3], source: NextAboveTier }`,
  `partition(&[0,1,2],[3]).after_height(8).heal_above(36)`. The inlet is present and
  delivers certificates in the window (assertion `:1648-1652`), but the node also receives
  certs through its own restored consensus links and its own marshal gap repair, so the
  test cannot attribute the clock movement to the inlet (see **D-03**). It is not a
  no-inlet control.
* Would the test be green with `clock.changed()` ignored (M1)? No. `[LIKELY]` The first
  assertion is `!out.timed_out` (`:1543-1549`); with the clock frozen the actor never
  enters epoch 1, never deals for epoch 2, no `PK_2` is produced and the chain stops at
  `last(1) = 63` — the journal's M1 transcript shows exactly `heights [63,63,63,63]`.
  Even if only the laggard's actor were frozen, assertion (2) `pinned_seats.contains(&seat)`
  (`:1596-1602`) would red, and the artifact could not pin a seat whose dealer never
  dealt. Restart needs no seed because the marshal republishes its stored tip
  (CW `:396-401`); I found no seed-dependent path.

### (4) TWO CHANNELS
* Writers: `git grep ordering_tip` shows exactly one production writer,
  `application.rs:1105`; exactly one `beacon_tip` writer, `application.rs:1110`, in the
  same `if let Update::Tip` arm, no `.await` between them. No other writer of either.
* Value drift: the two channels are written from the same `height.get()` in the same
  synchronous arm. Each channel has its own consumer (`epoch_manager.rs:531` for
  `ordering_tip`; `beacon/actor.rs:1244` for `beacon_tip`), and neither consumer reads the
  other channel, so a momentary ordering between the two `send_replace` calls cannot be
  observed as a cross-channel inconsistency. No drift.
* `beacon_tip` is `None` only on paths with no beacon actor (follower `dpos.rs:4116`,
  `Beacon::Static` stand `stand.rs:2957`); never `None` on a validator path.
* Second subscription: `git grep 'subscribe()' -- crates/dpos/consensus/src crates/node/src`
  shows `app.ordering_tip()` only at `epoch_manager.rs:531` (production) and
  `beacon_tip.subscribe()` only at `crates/node/src/dpos.rs:1899` and
  `testbed/stand.rs:2914` (plus the application test). Exactly one parked receiver per
  channel on the validator path.

### (5) POLLER REMNANT
`crates/node/src/dpos.rs:1465-1718` is the surviving poller task. Its remaining effects:
(a) resolve + publish the geometry freeze — `et.freeze_geometry` (`:1549`),
`geometry_tx.send_replace(Some(frozen))` (`:1572`), `committee_wake.anchor_advanced()`
(`:1580`); (b) the first `track_peers` registration (`:1618`); (c) the tombstone read and
transport severance (`:1676-1703`). It writes **no** height channel, no `PlaneClock`
writer, and nothing that reaches the actor's clock: `dkg_height_tx`, `note_height_drop`
and the `fin + K` sends are gone (`git grep dkg_height_tx|note_height_drop|
dkg_height_drops` under `crates devnet` is empty). `geometry_tx` only starts the actor
(activation/interval); it does not push a height. So the remaining poller does **not** feed
the beacon clock by any path. The removed identifiers survive only in documentation
(`cert_inlet.rs:1121`, `cert_inlet_tests.rs:23,1454`) and in the stale devnet comment
(D-09).

### (6) E5-40
Four names moved verbatim into `crates/dpos/consensus/src/beacon/mod.rs` as **private**
`const`s: `SEED_JOURNAL_PARTITION` (`:160`), `MINT_MEMO_PARTITION` (`:172`),
`ARTIFACT_JOURNAL_PARTITION` (`:177` — was the only `pub` one),
`AGREEMENT_JOURNAL_PARTITION_PREFIX` (`:190`). Readers (all inside `beacon/`):
`plane.rs:51-52,707,745,767` and its tests `:1339,1343,1347,1351`;
`dkg_engine.rs:90,285`. `git grep` over `crates bins devnet` finds no other reader, and
`crates/dpos/consensus/src/dpos.rs` has no reference, so **no re-export is needed**. The
one visibility change (`pub` → private) is D-10.

### (7) TESTS
Journal §0.4's 16 rows checked against the tree:
* Rows 1, 2, 5, 6, 8, 9, 11, 12: `tee_heights` → `delivered`, recorded in
  `RecordingSink::verify_block` (`stand.rs`, test-only), which stands on the first marshal
  call of the clean path — the same set the tee saw (no `.await`/`return` between the
  deleted tee line and `verify_block`; `cert_inlet.rs:714-767` has only resets between
  them). Re-expressed, not weakened.
* Rows 3, 7, 10, 16 (drop counter, `should_panic` guard): genuinely tee/channel-only
  properties; the `watch` cannot refuse a send and the counter/field are gone
  (`sync_metrics.rs` has no `drops`), so removing them is honest.
* Row 13: `boundary_cert_defers_…` was rewritten to `control_marshal.calls ==
  ["verified","report"]` (`cert_inlet.rs:1159-1183`) — verified.
* Row 14: the deleted `verified_cert_advances_the_dkg_deal_clock_tee` properties are
  covered by `matching_epoch_cert_passes_the_height_epoch_bind` (`cert_inlet.rs:1096-1112`)
  and `wrong_signature_cert_skips_with_no_report_and_returns_ok`
  (`cert_inlet.rs:1185+`) — verified.
* Row 15: the `TeeWiring::{Observed,Production}` test was removed with the enum — verified.

Tautology check: no rewritten test asserts only a value it set itself; `delivered` is a
recording of the node's marshal seam and the gauges are the node's own. But the
`dkg_clock >= delivered_top` half is explicitly non-exclusive (`cert_inlet_tests.rs:140-153`
doc), i.e. weak, not tautological. The `PlaneClock`/gauge observations read the same watch
the actor consumes: `dpos_dkg_clock_height` is written in `on_height` off the clamped watch
value (`actor.rs:2228`), and `dpos_ordering_finalized_height` is written from the same
`Update::Tip` that feeds the watch (`application.rs:1098`). See D-04 for the end-of-run
race.

### (8) DETERMINISM
`application.rs:1097-1111`: exactly one parked receiver per watch on the validator path
(`ordering_tip` → epoch manager, `beacon_tip` → `DkgActor`). `tokio-1.52.3`
`watch.rs:421-424` picks the `big_notify` shard with `thread_rng_n(8)` per `changed()`
call; two waiters on one channel are woken in process-random order. Other watches checked:
`epoch_clock` (`plane.rs:697`, one waiter `dkg_engine.rs:780`), `geometry`
(`plane.rs:867`, one waiter), `ConfirmPool.inputs` (`dkg_agree.rs:1119`, one waiter),
`open` (`dkg_agree.rs:2025`, one waiter), `ordering_tip` (one). The one exception is
`SafetyHalt.engaged`, which has two `wait_for` waiters (**D-06**). No second subscription
was introduced by this change.

### (9) HYGIENE
* `#[allow]`: no new `#[allow]` in the diff; the `too_many_arguments`/`type_complexity`
  ones are pre-existing.
* New production `unwrap`/`expect`/`panic!`/`unsafe`: none. All added occurrences are in
  `#[cfg(test)]` code (`testbed/` is `#[cfg(test)] mod testbed`, `lib.rs:74`;
  `RecordingSink`'s `.lock().unwrap()` is test-only).
* New `pub` outside `beacon/`: none — `with_beacon_tip`, `OuterBuilder.beacon_tip`,
  `SharedBeaconPlane.beacon_tip` are renames/replacements of the previously public
  `dkg_height_tx`/`with_dkg_heights` surface; inside `beacon/`, four names became *less*
  visible.
* Dead imports: none (clippy clean per the orchestrator; `mpsc` and `Counter` remain used
  elsewhere in the files they moved out of).
* Log levels: no new `tracing` calls in production.
* Comments: D-11, D-12.

### (10) Where this review is weakest (ranked)
1. **D-01 reachability.** I proved the mechanism (a coalesced jump over `E-1` makes
   `recover(E)` see `past_seal`), but not that a real validator can coalesce a full epoch
   without `cargo`/a stand. The severity is stated conditionally for that reason.
2. **D-02 reachability.** Same class: the >8-epoch jump is easy to prove in code and hard
   to bound in production.
3. **Test-causality (D-03).** I read the setup and assertions, but I could not run a
   no-inlet control, so my "the node also catches up on its own" claim is by code
   inspection of `Stand::partition`/heal and the engine layout, not measured.
4. **D-04 flake.** Without the deterministic runner I cannot say whether
   `dkg_clock == ordering` is stable for the current seed/select order.
5. **Coverage of the `mpsc`-era paths I did not trace end to end**: `on_message`'s
   buffering (`is_bufferable`) and the resolver/heal paths were read only around the clock
   question, not audited in full.

## Leave as is

* The one-writer/two-channel design is sound and is what the round-3 defect required: one
  writer (`application.rs:1097-1111`), one parked receiver per channel, order fixed by two
  adjacent synchronous statements. The mechanism citation
  (tokio `watch.rs:421-424` `thread_rng_n(8)`) checks out against the pinned source.
* `with_beacon_tip` is applied before the app is cloned (`outer.rs:1002-1006`), so the
  marshal's reporter clone and the epoch manager's clone share the writer. The application
  test `the_tip_is_published_on_the_watch_handed_in_and_on_every_later_subscription`
  (`application.rs:2713-2767`) is a real guard against the frozen-clock mistake and is not
  tautological.
* The clamp stayed where the task said it should (`actor.rs:2222`) and the gauge is still
  written at the clamp (`:2228`), so the gauge reads the clock the ceremony geometry runs
  on; the new unit test renames the property honestly
  (`the_dkg_clock_gauge_is_the_actors_clamp_not_the_last_height_handed_in`).
* The E5-40 move is clean: the four names are private, in the module that opens them, with
  no reader outside `beacon/` and no dangling `dpos.rs` re-export.
* The poller's remaining three jobs are real and its clock-part is gone by every path;
  `git grep` finds no `dkg_height_tx` / `note_height_drop` / drop-counter reference in
  `crates` or `devnet`.
* `RecordingSink` is a better seam than the old `TeeWiring` split: it records the same
  clean-path set without a second channel, and lets the drop-counter assertion be replaced
  by an exact "delivered == ingests" identity.
* The historical `LiveFrontierTee` prose in doc comments is deliberate record-keeping; I
  would not churn it.

## Commands run (read-only)

* `git status --short`, `git diff HEAD --stat`, `git diff HEAD` over the 14 files.
* `git grep` for `with_beacon_tip`, `beacon_tip`, `ordering_tip`, `subscribe()`,
  `changed()`, `wait_for`, `dkg_height_tx`, `LiveFrontierTee`, `TeeWiring`, `tee_heights`,
  `with_dkg_heights`, `note_height_drop`, `dkg_height_drops`, `cs_fin_num`, the four
  partition names, `engaged_edge`, `dpos_dkg_height_drops_total`.
* `sed`/`grep` on the pinned tokio-1.52.3 `src/sync/watch.rs` (`send_modify`, `send_replace`,
  `changed_impl`, `maybe_changed`, `subscribe`, `big_notify`) and the pinned commonware
  `consensus/src/marshal/core/actor.rs` (`:396-401`, `:585-593`, `:1404`, `:1454-1456`).
* `read` of `beacon/actor.rs`, `application.rs`, `outer.rs`, `beacon/plane.rs`,
  `beacon/mod.rs`, `cert_inlet.rs`, `dpos.rs`, `sync_metrics.rs`, `node/dpos.rs`,
  `node/cert_inlet.rs`, `testbed/{stand,cert_inlet_tests,tests}.rs`, and
  `devnet/local-dpos-smoke/dpos_harness/cases/smoke/verdicts_fault.py`.
* `dsh-input-journal.md` §0, Круг 2, Круг 3 read for the implementer's claims.
* No `cargo` invocation (per the task). No file outside this deliverable was modified.
