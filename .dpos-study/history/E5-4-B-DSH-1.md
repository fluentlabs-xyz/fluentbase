# Independent review, round 1 — DKG agreement instances + journal partitions move into the beacon launcher

Scope: exactly `git diff HEAD` (9 files, `+995/−686`), base `79dd18f9`. Read-only review; nothing
fixed. All claims carry `file:line` opened in this session. Confidence: `confirmed by code` = the
cited lines were read and the inference is direct; `inferred` = one causal step was reasoned from
comments/design rather than executed.

Inputs used: `dsh-input-task.md`, `dsh-input-journal.md`, the working tree, `git show HEAD:…`, and
the vendored commonware runtime checkout. No cargo was run (per instruction).

---

## 1. Answers to the ordered questions

### 1.1 SWEEP TIMING (Q1) — rebuilt table

Feeder audit for the actor's `height_now` (the clock the launcher's cutoff is taken from):

| Feeder | Where | Value | Relation to HEAD's `tip` |
|---|---|---|---|
| EL-finalized poller | `crates/node/src/dpos.rs:1510`, `:1539-1544` | `fin + K` (startup seed `cs_fin_num + K`, `crates/node/src/dpos.rs:1356-1361`) | `fin = ordering − K` (`crates/dpos/consensus/src/executor.rs:283-285`), so `fin + K ≤ tip` |
| marshal ordering tip | `crates/dpos/consensus/src/application.rs:1072-1085` (`dkg_height_tx.try_send(height.get())`) | exactly the epoch manager's `ordering_tip` (`application.rs:1080`) | equal to `tip` (lossy channel, so it can only lag) |
| cert-inlet live-frontier tee | `crates/dpos/consensus/src/cert_inlet.rs:781-785` | `uf.block.height` of a verified upstream cert | **sent BEFORE `marshal.verify_block`/`report_finalization`** (`cert_inlet.rs:803-814`), so it can lead `tip` |

Actor side: `height = self.height_now().max(height)` (`crates/dpos/consensus/src/beacon/actor.rs:2234`),
`now = self.epoch_of(height)` (`actor.rs:2242`), `epoch_of = epocher.containing(...).map_or(0, …)`
(`actor.rs:1177-1181`), and the cutoff is published as `AgreementClock { epoch: now, … }`
(`actor.rs:2364-2375`).

HEAD side: `live_epoch_of(geometry, tip) = epoch_of(tip)` **plus** the `last(e) == tip ⇒ e + 1` arm
(`crates/dpos/consensus/src/epoch_manager.rs:1746-1751`), and `prune_agreements(cutoff)` was called
from `abort_below(current)` (`epoch_manager.rs:1452-1465` before the change; call removed in this
diff).

Rebuilt table (A = activation, L = interval):

| cutoff E | HEAD first sweep of `[E−8, E)` | new first `on_tick(E)` (tip/poller feeders only) | Δ |
|---|---|---|---|
| `0` | first reconcile; band `[0,0)` empty | first tick with `now=0` publishes nothing (`Default` = `{0,false}`, `dkg_engine.rs:757-761`, `actor.rs:2368-2375`); band empty either way | equal |
| `E ≥ 1` | ordering tip `last(E−1) = A + E·L − 1` (the `+1` arm) | actor height `first(E) = A + E·L = last(E−1)+1` | **+1 height, later** |
| `E ≥ 1`, cert-inlet tee live | `last(E−1)` | can be reached at a `tip ≤ last(E−2)` when a boundary-crossing cert was skipped | **earlier by ≥1 epoch** |

