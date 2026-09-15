# Independent review — row 5.3 заход Д: the post-seal absentee heals instead of sitting out

Base `d8747086`; diff = `crates/dpos/consensus/src/beacon/actor.rs` (+~496),
`testbed/stand.rs` (+18), `testbed/tests.rs` (+149). I ran no cargo (forbidden by the
task); every claim below is from files/pin source I opened, or from the git diff. Tags:
`[KNOWN]` = read in this session; `[LIKELY]` = inference; `[GUESS]` = no evidence.
Comments in code are treated as claims, never as evidence.

**Verdict: no BLOCKER found.** The heal path provably does not broadcast; the pre-seal
re-deal gate is unchanged; the failure terminal is reached once; non-members never enter
the heal. Two SERIOUS integration findings concern the devnet smoke harness (the stated
purpose of the change), which the change does not update and which, by code, still cannot
read the new road. Details in the table; the positive traces are in `Leave as is`.

## Safety trace (Q1) — heal path emits nothing

`[KNOWN]` `heal_or_acquire` (`actor.rs:2896-2908`) returns `Acquiring(Acquire::Logs(..))`
or `Acquiring(Acquire::ArtifactForShare)`. `EpochState::ceremony()` returns `None` for
both (`actor.rs:382-390`), so:

- `on_height` seals only `Dealing` (`actor.rs:2247-2255`) and retransmits only live
  ceremonies via `ceremonies()` (`actor.rs:1317-1321`, `2367-2371`) — neither applies;
- `announce_agreement_targets` is driven only by `dealing_closed()` ceremonies
  (`actor.rs:2326`), and `confirmations.mint` by `publish_recorded_logs`, which iterates
  `ceremonies()` (`actor.rs:2490-2493`) — not the heal;
- `drive_acquisition` (`actor.rs:3816-3858`) calls `pull_artifact` (an inbound request)
  and `try_recompute`; `try_recompute` (`actor.rs:3954-4094`) only calls `adopt_share`
  (persist + store + notify, `actor.rs:2065-2113`); `recompute_scoped` explicitly
  discards the replayed acks (`ceremony.rs:1373-1375`, "nothing to re-broadcast");
- `recover`'s new arms return `quiet(...)` = empty outgoing (`actor.rs:3153`,
  `3264`, `3273`), and `decide`'s `dropped` sink stays empty
  (`debug_assert!`, `actor.rs:2932`).

A node that DID seal before crashing cannot re-seal: `start_fresh` is reachable only on
the `!past_seal` arms (`actor.rs:3275-3290`), and the seal condition in `on_height`
(`height >= epoch_start(E) − DKG_MARGIN_BLOCKS`, `actor.rs:2252`) is *identical* to
`past_seal` (`actor.rs:3244-3245`), so a sealed node always restarts with `past_seal`.
`evict_journal` cannot lose a share (shares live in the share file, `share_state.rs:119`,
`292-293`, and `share_held` is checked first, `actor.rs:3195-3210`) and cannot lose a
readable log record: `Torn` is defined as "the first record never decoded"
(`share_state.rs:606-635`), and the parse loop `break`s at the first error, so nothing
after it is ever readable (`share_state.rs:608-627`).

The reveal rule is confirmed at SOURCE, not from the comment:
`~/.cargo/git/checkouts/monorepo-9732103c47eb4665/3c4e02c/cryptography/src/bls12381/dkg.rs`
— `Player::resume` returns `MissingPlayerDealing` iff a log holds a *valid ack* of this
player and the replay lacks the view (`:1741-1754`); `Player::finalize` takes the per-dealer
share from `self.view.get(dealer)` OR `log.get_reveal(&self.me_pub)` (`:1836-1852`). So an
absentee's share is a pure function of the pinned bodies, and an acked-then-lost node is
terminal. `[KNOWN]`

## Termination (Q2)

