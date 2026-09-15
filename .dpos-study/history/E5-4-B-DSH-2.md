# Round-2 review — DKG epoch-key agreement launcher fix (заход 5.4-Б, круг 2)

Object: `git diff HEAD` in this workspace (HEAD `79dd18f9`, 10 modified files). Round-1 material is
re-reviewed only where round 2 touched it (`sync_metrics.rs`, `epoch_manager.rs`,
`beacon/{dkg_engine,actor,plane}.rs`). The four round-1 BLOCKERs left open (D-01/D-06/D-12/D-13) are
assumed accepted/deferred to 5.4-А (Д-5.4Б-3) and are **not** re-raised; I only check whether round 2
made them worse. All findings carry `file:line`; comments/docs are never used as evidence (tokio
sources are cited for `watch`/`select!` semantics). Cargo was not run (per brief), so gate numbers are
not independently verified — only code claims are.

External sources cited by absolute path:
- tokio 1.52.3: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.52.3/src/sync/watch.rs`
  and `.../src/macros/select.rs` (referenced below as `tokio watch.rs` / `tokio select.rs`).
- commonware runtime: `~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c/runtime/src/...`.

Lockfile confirms tokio 1.52.3 (`Cargo.lock:15129-15131`).

---

## 1. Answers to the ordered questions

### Q1 — LATCH (`sync_metrics.rs`)

- **Every HEAD property preserved.**
  - *engage idempotent*: `latch()` is `metrics.set(1)` + `send_if_modified(|e| !mem::replace(e, true))`
    (`sync_metrics.rs:544-548`); on a repeat the closure returns `false` and nothing is published, the
    gauge re-set is a no-op. HEAD did `swap(true)` + `notify_one` only on 0→1 (`git show
    HEAD:.../sync_metrics.rs`, `latch`).
  - *first reason wins*: `reason: Arc<OnceLock<SyncReason>>`, `self.reason.set(reason).is_ok()` before
    `latch()` (`:556`), unchanged.
  - *marker restore engages synchronously in the constructor*: `restoring` calls `restore_marker()`
    before returning (`:481-490`), which calls `engage`/`latch` synchronously (`:511,:525,:535`).
  - *`is_engaged` from any thread without await*: `*self.engaged.borrow()` (`:609-611`), a synchronous
    `watch::Sender::borrow` (tokio watch.rs:1253-1260); the `Ref` temporary is dropped at the end of
    the tail expression, no `await` in the function.
  - *metrics gauge exactly as before*: `self.metrics.safety_halt_engaged.set(1)` is executed before the
    edge in both HEAD and round 2 (`:545`).
- **Can the closure return `true` twice?** No. The closure returns `true` iff the old value was
  `false`; nothing ever writes `false` back (`git grep "engaged\." -- crates` shows only `borrow()` and
  `subscribe()`, no `send*` of `false`), and concurrent `latch()` calls are serialized by the watch
  write lock (`tokio watch.rs:1175-1206`), so exactly one call observes 0→1.
- **Is `borrow()` ever held across an await (deadlock with `send_*`)?** No. `is_engaged` is `fn` (not
  `async`) and returns `bool` (`:609-611`). `engaged_edge` uses `wait_for`, whose read lock is released
  before the internal `changed_impl().await` (`tokio watch.rs:902-934`), and the returned `Ref` is
  discarded by `let _ = ...;` (`:625`). No `Ref` is stored in a struct or held across an await.
- **Does `Clone` share ONE channel?** Yes. `#[derive(Clone)]` (`:434`) clones `watch::Sender`, which
  shares `Arc<Shared>` (`tokio watch.rs:201-209`); there is no per-clone channel, and `#[derive(Default)]`
  is valid because `watch::Sender<T: Default>: Default` (`tokio watch.rs:211-215`).
- **Any construction path bypassing the new field?** No. `git grep "SafetyHalt *{"` finds only the
  definition; all construction goes through `SafetyHalt::new` (`:459`), `restoring` (`:481`) or
  `Default` (`:434`), plus `SafetyHalt::default()` in `outer.rs:1593`.