**Can the actor's `now` be AHEAD of HEAD's `live_epoch_of(tip)`? Yes — via the tee.** The tee fires
before the marshal sees the cert (`cert_inlet.rs:781-785` then `:803-814`), and certs are *skipped*
before the tee without a marshal report on the committee-not-committed arm
(`cert_inlet.rs:647-668`), the BLS-verify-fail arm (`:678-704`) and the σ-refused arm (`:733-746`).
Concretely: if cert `H = last(E−1)` is skipped and the next teed cert is `first(E)`, the marshal tip
is still `H−1 = last(E−1)−1`, whose `live_epoch_of` is `E−1`, while the actor's `now` is `E`. A
one-height lead is normally absorbed by the `+1` arm, but a *skipped* cert removes that height from
the manager's tip, so the actor's epoch leads by one. A deep catch-up (the stated purpose of the tee:
`cert_inlet.rs:258-261`, `crates/node/src/dpos.rs:764-771`) makes the lead larger. Then
`prune_agreements` at `dkg_engine.rs:918-925` sweeps band `[E−8, E)` (`dkg_engine.rs:363`) while HEAD
would still be at `[E−9, E−1)`: the partition for epoch `E−1` is swept one epoch earlier than HEAD.
This is D-01.

The journal's §0.3 calls the tee "only on the deprecated `--dpos.follower-upstream`". That is wrong
for this tree: the flag also arms the cert-inlet on a **validator** (`bins/fluent/src/node_modes.rs:89-94`,
`is_validator: true`), and the tee is wired there (`crates/node/src/dpos.rs:772-787`); the type doc
says `Some` explicitly for "a validator-with-upstream" (`cert_inlet.rs:301-304`). The flag is
described as "NO LONGER REQUIRED … set it only for an explicit WS path", i.e. optional but supported
(`crates/node/src/dpos.rs:248-259`), not removed. This is D-06.

**Is `epoch_of` on a height above the frozen geometry ever a larger epoch than the manager's?** No.
The actor's `OriginEpocher` is built from the frozen `(activation, interval)`
(`actor.rs:1107-1110`, from the same `geometry` watch, `beacon/plane.rs:847-857`) and uses the shared
`epoch_at_block` (`crates/dpos/consensus/src/epocher.rs:64-77`); the manager's `Geometry::epoch_of`
calls the same function (`crates/dpos/consensus/src/committee/mod.rs:575-580`). On overflow both
answer `0` (`OriginEpocher::bounds` `checked_*` → `None` → `map_or(0)`; `unwrap_or(0)`). For any
height the actor's epoch is `≤` the manager's live epoch.

### 1.2 HALT (Q2)

- **Latch engaged before the plane is built (`SafetyHalt::restoring` from a marker).** No path spawns:
  `Launcher::on_request` reads `self.safety_halt.is_engaged()` **before** the committee read and
  before `start_one`, and settles the target (`dkg_engine.rs:848-857`); the only spawn site is
  `start_one` → `spawn_agreement` (`dkg_engine.rs:866-887`, `:985-1013`). `restoring` engages the
  latch synchronously in the constructor (`crates/dpos/consensus/src/sync_metrics.rs:474-484`,
  `:536-541`), and the node builds it before `beacon::build`
  (`crates/node/src/dpos.rs:1460-1465` vs `:1891`). Confirmed.
  - **Window:** the latch can also flip *during* `start_one`'s four `register_dkg_subchannel(...).await`
    calls (`dkg_engine.rs:965-983`), which happen before `spawn_agreement`. An instance then starts
    under an engaged latch; only the next clock edge can abort it. HEAD covered this exact case in
    the intake latch check (the manager aborted an instance that arrived after the notify edge). This
    is folded into D-02.
