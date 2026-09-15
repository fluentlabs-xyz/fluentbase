# Round-2 review — focus A (store ownership, window, transitions, signals)

Object: exactly `git diff HEAD` in `/tmp/dsh-ws-e5-3a1-review-2a` (`crates/dpos/consensus/src/beacon/{actor,
artifact,ceremony,dkg_agree,dkg_engine,dkg_oracle,log_resolver,log_store,metrics,plane,share_state}.rs`,
`testbed/tests.rs`, `node/src/dpos.rs`; 13 files, +4312/−1436). HEAD `60d6c3e3`.

Method / limits. Reading review only; **no `cargo`, no test, no stand was run** (the brief forbids it and there is no
build cache). Every claim is grounded in a `file:line` I opened in the current worktree; the journal's gate/mutation
figures are taken as *reported*, never reproduced. Production region of `actor.rs` is `:1-3973` (`#[cfg(test)] mod
clock_tests` starts `:3975`). All paths below are relative to `crates/dpos/consensus/src/beacon/` unless stated.
Confidence is `confirmed by code` (directly visible) or `inferred` (mechanism read, reachability argued). Severity
scale: BLOCKER / SERIOUS / MODERATE / MINOR / NIT. I did not run `cargo`, so any "green" statement is the journal's,
not mine.

Ambiguity / conservative assumption. Where the task says "BLOCKER = … a `Conflict` that a restart forgets", I read
`Conflict` as the *verdict the node's facts imply* (including a second certified value the store noted but had not yet
handed to the actor), not only the `EpochState::Conflict` slot. Under that reading EA-01 is a BLOCKER; under the
narrower "only an entered terminal counts" reading it is SERIOUS. I state both in EA-01.

---

## Part A — verdicts for DA-01 … DA-18