### Q2 — WAITERS

- `wait_for` tests the predicate on the current value **before** waiting:
  `wait_for_inner` reads the value and calls `f` first, returning `Ok` on `true`
  (`tokio watch.rs:896-936`, especially `:902-914`). Therefore a receiver created by `subscribe()`
  **after** the value became `true` resolves on its first poll; `subscribe()` takes the current version
  (`tokio watch.rs:1387-1394`), but `wait_for` does not depend on `has_changed` for the `Ok` return.
- **Two concurrent waiters**: each receiver is notified independently; `send_if_modified` calls
  `notify_rx.notify_waiters()` (`tokio watch.rs:1208`), so both armed `wait_for`s resolve. The new test
  `every_engaged_edge_waiter_resolves_and_a_late_one_resolves_at_once` pins exactly this
  (`sync_metrics.rs:854-887`).
- **Polled after completion / `resumed after completion`?** No. Both loops arm a single pinned
  `halt_edge` and guard the arm with `if !halt_seen` (`epoch_manager.rs:601-603,:621`;
  `dkg_engine.rs:757-759,:780`). Tokio's `select!` evaluates a disabled branch's `<async expression>`
  but **never polls it** (`tokio select.rs:27-42`, `:626-631`, `:691-694`), and it returns as soon as
  one branch is `Ready` (`:705-728`), disabling it, so the completed future is never re-polled. The
  branch pattern is `_` (always matches), so a `Ready` arm is always the selected one. `if !halt_seen`
  does therefore *not* re-poll; it prevents the spin. The manager's `halt_seen` is never reset
  (`epoch_manager.rs:603,:622`), nor the launcher's (`dkg_engine.rs:759,:781`).
  Caveat: the pinned `halt_edge` expression is still *evaluated* when disabled (a reborrow,
  `tokio select.rs:641`), which is harmless.

### Q3 — LAUNCHER

- **`on_halt` joins inline — deadlock?** No deadlock path found. `on_halt` does
  `mem::take(instances)` then `handle.abort(); drop(handle.await)` for each (`dkg_engine.rs:973-982`).
  The instance's shutdown cannot need the launcher: the launcher owns no channel the instance reads
  (its inputs are `requests` from the actor and `clock` from the actor), and the instance's outputs go
  to `out` (drained by the actor's write-back task) and to the muxers (`start_one`/`spawn_agreement`,
  `dkg_engine.rs:1041-1069`); `abort()` cancels the instance at its next await, so a full `out` cannot
  wedge the join.
- **After `on_halt` the map is empty — can a NEW target spawn again?** No: `start` re-reads
  `is_engaged()` before the committee read and before any spawn (`:898-905`). A request already queued
  in `requests` is handled by the same `start` gate (`:898`), so it is refused, never spawned.
- **Exit paths.** `requests` closed (`:762-766`) and `clock` writer dropped (`:767-775`) both `break`
  and then `launcher.abort_all(...)` (`:794`); halt takes `on_halt` (`:780-783`); a task-level abort
  is covered by runtime supervision (`runtime/src/utils/handle.rs:73-74` calls `tree.abort()`, which
  cascades to descendants at `runtime/src/utils/supervision.rs:144-167`). Every path aborts instances.
- **Finding E-01** below: the post-spawn re-check is narrower than the journal claims.

### Q4 — POST-SPAWN RE-CHECK

- Ordering is correct for the just-spawned instance: `self.started.insert(target_epoch)` (`:926`),
  `self.instances.insert(Epoch::new(target_epoch), handle)` (`:929`), *then*
  `if self.safety_halt.is_engaged() { self.on_halt().await }` (`:942-944`). The handle is in the map
  before the latch read, and `on_halt` takes the whole map, so a Running instance spawned under a latch
  that engaged during the four registrations (`:1023`) is aborted and joined before `start` returns.