- **Is the actor's `halted` edge guaranteed to be published?** **No.** The only writer of the watch is
  `send_if_modified` at `actor.rs:2368`, reached only from `on_height`, which is driven solely by the
  `heights.recv()` arm (`actor.rs:1257-1259`). After a halt, the manager aborts the per-epoch engines
  (`epoch_manager.rs:614-619`), so on a plane-native validator there is no further marshal ordering
  tip, the `dkg_height_tx` tip feeder stops (`application.rs:1081-1085`), the poller stops (its `fin`
  is execution-finalized, `node/dpos.rs:1516-1545`), and the actor never ticks again → the launcher's
  `on_tick` (`dkg_engine.rs:905-917`) never runs. Only a validator **with an upstream** keeps ticking,
  via the tee. This is D-02. (The journal's §0.4 claim "the ordering tip is a feeder independent of
  execution" holds only while the ordering engine runs; the halt arm aborts it.)
- **Does `on_height` reach `send_if_modified` on every tick?** Yes. A scan of `actor.rs:2227-2417`
  finds no `return` and no `?`; `send_if_modified` at `:2368` is the last action before step 3.
  Confirmed.
- **`engaged_edge`/`notify_one` still consumed by the same waiter?** Yes. The only production
  `engaged_edge` waiter is `epoch_manager.rs:604` (`git grep engaged_edge -- crates`). The actor and
  the launcher use `is_engaged()` only (`actor.rs:2366`, `dkg_engine.rs:850`, `:905`). Confirmed.
- **Follower (Д-5.4Б-2).** No halt behaviour is lost. `build_follower` never had an agreement plane
  (HEAD passed `agreement_intake: None`, `git show HEAD:.../dpos.rs:4117`); the follower's own latch is
  built by `launch_follower` from `FollowerLayerConfig.halt_marker`
  (`crates/dpos/consensus/src/dpos.rs:3155`, `:3317-3319`; set at
  `crates/node/src/cert_follow/mod.rs:227`), untouched by this diff. Confirmed.

### 1.3 OWNERSHIP (Q3)

- **Instance outside `Launcher.instances`?** No observable moment. `on_request` calls
  `start_one(...).await` (`dkg_engine.rs:866`); `start_one` awaits the four mux registrations
  (`:967`) and then calls `spawn_agreement`, which is a **synchronous** `fn` returning
  `Result<Handle<()>>` (`:395`, spawn at `:436-438`); after that there is no `.await` before
  `self.instances.insert(...)` (`:881`). The whole spawn→insert path therefore runs in one poll, and
  `on_tick`/`on_request` share the launcher task, so no sweep can interleave. Confirmed.
- **On launcher exit are instances aborted?** `abort_all` at `dkg_engine.rs:930-935` on the normal
  exit (`:747`), and commonware runtime supervision on an external abort: "When a parent task
  finishes or is aborted, all its descendants are aborted"
  (`~/.cargo/git/checkouts/monorepo-…/runtime/src/lib.rs:189`), which is why `spawn_agreement` uses
  the launcher's own `ctx` (`dkg_engine.rs:436-438`). Confirmed.
- **On actor exit does the launcher exit, and which sender drops first?** Both senders are `DkgActor`
  fields in declaration order `agreement_tx` (1005), `epoch_clock` (1007), `safety_halt` (1009);
  Rust drops fields in declaration order, so `requests` closes first and the launcher breaks on
  `None` (`dkg_engine.rs:731-735`); if the clock sender dropped first the launcher breaks on
  `Err` (`:736-744`). Either sender suffices; neither can be dropped while the other "holds the task
  alive". Confirmed.

### 1.4 `on_tick` AFTER A HALT (Q4)

`std::mem::take(&mut self.instances)` (`dkg_engine.rs:909`) empties the map before
`prune_agreements` (`:918`); therefore `aborted_one` is `false` (`:341-343`) and the band sweep runs
iff `cutoff > *swept_to` (`:360-362`). So **yes**, the band below the cutoff is still swept whenever
the cutoff moves, and `*swept_to = (*swept_to).max(cutoff)` (`:385`) stays monotone. Caveat: at a
halt edge whose cutoff equals `swept_to`, a partition recreated by a halt-aborted instance below the
cutoff is not re-collected until the cutoff next moves (D-04); HEAD's halt arm + next same-cutoff
prune had the same gate, so this is not a regression.

### 1.5 PARTITIONS (Q5)

`fn agreement_partition(prefix, epoch) = format!("{prefix}{AGREEMENT_JOURNAL_PARTITION_PREFIX}{epoch}")`
(`dkg_engine.rs:281-283`) with `AGREEMENT_JOURNAL_PARTITION_PREFIX = "dkg_epoch_"` (`dpos.rs:230`)
is byte-identical to HEAD's `format!("{prefix}dkg_epoch_{target_epoch}")`
(`git show HEAD:.../dkg_engine.rs:260-262`). The pre-existing test
`partition_is_disjoint_from_the_ordering_plane` (`dkg_engine.rs:1238-1254`) still pins
`dkg_epoch_7` / `node2-dkg_epoch_7`. `agreement_partition` is now private and has no reader outside
`beacon/` (`git grep agreement_partition -- crates` → only `beacon/dkg_engine.rs`; the removed
`pub(crate) use` at `git show HEAD:.../beacon/mod.rs:149`). Every `dkg_epoch_` occurrence is the
constant, the function, or a doc comment (`dpos.rs:230`, `dkg_engine.rs:292`, `:1242`, `:1249`,
`:1318`, `plane.rs:556`). Confirmed.

### 1.6 PLUMBING (Q6)

- No live leftover of `agreement_intake`, `dkg_agreements`, `with_agreement_intake`,
  `recv_agreement` anywhere under `crates/` (grep); the only remaining `agreement_intake` strings are
  in `.dpos-study` history docs, outside this change.
- `Tasks` is exactly `{supervised, drain}` (`beacon/plane.rs:174-186`), built at `:1006-1017` and
  `:1160`.
- `SyncMetrics::register` count: exactly one per process path. Validator:
  `crates/node/src/dpos.rs:1460-1461` (inside `build_beacon_plane`, before `beacon::build` at `:1891`);
  the layer's old registration is removed (`git show HEAD:.../dpos.rs:2121-2124`). Follower:
  `crates/dpos/consensus/src/dpos.rs:3312-3313` (a disjoint process path — the follower overlay never
  calls `build_beacon_plane`, `crates/node/src/dpos.rs:488-507`). No third site
  (`git grep SyncMetrics crates/node/src/dpos.rs` → one).
- `SharedBeaconPlane.{sync_metrics, safety_halt}` have one constructor
  (`crates/node/src/dpos.rs:1973-1986`), and `beacon::build` receives `safety_halt.clone()` at
  `:1926`; the same latch is destructured in `DposLayer::launch`
  (`crates/dpos/consensus/src/dpos.rs:2125-2126`) and reaches the executor, the manager and the
  OuterEngine supervisor (`:2977-2978`, `outer.rs:434`, `:953`, `:1052`, `:1087`) and the launcher
  (`plane.rs:839`, `:930`). Confirmed.

### 1.7 TESTS (Q7)

- Three moved tests are cell-for-cell modulo imports and the documented word changes. A mechanical
  extraction-and-diff against `git show HEAD:.../epoch_manager.rs` shows only: the `use` lines (now
  supplied by the module), `"the frontier's own instance"` → `"the cutoff epoch's own instance"`
  (`dkg_engine.rs:1303`), the two comment words `frontier's` → `cutoff epoch's`
  (`dkg_engine.rs:1326-1327`), and `abort_below runs per finalized block` → `the launcher prunes per
  clock tick` (`dkg_engine.rs:1402-1403`, `:1416`). No logic cells changed. Confirmed.
- `the_agreement_clock_moves_on_the_epoch_edge_and_on_the_halt_and_never_per_tick`
  (`actor.rs:7956`) really drives two ticks inside one epoch (`:7987-7988`) and uses
  `has_changed()` (not value equality), so an unconditional `send_replace` would fail; it then pins
  the halt edge (`:7998-8008`) and no re-publication (`:8012-8016`). A broken epoch edge, a missing
  latch read, or a per-tick publish all redden it. Confirmed.
- `a_halted_node_starts_no_agreement_instance_and_aborts_the_ones_it_has`
  (`dkg_engine.rs:2061`) pins the refusal before the committee read (`reads` counter) and the abort
  on `on_tick` (the pre-halt `on_tick(TARGET-1)` would leave epoch `TARGET` in the map because
  `TARGET > cutoff`). Removing either the latch check or the halt abort reddens it (journal §0.6 M2).
  Confirmed.
- `a_repeat_prune_at_a_cutoff_already_swept_does_not_touch_storage`
  (`dkg_engine.rs:1376`) reddens when the memo gate is removed (§0.6 M3). Confirmed.
- **Gap:** the rewritten `the_launcher_starts_one_instance_per_target_and_retries_an_unreadable_committee`
  (`dkg_engine.rs:1985`) and the halt test now instantiate `Launcher` directly and call
  `on_request`/`on_tick`; nothing exercises `spawn_agreement_launcher`'s `select!` loop, its
  break-on-`None`/`Err`, or `abort_all` on exit (`dkg_engine.rs:727-748`). The HEAD version did
  exercise the task via the `adopted` receiver. This is D-05.

### 1.8 HYGIENE (Q8)

- No new `#[allow]` (diff grep); the existing `#[allow(clippy::too_many_arguments)]` is on
  `spawn_agreement_launcher` (`dkg_engine.rs:707`).
- No **new** `unwrap`/`expect`/`panic!` on a production path: every added hit is inside `#[test]`
  code (`actor.rs:7978, 7990, 7999, 8002, 8014`; `dkg_engine.rs:1331, 1386, 1398, 1420, 2114`).
  The pre-existing production `expect("four routes")` at `dkg_engine.rs:980-983` is untouched.
- New `pub`: `pub struct AgreementClock` (`dkg_engine.rs:758`, in a **private** `mod dkg_engine`,
  `beacon/mod.rs:94`, not re-exported) and `pub(crate) const AGREEMENT_JOURNAL_PARTITION_PREFIX`
  (`dpos.rs:230`). Neither is a new external API.
- `Default for AgreementClock` = `{epoch: 0, halted: false}` (`dkg_engine.rs:757-761`): harmless for
  the sweep (`cutoff 0` ⇒ empty band, `dkg_engine.rs:363`; `swept_to` starts 0, `:837`), and the
  first non-zero cutoff publishes because it differs. Confirmed.
- Log levels: no existing `info!`/`warn!` was downgraded; the new warns are on refusal and halt
  abort (`dkg_engine.rs:851`, `:910`).

### 1.9 WHERE THIS REVIEW IS WEAKEST (Q9)

1. D-01's reachability depends on the exact interleaving of the cert-inlet task, the actor and the
   marshal report; I read all three but did not run the stand.
2. D-02's conclusion that the halt arm stops the ordering tip is inferred from
   `epoch_manager.rs:614-619` plus the marshal being the only tip source; I did not run a halted
   plane-native node.
3. The launcher's `select!` loop (coalescing, break paths, `abort_all`) is reasoned statically; the
   new tests bypass it (D-05).
4. The join-inline liveness of D-09 is inferred (an aborted instance should stop at its next await).
5. The doc/CHANGELOG deliverables of the brief are not present in this checkout, so I could not
   verify them (D-11).

---

## 2. Findings

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| D-01 | **BLOCKER** | `beacon/actor.rs:2364-2375`, `beacon/dkg_engine.rs:904-925`, `:363`; `cert_inlet.rs:781-785`, `:803-814`, `:647-668`, `:678-704`, `:733-746`; `epoch_manager.rs:1746-1751` | The launcher's sweep cutoff is the actor's `now`, which the cert-inlet tee can push ahead of HEAD's `live_epoch_of(tip)`: the tee is sent before the marshal report and skipped certs are never teed, so a boundary-crossing skip makes `epoch_of(height_now) > live_epoch_of(tip)`. `prune_agreements` then sweeps `[E−8, E)` where HEAD's next cutoff would still be `E−1`, i.e. epoch `E−1`'s partition is removed earlier than HEAD would remove it. | The `+1` arm absorbs a one-height lead when every cert is reported, so the earlier sweep needs a skipped/deferred cert or a lagging marshal; the harm is a journal for an epoch the local tip has not passed (restart/resume durability), and the abort pass still precedes the sweep for any live instance. The criterion nevertheless says "partition swept earlier than HEAD" = BLOCKER. | confirmed by code (ordering + skip paths); inferred that the resulting lead outlives the `+1` cushion in a real catch-up |
| D-02 | **BLOCKER** | `beacon/dkg_engine.rs:905-917`, `:848-857`, `:965-983`; `beacon/actor.rs:1257-1259`, `:2364-2375`; `epoch_manager.rs:614-619`; `application.rs:1081-1085`; `cert_inlet.rs:781-785`; `node/dpos.rs:1516-1545` | The halt abort runs only on a clock edge, and the edge is published only from `on_height`, whose only driver is `heights.recv()`. After the halt the manager aborts the per-epoch engines, so on a plane-native validator there are no further marshal tips (and the execution-finalized poller stops too): the actor stops ticking, `send_if_modified` never runs, and the already-running agreement instance keeps voting under the engaged latch indefinitely. A latch flip during `start_one`'s four awaits has the same exposure. HEAD aborted the instances on the manager's `engaged_edge`, independent of heights. | If the ordering tip keeps advancing after the halt (a validator with `--dpos.follower-upstream`, whose tee keeps feeding heights), the edge arrives within one tick and the abort happens; the journal claims this is the case. On the default plane-native validator the engine abort removes the tip source, so the guarantee fails. | confirmed by code for the dependency chain; inferred for "engine abort ⇒ no more tips" |
| D-03 | MINOR | `beacon/dkg_engine.rs:850-857` vs `:896-899` | The halt-refusal path returns immediately after `started.insert`, so the `started.retain(floor)` that bounds the dedup set is skipped for every refused request. On a long-halted process the set grows one entry per announced epoch without bound. | The actor only announces epochs inside its retention window, so growth is one small entry per epoch (negligible in practice); the latch is permanent, so no correctness effect. | confirmed by code |
| D-04 | MINOR | `beacon/dkg_engine.rs:909-925`, `:341-343`, `:360-362` | The halt path `mem::take`s the instance map before `prune_agreements`, so `aborted_one` is false; when the halt edge does not move the cutoff (`cutoff == swept_to`), a partition recreated by a halt-aborted instance below the cutoff is not reclaimed until the cutoff next advances. | Not a regression: HEAD's manager halt arm also emptied `dkg_agreements` and the next same-cutoff prune also short-circuited on the same gate, and the partition is swept on the next cutoff edge. | confirmed by code |
| D-05 | MINOR | `beacon/dkg_engine.rs:1985`, `:2061`; `:727-748` | The rewritten launcher test and the new halt test instantiate `Launcher` and call `on_request`/`on_tick` directly. Nothing anymore exercises `spawn_agreement_launcher`: the `select!` over `requests`/`clock`, the break on `None`/`Err`, watch coalescing, and `abort_all` on exit are untested. HEAD's version drove the real task through the `adopted` receiver. | The loop is four lines and delegates wholly to the tested methods; the risk is regression in the break/exit paths, not in the moved logic. | confirmed by code |
| D-06 | MODERATE | `cert_inlet.rs:254-261`, `:301-304`; `node/dpos.rs:248-259`, `:772-787`; `bins/fluent/src/node_modes.rs:89-94` | The journal's §0.3 premise that the tee is "only on the deprecated `--dpos.follower-upstream`" is false: the flag also arms the cert-inlet on a validator (`is_validator: true`), and the tee is wired there. This premise is what hides D-01. | The flag is documented as no longer required and "set it only for an explicit WS path", so deployments may not use it; but it is supported production wiring, not deprecated. | confirmed by code |
| D-07 | NIT | `beacon/dkg_engine.rs:758-761`, `:905`; `beacon/actor.rs:2366` | `AgreementClock::halted` is never read as a decision input in production; the launcher re-reads the latch, so the field's only production role is making the watch value differ so `send_if_modified` fires on a flip. | That is exactly its documented job (the change token for the one-shot latch edge), and the actor test pins it; a launcher read of `clock.halted` would be worse (stale by the time it is processed). | confirmed by code |
| D-08 | NIT | `beacon/dkg_engine.rs:930-935` | `abort_all` aborts instances but does not `join` them, unlike `prune_agreements` (`:356`) and the halt path (`:915`); the comment says supervision covers it. | No sweep follows `abort_all`, so the join has no purpose there, and the runtime aborts descendants when the launcher task ends (`runtime/src/lib.rs:189`). | confirmed by code |
| D-09 | MINOR | `beacon/dkg_engine.rs:905-917`, `:918-925` | `on_tick` aborts **and joins up to every live instance inline** in the launcher's `select!` task; while joining, the launcher cannot process requests or further clock edges. `prune_agreements` already joined before sweeping, but the halt path is new and can touch future-epoch instances too. | `abort()` cancels at the next await and the run loop is async, so each join should be prompt; the same join-inline pattern already existed in the moved prune path. | inferred |
| D-10 | NIT | `beacon/plane.rs:556` | The re-pointed doc link `[`crate::dpos::AGREEMENT_JOURNAL_PARTITION_PREFIX`]` names a `pub(crate)` item (private to rustdoc), so it is an unresolved/private-link warning; the journal records the count as held at 6 by a 1:1 swap. | Pre-existing style (the old link named the then-`pub(crate)` re-export too); it is within the gate tolerance. | confirmed by code |
| D-11 | MINOR | `git diff HEAD --stat` (9 code files); no `.claude/`, no `CHANGELOG` in this checkout | The brief's item 6 (docs in `.claude/dpos_architecture/{03,08,13,00}`, CHANGELOG) is not part of the change; `git grep agreement_intake\|dkg_agreements\|prune_agreements` still hits `.dpos-study` history/PLAN. | The target doc paths do not exist in this workspace, so the omission may be environmental rather than an implementer error; `.dpos-study` is historical/plan material, not architecture docs. | confirmed that the paths are absent; inferred that the brief intended them |
| D-12 | NIT | `beacon/dkg_engine.rs:360-386`; no test references a HEAD-equivalent cutoff | No test pins the sweep-timing property ("the new cutoff is never earlier than HEAD's `live_epoch_of(tip)`"); §0.3 is prose, and it is exactly the property D-01 breaks. | The task asked for a proof table, not a test, and the property spans two subsystems (manager and actor) that no in-crate unit can compose cheaply. | confirmed by code |
| D-13 | MINOR | `beacon/actor.rs:2234`, `:2364-2375`; `node/dpos.rs:1510`, `:1539-1545`; `cert_inlet.rs:782` | In the other direction, the new cutoff is a running max over *lossy* `try_send` feeders (`fin + K` and the tip feeder), so it can lag the manager's lossless `ordering_tip`; instances/partitions that HEAD would have pruned at a given tip can be retired later (or, under sustained drops, not until the actor catches up). | This is the safe direction per §0.3 ("not earlier"), the actor's `now` is monotone, and it also drives the actor's own deals, so the launcher stays consistent with the actor; it is a consequence of the named cutoff move, not an extra behaviour. | confirmed by code for the lossy feeders; inferred for the practical lag |

---

## 3. Leave as is

- `prune_agreements` body: abort → join → remove, `aborted_one` taken before the loop, the
  `contains_key` live-skip, silent `PartitionMissing`, edge-driven memo `*= max`. The extracted body
  differs from HEAD only in two comment words (`spawn_agreement` for `run_agreement`, `started` for
  `adopted`), and the mutation history (M1 green, M1′ green, M1+M1′ red) shows the live-partition
  property is held by the band boundary alone. Leave it as the brief demands ("move as is").
- `AGREEMENT_SWEEP_SPAN = SCHEME_RETENTION_EPOCHS` and the band `cutoff − span .. cutoff`
  (`dkg_engine.rs:312`, `:363`).
- `#[allow(clippy::too_many_arguments)]` on `spawn_agreement_launcher` — pre-existing, the argument
  count merely grew 6 → 7.
- `AgreementClock::default()` epoch 0 — harmless (empty band, `swept_to = 0`).
- The followership latch (`FollowerLayerConfig.halt_marker`, `launch_follower`) untouched; Д-5.4Б-2
  loses nothing (the follower never ran agreement instances).
- The `SafetyHalt` latch ownership model: one instance built in the node before `beacon::build`,
  carried in `SharedBeaconPlane`, cloned into the beacon, the manager, the executor and the Outer
  supervisor.
- `partitions` byte-identity and the `partition_is_disjoint_from_the_ordering_plane` test.
- The follower `build_follower`/`FollowerInputs` path and its stub removal.
- `started.retain(floor)` semantics for the normal (non-halted) path.
- The unreachable live-skip guard — left in place per "move as is"; the journal's own §0.8.1 ranks
  this as its weakest point, and the review agrees the redundancy is deliberate.

---

## 4. Verification limits

No cargo was run (per instruction). Byte-identity of moved test bodies and of `prune_agreements` was
checked mechanically by extracting functions from `git show HEAD` and from the working tree and
diffing. All other findings are from reading the cited lines in this session.