`[KNOWN]` For an acked, journal-lost absentee: after every pinned body is journaled,
`want` empties; `try_recompute` (`actor.rs:3954-3963`) runs `recompute_scoped`, gets
`MissingPlayerDealing`, sets `Unrecoverable` once and raises one latch
(`actor.rs:4013-4029`). The unit `an_absentee_heal_over_an_acked_dealing_is_unrecoverable_not_a_loop`
(`actor.rs:12105-12222`) drives exactly this and additionally asserts no epoch-2 key stays
in the resolver's in-flight set. Every pinned body is fetched at most once per input
change: `ingest_recompute_log` removes the id only on a durable append
(`actor.rs:4255-4267`), and `attempted` (`actor.rs:3958-3961`, `3996`) blocks re-running
the crypto until a body lands (`4268`) or a persist fails (`4067-4072`). If a pinned body
is held by nobody, the fetch re-issues each tick but is bounded by the epoch's sweep:
`fetch_missing_logs`'s `retain` cancels keys the tick did not re-issue
(`actor.rs:3768-3779`), the slot lives while `e + JOURNAL_RETENTION_EPOCHS >= now`
(`actor.rs:2119-2120`), and `JOURNAL_RETENTION_EPOCHS = SCHEME_RETENTION_EPOCHS = 8`
(`beacon/mod.rs:137`, `lib.rs:37`). With `want` empty and an unreadable journal,
`try_recompute` raises `StallReason::HealFailed` once and re-reads the file per tick
without crypto or a line (`actor.rs:3978-3988`).

Load-bearing assumption: an epoch past `now − 8` cannot be re-decided at all
(`decidable_epochs`, `actor.rs:623-628`; `decide` guards on `contains_key`,
`actor.rs:2950`), which is what makes the sweep the bound. `[KNOWN]`

## Artifact-less entry (Q3)

`[KNOWN]` On artifact arrival, `apply_artifact`'s `ArtifactForShare` arm calls
`key_held_share` (`actor.rs:1745-1755`); with no share in `store`, `key_held_share`
returns `Acquiring(Logs(heal_over(..)))` (`actor.rs:1821-1826`) — byte-for-byte the state
`heal_or_acquire` would have produced with the artifact in hand. `needs_artifact()`
includes `ArtifactForShare` (`actor.rs:420-430`), so `drive_acquisition` pulls for it
every tick (`actor.rs:3845-3856`). A committee-read failure in `apply_artifact` keeps the
state (`actor.rs:1751-1755`) but is not silent: the epoch is still in the `acquiring` set,
so `drive_acquisition` raises the `NoArtifact` latch once `e <= now` and re-pulls
(`actor.rs:3852-3855`). The one inaccuracy: when the artifact is already held but the
*committee* read is the thing failing, the latch and the pull name the wrong cause.

## The other cells (Q4)

`[KNOWN]` `(Torn|NoFile, h < seal)`: unchanged — `evict_journal` + `start_fresh`
(`actor.rs:3275-3290`), and `start_fresh` re-seeds the dealer deterministically
(`actor.rs:3315-3334`). `(Present, h ≥ seal)`: unchanged
(`resume_from_journal(..., !past_seal, ..)`, `actor.rs:3250-3255`, `3351-3368`).
`decide_window`/`decidable_epochs` untouched (`actor.rs:623-628`, `2920-2934`), so a
clock jump past an epoch still decides it while inside `[now−8, now]`. A non-member is
returned before any journal read (`actor.rs:3230-3237`): `KeyOnly` with the artifact,
`Acquiring(ArtifactForKey)` without — never the heal.

## SatOut removal (Q5)

`[KNOWN]` `EpochState::SatOut` is gone from every match: `name` (`actor.rs:363-379`),
`held_digest` (`404-417`), `needs_artifact` (`420-430`), `enter` (`1370-1375`),
`apply_artifact` (`1764-1767`) and `carries` (`487-512`). `StallReason::SatOut`
(`metrics.rs:52-53`) survives but has **no producer**: `carries` returns `false`
(`actor.rs:509`), and the only construction is the `ALL` array (`metrics.rs:65-76`) and
`as_str` (`metrics.rs:87`). It is therefore not a compiler `dead_code` case, but the
`dpos_dkg_stalled{reason="sat_out"}` series is permanently zero. `git grep` over
`crates devnet .claude` finds no reader of that label; the only `sat_out` hits outside
`metrics.rs`/`carries` are the devnet function names `assert_sat_out_torn`
(`devnet/.../asserts_prod_dkg.py:187`, `355`, `665`), which read log lines, not the metric.