- `started.retain(floor)` runs on every `on_request` outcome (`:889-890`), including refusals; the
  dedup early-return (`:880-882`) does not grow the set.
- **Gap (E-01):** the re-check exists only in the `Started::Running` arm (`:942-944`). When the latch
  engaged during `start_one` and the outcome is `Started::NotAMember` (`:947-949`) or `Started::Failed`
  (`:951`), `start` returns without any halt handling, and pre-existing instances stay alive until the
  `select!` halt arm wins. Likewise, a latch that engages *after* line 942 but before the next select
  leaves the newly spawned instance alive for that window. The halt edge is never lost (the pinned arm
  is ready), so this is a latency/completeness gap, not a lost signal.

### Q5 — ACTOR

- **No latch in the actor**: `grep -n "safety_halt\|is_engaged\|SafetyHalt" beacon/actor.rs` returns
  only the doc comment at `:816`; `Wiring.safety_halt` is gone (`git grep` empty), and
  `AgreementClock` no longer exists anywhere in `crates/` (`git grep AgreementClock` empty).
- **Clock publishes only on epoch change**: `on_height` computes `now = epoch_of(height)` (`:2226`,
  `:1161-1165`) and at step 2c does `self.epoch_clock.send_if_modified(|v| if *v != now { *v = now;
  true } else { false })` (`:2347-2354`). `on_height` has no early return before step 2c (no `return`
  between `:2211` and `:2396`), and no other site writes `epoch_clock` (`git grep epoch_clock`).
- **Renamed test really drives the property**: `the_agreement_clock_moves_on_the_epoch_edge_and_never_per_tick`
  (`actor.rs:7933-7972`) ticks `INTERVAL*2`, `INTERVAL*2+1`, `INTERVAL*2+2`, `INTERVAL*3`,
  `INTERVAL*3+1`; `INTERVAL = 30` (`:4451`) with activation 0, and `epoch_of` uses
  `epocher.containing(Height)` (`:1161-1165`), so the heights are epochs 2,2,2,3,3. It asserts the
  value/edge on the epoch changes and `!has_changed()` inside each epoch. Good pin.
- `plane.rs` builds `watch::channel(0u64)` (`:679`), passes `epoch_clock_rx` to the launcher (`:930`)
  and `epoch_clock_tx` into `Wiring` (`:882`); `Wiring::inert` builds one with the receiver dropped
  (`actor.rs:4352,:4366`). Consistent `u64` everywhere.

### Q6 — TESTS

- Five new tests, matching the +5 by name: `sync_metrics::tests::every_engaged_edge_waiter_resolves_and_a_late_one_resolves_at_once`
  (`sync_metrics.rs:855`); `dkg_engine::tests::{the_launcher_task_starts_on_a_request_prunes_on_the_clock_edge_and_aborts_on_exit
  (:2368), a_latch_that_engages_during_the_spawn_retires_the_instance_it_started (:2309),
  a_halt_edge_aborts_the_running_instance_without_a_clock_tick_and_the_task_keeps_answering (:2436),
  a_latch_engaged_before_the_launcher_is_built_refuses_the_first_request (:2504)}`.