| id | verdict | file:lines (current) | why (and, for FIXED, the edit under which the old form hid it) | refutation attempt | confidence |
|---|---|---|---|---|---|
| DA-01 | **FIXED** | `actor.rs:3579-3609` (`reconcile_with_store`), `:3524-3527` (first step of `drive_acquisition`), `:189-212` (`StoredArtifact`/`AgreedOutcomeAt`), `:1563-1569` (`apply_artifact` store cross-check), `:2879-2908` (`recover` store read), `:1743-1761` (`conflict`), `artifact.rs:456,566-597,1652-1666`, `dkg_engine.rs:441-443` | The store is now the owner and the actor reads it: `drive_acquisition` runs `reconcile_with_store()` **before** the pull loop (`:3525` vs `:3558-3560`); `apply_artifact` compares an incoming value with the store's held digest when the actor holds none (`:1563-1569`); `recover` reads `stored()` first (`:2879`); a second certified value is noted (`artifact.rs:566-582` from `deliver` `:1653` and from `dkg_engine.rs:441-443`) and reaches `Conflict` on the tick (`:3599-3606`) or on restart (`:2880-2892`). Old form: `apply_artifact` compared only against the actor-local `held_digest()` (`:1557-1561`) and `drive_acquisition` never read the store (only `recover` did), so a lost `try_send` left the store leading A while the actor adopted a later B as its first artifact — the F-02 key-mismatch. The lost-hand-off heal is directly tested (`actor.rs:12270`, `:12387`). | No in-process path is left that adopts B while the store serves A: every producer inserts before it sends (`artifact.rs:1667` before `:1662/:1673`; `dkg_engine.rs:441` before `:457`), so the store always held the value the actor applied, and `reconcile_with_store` compares each tick. The one residual is the RAM-only `divergent` note across a restart — a *different*, narrower harm (a missed terminal, not a key mismatch) — reported as EA-01. | confirmed by code (mechanism); inferred (reachability of the two-certified-artifact case) |
| DA-02 | **FIXED** | `artifact.rs:328-338` (`value_digest`), `actor.rs:266` (`AgreedSet::of`), `:1543,1564`, `:2882,2932,2946`, `:3591,3600`; old key `dkg_agree.rs:455-461` (`DkgProposal::digest`, covers `confirms`) | Identity is now `value_digest = keccak(epoch‖len‖(idx‖hash)*‖encode_outcome(group_key))`, which omits `confirms`; all `Conflict` comparisons and the bridge's divergence note use it (`actor.rs:1543,1564,2882,2932,2946,3591,3600`; `artifact.rs:570,1656-1657`). Old form: `AgreedSet::digest`/`apply_artifact`/`recover` keyed on `proposal.digest()`, which encodes `confirms` (`dkg_agree.rs:455-461`), so two certificates over one value read as two. Direct test `actor.rs:12451` asserts `digest()` differs while `value_digest` agrees and the epoch stays `keyed`, counter 0. | `value_digest` is order-sensitive over `proposal.logs`, but `DkgProposal` decode enforces strictly-ascending `idx` (`dkg_agree.rs:443-446,506`), and every call site handles decoded or in-process-built proposals, so two encodings of one set cannot differ. | confirmed by code |
| DA-03 | **FIXED** | `actor.rs:2538-2564` (latch gate `:2546-2563`), `:1399-1424` (`stall`) | The "pinned set names indices outside the committee" `error!` is inside the same once-per-`(epoch,reason)` latch gate as `dkg_finalize_deferred` (`:2546-2563`); the counter `dkg_pinned_idx_out_of_range` deliberately stays per tick (`:2539-2541`). Old form emitted the `error!` before the latch (unconditional per tick). | The condition should be impossible in honest operation, but the log-flood argument is about a diagnostic that only fires when something is already wrong, so it survives. | confirmed by code |
| DA-04 | **FIXED** | `actor.rs:2653-2658` (`drive_finalization`), `:1643-1646` (`apply_artifact`), `:2756-2759` (`decide`), `:1444-1448` (`on_body_lost`), `:1759-1760` (`conflict`), `:3706-3707,3738-3739` (`try_recompute`) | Every stall site now calls `set_state`/`enter` **before** `stall`, so `stall` (`:1406`) reads the real phase. Old form raised `PersistFailed` while the slot still held the `take_state` placeholder, logging `state=unrecoverable`. | Only the sites between a `take_state` and a `set_state` could ever mislabel; those are now ordered. No remaining site violates it. | confirmed by code |
| DA-05 | **RECORDED, correctly** | `dkg_engine.rs:404-408` (`try_send`), `actor.rs:1431-1453`, `:3528-3547` | The body-lost signal is still a fire-and-forget `try_send` with no retry/counter; a lost signal leaves a `Sealed` epoch to the boundary conversion at `:3533-3547`. The implementer records exactly this (journal §2.1 DB-28/DA-05). §5.2 sanctions a new channel and the fallback exists, so the recording is correct; a drop counter would be a cheap improvement. | On `E > now` the lost signal costs the whole pre-boundary window; that is the R-026 liveness the signal exists for. Not refuted as a cost, refuted as a correctness defect. | confirmed by code |
| DA-06 | **FIXED** | `actor.rs:2606-2611` (`drive_finalization` refusal arm), `:466-473` (`AdoptRefusal::stall`), `metrics.rs:48` (`StallReason::OffPolynomial`) | A live `adopt_share` refusal now raises `Stalled{OffPolynomial}` (`:2606-2611`) in addition to the counter and `error!` (`:1706-1718`). Old form transitioned to `Acquiring(Logs)` with no event. Direct test `actor.rs:12510` asserts the latch, then `Unrecoverable` after the heal. | A refusal is per `(epoch, reason)` latched, so no flood; the event list of §5.4 is satisfied. | confirmed by code |
| DA-07 | **FIXED** | `actor.rs:2733-2742` (`decidable`), `:2747-2750` (`decide` gate), `:2703-2721` (`decide_window`), `:1514-1516` (`on_artifact` calls `decide`) | `decide` refuses an epoch outside `[max(2, now−R), now] ∪ {now+1}` (`:2748`), so `on_artifact` can no longer start a far-future ceremony; the store keeps the artifact. Old form: `decide` had no window (`on_artifact` started any epoch). Test `actor.rs:12735` asserts 4/5/11 at `now=2` start nothing and 3 starts `dealing`. | The lower clamp on `{now+1}` (bootstrap) is required by the stand (`decidable` does not clamp `now+1`); that is disclosed (Д-А1-17) and exercised. | confirmed by code |
| DA-08 | **RECORDED, correctly** | `actor.rs:2919-2936` (`recover` requires a readable committee and `mints_at`), `:2925`, `:2934` | Design §5.2 `Sealed ─(committee unreadable)─▶ Acquiring{artifact}` is still not implemented: an unreadable `committee_for(E)` makes `recover` answer `None` (the `?` at `:2925`), so no slot is created and nothing pulls. The implementer records it as parity with pre-A1 (Д-А1-3). | The read is transient and the launcher re-asks each tick; changing it is a liveness experiment. Not refuted as a deviation, refuted as a regression. | confirmed by code |
| DA-09 | **RECORDED, correctly** | `actor.rs:2518-2534` (`Agreed` + `Stalled{BodyMissing}`), `:3390-3486` (`fetch_missing_logs`) | The design row `Agreed ─(bodies missing)─▶ Acquiring{logs} ─▶ Finalizing` is still implemented as "stay `Agreed`, fetch the pinned bodies via the resolver" (Д-А1-8). `Acquire::Logs` remains the journal-recompute heal only. | Functionally the fetch-and-wait is the same shape; a resting `Acquiring(Logs)` would have to re-derive from a journal rather than finalize the live ceremony. | confirmed by code |
| DA-10 | **RECORDED, correctly** | `actor.rs:3554-3557` | `Stalled{NoArtifact}` is still raised only when `e <= now`; before the boundary the wait is silent by design (avoiding a WARN on every normal mint epoch). Recorded (DA-10). | `drive_acquisition` still pulls every tick and the gauge is for latched conditions; not refuted. | confirmed by code |
| DA-11 | **RECORDED, correctly** | `ceremony.rs:930-975` (`resume`/`file_log`), `:1146` (`signed_log_hash`); `actor.rs:2301-2351` (`publish_recorded_logs`) | D-10 fixes only the `Player::resume` view (`file_log` at `:966-973`); `signed_log_hash` and the publish/confirm path still stand on the first-recorded body. Recorded as intentional (the node proposes/confirms what it recorded first); finalize/fetch use the artifact's pinned hash. | The pinned set used for finalize/recompute/fetch comes from the artifact, so the claim index does not select bodies for this epoch. Not refuted as a deliberate split. | confirmed by code |
| DA-12 | **FIXED** | `actor.rs:6376-6380` (`standalone_actor_at`), `:12597-12635` (zero-window test), `:4074-4087` (geometry comment) | A `DKG_MARGIN_BLOCKS`-interval fixture is back and asserts that a missing journal at a zero-width deadline is `sat_out` with no journal dealt (`:12624-12632`) — the old R-036 geometry. The stale `INTERVAL=20` comments are updated (`:4076-4087`). Old form: the dedicated old-geometry coverage was gone and the comments still named 20. | The 61 pre-existing fixtures still ride `INTERVAL=30`; the finding was a coverage note, and the round-2 brief asked for "at least one old-geometry fixture", which is present. | confirmed by code |
| DA-13 | **RECORDED, correctly** | `actor.rs:1522-1525` (`on_artifact` fetches only when `apply_artifact` returns true), `:3418-3436` (`fetch_missing_logs` handles `Dealing{agreed:Some}`) | An artifact that lands while `Dealing{agreed:None}` sets the set (`apply_artifact` `:1574-1584`, returns `false`), so this same call does not fetch; the next `on_height` does (`:2211`). Recorded (DA-13). | Pre-seal there is nothing to finalize; on a halted chain nothing consumes the bodies either. Not refuted. | confirmed by code |
| DA-14 | **FIXED** | `actor.rs:480-504` (`carries`), `:1336-1364` (`set_state`), `:1995-2050` (sweep) | Latch retention is now a function of the new phase (`carries`, `:480-504`): `set_state` removes every reason the new phase does not carry and steps the gauge down (`:1354-1363`). `KeyOnly{Some}` loses `NoArtifact` (`carries` → `needs_artifact` false), `Agreed`→`Acquiring(Logs)` loses `QuorumMissing`/`BodyMissing`, etc. Old form cleared only on `Keyed` and sweep. `the_epoch_slots_ride_the_retention_window` asserts gauge balance (`:7711-7789`). | Terminals still carry their own reason (`:500-502`), which is intended; no underflow found (every `stall` insert is paired). | confirmed by code |
| DA-15 | **FIXED** (residual → EA-02) | `actor.rs:301-307` (`InTransition`), `:1371-1375` (`take_state`), `:1378-1386` (`debug_assert_settled`), `:1452,1526,2212` (call sites) | The placeholder is no longer a *real* terminal: `InTransition` reads as nothing (no ceremony, no digest, `needs_artifact` false) and `debug_assert_settled` fires at the end of `on_height`/`on_artifact`/`on_body_lost`. Old form inserted `Unrecoverable{key:None}`, which reads as a real terminal. All six production `take_state` sites restore the state; the invariant is now at least visible. | The check is `debug_assert!` only, misses `on_message`/`on_resolver_message`, and the three refutable `let-else continue` sites (`:2111,2576,3540`) can still leak `InTransition` if the pattern fails — unreachable today, but the class is not type-closed. Reported as EA-03. | confirmed by code (mechanism); inferred (leak unreachable) |
| DA-16 | **FIXED** | `artifact.rs:1326-1349` (`restart_replay` doc), `:1258-1283` (open_mint_memo, no stale actor text) | The `restart_replay` doc now states the actor takes the set from the channel **and** from its per-tick store read, and that replay is only the first-tick shortcut (`:1328-1336`). The stale "return on exactly that check"/"ONE place" text is gone. Old form claimed the channel was the only adoption source. | The behavioural selection at `:1356-1361` still filters on held-shares/journal, but the doc no longer contradicts it. | confirmed by code |
| DA-17 | **FIXED** | `dkg_agree.rs:995`, `dkg_oracle.rs:17`, `log_resolver.rs:449`, `log_store.rs:51`, `node/src/dpos.rs:1295` | The five comment-only files were rewritten to the new symbols (`recover`/`try_recompute`/`Acquiring(Logs)`/`Stalled`/`drive_acquisition`). `git grep` over `crates/dpos crates/node` for the eight deleted carrier names returns nothing (checked). | Comments are not evidence, but the review contract asks for no stale names; the requirement is met. | confirmed by code |
| DA-18 | **FIXED** | `actor.rs:2703-2721`, assert at `:2716` | `decide_window` still discards a window epoch's outgoing through a throwaway `dropped` vec, but the invariant it relies on ("a past-boundary resume sends nothing", `:2998-3000`) is now asserted where it is relied on: `debug_assert!(dropped.is_empty())`. | Debug-only, but the assert documents and tests the invariant. No residual. | confirmed by code |

