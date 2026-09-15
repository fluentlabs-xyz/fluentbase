# Round-2 review, focus B — the adopt gate, durable `Conflict`, value digest, off-poly, tests, hygiene

Reviewer: focus B. Object: `git diff HEAD` of 13 files under `crates/` (HEAD `60d6c3e3`), production region
`crates/dpos/consensus/src/beacon/actor.rs:1-3975` (`mod clock_tests` at `:3976`). Reading review only; **no cargo was
run** — every gate figure in `dsh-input-journal.md` §2.5 is taken as reported, not reproduced. All citations are to the
current worktree, opened with the read tool. Part A is the verdict for each DB number present in
`dsh-input-round1-B.md`; Part B is a fresh pass with new `EB-nn`. Nothing was modified.

Ambiguity handling: where the design (`dsh-input-design-5.2-5.4.md`) and the code diverge, I treat the design as the
spec and the code as the object, and I state the assumption inline. `indexed`/`1-based` line numbers are the current
worktree unless marked "HEAD".

---

## 1. Part (A) — verdicts for the round-1 findings in my half (DB-01…DB-33, numbers present)

| id | verdict | file:lines (current) | why + the edit under which the old form hid the defect | refutation attempt | confidence |
|---|---|---|---|---|---|
| DB-01 | **FIXED** | `share_on_artifact` `actor.rs:1696-1719`; `adopt_share` gate call `:1915`; `key_held_share` `:1660-1691` (gate `:1672`); `apply_artifact` `ArtifactForShare` arm `:1605-1616`; `recover` share∧artifact `:2899-2908`; live `drive_finalization` `:2597`; heal `try_recompute` `:3724` | The three ways into `Keyed` are now one gate. `key_held_share` **replaced** the old unconditional `Acquiring(ArtifactForShare) => Keyed` (HEAD `:1480-1482`) and the old `recover` share∧artifact branch (HEAD `:2586-2595`); the live and heal paths pass `adopt_share`→`share_on_artifact` (`:1915`). All three call `validate_share_on_poly` (`outcome.rs:99-112`) against the **artifact's** `group_key` (`set.group_key`), which comes from `AgreedSet::of` (`actor.rs:262-268`) — from `on_artifact` (channel, both producers verify: `artifact.rs:1613-1638`, `dkg_engine.rs:384-419`), from `reconcile_with_store` (`:3593`, store-quorum-checked) or from `recover`'s `stored.held` (`:2893/:2947`). Failure ⇒ `drop_share` + `Acquiring(Logs)` + `Stalled{OffPolynomial}` (`:1675-1683`). Tests `:12055`, `:12121`; mutation M-DB01 (§2.6). | "The store is first-wins and durable, so `ArtifactForShare` is unreachable." The store's durable artefact can be absent (its write lags/reads fail — `artifact.rs:526-543`), which is exactly the branch. "`validate_share_on_poly` may still be skipped on some path." Grepped every `EpochState::Keyed {` construction: `:1673`, `:2604`, `:3769`, all three dominated by `share_on_artifact` or `adopt_share`. Not refuted. | confirmed by code |
| DB-25 | **FIXED** | `stop_signing` `actor.rs:1769-1798` (+`drop_share` `:1725-1737`); `persist_conflict` `share_state.rs:670-685`; `load_conflict` `:690-704`; `evict_conflict` `:707-714`; `reconcile_journals` conflict loop `:748-752`; `scan_beacon_dir` `:777-808`; `recover` marker-first `:2869-2878`; test `:12168`; unit `share_state.rs:1300-1330` | The old `conflict()` removed the share from RAM only (`store.remove`); now `stop_signing` evicts the **file** (`:1734`) and writes a 64-byte `beacon-conflict-e<E>.bin` marker (`persist_conflict`, fsync), and `recover` reads the marker **before** the share/artifact/journal (`:2869`), calling `drop_share` for a share that survived eviction. A restart (`load_all` + `recover`) returns `Conflict` (`:12228-12235`). | "The marker can be a false terminal." Only `conflict`/`stop_signing` write it and only for two distinct `value_digest`s (`:1758`); epochs are never reused. "The marker is not read first." It is the first statement of `recover` (`:2869-2878`). Survives — **but the write ORDER is fail-dangerous; see EB-01.** | confirmed by code |
| DB-27 | **FIXED** | `artifact.rs:1640-1666` (`note_divergent` `:1653`), `:562-582`; `dkg_engine.rs:441-443`; `actor.rs:2880-2892` (`recover`), `:3600-3606` (`reconcile_with_store`); test `:12387` | The divergent value no longer depends on the one-shot `try_send`: both producers file it in `ArtifactStore.divergent`, first-wins by `value_digest` (`:566-582`), and the actor reads the note on every tick (`reconcile_with_store`) and in `recover`. The old defect (a full `EDGE_MAILBOX` dropped the only copy) is gone. | "Two producers can still race so that the note is never written." That race is real for the **bridge** path — see **EB-02**. The finding as stated (a lost `try_send`) is fixed. | confirmed by code |
| DB-03 | **FIXED** | `RecomputeState.attempted` `actor.rs:234-247`; filter and set `:3640-3666`; terminal/`Unrecoverable` `:3726-3741`; re-arm `:3751` and `:3947`; live refusal latch `:2606-2611`; test `:12510` | The live fallback no longer re-runs the crypto and re-emits the unlatched ERROR every tick: `attempted` is true after one run over current inputs, re-armed only by a journal change (`ingest_recompute_log`, `:3947`) or a persist failure (`:3751`); an off-poly share over a full `want` is `Unrecoverable` (`:3738`) and the live refusal raises `Stalled{OffPolynomial}` (`:1610`, `:2606-2611`). Test asserts exactly one extra recompute and no further counting (`:12580-12584`). | "`attempted` is set even when the recompute cannot run, so a transient failure is not retried." True but narrow — the no-journal sub-case needs `want` already empty; the `Err(other)` sub-case is real and is **EB-03**. The reported per-tick ERROR spin is fixed. | confirmed by code |
| DB-11 | **FIXED** | `standalone_actor_at` `actor.rs:6376-6426`; test `at_a_zero_width_deal_window_a_missing_journal_at_the_deadline_sits_out` `:12597-12635`; geometry doc `:4076-4087` | The zero-width fixture is back: `at(..., interval = DKG_MARGIN_BLOCKS)` makes epoch 2's deadline equal epoch 1's first height (`assert_eq!(actor.epoch_of(deadline), BOOTSTRAP - 1)`, `:12617-12621`) and asserts `NoFile` at the deadline ⇒ `sat_out`, no ceremony, no journal, `Stalled{SatOut}` — R-036 in the old geometry. | "It proves only `sat_out`, not that the missing journal is the cause." The dir is fresh, `_journal` is never written, and the assertion is `NoFile`-specific (`:12625-12628`). Not refuted. | confirmed by code |
| DB-09 | **FIXED** | test `a_resume_stands_on_the_pinned_body_of_a_two_log_dealer` `actor.rs:12644-12728`; `ceremony.rs:962-975` (`file_log`) | A direct D-10 test now exists: first-recorded body ⇒ `Err(MissingPlayerDealing)`, pinned body ⇒ `Ok`, a preferred hash not in the journal falls back (`:12709-12726`). | "The observable is commonware's own integrity check, not the actor." True — the test exercises `DkgCeremony::resume` directly (`:12698-12708`), which is what `resume_from_journal` calls (`:3065-3072`). It is the direct behaviour, not a proxy. Not refuted. | confirmed by code |
| DB-16 | **FIXED** | `actor.rs:2538-2564` | The `unmappable > 0` ERROR and the `dkg_finalize_deferred` increment are inside `if !slot.stalled.contains(&reason)` (`:2546-2563`), i.e. once per `(epoch, reason)`; the counter `dkg_pinned_idx_out_of_range` stays per-tick as at HEAD. | "The condition can only fire when something is already wrong." It still fires per tick while pinned indices are outside the committee, but the unbounded part is a counter, and the log is latched. The reported flood is gone. | confirmed by code |
| DB-17 | **FIXED** | `drive_finalization` `:2653-2658`; `apply_artifact` `:1643-1646`; `decide` `:2756-2759` | Every `stall` now runs after `set_state`/`enter`, so the emitted `state=` names the real phase, not the `take_state` placeholder (`:2655-2657`). | "Another site still stalls mid-transition." Grep of `self.stall(` production: `:1326` (after `enter`'s insert), `:1448`, `:1645`, `:1760`, `:2564`, `:2657`, `:2758`, `:3556`; all run with the slot settled. Not refuted. | confirmed by code |
| DB-24 | **FIXED** | `EpochState::InTransition` `:301-307`; `take_state` `:1371-1375`; `debug_assert_settled` `:1378-1386`; calls `:1452`, `:1526`, `:2212` | The old `Unrecoverable{key:None}` placeholder is replaced by a distinct `InTransition` that reads as nothing (`held_digest`/`needs_artifact` `:405-431`), and `debug_assert_settled` catches a forgotten put-back at the end of the three inputs. | "The assert is not on every input that can take a slot" — true, `on_message`/`on_resolver_message` are not covered (**EB-06**); but the type-level placeholder is in place. Not refuted. | confirmed by code |
| DB-06 | **FIXED** | `carries` `:480-504`; `set_state` `:1336-1364` | A latch now lives with the phase that carries it: `set_state` drops every reason `carries(&new, r)` rejects with a gauge step down (`:1354-1363`). `KeyOnly{Some}` after `ArtifactForKey` loses `NoArtifact` (`carries`→`needs_artifact()` false); terminals carry their own. | "`SatOut`/`Unrecoverable` remain stalled by design." True and intended; the overstatement case (`KeyOnly{Some}`) is closed. Not refuted. | confirmed by code |
| DB-26 | **FIXED** | `on_artifact` `:1489-1527`; `reconcile_with_store` `:3579-3609`; test `:12387` | An artifact whose slot cannot be created now survives as a fact in the store (both producers file before sending), and the next tick's `reconcile_with_store` applies it or turns the store's `divergent` note into `Conflict`. The one-shot channel is no longer the only carrier. | "The divergent second value is not stored." It is stored as the `divergent` witness (`artifact.rs:566-582`) and read at `:3600-3606`. Not refuted. | confirmed by code |
| DB-28 | **RECORDED, correctly** | `dkg_engine.rs:404-408`; journal §2.4 | `body_lost` is still `try_send` with no retry; a full mailbox drops the early signal and the heal waits for the boundary pull. The implementer records it and explains why the store cannot help (there is no artifact to read). | "The actor's own tick pull heals it anyway." True at/past the boundary; for `E > now` the whole pre-boundary window is lost, which is exactly the recorded cost. Correctly recorded. | confirmed by code |
| DB-08 | **RECORDED, correctly** | `retry_nondurable_journals` `actor.rs:2267-2293`; `ceremony.rs` `withhold_ack`; journal §2.3 п.5 | A failed `ReceivedDealing` journal write is withheld but never retried: `nondurable_logs` holds only `step.recorded_log`, and the retransmit path returns an empty step. Recorded for A2; safe because the dealer reveals the point. | "`retry_nondurable_journals` retries it." It cannot: `journal_record_for` is looked up on the live ceremony, and the withheld ack means the retransmit never re-forms the record. Correctly recorded, not fixed. | confirmed by code |
| DB-05 | **FIXED** | `set_state` `:1348-1353` | `announced` is cleared whenever the new phase is not `Sealed`/`Agreed` (`:1348-1352`), so leaving the announced phase drops the flag with it. | "An `Agreed` epoch that never keys keeps re-announcing." Yes, until sweep — that is DB-33. The reported "retained past the ceremony" over-retention is fixed. | confirmed by code |
| DB-13 | **FIXED** | `the_epoch_slots_ride_the_retention_window` `:7711-7803` | The self-check is now `phase(reachable) == key_only` on the **production** `recover` path (`:7756-7760`), and the gauge balance is asserted (2→1, `:7771-7779`). | "It hand-inserts the `SatOut` slots." True (`:7768-7770`), but the production-path self-check and the gauge assertion are exactly what the round-1 finding asked for; the inserted slots are the load-bearing phase the sweep must retain. Not refuted. | confirmed by code |
| DB-29 | **RECORDED, correctly** | `on_body_lost` `:1431-1453`; journal §2.4 | Only `Sealed` acts on the signal; a `Dealing` epoch drops it, and the at-once heal becomes a boundary heal. Recorded. | "The instance cannot run while the actor is `Dealing`." The instance is spawned asynchronously from the announcement, so the window exists; the cost is bounded by `drive_acquisition`'s `entered` conversion (`:3533-3547`). Correctly recorded. | confirmed by code |
| DB-31 | **FIXED** | `decidable` `:2733-2742`; `decide` `:2747-2750`; test `:12735-12765` | `on_artifact`→`decide` now refuses an epoch outside `[max(2, now−8), now] ∪ {now+1}` (`:2748`), so a far-future artifact starts nothing and waits in the store; `{now+1}` is deliberately not clamped by the bootstrap floor (Д-А1-17). | "The window is derived twice and can drift." True — **EB-05**; but the behaviour is fixed. Not refuted. | confirmed by code |
| DB-20 | **FIXED** | `dkg_agree.rs:995`, `dkg_oracle.rs:17`, `log_resolver.rs:449`, `log_store.rs:51`, `crates/node/src/dpos.rs:1295` | All five stale names/line-references are corrected in the diff (`deferred_reported`→latch, `drive_recompute`→`try_recompute`, `recompute_pending`→`Acquiring(Logs)`, `maybe_start`→`recover`). | "The harness python still cites `maybe_start` / the old `actor.rs:1060-1067`." True and out of the Rust write list (`devnet` python); the five Rust files are clean. Not fully refuted, but the in-scope half is fixed. | confirmed by code |
| DB-21 | **FIXED** | `:4076-4087`, `:6454`, `:6897-6898` | The geometry comments are now relative: the `INTERVAL` doc block names the old `INTERVAL = 20` as history and the current `INTERVAL = 30` (`:4076-4087`), and the two `now = 5` comments say "height `INTERVAL * 5`" (`:6454`, `:6897-6898`). No `INTERVAL = 20` / "height 100" remains outside that historical block. | "Some other stale geometry comment exists." Grep for `INTERVAL = 20|height 100|INTERVAL 20` returns only the historical mention at `:4078`. Not refuted. | confirmed by code |
| DB-22 | **RECORDED, correctly** | diff `+4312/−1436` (13 files); `actor.rs` production ~+900 net | Volume above the task's guideline; the journal's §2 header and §2.3 record it. Process observation, not a code defect. | "The state machine touches all four inputs, so the size is intrinsic." True; recorded. | confirmed by code |
| DB-33 | **RECORDED, correctly** | `announce_agreement_targets` `:1807-1846`; `on_height` `:2165` | An `Agreed` epoch that can never become ready re-announces every tick until sweep; the launcher dedups, the cost is a `try_send`. Recorded. | "The epoch stops being announced after a failed finalize." It does (phase leaves `Agreed`), which is the intended bound; an `Agreed` epoch that never gets a referee keeps announcing. Correctly recorded. | confirmed by code |

### Positive results re-verified (not findings)

- **The pinned-set agreement is unchanged.** `AgreedSet::of` copies `proposal.logs` (`actor.rs:262-268`), `DkgProposal.logs`
  is canonical ascending (enforced at decode, `dkg_agree.rs:482-513`, and built from `BTreeMap`, `:974-983`,
  `:1157`), `heal_over` maps it through `pinned_by_dealer` (`:2667-2691`), and `finalize_over_pinned` selects by it.
- **`Conflict` is reachable only from verified values.** The channel is fed by the instance's certified output
  (`dkg_engine.rs:384-457` → `spawn_write_back`, `plane.rs:466-495`), by `restart_replay` from the durable store
  (`artifact.rs:1350-1362`), and by the bridge after `verify_artifact_for_epoch` (`artifact.rs:1613-1638`); the store
  note is written only after the same verification (`:1652-1663`, `dkg_engine.rs:441-443`).
- **Hygiene holds.** `git diff | grep '^+.*#\[allow'` is empty; no new production `unwrap` (only `unwrap_or*`) and
  exactly one new production `expect`, the replacement `expect("just entered Dealing")` (`:2786`) for HEAD's
  `expect("just started")`; the other production `expect`s are HEAD's or `#[cfg(test)]` (`:1016-1038`); new `pub`
  items live under the private `mod actor`/`mod metrics` (`mod.rs:68-83`, `pub use` list unchanged).

---

## 2. Part (B) — fresh pass, new findings `EB-nn`

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| EB-01 | **SERIOUS** | `actor.rs:1769-1786` (`stop_signing`), `:1730-1735` (`drop_share`), `:1772` (`persist_conflict`); `share_state.rs:670-685` | `Conflict`'s durable record is written **after** the share file is evicted. `drop_share` removes the file (`:1734`) and then `persist_conflict` writes the marker (`:1772`). If the process dies between the two, or if `persist_conflict` returns `Err` (`durable=false`, only a warn at `:1774-1783`), the epoch is left with **no marker and no share**. On restart `recover` (`:2869-2878`) sees no marker, `load_all` sees no share, the store rehydrates `held` (first-wins, durable) and a `Present` journal resumes → `Agreed` → `finalize_over_pinned` → `adopt_share` → `Keyed` over the held artifact. The terminal is forgotten and the node re-signs an epoch it declared `Conflict`. | "The share file is evicted, so no share can re-key." Without the marker, the epoch is *re-derived from the journal*, not re-keyed from the file — the heal is exactly the path that produces a new share. "A `persist_conflict` failure is a disk failure that would also break everything else." `evict_share` is a delete and can succeed under ENOSPC/permission cases where `write`+`fsync` fails. Writing the marker first is strictly safer and costs nothing. Not refuted. | confirmed by code (order + failure path); inferred (crash window) |
| EB-02 | **SERIOUS** | `artifact.rs:1667-1677` (`deliver`), `:516-524` (`insert` takes the artifact by value), `:1652-1663` (the branch that does note) | The bridge drops a **divergent pulled value** when it loses the insert race. `deliver` checks `store.get(epoch)` (`:1652`), gets `None`, then calls `store.insert(epoch, artifact)` (`:1667`) which **moves** the artifact. If a concurrent producer (the agreement instance, `dkg_engine.rs:441`) inserted between the `get` and the `insert`, `insert` returns `false` and the bridge neither `note_divergent`s nor adopts (`:1671-1676`): the second certified value vanishes. The instance's own loser path notes correctly, so the loss is asymmetric — it happens exactly when the **pulled** value is the loser. Result: two quorum-certified values exist, the store/actor see one, no `Conflict`. | "The bridge and the instance cannot both run for one epoch." They can: the bridge pulls whenever `needs_artifact()` (`:3548-3560`), and the local agreement instance can certify concurrently. "The earlier `get` closes the race." It is a separate lock acquisition from `insert`; the code even relies on `stored` to detect it but discards the value. "The send via `spawn_write_back` carries A, so at least one value reaches the actor" — true, but a `Conflict` needs two. Not refuted. | confirmed by code (race, no note on `stored==false`); inferred (concurrency reachability) |
| EB-03 | **MODERATE** | `actor.rs:3666` (`attempted = true` before the work), `:3670-3674` (`NoFile`/`Torn` → `continue`), `:3710-3719` (`Err(other)` → `continue`); the only re-arms `:3751`, `:3947`; filter `:3644-3647` | A recompute that fails with `Err(other)` when `want` is empty leaves `attempted = true`, no `Stalled{…}` latch and no gauge (only a one-shot `warn!`, `:3713`). Because the filter requires `want.is_empty()`, `fetch_missing_logs` requests nothing (`:3437`) and no resolver delivery can call `ingest_recompute_log` to re-arm it (`:3947`). The epoch silently parks in `Acquiring(Logs)` until the sweep, invisible to `dpos_dkg_stalled`. `attempted` is also set before `load_journal`, so a `NoFile`/`Torn` there consumes the only attempt. | "`Err(other)` is not retryable anyway." The journal itself records the retryability as unproven (§2.7 п.4) and the design's `Err(other) ⇒ Acquiring{logs}` implies keeping the retry rail; either way a stall with no event is a diagnostic hole. "The sweep re-decides it." No: the slot is removed, not re-decided. Not refuted. | confirmed by code |
| EB-04 | **MODERATE** | `artifact.rs:449-456` (the field doc says RAM-only), `:566-582` (`note_divergent` never persists); `actor.rs:2880-2892`, `:3600-3606` | The store's `divergent` witness is RAM-only. A restart between the store noting the second certified value and the actor's `stop_signing` loses the witness (the durable store keeps only `held`), so `recover` returns `Keyed`/`SatOut` and no marker is ever written. This is the implementer's own weakest point Д-А1-16 and is **not** covered by any durable fallback on the store side. | "The second value is also pushed to the actor and the actor's `apply_artifact` catches it." Only if the hand-off lands before the crash; the whole reason DB-27/DA-01 were raised is that the hand-off is lossy. "A peer will re-serve it." Only if a pull is issued; a `Keyed` epoch does not pull (`needs_artifact()` false). Not refuted; recorded, but it is a live hole. | confirmed by code |
| EB-05 | **MINOR** | `decide_window` `:2703-2721` vs `decidable` `:2733-2742` | The decidable window is derived twice (`lo = now.saturating_sub(JOURNAL_RETENTION_EPOCHS).max(DETERMINISTIC_BOOTSTRAP_EPOCH)`; `(lo..=now) ∪ {now+1}`). They agree today, including the deliberate `{now+1}`-not-clamped rule (Д-А1-17). A future edit to one (window width, bootstrap rule, `now+2` addition) silently desynchronises "which epochs are walked" from "which epochs `on_artifact` may decide". This is the implementer's weakest point #2. | "The two are trivially identical and covered by the same tests." The tests exercise the union only through `decide`; no test would catch a one-sided edit. The fix is one predicate (`decidable`) that `decide_window` also uses for its bounds. Not refuted (drift hazard, not today's bug). | confirmed by code |
| EB-06 | **MINOR** | `debug_assert_settled` calls `:1452`, `:1526`, `:2212`; `on_message` `:3205-3366` (calls `drive_finalization` `:3327`, no trailing assert); `on_resolver_message` `:3781-3815` | The `InTransition` invariant is asserted at the end of `on_height`, `on_artifact` and `on_body_lost` only. `on_message` and `on_resolver_message` both reach `drive_finalization`, whose `let Some(EpochState::Agreed{..}) = self.take_state(e) else { continue; }` (`:2576-2578`) would leave a slot in `InTransition` on any state mismatch — and neither input would notice (in debug builds). Single-threaded today, so latent. | "The plans filter guarantees `Agreed`." It does in the same synchronous function, so it is unreachable today; the assert's stated purpose is to catch a future regression, and it is absent on two of the four inputs. Retained as a coverage gap. | confirmed by code |
| EB-07 | **MINOR** | test `a_conflict_survives_a_restart` `actor.rs:12168-12238` | The test proves the marker is written and survives a restart, but the restart has **no share file** (`load_all` is asserted empty at `:12204`), so it does not exercise the documented "marker read FIRST" order (`:2869-2878`): a `recover` that read/loaded the share before the marker would still pass. The scenario the order exists for — a share whose `evict_share` failed — is untested, and that is exactly the residual of EB-01. | "The `drop_share` in `recover` is exercised." No: with no share file and an empty RAM store it is a no-op. A test that re-creates `beacon-share-e2.bin` after the verdict and then restarts would close it. Not refuted. | confirmed by code |
| EB-08 | **NIT** | `metrics.rs:292-299` vs `:34-75` | The `dpos_dkg_stalled` HELP text enumerates "quorum_missing, body_missing, body_lost, no_artifact, persist_failed, unrecoverable, sat_out, conflict" — eight of the nine `StallReason` values; `off_polynomial` (`:48`, `as_str` `:63-75`) is missing. An operator grepping the help for the reason cannot find it. | "The label's value is self-describing." Only if you know it exists; the help is the registry's discoverability surface. Cosmetic. | confirmed by code |
| EB-09 | **NIT** | `artifact.rs:1712-1716` (`adopt`'s `Full` arm) | The `Full` warn says "was not adopted and waits for the recompute heal". Since DA-01 the store owns the fact and the actor applies it via `reconcile_with_store`; the recompute heal is only the off-poly/`Err` fallback. The line misdirects a postmortem. | "The artifact could also be pre-agreement." No — `deliver` only adopts a value already `store.insert`ed (`:1667`). Cosmetic. | confirmed by code |
| EB-10 | **NIT** | `actor.rs:1491-1498` (`on_artifact` empty-log early return) vs `:3593` (`reconcile_with_store` → `apply_artifact`, which has no such guard) | The same artifact is dropped on the channel path when `logs.is_empty()` but is applied from the store path, so the two intake rails treat one input differently. Unreachable today (a quorum-certified empty pinned set cannot be produced), but it is an asymmetry a future change could turn into a divergence. | "An empty artifact cannot pass the agreement." Correct (the entry bar/`select` need a quorum of logs), so this is latent only. Not refuted as an inconsistency. | confirmed by code (inconsistency); inferred (unreachability) |

---

## 3. Re-answers to the seven questions, against the NEW code

**(1) THE ONE GATE (`share_on_artifact`).**
All `Keyed` writes are `:1673` (`key_held_share`), `:2604` (live `drive_finalization`, inside the `Ok` arm of
`adopt_share` `:2597`), `:3769` (heal `try_recompute`, after `adopt_share` `:3724`). The two `adopt_share` call sites
both enter `share_on_artifact` at `:1915`; `key_held_share` enters it at `:1672`. The third path is reached from
`apply_artifact`'s `ArtifactForShare` arm (`:1605-1616`, from `on_artifact`/`reconcile_with_store`) and from `recover`'s
share∧artifact row (`:2899-2908`). **No path bypasses it.** Provenance of the compared value: live/heal use
`AgreedSet::of(proposal).group_key`, where `proposal` is (a) the instance's certified output routed through
`spawn_write_back` (`plane.rs:466-495`) or `restart_replay` (`artifact.rs:1350-1362`), (b) the bridge's verified pull
(only after `verify_artifact_for_epoch`, `:1613-1638`), or (c) `stored.held` from the store (`actor.rs:2893`, `:3593`);
`validate_share_on_poly` additionally requires `outcome.players() == committee` (`outcome.rs:99-107`). The comparison
target is the **artifact's** polynomial, never the local ceremony output (`_out` is bound and unused at `:2589`).
Refutation attempts in DB-01 / EB-02 above.

**(2) DURABLE `Conflict`.**
Order proven: `stop_signing` `:1769` → `drop_share` `:1770` (file eviction `:1734`) → `persist_conflict` `:1772`.
`recover` reads the marker **first**, before `stored`, before the share, before `mints_at`, before `load_journal`
(`:2869-2878`). `reconcile_journals` reclaims markers on the journal window (`share_state.rs:748-752`, first tick only,
`actor.rs:2078-2083`). Failure semantics: `persist_conflict` failure ⇒ `durable=false` and a warn; a surviving share
file is then the only record and `recover` will re-key/re-heal — **EB-01**. `evict_share` failure ⇒ the marker (if
written) still wins via marker-first, and `recover` retries `drop_share`; if the marker is also missing, the share
reloads and the epoch re-keys — the same hole. Marker cleanup: evicted only by the one-shot startup reconcile (and only
for epochs past the window), never by the running sweep; bounded (no in-window epoch is affected) but it means a marker
lives for the process lifetime. False terminal: no — only two distinct `value_digest`s produce it, epochs are never
reused, and once `Conflict` the slot is sticky (excluded from `reconcile_with_store`, unreachable from
`drive_finalization`/`try_recompute`, never re-`enter`ed because the key persists).

**(3) VALUE DIGEST.**
`value_digest` is defined once (`artifact.rs:328-338`), imported by the actor (`actor.rs:48`), and every site calls it:
`AgreedSet::of` (`:266`), `apply_artifact` (`:1543`), `recover` (`:2882`, `:2932`, `:2946`), `reconcile_with_store`
(`:3591`, `:3600`), `note_divergent` (`:570`), the bridge log (`:1656-1657`), plus tests. Identity is
`epoch‖len‖(idx‖hash)*‖encode_outcome(group_key)`; `logs` is canonical strictly-ascending at decode
(`dkg_agree.rs:482-513`) and built from a `BTreeMap` in-process (`:974-983`, `:1157`), so identical pinned sets cannot
differ by order. Two genuinely different sets cannot collide except by a keccak break; two mathematically identical
`group_key`s re-encode identically (`encode_outcome`, `outcome.rs:56-58`), so identical values cannot differ. `confirms`
is deliberately excluded (differs between `DkgProposal::digest`, `dkg_agree.rs:559-570`) and the test `:12451` pins it.

**(4) OFF-POLY WITHOUT SPIN.**
`attempted` is set at `:3666`, reset only at `:3751` (persist failure) and `:3947` (a durable recompute-body ingest);
`heal_over` starts `false` (`:2689`). `Unrecoverable` is entered when `want` is empty and the recompute is still
off-poly (`:3726-3741`) or `MissingPlayerDealing` (`:3692-3709`). A later log arrival **cannot** change that: with
`want` empty, `fetch_missing_logs` issues nothing (`:3437`, `:3440-3442`) and the resolver requests nothing, so the
only re-arm (`ingest_recompute_log`) cannot fire; the recompute is deterministic over the retained journal plus the
pinned set, so `Unrecoverable` is not entered too early. The real gap is the `Err(other)`/unreadable-journal arm
(`:3670-3674`, `:3710-3719`), which parks silently — **EB-03**.

**(5) TESTS.**
The twelve new tests exist at `:12055`, `:12121`, `:12168`, `:12270`, `:12341`, `:12387`, `:12451`, `:12510`, `:12597`,
`:12644`, `:12735` and `share_state.rs:1300`. Each was read against its property: none is vacuous, and the
assertion-bearing lines are the ones the journal names (e.g. `:12096`, `:12200`, `:12320`, `:12494`). Green-while-broken
gaps found: `a_conflict_survives_a_restart` does not exercise marker-beats-share (**EB-07**); the zero-window fixture
does prove `NoFile` at the deadline ⇒ `SatOut` and nothing weaker (`:12617-12632`); D-10's direct test's observable is
commonware's integrity check, which is the property under test (`:12709-12726`). The four mutations are the right
mutation sites; none of them covers EB-01/EB-02/EB-03.

**(6) HYGIENE.**
No `#[allow]` added; no new production `unwrap`; exactly one new production `expect` (replacement at `:2786`); new
`pub` items confined to private modules; `can_finalize` deleted with no references; the five comment-only files
(`dkg_agree.rs`, `dkg_oracle.rs`, `log_resolver.rs`, `log_store.rs`, `node/src/dpos.rs`) plus `testbed/tests.rs` are
comment-only in the diff. Residuals: EB-08 (metric help), EB-09 (stale warn), EB-10 (intake asymmetry). I could not
run clippy/fmt/doc/nextest, so "no dead code / no unused import" is unverified by compilation.

**(7) Where this review is weakest — ranked.**
1. No cargo: dead code, unused imports, the 711/58/49 gates, clippy and fmt are unverified; my hygiene section is
   diff-reading only.
2. EB-01/EB-02/EB-04 rest on crash/race orderings I read but did not execute; their reachability is inferred, their
   code paths are confirmed.
3. Durable-store internals (`ArtifactJournal` batching, `Metadata` fsync semantics, directory-entry durability) were
   skimmed; EB-01's parent-directory question is therefore stated conservatively.
4. The actor's production region is ~3975 lines and the stand behaviour of `decidable`/`reconcile_with_store` is only
   as good as the journal's reported gates.
5. Byzantine-condition findings are inferred at the network level from code, not produced by a harness.

---

## 4. Leave as is

- **The single gate shape.** `share_on_artifact` + `key_held_share` + `adopt_share` is the right factoring: the
  `committee` parameter makes a future fourth path fail to compile without a committee, and `persist`-before-`insert`
  in `adopt_share` is enforced by ownership. Do not split the gate again.
- **`ArtifactStore` first-wins plus `note_divergent`.** First-wins preserves the locally agreed value; the witness is
  the correct way to carry a second value without letting a fetched artifact displace the held one. Fix the bridge
  race (EB-02) by noting, not by changing first-wins.
- **`Conflict` as a separate marker file** (`share_state::persist_conflict`) rather than a journal record: it is
  reachable for non-members and after `evict_journal`, and it needs no `ceremony::resume` change (Д-А1-13). Keep it —
  but write it **before** evicting the share (EB-01).
- **`RecomputeState.attempted` one-shot + `Unrecoverable` when `want` is empty**: the right answer to the R-038 spin.
  Leave the cadence; if the `Err(other)` arm is to get an event, add a `Stalled` latch, not a retry timer (EB-03).
- **`EpochState::InTransition` + `debug_assert_settled`**: acceptable for a single-threaded actor; extending the assert
  to `on_message`/`on_resolver_message` (EB-06) is cheaper than a typed `take`/`put` API.
- **`value_digest` without `confirms`** (Д-А1-15): correct value identity; the canonical `logs` order at decode is
  what makes it sound.
- **`decidable` as the single read-side window predicate**: keep it, and make `decide_window` call it rather than
  re-deriving `lo` (EB-05).
- **The five out-of-scope comment edits**: correct and worth keeping; the python harness prose remains for a separate
  pass.