- **Can a test pass by timing out?** No. The dkg_engine tests use `Runner::timed(600s)`
  (`:2369,:2310,:2437,:2505`) and the deterministic runtime panics with `runtime timeout` at the
  deadline (the journal's M4 confirms); the sync test uses `Runner::default()` and manual polling, so a
  broken property fails an `assert` (`:867-884`), it cannot silently time out.
- **Could a test be green with its property broken?** Each was checked against its falsifier:
  - `a_halt_edge...` needs event 2, which only the halt arm can publish (one request, no clock edge),
    so removing the arm cannot be green (`:2452-2476`).
  - `a_latch_engaged_before...` needs two events and `reads == 0`; with the arm removed `after(2)`
    never resolves (runtime stalls), and a spawn would raise `reads`.
  - `a_latch_that_engages_during_the_spawn...` asserts `reads == 1`, empty map, `started` contains the
    target, route free (`:2338-2355`); only the post-spawn check can clear the map here.
  - The `+5` test `the_launcher_task_...`'s **exit** half is weaker than its doc claims — see E-02.
- **`#[cfg(test)]` probe**: the field/type are `#[cfg(test)]` (`dkg_engine.rs:691-692,:698-703`), the
  clone/publication are `#[cfg(test)]` (`:747-750,:785-792`), and production passes `probe: None`
  (`plane.rs:919-920`). `mod testbed` is `#[cfg(test)]` only (`lib.rs:73-74`), and the crate's
  `dpos-devnet-byzantine` feature only pulls in `byzantine` (`lib.rs:45-46`); the stand does not
  reference `probe` (`git grep probe -- testbed` empty). So the probe cannot leak into a non-test
  feature build and cannot alter `cfg(not(test))` behaviour. See E-05.
- **Manager halt arm has no unit test** (confirmed: `epoch_manager.rs` has no `#[test]` touching
  `engaged`; the journal says the same). Per the brief no test was invented. See E-03.

### Q7 — MUTATION M6 (`send_replace` for `send_if_modified`)

The mutation staying green is **acceptable and I reproduced the reasoning from code**:
`git grep "engaged\." -- crates` yields only `self.engaged.borrow()` (`sync_metrics.rs:610`) and
`self.engaged.subscribe()` (`:623`). There is no `changed()` waiter on the latch; the only two waiters
use `wait_for` (level-triggered: `tokio watch.rs:902-914`) and disarm after the first firing
(`epoch_manager.rs:621`, `dkg_engine.rs:780`). A second `send_replace(true)` only bumps the version and
wakes receivers that nobody polls. So the 0→1-exactly-once property is genuinely unobservable *today*;
it would become observable the moment a `changed()` consumer is added. This is a coverage gap (E-05 /
NIT), not a correctness bug.

### Q8 — HYGIENE

- **No new `#[allow]`**: the only one in `dkg_engine.rs` is `#[allow(clippy::too_many_arguments)]` at
  `:726`, which already existed at HEAD (`git show HEAD:.../dkg_engine.rs`, line 580). The others in
  `actor.rs`/`plane.rs` are pre-existing.
- **No new production `unwrap`/`expect`**: every `expect` added by the diff is in `#[cfg(test)]` code
  (`dkg_engine.rs:2115,2273,2381,2404,2413,2452,2480,2521`; `sync_metrics.rs` test uses `expect` only
  in the marker tests); production `start_one`'s `expect("four routes")` (`dkg_engine.rs:1036-1039`)
  is pre-existing.
- **New `pub`**: `Wiring.epoch_clock` (`actor.rs:818`, required plumbing), and the `#[cfg(test)]`
  `AgreementPlaneConfig.probe` (`dkg_engine.rs:692`) and `LauncherProbe` (`:700`). No new outward `pub`
  in a non-test build.
- **`cfg(test)` field in a production struct** — see E-05 (no production effect).
- **Log levels**: halt/refusal use `warn!` (`dkg_engine.rs:899-902,:975-978`), unchanged in kind.
- **No dead imports**: `AtomicBool` and `Notify` removed from `sync_metrics.rs` (`:23` now
  `AtomicU8, Ordering`; `Ordering` still used at `:351,:360,:375`); `Epoch` removed from `plane.rs`
  (`git diff`); `agreement_partition` re-export removed (`mod.rs:149` region) with all users updated.
- **Stale doc (round-1 residue, not round 2)**: `epoch_manager.rs:1410-1411` still says
  `abort_below(...).await` "does file I/O", but `abort_below` (`:1460-1484`) has no `.await` after the
  sweep moved out in round 1. Also `abort_below` remains `async` with no await. See E-06.

### Q9 — Where this review is weakest (ranked)

1. **No cargo run.** I did not execute the tests/mutations; all scheduling claims rely on reading
   tokio/commonware sources and on the journal's verbatim mutation output. If a gate is actually red,
   this review would not know.
2. **No end-to-end composition.** The interaction "latch engaged by the executor during `start_one`'s
   four `register_dkg_subchannel` awaits, outcome NotAMember, pre-existing instance alive" (E-01) is
   reasoned from code; it was not reproduced with a test or a mutation that forces the
   NotAMember/Failed outcome while engaged.
3. **`select!` starvation nuance.** I argue the halt arm is ready every iteration and therefore fires
   with probability 1; I did not quantify how many iterations a saturated `requests` queue can delay it
   in the deterministic runtime (no per-branch fairness guarantee).
4. **Round-1 vs round-2 attribution.** The working tree is a single diff against HEAD, so for
   `dpos.rs`/`outer.rs`/`stand.rs`/`node/dpos.rs` I relied on the journal's claim that round 2 did not
   touch them; I only checked they are consistent with the round-2 semantics.
5. **Docs.** `.claude/dpos_architecture/*` and `CHANGELOG` are absent from this workspace, so the
   round-2 doc edits (item 5) could not be verified at all (same as round-1 D-11).

---

## 2. Findings

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| E-01 | **SERIOUS** | `beacon/dkg_engine.rs:936-944` (post-check in `Started::Running` only), `:947-951` (NotAMember/Failed no check), `:780-783` (halt arm), `:1021-1035` (four awaits) | The post-spawn latch re-read runs **only** in the `Running` arm. If the latch engages while `start_one` is parked in one of the four `register_dkg_subchannel(...).await` calls (`:1023`) and the outcome is `Started::NotAMember` (`:947`) or `Started::Failed` (`:951`), `start` returns without touching the map, so **pre-existing instances stay alive under the engaged latch** until the `select!` halt arm happens to be chosen. The same holds for the newly spawned instance when the latch engages *after* the `is_engaged()` read at `:942` but before the next `select!`. Because branch selection is random over ready branches (`tokio select.rs:669-741`), a busy `requests`/`clock` arm can win several iterations before the halt arm. | The edge is **not lost**: `halt_edge` is pinned and armed once (`:757-759`) and is `Ready` from the engage onward (`tokio watch.rs:902-914`), so `on_halt` runs on the first iteration the arm wins; the four-await window is closed for the common `Running` outcome. This is a bounded-latency gap, not the indefinite gap D-02 described, and it is strictly better than round 1 (which needed a clock edge). Under the literal BLOCKER rubric ("an instance kept alive under an engaged latch beyond the same task iteration") this is the only candidate; I do not mark it BLOCKER because the arm is provably ready in the same loop. A `biased;` select with the halt arm first, or hoisting the re-check out of the `Running` match, would remove it. | confirmed by code for the placement and the outcome branches; inferred for the scheduling window |
| E-02 | MINOR | `beacon/dkg_engine.rs:2409-2416` vs `runtime/src/utils/handle.rs:73-74`, `runtime/src/utils/supervision.rs:144-167` | The exit half of `the_launcher_task_...` asserts the instance's mux route is freed after `drop(requests); handle.await`, but it would also pass with `Launcher::abort_all` (`dkg_engine.rs:794,:986-991`) removed: the launcher calls `tree.abort()` when its future completes, which aborts every descendant task (the instance), freeing the route. So the test does not pin `abort_all`; the doc's falsifier "the task … exiting with the instance still registered on the mux" is not actually tested. | `abort_all` is documented as a log line only (D-08) and no sweep follows the exit path, so pinning it is not required for correctness; the test still pins task exit and instance death. The route check is the honest observable, just not specific to `abort_all`. | confirmed by code (runtime supervision cascade) |
| E-03 | MINOR | `epoch_manager.rs:601-603,:613-635` | The round-2 change to the manager (fresh `notified()` per loop → one pinned `wait_for` + `halt_seen`) has no unit test; the two `select!`-loop hazards (re-poll panic, hot loop) are only covered indirectly by the stand tests. The journal admits this. | The stand tests would freeze in virtual time on a hot loop and panic on a re-poll, so the property has live coverage; the brief explicitly said not to invent a manager test. | confirmed by code (no test); inferred for stand coverage |
| E-04 | NIT | `sync_metrics.rs:609-611` | `is_engaged` changed from a lock-free `AtomicBool::load` to `watch::Sender::borrow()` (a read lock on the watch value). Production readers include the executor per dispatch (`executor.rs:1366,:1666,:1844`) and now the launcher per spawn (`dkg_engine.rs:898,:942`). This is a behaviour/performance change outside the named edge fix (uncontended read lock vs. plain load). | No `Ref` is held across an await (`:609-611`; `engaged_edge` discards its `Ref` at `:625`), the write path takes the lock only once per process, and the journal measured nothing but reasons it is uncontended. Not a deadlock or a data-race. | confirmed by code |
| E-05 | NIT | `beacon/dkg_engine.rs:691-692,:698-703,:747-750,:785-792`; `beacon/plane.rs:919-920`; `lib.rs:73-74` | A `#[cfg(test)]` `probe` field plus a `#[cfg(test)]` publication block live inside the production launcher task (and a `cfg(test)` field on the public `AgreementPlaneConfig`). In test builds the loop takes an extra `if let Some(...)` per event. | All references are `#[cfg(test)]`, production passes `None`, and `testbed` is `#[cfg(test)]`; `cfg(not(test))` code is byte-identical to round 1 modulo the fix. No leak into the `dpos-devnet-byzantine` feature build (only `byzantine` is gated on it, `lib.rs:45-46`). | confirmed by code |
| E-06 | NIT | `epoch_manager.rs:1460-1484`; `:1410-1411` | Round-1 residue (not round 2): `abort_below` is still `async` but contains no `.await` after its sweep call was removed in round 1, and the doc at `:1410-1411` still asserts the awaited `abort_below` "does file I/O", which is no longer true. | Clippy default does not flag `unused_async`; callers still `.await`; the substantive point (the tip can move before the liveness gate) may still hold through other awaits, so this is a stale comment, not a code defect. Flagged because it is a `file:line` claim that no longer matches the code. | confirmed by code |
| E-07 | NIT | `beacon/dkg_engine.rs:780-783,:973-982` | `on_halt` aborts and joins every instance inline inside the `select!` handler, so while joining, the launcher processes no request and no clock edge (and the halt arm itself is blocked). This extends the accepted D-09 pattern to the halt path. | No deadlock found (the instance awaits nothing the launcher owns; `abort()` cancels it at its next await; `out`/`pinned` go to the actor, routes to the muxers). A halt is terminal, so blocking the request arm during the join has no contractual cost. | confirmed by code for the absence of a deadlock path; inferred that joins are prompt |
| E-08 | NIT | `beacon/dkg_engine.rs:942-944` vs `:780-783` | The post-spawn `on_halt()` does not set `halt_seen`, so the pinned halt arm still fires on a later iteration and calls `on_halt()` a second time on the (now empty) map; in the probe tests this shows up as an extra event (`events` counts it). | `on_halt` is idempotent (`mem::take`, `:974`), and the second firing is exactly what disarms the arm, so the extra call is required to stop the arm from being re-armed; it is not an extra abort. | confirmed by code |
| E-09 | NIT | `sync_metrics.rs:854-887` | The new latch test polls futures manually with `futures::task::noop_waker()` (`:861-862`), so it depends on tokio's cooperative budget not being exhausted inside `wait_for`/`changed_impl` (`tokio watch.rs:893,:983-999`); a depleted budget would return `Pending` and fail the `is_ready()` asserts even though the channel is correct. | The task is fresh and the test performs at most ~6 cooperative polls against a budget of 128 (`runtime/src/task/coop/...`), and `Coop` restores budget on every `Ready` (`:449-451`), so this cannot trip here; it is a fragility note, not a failure. | inferred |
| E-10 | NIT | `beacon/dkg_engine.rs:2436-2494` (and `:2504-2543`) | The hot-loop failure mode of the disarm is a **real-time hang** (journal M5′: "has been running for over 60 seconds", killed by `timeout 240`), not a bounded assertion failure, because the virtual-time runner cannot advance while the launcher is always runnable. Neither test has a per-test real-time bound, so a regression of this shape would hang CI rather than fail. | The committed code always disarms (`:780`), so this is a test-observability limitation; M5 (same pinned future, no disarm) does panic in bounded time, so only the recreate-per-iteration shape hangs. | confirmed by the test structure and the journal's M5′; inferred for CI behaviour |

### On the deferred round-1 BLOCKERs (not re-raised)

Round 2 did **not** make D-01/D-06/D-12/D-13 worse: the launcher's pruning cutoff is still the actor's
`now` (`dkg_engine.rs:957-966`, `actor.rs:2347-2354`), the cert-inlet tee is untouched, and the halt
path no longer touches the clock at all. D-01 remains the open BLOCKER it was in round 1 and is
correctly recorded as Д-5.4Б-3. D-03 is fixed (`dkg_engine.rs:889-890` on every outcome); D-05 is
fixed by the four real-task tests; D-07 disappeared with `AgreementClock`; D-04/D-08/D-09/D-10 stand.

---

## 3. Leave as is

- The `SafetyHalt` rewrite itself: one `watch::Sender<bool>`, `send_if_modified` on 0→1, `borrow()` for
  the bit, `subscribe()+wait_for` for a multicast, late-joinable edge (`sync_metrics.rs:442-548,:609-626`).
  It preserves every HEAD property (Q1) and is the correct primitive for two independent waiters.
- The one-shot pattern in both loops: arm once before the loop, `tokio::pin!`, `if !halt_seen`
  (`epoch_manager.rs:601-603,:621`; `dkg_engine.rs:757-759,:780`). This is the only shape that both
  avoids the re-poll panic and avoids the hot loop; verified against tokio's `select!` semantics.
- `Launcher::on_halt` abort-then-join with `mem::take` (`dkg_engine.rs:973-982`), and the ordering
  `started.insert` → `instances.insert` → latch re-read in `start` (`:926-944`).
- `started.retain(floor)` on every outcome (`:889-890`) — fixes D-03 without changing dedup.
- `on_tick` reduced to pruning only (`:957-966`): aborting on the clock edge was D-02 and is replaced by
  the latch edge.
- The actor clock as `watch<u64>` published with `send_if_modified` at the end of `on_height`
  (`actor.rs:2347-2354`) and the removed actor latch plumbing; the renamed test is a correct pin.
- Exit paths: `break` + `abort_all` on closed channels (`dkg_engine.rs:762-775,:794`) and runtime
  supervision as the backstop (`runtime/src/utils/handle.rs:73-74`).
- Keeping `is_engaged` on `borrow()` rather than a second `AtomicBool`: a duplicated bit could diverge
  from the edge; the read-lock cost is the honest price (E-04 is informational).
- No new `#[allow]`, no new production `unwrap`/`expect`, no dead imports.

---

## 4. Verification limits

- Cargo was not run (brief), so the gate table (732/0, 58/0, clippy, fmt, doc, harness) is taken on
  trust; this review makes no independent claim about it.
- The E-01 interleaving (engage during `start_one` + `NotAMember`/`Failed`) was not reproduced by a
  test or mutation; it is derived from the code paths at `dkg_engine.rs:936-951` and tokio's
  random-order `select!`.
- `.claude/dpos_architecture/*` and `CHANGELOG` are not present in this workspace, so the round-2 doc
  edits (brief item 5) could not be verified; the only doc evidence available is the code comment
  updates, which were read but are not treated as evidence.
- `E-02`/`E-07` rely on commonware's supervision implementation in the git checkout; I read
  `handle.rs:38-90` and `supervision.rs:100-215` directly.