Net: 12 FIXED, 6 RECORDED-correctly, 0 NOT FIXED, 0 PARTIALLY FIXED. The residual of DA-01/DA-15 is carried into EA-01/EA-02.

---

## Part B — fresh pass, findings EA-nn

### (1) Store as owner — `reconcile_with_store` / `stored()` / `AgreedOutcomeAt`

*Is the store consulted before every network pull and before every transition that needs the artifact?* Yes.
`drive_acquisition` calls `reconcile_with_store()` as its first statement (`actor.rs:3525`) before the `pull_artifact`
loop (`:3558-3560`), and `recover` reads `stored()` before any branch (`:2879`). The push path (`on_artifact` →
`apply_artifact`) cross-checks the store (`:1563-1569`). The pull seam `ArtifactBridge::deliver` inserts/notes before it
`adopt`s (`artifact.rs:1652-1677`, `:1706-1723`), and the instance inserts before `out.send`
(`dkg_engine.rs:441-457`).

*Can the actor and the store still disagree silently?* In-process, no: the store's `held` is first-wins and is always
written before the value is pushed, and `apply_artifact`/`reconcile_with_store` compare the actor's digest against
`stored.held` on every transition and every tick (`:1557-1569`, `:3591-3597`). The one non-durable direction is the
`divergent` note (EA-01): the store can *know* of a second certified value while the actor does not, and a restart
drops the knowledge. The reverse (`actor Keyed on X` while the store serves `Y`) cannot persist: `Y` can only be held
if the store was empty when `Y` was inserted, which contradicts the actor having applied `X ≠ Y` (which required the
store to hold `X` first).