## Stand test (Q6)

`[KNOWN]` The deleted files are real: `StandConfig::live` puts `share_root` under
`std::env::temp_dir()` (`stand.rs:546-557`), the same `cfg` (hence the same root) is used
for both phases (`tests.rs:2038`, `2089-2091`), and `beacon::build` reloads the share
store from `share_dir` (`stand.rs:2920-2921`, `plane.rs:658-660`). The test asserts each
file exists before deleting it (`tests.rs:2082-2086`). The replay reconstructs every node
(`Stand::replay` → `Runner::from(checkpoint)` → `drive` → `build_node`,
`stand.rs:1363-1388`), so the in-memory `CeremonyStore` of phase 1 cannot carry the share.
The heal runs through the real artifact read + `try_recompute` (`tests.rs:2103` asserts the
new WARN line; `signable[3]` is the production `Beacon::can_participate == Ready` gate,
`stand.rs:915-917`, `1995-2004`). Liveness proves node 3's partial: after `cut_at = 102`
only `{0,2,3}` remain and the run must reach 130; with a verify-only node 3 there are two
signers of a 3-of-4 threshold, which is why the pre-fix run parked at 102 (journal §0.5,
`[LIKELY]` — I did not run it). The test **does** cover the `heal_over` arm only; at
restart node 3 already holds epoch 2's artifact in its store, so `recover` takes the
`Some(set)` branch. The smoke's arm (`ArtifactForShare`, artifact never in the store) is
covered by one unit that never delivers the artifact (`actor.rs:11896-11942`), leaving the
`key_held_share(None)` transition untested end to end.

## Unit tests (Q7)

`[KNOWN]` The three renamed units assert the new cell, not merely "not sat_out":
`..._waits_for_the_artifact_to_heal` → `acquiring_artifact_for_share`, no ceremony, no
journal, `{NoArtifact}` after ticks, pull asked (`actor.rs:11896-11942`);
`..._evicts_it_and_heals_as_a_player` → `acquiring_logs`, `load_journal == NoFile`,
`want.len() == pinned.len()`, nothing sent (`12257-12307`); `..._heals_not_deals` →
`acquiring_artifact_for_share`, no journal, no latch (`13794-13845`). The reveal unit's
fixture builds a genuinely pinned set and self-checks that every dealer log reveals node 0
(`actor.rs:11956-12003`, `11975-11981`), then asserts the stored share passes
`validate_share_on_poly` (`12083-12086`). The acked unit builds four `check`-valid logs
from `node0_pre_seal_journal_full_sealed` and observes `Unrecoverable`
(`12105-12222`) — it proves the withholding only *indirectly* (by the outcome), which is
sound because a reveal would have produced `Ok` instead.

## Hygiene (Q8)

`[KNOWN]` No new `#[allow]`; `git diff | grep -E '^\+.*(unwrap\(|expect\(|panic!|todo!|unimplemented!)'`
shows every hit inside `#[cfg(test)]` modules (`clock_tests`, `testbed`); no new `unsafe`;
the only new `pub` is `Outcome::signable` on a `pub(super)` struct (`stand.rs:890`, `917`).
The two new `warn!` in `recover` are **per epoch decision, not per tick**: `decide` guards
on `epochs.contains_key` (`actor.rs:2950`) and `recover` returns `Some`, so the slot is
inserted and `recover` is not re-entered for that epoch. New comments were trimmed after
the fact (journal §0.10) but the new test docs still carry design-cell and history labels
(`Cell (NoFile, h ≥ seal)`, `§5.2 restart table`, `Before 5.3-А1`, `(B4″)`) — the
workspace style bans document/history references by shape, though this file's baseline is
comment-heavy.