*Is `divergent` set on every path a second certified value can arrive?* Bridge serve: `artifact.rs:1653`
(`note_divergent` after a verified `Have` while a value is held). Instance certify: `dkg_engine.rs:441-443`
(`insert == false`). Local pull goes through the same `deliver`. `restart_replay` (`artifact.rs:1350-1362`) replays only
`store.get` (the held value) and cannot introduce a divergent. So yes for every intake.

*What a restart before `stop_signing` loses.* The `divergent` map is RAM (`artifact.rs:456,492`); the durable artifact
journal holds only `ram`. `recover` can re-derive nothing else (the second artifact's bytes are nowhere on this node).
That is EA-01.

### (2) Decidable window — `decide_window` vs `decidable`

The two derive the same bound, but twice: `decide_window` builds `lo = now−R` and walks `lo..=now`, then
`decide(now+1)` (`actor.rs:2704-2719`); `decidable` rebuilds `lo` and tests `(lo..=now).contains(&epoch) || epoch ==
now+1` (`:2738-2741`). They agree today; a one-sided edit silently splits "decide all on the tick" from "decide one
per `on_artifact`" (EA-05). `decide` itself is the single gate (`:2748`), so `on_artifact` cannot start an
out-of-window epoch.

*Artifact for `now+1` before the node dealt (Д-А1-17).* `on_artifact` sees no slot, `last_height` is `Some`, calls
`decide(now+1)` (`:1514-1515`); `recover` reads the store (`:2879`), `mints_at(now+1)` is true, the node is a member,
`past_seal` is false, `load_journal` is `NoFile` → `start_fresh` (`:2985-2988`), and the phase match yields
`Dealing { ceremony, agreed: Some(set) }` (`:2990-2991`). The node deals a fresh (seeded) ceremony standing on the
pin artifact's set; the following `apply_artifact` is a no-op on the equal digest (`:1557-1561`). No second ceremony,
no early seal.

*Artifacts outside the window.* Deferred, not dropped: every producer inserts into the store before sending
(`artifact.rs:1667`, `dkg_engine.rs:441`), and the store has no eviction (`artifact.rs:434-444,516-525`). When the
epoch enters `[lo, now]`, `decide_window` calls `decide`, `recover` reads the artifact from the store, and
`reconcile_with_store` applies it. `now+2` and beyond are never seated by design; the store keeps the value. The only
loss is the accepted §5.4 artifact-persist failure (RAM-only store after a hard kill), which is not this change.

### (3) Transitions S0–S17 and exits

The phase set is the round-1 set plus `InTransition` (non-phase, `:301-307`); no `S` state lost a home. `Finalizing`
remains non-resting, as before. The new exits are the same plus `reconcile_with_store`'s store-driven
`None → Agreed/Keyed` and store-divergence → `Conflict`. No phase HEAD could leave lacks an exit in the new code; the
only "no exit" candidate is a *leaked* `InTransition`, which the sweep removes (`:2006-2019`) and which no current
path leaks.

*`InTransition` / `debug_assert_settled`.* The forgotten write is caught **only in debug builds**
(`debug_assert!`, `:1379-1385`) and only at three entry points: `on_height` (`:2212`), `on_artifact` (`:1526`),
`on_body_lost` (`:1452`). `on_message` (`:3205`) and `on_resolver_message` (`:3781`) never call it, and the refutable
`let-else continue` after `take_state` (`:2111`, `:2576`, `:3540`; plus `on_body_lost` `:1441` whose `return` skips the
assert) can leave `InTransition` if the pattern fails. Unreachable today (the filter/plan and the `take` are in one
synchronous block), but a release build would silently carry an epoch that reads as nothing until the sweep. EA-02.

### (4) Body-lost and other lossy signals

Still `try_send`-dependent after round 2:
* `dkg_engine.rs:407` — the body-lost signal (no retry, no counter) → DA-05.
* `artifact.rs:1707` (`ArtifactBridge::adopt`) and `artifact.rs:1026` (follower `adopt`, *silent* `let _ =`) — the
  artifact hand-off; mitigated by the store read + `divergent` note (but see EA-01, EA-09).
* `actor.rs:1823` (`agreement_tx.try_send`) — deferred to the next tick on `Full` (documented, fine).
Everything else on the actor's outbound path is an awaited `send` (`broadcast_all` `:3958-3972`), and the pinned-set
mailbox awaits (`:666-683`).

### (5) Findings table

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| EA-01 | **BLOCKER** (see assumption) | `artifact.rs:456,492,566-582`; `actor.rs:2869-2892,3599-3606,3591-3597`; `share_state.rs:670-714` | The store's `divergent` witness is RAM-only (`Arc<RwLock<BTreeMap>>`, default-initialised at `artifact.rs:492`; `with_persistence` rehydrates only `ram`). If the process restarts after a second quorum-certified value was noted (`artifact.rs:1653` or `dkg_engine.rs:441-443`) but before the actor's next `on_height` enters `stop_signing` (`actor.rs:3599-3606`), the witness is gone for good: the second value is not durable anywhere on this node, and a `Keyed` epoch does not pull. `recover` then re-keys from the still-held first value via `key_held_share` (`:2899-2908`) instead of `Conflict`. This is the task's "`Conflict` that a restart forgets" (the implementer's Д-А1-16 residual / weakest #1). | Requires ≥ 2q−n Byzantine signers certifying two values **and** a restart in the window between the note and the next tick; the durable terminal (`persist_conflict` marker, read first by `recover` at `:2869-2877`) covers every case *after* `stop_signing`, and the node still signs on the value its own store serves. So it is "should have stopped but did not", not "signs under a key it does not hold". Not refuted as a lost terminal; the harm is a missed `Conflict`, which the task lists as BLOCKER. | confirmed by code (mechanism); inferred (window/impact) |
| EA-02 | **MINOR** | `actor.rs:1378-1386`, `:2111,2576,3540,1441`; `:1452,1526,2212` | `debug_assert_settled` is debug-only and absent from `on_message`/`on_resolver_message`, and the three `let Some(EpochState::X) = self.take_state(e) else { continue; }` sites (plus `on_body_lost`'s `return`) would leave the slot in `InTransition` if the pattern ever failed — a phase that reads as nothing (no ceremony, no digest, `needs_artifact` false) and is only removed by the sweep (`:2006-2019`). | All four sites are unreachable today: the pattern is selected by the immediately preceding filter (`:2098-2106`, `:2493-2536`, `:3533-3538`, `:1432`) and there is no `await` between the filter and the `take`; the actor is single-threaded. Not a current defect; it is the residual of DA-15's "held by attention". | confirmed by code (mechanism); inferred (unreachable) |
| EA-03 | **MINOR** | `share_state.rs:707-714,739-762`; `actor.rs:1995-2050,2078-2083` | `beacon-conflict-e<E>.bin` markers are evicted only by the one-shot `reconcile_journals` on the first tick (`actor.rs:2078-2083` → `share_state.rs:748-752`) and by tests; the running `sweep_epoch_state` evicts journals but never calls `evict_conflict`. A marker written after the first tick therefore survives for the rest of the process (one 64-byte file per conflicted epoch), contrary to the doc "Reclaimed with the epoch's journal (`reconcile_journals`, the sweep)" (`share_state.rs:668-669`). | Conflict is a ≥ 2q−n-Byzantine event, so the count is ~0 and the bytes are tiny; the next restart's first-tick reconcile reclaims them. Bounded leak, not a correctness defect. | confirmed by code |
| EA-04 | **MINOR** | `actor.rs:3666,3670-3675,3641-3647,3946-3947` | `try_recompute` sets `st.attempted = true` **before** `load_journal` (`:3666` vs `:3670`). If the journal is `NoFile`/`Torn` at that instant, the code `continue`s with `attempted = true`; because `want` is empty, `fetch_missing_logs` issues no fetch and `ingest_recompute_log` never re-arms (`:3946-3947` sets `attempted = false` only for a delivered body). The epoch then sits in `Acquiring(Logs)` with no retry until the sweep. | `want == ∅` means `heal_over` (`:2667-2690`) saw every pinned body via `parse_journal`, which calls the same `load_journal` (`log_store.rs:140-152`), in the same tick; so `NoFile/Torn` there requires a read/disk race between two reads in one synchronous chain. Very narrow. The previous per-tick spin was deliberately traded for this; the `Err(other)` arm has the same shape and is acknowledged (journal weakest #4). | confirmed by code (ordering); inferred (reachability) |
| EA-05 | **MINOR** | `actor.rs:2704-2706,2717` vs `:2738-2741` | The decidable window is derived twice (`decide_window` and `decidable`); they agree today but a future edit to one bound silently desynchronizes "decide every epoch on the tick" from "decide this one via `on_artifact`". The implementer flags this (weakest #2). | The two expressions are textually identical (`now.saturating_sub(JOURNAL_RETENTION_EPOCHS).max(DETERMINISTIC_BOOTSTRAP_EPOCH)`), and `decide` is the only gate; no current defect. | confirmed by code |
| EA-06 | **NIT** | `actor.rs:1743-1746,1499-1526,1563-1568` | `conflict()` is a silent no-op when the epoch has no slot (`take_state` returns `None` at `:1744-1746`), so a divergent value pushed for an *undecided / out-of-window* epoch is not recorded by the actor (`apply_artifact` calls `conflict` at `:1566` but nothing stands). The fact survives only as the store's RAM `divergent` note and is picked up by `recover` when the epoch enters the window (`:2880-2892`) — i.e. it inherits EA-01's durability hole. | For a divergent push to happen the producer must have noted it in the store first (`artifact.rs:1653`, `dkg_engine.rs:441`), so the store note always exists in-process; the actor path is redundant. | confirmed by code |
| EA-07 | **NIT** | `dkg_engine.rs:494-499` | `resolve_artifact` still takes its local shortcut by `held.0.digest() == certificate.proposal.payload` — the full payload identity that includes `confirms` — while the round-2 identity for a second value is `value_digest` (no `confirms`). A certificate over the same pinned set/key with different confirmation metadata will not match the held artifact and will wait out the body; if the body is lost it raises `dkg_agree_body_lost` and pushes the actor into `Acquiring(ArtifactForCeremony)` for a value the store already holds. | The actor's `reconcile_with_store`/`recover` applies the held value anyway, so the cost is a redundant pull plus a spurious `body_lost` counter, not a wrong key. Low impact, but it is the one place the two identity notions still meet. | confirmed by code (mechanism); inferred (impact) |
| EA-08 | **NIT** | `artifact.rs:1021-1027` vs `:1703-1722` | The follower's artifact hand-off uses `let _ = adopt.try_send(artifact)` (`:1026`) with no warn/counter, unlike `ArtifactBridge::adopt` (`:1712-1721`, which warns on `Full`/`Closed`). A lost follower hand-off is invisible in logs/metrics. | The store owns the fact (this change), so the value is read on the actor's next tick; the log gap is diagnostic only. | confirmed by code |
| EA-09 | **NIT** | `actor.rs:1303-1328` | `enter` unconditionally `self.epochs.insert(epoch, slot)` and raises the terminal latch. Called on an already-existing slot it would drop that slot's `stalled` set without stepping the `dpos_dkg_stalled` gauges down. Production's only caller `decide` guards with `contains_key` (`:2748`), so this is latent. | All production entries are guarded; only tests call `enter` directly. Not reachable. | confirmed by code |
| EA-10 | **NIT** | `share_state.rs:687-699`; `actor.rs:2869-2877` | A malformed conflict marker (≠64 bytes) is ignored with a warn and never deleted (`share_state.rs:691-698`); `recover` reads it again each time it is asked (until the epoch is decided or the marker ages out on a later restart). It also means a torn marker cannot stop the epoch: the share file was already evicted by `drop_share` before `persist_conflict`, so the epoch falls into the journal path rather than `Keyed`. | A malformed marker needs a partial write/corruption; `persist_conflict` writes then fsyncs (`:681-684`). The marker is only a backstop for a share that outlived the eviction. | confirmed by code |

BLOCKER candidates checked and **not** found: a share adopted over a non-pinned/unverified polynomial (all three
`Keyed` sites pass `share_on_artifact`: `actor.rs:1660-1691`, `:2597`, `:3724`; `validate_share_on_poly` at
`outcome.rs:99-112`); a `Conflict` from an unverified value (only the verified bridge/instance `deliver`/`insert`
paths feed `note_divergent` and the actor channel — `artifact.rs:1612-1671`, `dkg_engine.rs:441-457`); a state with no
exit HEAD could leave (the only candidate is a leaked `InTransition`, EA-02, swept and unreachable); a stand-visible
liveness regression (cannot be verified here — see weakest-ranked #1/#4).

---

## Coverage of the "four inputs" recheck

* **clock / `on_height`** (`:2052-2213`): order is `decide_window` → seal loop → `pending` eviction →
  `drive_finalization` → `confirmations.mint` → `announce_agreement_targets` → `sweep_epoch_state` →
  `retransmit`/`broadcast_all` → `drive_acquisition` (`reconcile_with_store` → boundary conversion → pulls →
  `try_recompute`) → `fetch_missing_logs` → `debug_assert_settled`. The store reconcile sits *after* the same-tick
  finalize/announce/confirm passes, so a store-only discovery finalizes on this tick (`:3525-3527`) but its
  confirmations/announcement wait for the next one. Minor latency, not a defect.
* **artifact / `on_artifact`** (`:1489-1527`): no slot → `decide` (window-bounded); no `held_digest` → `apply_artifact`
  compares against the store; digest equal → no-op; digest different → `Conflict`.
* **p2p / `on_message`** (`:3205-3366`): unchanged ingress/window/seat rules; `Confirm` intercepted before ceremony
  dispatch; buffer per-sender latest-wins.
* **resolver / `on_resolver_message`** (`:3781-3915`): `Produce` serves the exact `(dealer,hash)`; `Deliver` into a
  live ceremony or an `Acquiring(Logs)` heal (`ingest_recompute_log`, `:3920-3956`), re-arming `attempted` on a durable
  body (`:3946-3947`).

## S0–S17 mapping delta vs round 1

The state set and coverage are unchanged from the round-1 table; the only additions are the non-phase `InTransition`
(`:301-307`, replacing the old `Unrecoverable{None}` placeholder) and the store-driven edges into the existing
`Agreed`/`Keyed`/`Conflict`. `Acquire::ArtifactForShare` now resolves through `key_held_share` (`:1605-1616`,
`:1660-1691`), which is the DB-01 third-path fix. No `S` state became homeless and none was removed.

---

## Leave as is

* `ArtifactStore::insert` first-wins (`artifact.rs:516-525`) plus the RAM `divergent` witness
  (`:566-582`) and the `reconcile_with_store` per-tick read (`actor.rs:3579-3609`) — the right single-owner shape; the
  durability gap is a separate ticket (EA-01), not a reason to move the artifact fact back into the actor.
* `value_digest` as the `Conflict`/first-wins identity (`artifact.rs:328-338`) and `DkgProposal::digest` remaining the
  *certificate* payload identity (`verify_artifact` `artifact.rs:269`) — two distinct questions, keep them distinct
  (EA-07 is only the one stale shortcut).
* The `body_lost` `try_send` (`dkg_engine.rs:407`) with the boundary fallback (`actor.rs:3533-3547`): a deliberate
  best-effort signal, sanctioned by §5.2, with a real fallback; do not make it a blocking send on the agreement
  instance's teardown path.
* `try_recompute`'s once-per-input `attempted` instead of a per-tick crypto spin (`:3641-3666`) — the right trade;
  EA-04 is a corner, not a reason to re-add the spin.
* `decidable` refusing `now+2` while `on_artifact` defers the value in the store (`:2733-2742`, `:1514-1521`) — the
  window is right and the artifact is not lost.
* `key_held_share`'s heal on a missing share (`:1684-1690`) — correct; a share that left the store must not re-key.
* `Stalled` reasons as a `BTreeSet` on the slot with `carries` retention (`:441-504`) — the type-level closure of the
  old per-map latch class; keep.
* `conflict()` seeding the dropped ceremony's logs into `log_store` before `stop_signing` (`:1747-1757`) — keeps the
  epoch's logs servable to a peer after the terminal; keep.
* `persist_conflict` as a separate 64-byte file beside the share rather than a journal tag (Д-А1-13) — reachable for a
  non-member and after `evict_journal`; keep (modulo EA-03's sweep eviction).
* The five comment-only edits (DA-17) and the geometry-comment rewrite (DA-12) — hygiene only.

## Where this review is weakest (ranked)

1. **No execution.** No `cargo`, clippy, tests or stand; every 711/58/49/57/64/16, clippy-zero and mutation claim is
   the journal's, and the stand-visible liveness question in the brief is therefore unanswered by me.
2. **Byzantine reachability.** EA-01/EA-06 rest on two quorum-certified artifacts plus a restart in a window; the
   mechanism is confirmed in code, the frequency is an argument.
3. **Agreement internals.** I read `verify_artifact`, `value_digest`, `derive_pinned`'s consumers and the delivery
   seams, but not the full `dkg_agree` certificate/pairing protocol, so I cannot exclude other ways two values for one
   epoch arise.
4. **`testbed/stand.rs`.** Not read; a stand-only regression would not show here.
5. **Line drift.** Round-1 tables' line numbers were stale; I relocated every finding by symbol, but a symbol with two
   similar call sites (e.g. `drive_finalization` callers) could still be mis-cited.