## Findings

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| D-01 | SERIOUS | `devnet/local-dpos-smoke/dpos_harness/cases/smoke/asserts_prod_dkg.py:187-202,355,415-419,665-677`; `verdicts_rotation.py:1047-1086` | The devnet smoke harness still asserts the removed semantics: `assert_sat_out_torn` + `evaluate_shareless`/`evaluate_did_not_promote` for the torn victim (durability phase 3), and the halt case requires both torn victims shareless so the committee stays below dealer-quorum (`:355`, `:415-419`). After this change a post-seal `Torn` victim heals (or becomes `Unrecoverable`); if it never acked, `evaluate_shareless` goes red. The harness was not updated (devnet was outside the implementer's writable list). | The victims are stopped/restarted around the deal window and very likely acked at least one dealer, in which case the heal ends `Unrecoverable`, `share_computed` stays false and the old assertions still pass; the tear could also land pre-seal, which is unchanged. The halt case's "Torn sits out unconditionally" doc is already false at base (`actor.rs:3275-3290` re-deals pre-seal). I did not run the smoke. | inferred by code |
| D-02 | SERIOUS | `actor.rs:2879-2884` vs `devnet/.../verdicts_fault.py:714,824-841,844-895`; caller `asserts_fault.py:640-647,667` | The stated purpose (make `smoke-vrf-dkg-live-heal` pass) is not achieved by the code alone. The case's hard precondition gate `evaluate_victim_held_nothing` reads the heal line's `want`/`dealers` pair (`heal_start_counts` regex `\bdealers=(\d+)`), but `heal_over` logs `want=` and `pinned=`; with the new design the victim takes road B (heal) and `fresh` is empty, so `counts is None` → the gate returns False. The field was renamed `dealers` → `pinned` in `cee2e037` without updating the harness. | Maybe the harness's road-B reader is only informational and the smoke takes road A; but road A needs `start_fresh`, which is forbidden post-seal, and `docs`/journal say the case is post-seal. Maybe a harness copy elsewhere logs `dealers=`; `git grep` finds one emitter. I did not run the smoke. | confirmed field mismatch (code); smoke failure inferred |
| D-03 | MODERATE | `testbed/tests.rs:2028-2175`; `actor.rs:11896-11942` | The mandatory stand test exercises only the artifact-already-held arm (`heal_over`). The arm the live smoke takes — `recover` finds no artifact → `ArtifactForShare` → artifact arrives → `key_held_share(share=None)` (`actor.rs:2906`, `1745-1755`, `1821-1826`) — has no end-to-end test; the only unit stops before artifact delivery. | The `key_held_share(None)` branch is a one-line pre-existing transition, and `ArtifactForShare`'s pull path is pre-existing; the risk is low. | confirmed by code |
| D-04 | MODERATE | `actor.rs:3256-3265`; `share_state.rs:528-534,590-636` | `evict_journal` on a `Torn` file is irreversible, and `Torn` also covers an *undecryptable* first record (encrypted frame with no/wrong seal key, `share_state.rs:528-531`), not only corruption. A process restarted without its keystore loses the journal permanently even though the bytes are intact. Pre-seal already evicted, but the change extends the eviction past the seal where the node may have sealed. | The records are unreadable to this process either way, and the share is in the share file, not the journal; the pre-seal arm set the precedent. | confirmed by code |
| D-05 | MINOR | `metrics.rs:52-53,65-76,87` | `StallReason::SatOut` now has no producer; its doc ("A damaged or absent journal at/after the seal deadline: sat out (R-036)") is false and the `dpos_dkg_stalled{reason="sat_out"}` gauge is a permanently-zero registered series. | Keeping the variant avoids renumbering/removing a registry label; the journal records removal as a follow-up and `metrics.rs` was outside the writable list. No reader of the label exists in `crates`/`devnet`/`.claude`. | confirmed by code |
| D-06 | MINOR | `actor.rs:2963,3794,4461-4463`; `share_state.rs:560-566,584-586,629-632` | Stale prose that now describes a state that cannot happen: `decide`'s "Not dealing (sat out, ...)", `drive_acquisition`'s "a sat-out / unrecoverable member", the `INTERVAL` doc "at/after the deadline it sits out" naming the *renamed* test `..._sits_out`, and `JournalLoad`'s "the caller SITS OUT"/"must SIT OUT" for `Torn`. The touched file's own docs were only partly updated. | Pre-existing comments are "not yours to remove", and the surrounding code is unchanged; but line 4463 now names a test that no longer exists, and line 3794 is in the doc of a function the change semantically extends. | confirmed by code |
| D-07 | MINOR | `testbed/tests.rs:2064-2068` | The assertion message is inverted: `dealers_of(&artifact1) == 3` means the set does NOT include the absentee's log (4 committee members), but the message says "PK_2 was minted over a set that includes the absentee's log". A reader is told the opposite of what is asserted. | Diagnostic text only; the assertion itself is correct. | confirmed by code |
| D-08 | MINOR | `testbed/tests.rs:2063-2068` | The fixture establishes the absentee only by the *count* (3 of 4 logs); it never asserts that node 3's pubkey is the one absent from `artifact1.logs`. A 3-log set missing a different member would satisfy the guard. | The cut isolates only node 3 for the whole window and the test also asserts the epoch-2 WARN and `signable[3]`, so the count is sufficient in context. | confirmed by code |
| D-09 | NIT | `testbed/tests.rs:2103,2126-2129` | The anti-vacuity guard greps `"no ceremony journal at or after the seal deadline"`, a substring present in BOTH the old `SatOut` warn and the new heal warn, so it cannot distinguish the implemented cell. The discriminating assertions are `!timed_out`, `signable[3]` and `ceremony_ok == 1`. | The guard's stated job is only to prove the cell was hit, not which arm; the other asserts carry the load, and `ceremony_ok == 1` plus liveness would fail under the old code. | confirmed by code |
| D-10 | NIT | `dsh-input-journal.md:5,91,95` vs `git status`; `.gitignore:47` | The journal lists `.claude/dpos_architecture/00_preamble.md`, `08_*.md`, `DPOS_ARCHITECTURE_CHANGELOG.md` as edited, and the task mandated those doc updates in the same change. The tracked diff is only the 3 `crates/` files; `.claude` is gitignored and absent from this workspace, so the doc edits cannot be confirmed or refuted here. The design's old cell is still in `.dpos-study/history/E5-BEACON-DESIGN.md:612,616,637` (correctly out of scope, and deliberately not edited). | The implementer's working tree may contain the (gitignored) docs; the review workspace simply lacks them. | confirmed for the tracked diff; unverifiable for the docs |
| D-11 | NIT | `actor.rs:1745-1755`, `3845-3856` | When the artifact is *held* but `committee_for` fails, the phase stays `ArtifactForShare` and `drive_acquisition` raises `Stalled{NoArtifact}` and re-pulls: the diagnostic names a missing artifact that is present, and spends pull budget on a local read race. Pre-existing shape, now newly reachable from the absentee path. | `pull_artifact` is throttled/deduped (`actor.rs:3803-3807`) and the latch is once-per-epoch; the store reconcile fixes it on the next successful committee read. | confirmed by code |
| D-12 | NIT | `actor.rs:12005,12257-12259,13794-13800` | New test docs cite design cells and change history (`Cell (NoFile, h ≥ seal)`, `§5.2 restart table`, `Before 5.3-А1`, `(B4″)`), which the workspace comment rule bans by shape. | The file's existing tests all follow this style; the rule says match the human baseline, and the author trimmed the count from 121 to 54. | confirmed by code |
| D-13 | NIT | `actor.rs:4067-4072`, `4087-4095` | `PersistFailed` re-arms `attempted = false`, so `try_recompute` retries and `adopt_share` emits an ERROR every height tick while the disk keeps failing (bounded only by the epoch's sweep). Newly reachable from the absentee heal. | This is the documented §5.4 retry ("the disk, not the inputs, is what failed") and is identical on the pre-existing heal paths; bounded by `JOURNAL_RETENTION_EPOCHS`. | confirmed by code |

## Leave as is

1. **The heal emits nothing** (traced in §Q1): no `Outgoing` is ever produced on the new
   arms, `recompute_scoped` throws its re-derived acks away, and `ceremonies()` excludes
   both new phases — so no dealing, ack, seal or share-confirmation can leave the node.
2. **A sealed node cannot re-seal**: `start_fresh` is gated on `!past_seal`, and the seal
   predicate equals `past_seal`; `evict_journal` loses no share (separate file, checked
   first) and no readable journal record (`Torn` = first-record failure, parse breaks).
3. **The acked-absentee case is terminal once**: `MissingPlayerDealing` → `Unrecoverable`
   + one latch, `attempted`/`want` prevent refetch and recrypto, and the fetch is bounded
   by the 8-epoch sweep; the two new `warn!` are per epoch decision.
4. **Non-members cannot enter the heal**: the membership test returns `KeyOnly` /
   `ArtifactForKey` before `load_journal`.
5. **The `ArtifactForShare`-with-no-share transition is the same state `heal_over`
   produces**, and `needs_artifact()` keeps pulling; the artifact is not silently waited on.
6. **The stand test is non-vacuous by construction**: the share root is a real temp dir,
   `beacon::build` reloads from disk, the files are asserted to exist before deletion, the
   replay reconstructs the node, and liveness with node 1 cut requires node 3's signature.
7. **No new `unwrap`/`expect`/`panic!` on production paths, no `#[allow]`, no new
   production `pub`, no `unsafe`.**

## Weakest points of this review — ranked (Q9)

1. **The devnet verdicts (D-01/D-02).** I read the harness predicates and the product log
   fields, but I did not run the smoke and cannot know which harness gate fires first in a
   live run; the two SERIOUS findings are code-level mismatches whose *runtime consequence*
   is inferred. This is the weakest area and the one most worth a human check.
2. **The stand test's red-before-fix.** I did not run cargo (forbidden), so the claim that
   `!timed_out` fails without the heal rests on the implementer's journal §0.5 and on the
   quorum arithmetic, not on my own run.
3. **`ArtifactForShare` end-to-end.** I traced the transition by reading, but no test
   delivers an artifact to an absentee, so the smoke's exact path is unproven here.
4. **Reachability of `Err(other)` in `try_recompute` for an absentee.** I confirmed the
   latch/`attempted` bound by reading, not by enumerating every error path through
   `recompute_scoped`/`Player::resume`/`finalize`.
5. **The `Torn` "wrong keystore" eviction risk (D-04).** I established the code path, but
   not how often a real restart opens the journal without its seal key.

## Assumptions

- "Broadcast" is read as any outbound DKG message (dealing, ack, seal, share-confirmation);
  artifact pull requests are inbound reads and not counted.
- The design doc (`E5-BEACON-DESIGN.md`) is context, not evidence; its `SatOut` cell is
  deliberately not edited, per the brief.
- The pre-fix/red and green cargo transcripts in `dsh-input-journal.md` are treated as
  `[LIKELY]` claims I could not reproduce (no cargo allowed).

## Commands / evidence

`git log/status/diff HEAD`; `git show HEAD:...actor.rs`; `git grep sat_out|sat out`;
`grep`/`read` over `actor.rs`, `share_state.rs`, `log_store.rs`, `ceremony.rs`,
`metrics.rs`, `stand.rs`, `tests.rs`; `read` of the pinned commonware
`cryptography/src/bls12381/dkg.rs:1700-1875`; `read` of the devnet harness
`asserts_prod_dkg.py`, `asserts_fault.py`, `verdicts_fault.py`, `verdicts_rotation.py`.
No cargo command was run.
