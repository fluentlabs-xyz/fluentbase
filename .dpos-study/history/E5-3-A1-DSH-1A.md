# Independent code review — А1 explicit DKG ceremony automaton

Round 1, focus A: state coverage, transitions, `recover`, artifact input, body-lost. Change under
review is exactly `git diff HEAD` (8 files, ~6200 diff lines): `beacon/{actor,artifact,ceremony,
dkg_engine,metrics,plane,share_state}.rs` + `testbed/tests.rs`. Reviewer read the whole diff, the
new `actor.rs` production region `:1-3568`, the old `HEAD` versions of the changed functions, the
new `artifact.rs`/`dkg_engine.rs`/`ceremony.rs` regions, and the four input documents.

Method / limits: read-only; `cargo` was **not** run (no build cache, per brief). Every claim below
was checked against the code text; line numbers are the current worktree. Test-suite and mutation
claims in the journal could not be reproduced and are not treated as evidence. The new
`.claude/dpos_architecture/*` doc updates are not in `git diff HEAD` (untracked/gitignored) and are
not verifiable here.

Severity scale used: BLOCKER / SERIOUS / MODERATE / MINOR / NIT. Confidence: `confirmed by code`
(claim is directly visible) or `inferred` (mechanism read, reachability argued).

---

## Q1 — State coverage: rebuild S0–S17 → `EpochState`

Types: `EpochState` `actor.rs:282-333`, `Acquire` `:257-272`, `EpochSlot` `:411-427`,
`AgreedSet` `:238-252`, `RecomputeState` `:225-232`. `Idle` is the absent slot (`:274-276`,
`:1284-1290`), as the journal says.

| S | inventory fact | home in the new machine | code |
|---|---|---|---|
| S0 | unreachable / bufferable | absent slot + `pending` (input buffer) | `is_bufferable` `:2803-2820` |
| S1 | undecided (committee/bit unreadable) | absent slot; `recover` → `None` | `:2608`, `:2612`, `decide` `:2469-2475` |
| S2 | carry-forward | `KeyOnly { digest: None }` | `:2609-2611` |
| S3 | not a member | `KeyOnly { digest: Some }` / `Acquiring(ArtifactForKey)` | `:2616-2626` |
| S4 | Dealing | `Dealing { ceremony, agreed }` | `:2680-2681`, `:2715-2733` |
| S5 | Sealed / dealing closed | `Sealed` (E>now) or `Acquiring(ArtifactForCeremony)` (E≤now) | `:2682-2686` |
| S6 | Pinned / deferred sub-states | `Agreed { ceremony, set }` + `slot.stalled` latches | `:2279-2295`, `:1466-1479` |
| S7 | Keyed | `Keyed { digest }` (share stays in `store`) | `:2589-2591`, `:2363`, `:3365` |
| S8 | finalize-Err, player spent | split: `Unrecoverable` / `Acquiring(Logs)` | `:2373-2403` |
| S9 | Refused (off-poly / persist) | `Acquiring(Logs(RecomputeState))` | `:2365-2371`, `:2414-2437` |
| S10 | SatOut-Torn | `SatOut { key }` only at `h ≥ seal` | `:2645-2653` |
| S11 | SatOut-ResumeErr (no memory) | `Unrecoverable { key }` (memory) | `:2639-2643`, `:2775-2790` |
| S12 | Recompute | `Acquiring(Logs)` | `:2414`, `:3275-3372` |
| S13 | Unrecoverable | `Unrecoverable { key }` | `:3313-3331` |
| S14 | partial success | `Acquiring(ArtifactForShare)` → `Keyed` | `:2592-2597`, `:1480-1482` |
| S15 | swept | slot removed + latches cleared | `:1758-1813` |
| S16 | non-member waits for artifact | `Acquiring(ArtifactForKey)` | `:2622`, `:3208-3243` |
| S17 | Unfrozen | out of actor (plane geometry wait), unchanged | `plane.rs:840-850` |

Result: every S-state has exactly one home; no S-state is homeless. The states that lost a
distinction relative to HEAD:

- **S2 vs S3** are one variant `KeyOnly`; the distinction that HEAD still needed (carry-forward vs
  non-member-with-key) survives in `digest: Option<B256>` (`:310-312`), so it is not lost.
- **S8 vs S9** both funnel to `Acquiring(Logs)`; the distinction HEAD carried (`agreed_pinned`
  retained vs removed) is no longer needed because the healed `AgreedSet` is passed by value into
  `heal_over` (`:2335-2369`, `:2414-2437`). Not lost.
- **S6 sub-states** (`missing_body`/`below_quorum`) become latches in `slot.stalled`
  (`:2289-2294`); the distinction survives but only as a `BTreeSet<StallReason>` side field, not as
  a phase. Acceptable.

No finding from Q1 itself. The coverage table in the journal is accurate against code.

---

## Q2 — Transitions vs design §5.2, row by row

| §5.2 row (design `:5-19`) | implemented? | code |
|---|---|---|
| `Unfrozen ─(geometry)─▶ Idle` | plane, unchanged | `plane.rs:840-850` |
| `Idle ─(h≥start(E−1), change∨BOOTSTRAP, me∈C)─▶ Dealing` | yes for the only epoch `decide` is driven for (`now+1`); the `h≥start(E−1)` precondition is implicit in `E=now+1`. **But `on_artifact` can drive `recover(E)` for E≫now+1 and start a ceremony early** — see DA-07 | `:2449-2464`, `:2469-2532`, `:1397-1415` |
| `Dealing ─(h≥seal)─▶ Sealed ─(certified)─▶ Agreed` | yes | `:1861-1897`, `:1466-1479` |
| `Sealed ─(body lost)─▶ Acquiring{artifact}` | yes, via the new channel | `:1330-1351`, `dkg_engine.rs:406-408` |
| `Sealed ─(committee unreadable)─▶ Acquiring{artifact}` | **not implemented** (Д-А1-3: retried by announce; the epoch is not even seated without a readable committee) | `:2612`, `:1558-1597` → DA-08 |
| `Agreed ─(all bodies held)─▶ Finalizing ─Ok─▶ Keyed` | yes | `:2255-2299`, `:2335-2364` |
| `Agreed ─(bodies missing)─▶ Acquiring{logs} ─▶ Finalizing` | **not implemented literally**: stays `Agreed` + `Stalled{BodyMissing}` and fetches via the resolver; `Finalizing` is not a resting state (Д-А1-8) | `:2280-2295`, `:3104-3129` → DA-09 |
| `Finalizing ─Err(MissingPlayerDealing)─▶ Unrecoverable` | yes | `:2373-2393` |
| `Finalizing ─Err(other)─▶ Acquiring{logs}` | yes | `:2394-2403` |
| any state ─(2nd quorum artifact)─▶ `Conflict` | yes for phases that already carry a digest; a phase with no digest adopts the first artifact. Gap when the first artifact is in the local store but was never delivered to the actor → DA-01 | `:1446-1451`, `:1515-1549` |
| `me∉C ∨ ¬change: KeyOnly ─▶ Acquiring{artifact} ─▶ Keyed{no share}` | yes (`KeyOnly` carries the digest instead of a distinct `Keyed{no share}`) | `:2616-2626`, `:1483-1488` |
| `Torn, h<seal ─▶ Dealing` (reconstruct) | yes, torn file evicted first | `:2663-2673` |
| `Torn, h≥seal ─▶ SatOut` | yes | `:2645-2653` |
| `NoFile after seal ─▶ SatOut` (R-036) | yes | `:2654-2662` |
| persist Err share ─▶ not Keyed, `Stalled{PersistFailed}`, clock retry | yes | `:2365-2370`, `:1681-1704` |
| persist Err artifact ─▶ accepted in RAM, write retried | store-side, unchanged | `artifact.rs:485-513` |

Deviations Д-А1-1…12, judged:

- **Д-А1-1** new `body_lost` channel and `Option` seam. Justified (design explicitly permits a new
  channel). Weakens the "immediately" guarantee because delivery is `try_send`-lossy — DA-05.
- **Д-А1-2** recovered past-boundary epoch without artifact → `Acquiring(ArtifactForCeremony)`.
  Justified and required: no instance runs for a past epoch, so `Sealed` would hang. Necessary.
- **Д-А1-3** "committee unreadable" stays an announce retry. Justified as parity, but it *does*
  leave the design row unimplemented and issues no pull — DA-08.
- **Д-А1-4** no re-broadcast on a past-boundary resume (`:2690`). Justified; discarded acks are
  useless (dealers sealed) and the node's own log is re-fetchable by pinned hash.
- **Д-А1-5/Д-А1-10** artifact before own seal → `Dealing{agreed:Some}`. Necessary for a lagging
  clock; the set is the artifact's, so the finalize scope is unchanged once sealed.
- **Д-А1-6** `SatOut`/`Unrecoverable` keep the digest and keep pulling. Justified (I4: the key is
  needed to verify the epoch's certificates).
- **Д-А1-7** `Sealed` with E≤now → `Acquiring(ArtifactForCeremony)` on the tick. Necessary; the
  journal's C8 evidence is plausible and I could not falsify it. It is the replacement for the old
  boundary pull.
- **Д-А1-8** `Finalizing` not a resting state. Correct: `finalize_over_pinned` is synchronous and
  the ceremony is consumed, so a resting `Finalizing` would need a new owner for the consumed
  player.
- **Д-А1-9** `Idle` = absent slot. Correct, no distinction lost.
- **Д-А1-11** `INTERVAL=30` in unit tests. This is the item that removes the old zero-dealing-window
  geometry and rewrites the pre-existing fixtures — DA-12.
- **Д-А1-12** `withhold_ack`. Correct fix (the old `try_ack` cache re-emitted an ack after a failed
  `ReceivedDealing` write); it is a new production behaviour change in `ceremony.rs`, not just a
  machine refactor, but it closes a real gap.

Design guarantee that is genuinely weakened: the "second quorum artifact ⇒ Conflict" is mediated by
the full proposal digest and by the actor's own copy, not by the store owner — DA-01 (and the
digest-includes-`confirms` variant DA-02).

---

## Q3 — No-exit states

Exit per variant (`needs_artifact` `:392-402`, `drive_acquisition` `:3208-3244`):

- `Dealing` → seal deadline (`:1861-1897`).
- `Sealed` → artifact (`:1466`), body-lost (`:1340`), or boundary tick (`:3214-3228`).
- `Agreed` → `drive_finalization` (`:2325-2406`).
- `Acquiring(ArtifactForCeremony)` → artifact → `Agreed` (`:1473-1479`); pulls every tick; sweep.
- `Acquiring(ArtifactForShare)` → artifact → `Keyed` (`:1480-1482`); pulls every tick; sweep.
- `Acquiring(ArtifactForKey)` → artifact → `KeyOnly` (`:1483-1488`); pulls every tick; sweep.
- `Acquiring(Logs)` → `try_recompute` → `Keyed`/`Unrecoverable` (`:3275-3372`); retryable errors
  re-loop (R-038, intentionally out of scope); sweep.
- `Keyed`/`KeyOnly`/`SatOut`/`Unrecoverable`/`Conflict` → sweep (`:1758-1813`); `SatOut{None}` /
  `Unrecoverable{None}` additionally pull (`:392-402`).

There is **no new state that HEAD could leave and this machine cannot**: every HEAD exit
(`on_height` seal, `drive_recompute` heal, `drive_finalization` retry, `adopt_share` refusal retry,
sweep) has an equivalent transition. `Acquiring(Logs)` with a body no peer holds and no resolver is
a pre-existing stall (HEAD's `recompute_pending` had the same shape) and is bounded by the sweep.
`Acquiring(ArtifactForShare)` is a deliberate liveness trade already named in §5.4 (loss of the
local copy of PK_E from the share file). So: no Q3 finding.

One gauge staleness (not a state exit): latches are cleared only on `Keyed` (`:1274-1278`) and on
sweep (`:1777-1779`), so `Stalled{BodyLost}`/`Stalled{NoArtifact}`/`Stalled{QuorumMissing}` can
outlive the condition on an epoch that later lands in `Acquiring(Logs)` or `KeyOnly` — DA-14.

---

## Q4 — `recover(E)`: restart table, window, exactly-once

`recover` `:2576-2692`. Rebuilt table (compare design `:21`):

| cell (share × artifact × journal × h vs seal) | code | matches design? |
|---|---|---|
| share, artifact | `Keyed{digest}` `:2589-2591` | yes |
| share, ¬artifact | `Acquiring(ArtifactForShare)` `:2592` | yes |
| ¬share, ¬mint | `KeyOnly{None}` `:2609-2610` | yes |
| ¬share, ¬member, artifact / ¬artifact | `KeyOnly{Some}` / `ArtifactForKey` `:2618-2624` | yes |
| ¬share, member, Present, h<seal | resume reconstruct → `Dealing{agreed}` `:2640,:2681` | yes |
| ¬share, member, Present, h≥seal, E=now+1 | `Sealed` `:2683` | yes |
| ¬share, member, Present, h≥seal, E≤now | `Acquiring(ArtifactForCeremony)` `:2684-2686` | deviation Д-А1-2, justified |
| ¬share, member, Present, artifact held | `Agreed` `:2682` (then finalize) | yes |
| Torn/NoFile, h<seal | fresh `Dealing` (torn evicted) `:2663-2679` | yes |
| Torn, h≥seal | `SatOut` `:2645-2653` | yes |
| NoFile, h≥seal | `SatOut` `:2654-2662` | yes |
| resume/start Err | `Unrecoverable{key}` `:2642,:2672,:2677` | yes |

Window: `decide_window` `:2449-2464` is `[max(BOOTSTRAP, now−JOURNAL_RETENTION_EPOCHS), now] ∪
{now+1}`, with `JOURNAL_RETENTION_EPOCHS = SCHEME_RETENTION_EPOCHS = 8` (`mod.rs:137`,
`lib.rs:37`). `now+1` (not `now+2`) is right: `now+2` frames are the peer-ahead ingress window
(`INGRESS_LOOKAHEAD_EPOCHS=2`, `:164-177`) and are only *buffered* (`is_bufferable` `:2803-2820`);
the deal decision is anchored at `h ≥ start(E−1)`, i.e. `E = now+1`. Deciding `now+2` would start a
ceremony an epoch early and duplicate the peer's deal window. The sweep floor `e+8 ≥ now`
(`:1767`) equals the low bound, so `recover` never re-seats a swept epoch.

Exactly once: `decide` short-circuits on `self.epochs.contains_key(&epoch)` (`:2470`), the slot is
created only through `enter`, and after a sweep the epoch is below `lo` forever (monotone `now`)
and can never be the `now+1` target again. Confirmed by code.

---

## Q5 — Artifact input (`on_artifact` / `apply_artifact` / bridge)

- The quorum check + bridge: `ArtifactBridge::deliver` verifies with
  `verify_artifact_for_epoch` before `adopt`/insert (`artifact.rs:1549-1575`), and the actor's
  channel precondition is documented at `actor.rs:779-787`. The new divergent-`adopt` at
  `artifact.rs:1596` is therefore downstream of the check. Confirmed: **a single forged artifact
  cannot reach `Conflict`**; the test path `agreed_artifact` in `actor.rs:11408-11430` confirms the
  intended contrast.
- **Same-digest duplicate cannot create `Conflict`**: `apply_artifact` compares
  `held_digest()` first (`:1446-1451`), returns `false` on equality, and the `Conflict` arm only
  warns for a third digest (`:1433-1445`). Confirmed.
- **`Conflict` key is the full `DkgProposal::digest()`, which includes `confirms`**
  (`dkg_agree.rs:455-461`, write at `:560-570`). So two certificates over identical `logs` and
  `group_key` but different confirmation metadata are "different" and would trigger `Conflict`
  even though the pinned set and key are identical. DA-02.
- **A `Keyed` epoch on a conflicting artifact**: `conflict()` (`:1515-1549`) removes the share from
  the shared `store`, fires `share_notify`, and lands in `Conflict{held,second}`; the recorded logs
  were already in `log_store` from the finalize, so serving survives. Correct per design.
- **Gap (the important one)**: `ArtifactBridge::deliver` keeps the *held* artifact in the store but
  hands the *second* to the actor (`artifact.rs:1586-1600`); the actor's `held_digest` lives only
  in the actor. If the first artifact's `adopt` was lost — `adopt` is a `try_send` that warns and
  drops on `Full` (`artifact.rs:1640-1656`) — the store holds A while the actor is still
  `Sealed`/`Dealing`/`Acquiring(ArtifactForCeremony)` (no digest). A later divergent B is then
  applied as the actor's first artifact, `finalize_over_pinned` runs over B's set, and the share is
  adopted on B's polynomial while `ArtifactStore`/`KeyIndex` keep serving A as `PK_E`. No
  `Conflict` is entered. This is the F-02 harm class (a share that does not match the key this node
  serves), reached through a lost hand-off rather than a bad ceremony. DA-01.
  Refutation attempt: for the store to hold A without the actor, A must have come in on the
  `try_send` path (the instance path uses an awaited `send` through `spawn_write_back`,
  `plane.rs:479-489`, so it is reliable); `try_send` returns `Full` only under ≥16 queued edge
  events, and B additionally requires a second quorum-certified artifact (≥2q−n signers), i.e. the
  network is already in the case the design wants `Conflict` for. So it needs two rare events, but
  when they coincide the machine does the *wrong* thing (adopt B, keep signing) instead of stopping.
- Re-pull cannot heal the lost hand-off: `ArtifactPull::pull` returns `Have` from the local store
  without calling `bridge.adopt` (`artifact.rs:1770-1785`), so an actor stuck in a `needs_artifact`
  phase with the artifact already local spins on local hits until the sweep. Pre-existing (HEAD had
  the same local-hit short-circuit), but it is what makes DA-01 unrecoverable rather than
  self-healing.

---

## Q6 — Body-lost channel

- Producer: `dkg_engine.rs:394-408`, `let _ = tx.try_send(target_epoch)`; the epoch is the
  agreement's `target_epoch` (correct). Consumer: `run` `actor.rs:1157-1160` → `on_body_lost`
  `:1330-1351`, sync, so the actor drains promptly.
- `try_send` on `EDGE_MAILBOX = 16` (`plane.rs:89,735`): on `Full` the signal is silently dropped
  (no retry, no counter). The epoch then waits for the boundary: `drive_acquisition` only converts
  `Sealed(E>now)` to `Acquiring(ArtifactForCeremony)` and pulls once `E ≤ now` (`:3214-3242`).
  So "immediately" is best-effort. The comment at `dkg_engine.rs:404-405` states this. DA-05.
- `Option` seam: `body_lost_rx.take()` in `run` (`:1097`); when the launcher's sender drops, the
  arm sets `body_lost_rx = None` and parks (`:1159`). Fine.
- Signal for an epoch the actor has no slot for: `on_body_lost` checks `state(epoch)` is `Sealed`,
  else debug-logs and returns (`:1331-1338`). No panic, no phantom slot. Not a bug. (In-process the
  instance only starts after the actor announced a `Sealed`/`Agreed` slot, so the lost-signal-
  before-first-tick race cannot arise.)
- `Acquiring(ArtifactForCeremony)` created by body-lost keeps the ceremony, so `serve_log` still
  serves its recorded logs (`:3422-3428`). Good.

---

## Q7 — Where my review is weakest (ranked)

1. **No execution.** No `cargo`/tests/mutations run; every coverage statement (e.g. "the 11 new
   tests hit these cells") is read, not observed. The journal's own mutation results are unverified.
2. **Reachability of DA-01.** The mechanism is confirmed by code, but the frequency of a lost
   `adopt` co-occurring with a divergent second artifact is an argument, not a measurement.
3. **`dkg_agree`/agreement internals.** I read `DkgProposal::digest`, `resolve_artifact` and the
   certificate pairing, but not the full agreement protocol, so I cannot fully exclude other ways
   two distinct digests for one epoch arise (the `confirms` question in DA-02 is the main one).
4. **`testbed/stand.rs`.** Not read in depth (only the 8-line doc-only diff of `testbed/tests.rs`);
   a stand-visible liveness regression would show up there and I cannot rule it out.
5. **The `outcome_at = None` (test/default) seam.** I traced the production wiring (`plane.rs`
   always `Some`), but the tests that build the actor with `None` and then inject shares are only
   spot-checked.

---

## Findings

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| DA-01 | SERIOUS (BLOCKER-candidate) | `actor.rs:1446-1451,1456-1496,2586-2596`; `artifact.rs:1586-1600,1640-1656,1770-1785` | A lost artifact hand-off (`ArtifactBridge::adopt` `try_send` `Full`/`Closed`) leaves the store holding A while the actor has no digest. A later divergent B is adopted as the actor's first artifact, so the share is finalized/adopted on B's polynomial while `ArtifactStore`/`KeyIndex` serve A as `PK_E`, and no `Conflict` is raised (the actor compares B only against its own digest, never the store's). The local re-pull cannot heal it because `ArtifactPull::pull` short-circuits on the local store without re-adopting. | The store can only lead the actor on the lossy `try_send` path (the instance path awaits delivery), and B requires a second quorum-certified artifact. Both are rare and the network is already >f-Byzantine, so this is a narrow window; under a single artifact the store and actor always agree. | confirmed by code (mechanism); inferred (reachability) |
| DA-02 | MODERATE | `actor.rs:1432,1446-1448`; `dkg_agree.rs:455-461,560-570` | `Conflict` is keyed on `DkgProposal::digest()`, which covers `confirms` as well as `logs`/`group_key`. Two certificates over the *same* pinned set and key but different confirmation metadata are treated as a conflicting pair, so an honest-but-divergent second instance would stop the epoch's signing and drop the share. | `confirms` is part of the payload the instance votes on, so two distinct digests really are two certified values; and two instances for one epoch are already anomalous (launcher `started`). The design's trigger is "second quorum artifact", which the code follows literally. | confirmed by code (digest covers confirms); inferred (reachability) |
| DA-03 | MINOR | `actor.rs:2300-2312` | The unmappable-pinned-set `error!` is emitted **every height tick** for the whole retention window: it sits before the latch gate (`stall` at `:2323`) and is not guarded by `slot.stalled`. HEAD gated this line with `deferred_reported`. A pinned set whose indices exceed the committee now floods the log once per tick per epoch. | The paired `dkg_pinned_idx_out_of_range` counter was already per-tick in HEAD, so the metric is not a regression; only the log line is. The condition should be impossible in honest operation. | confirmed by code |
| DA-04 | MINOR | `actor.rs:2335,2366-2367,1289,1305` | The `PersistFailed` stall on the live finalize path logs the wrong `state`: `take_state` has already swapped in the placeholder `Unrecoverable{key:None}`, so `stall()` reports `state=unrecoverable` for a persist failure. The latch/gauge are correct; only the log field is wrong (and the placeholder is observable to any future reader between the take and the set). | Only this one stall site runs between `take_state` and `set_state`; all other stall sites run after `enter`/`set_state`. The reason label is correct, so an operator sees the real cause. | confirmed by code |
| DA-05 | MINOR | `dkg_engine.rs:406-408`; `actor.rs:1157-1160,3214-3242` | The body-lost signal is `try_send`-lossy with no retry and no drop counter. If it is lost, a `Sealed` epoch does not move to `Acquiring(ArtifactForCeremony)` and does not pull until `E ≤ now` — a full epoch later (or never, on a halted chain). The design's "немедленно" is best-effort. | The fallback exists (`drive_acquisition`), and the channel is 16 deep with one send per epoch; the actor drains it synchronously. On a halted chain HEAD also waited for the boundary. | confirmed by code |
| DA-06 | MINOR | `actor.rs:2365-2370` | A live-finalize `adopt_share` refusal with `AdoptRefusal::OffPolynomial` moves the epoch to `Acquiring(Logs)` with **no** `Stalled{...}` event, unlike the `PersistFailed` arm. The condition (local finalize produced a share off the certified polynomial) is exactly the F-02 class and is surfaced only by an `error!` line + `dkg_share_off_polynomial`. | The metric and error line are emitted in `adopt_share` (`:1667-1678`), so it is observable; the design's §5.4 event list does not name this arm. | confirmed by code |
| DA-07 | MINOR | `actor.rs:1397-1415`; compare `:2449-2464`, `:164` | `on_artifact` calls `decide(epoch)` for any epoch with no slot, and `recover` has no `h ≥ start(E−1)` / lookahead check. A quorum-certified artifact for an epoch far above `now+2` (or `now+2`) therefore starts a fresh `Dealing` ceremony immediately, bypassing the §5.2 `Idle → Dealing` gate and the actor's own `[now, now+2]` ingress window. HEAD's `on_artifact` stored the set but never started a ceremony. | Producing a far-future certified artifact requires a quorum agreement for that epoch (beyond the f-fault model); for `now+2` it merely starts the (deterministic, seeded) dealer an epoch early, and the seal still happens at the normal deadline. | confirmed by code (mechanism); inferred (impact) |
| DA-08 | MINOR | `actor.rs:2612`, `:1558-1597` | Design §5.2 row `Sealed ─(committee unreadable)─▶ Acquiring{artifact}` is not implemented (Д-А1-3). If `committee_for(E)` stays unreadable for a member, `recover` answers `None`, the epoch is never seated, nothing pulls, and no `Stalled` is raised — the design's explicit pull outcome is missing. | It is parity with HEAD (`maybe_start` also required a readable committee) and the read is transient; the launcher re-announces each tick once the record is readable. | confirmed by code |
| DA-09 | MINOR | `actor.rs:2280-2295`, `:3104-3129` | Design §5.2 row `Agreed ─(bodies missing)─▶ Acquiring{logs} ─▶ Finalizing` is not implemented literally: the epoch stays `Agreed` with `Stalled{BodyMissing}` and fetches pinned bodies through the resolver. `Acquire::Logs` is reserved for the journal-recompute heal. | Functionally the same fetch-and-wait; the journal's transition table discloses it. `Acquiring(Logs)` would be wrong here because that mechanism recomputes from the journal rather than finalizing the live ceremony. | confirmed by code |
| DA-10 | MINOR | `actor.rs:3236-3238`; §5.4 row "Локального артефакта нет (любая причина)" | `Stalled{NoArtifact}` is raised only for `E ≤ now`; before the boundary a missing artifact is silent. The design puts the stall on "any reason". An operator cannot see an epoch that never received its artifact until the boundary. | Deliberate (avoids a WARN on every normal mint epoch); `drive_acquisition` still pulls every tick and `dpos_dkg_stalled` is only for latched conditions. | confirmed by code |
| DA-11 | NIT | `ceremony.rs:1146-1156`; `actor.rs:2079-2099` | D-10 fixes only the `Player::resume` `log_map` (the view). `signed_log_hash`/`first_log` still return the first-recorded body, so `publish_recorded_logs` and `Confirmations` can claim/confirm a hash for a dealer that is not the artifact's pinned body when the journal holds an equivocation pair. | The pinned set used for finalize/recompute comes from the artifact (`scoped_pinned_logs`, `heal_over`), and `fetch_missing_logs` fetches the pinned hash, so the claim is not used to select bodies for this epoch; the agreement that would consume the index already ran. | confirmed by code |
| DA-12 | MINOR | `actor.rs:3669-3679` | Д-А1-11 changes the test geometry `INTERVAL 20 → 30` and rewrites the existing 61 unit fixtures. The old geometry (start exactly at the seal deadline) is no longer exercised by the pre-existing tests; only the new dedicated `recover`/R-036 tests cover the at/after-deadline fresh-start case. This is a fixture change, not just a machine refactor, so "existing tests unchanged" is not literally true. | The change is test-only (production interval ≈1200; stand uses 32), and the new geometry is the only one that makes the *fixed* `NoFile after seal ⇒ SatOut` path start a ceremony at all, which is why the old tests only passed via R-036. | confirmed by code |
| DA-13 | MINOR | `actor.rs:1409-1415`, `:3080-3176` | An artifact that arrives while the epoch is `Dealing{agreed:None}` stores the set and returns `false` from `apply_artifact`, so `on_artifact` does **not** call `fetch_missing_logs` (only `finalizable` does). HEAD's `on_artifact` always called it. The pinned bodies are then fetched on the next `on_height` (or never, on a halted chain), whereas the artifact input is documented as the arm that moves an epoch when the clock is frozen (`:1146-1149`). | Finalize cannot run pre-seal anyway, so on a halted chain nothing would consume the bodies; on a live chain the next tick is at most one interval away. | confirmed by code |
| DA-14 | NIT | `actor.rs:1274-1278`, `:1777-1779`, `:2289-2294` | Latches are only cleared on `Keyed` and sweep, so `Stalled{BodyLost}` (or `NoArtifact`/`QuorumMissing`) stays raised on an epoch that later resolves to `Acquiring(Logs)`/`KeyOnly` without ever going `Keyed`. The `dpos_dkg_stalled` gauge then counts resolved epochs as stalled. | The doc states the latch semantics explicitly ("until the epoch keys or ages out"), and the epoch is still not keyed, so the gauge is self-consistent; a terminal `SatOut`/`Conflict` is intended to stay latched. | confirmed by code |
| DA-15 | NIT | `actor.rs:1284-1290` | `take_state` swaps in a **real** terminal `Unrecoverable{key:None}` as a placeholder. The invariant "every call site writes a state back" is held by attention, not type: a future `?`/early return between `take_state` and `set_state` would silently leave the epoch terminal-unrecoverable (and would also mis-label DA-04's log). All six current sites do restore (`:1340-1346`, `:1452-1506`, `:1516-1547`, `:1874-1896`, `:2321-2326`, `:2335-2405`). | Only a placeholder; no current path leaks it. A `Vacant`/`Idle` variant or an RAII guard would remove the class. | confirmed by code |
| DA-16 | NIT | `artifact.rs:1258-1283` | Stale docs in a file touched by the diff: `:1279-1280` still says a held share makes `on_artifact` "return on exactly that check" (it no longer does; the equal-digest check in `apply_artifact` is the mechanism), and `:1261-1264` says the actor adopts a pinned set "from exactly ONE place: the artifacts channel" (now also `recover`'s `outcome_at` store read, `actor.rs:2582-2591,2637`). `restart_replay` is now partly redundant with `recover` for journaled epochs. | The behavioural selection in `restart_replay` (`:1292-1298`) still protects the old assumption, so the stale text is not a live bug; the doc is just ahead/behind the code. | confirmed by code |
| DA-17 | NIT | `log_store.rs:51`, `dkg_oracle.rs:17`, `log_resolver.rs:449`, `dkg_agree.rs:995`, `node/src/dpos.rs:1295` | Comments in five files outside the write list still name deleted carriers (`maybe_start`, `drive_recompute`, `recompute_pending`, `deferred_reported`). The journal discloses these; the brief's §0.6 requires no stale references. | These are comments, not code; the files were out of the allowed write list. Still, a comment disagreeing with code is a defect by the review contract. | confirmed by code |
| DA-18 | NIT | `actor.rs:1815-1975` | `decide_window` passes window-epoch outgoing through a throwaway `dropped` vec (`:2454-2458`). Today `recover` blanks outgoing for `past_boundary` (`:2688-2690`), so nothing is lost, but the discard is only safe because of that other function's guarantee; a resume that ever returned acks for a past epoch would drop them silently. | Same-effect as Д-А1-4; the invariant is stated. | inferred |

Also observed, not defects: the new `Option` seam `body_lost_rx` (`:788-792`) is Д-А1-1 and belongs
to А2; the new `pub` builder `with_body_lost` stays module-private because `mod actor` is private
(`mod.rs:68`); `restart_replay`'s selection (`artifact.rs:1292-1298`) keeps the old "held share ⇒
skip" behaviour even though `on_artifact` no longer has that early return.

---

## Leave as is

The following are deliberate and should not be changed on this pass:

- **Д-А1-1** new `body_lost` channel and the `Option` seam — the design sanctions a new channel and
  the seam is explicitly deferred to А2.
- **Д-А1-2, Д-А1-5, Д-А1-6, Д-А1-7, Д-А1-9** — all are necessary consequences of the four-input
  design (artifact can move a halted epoch; a past-boundary epoch cannot join a dead instance; a
  sat-out member still needs `PK_E`); no guarantee is weakened.
- **Д-А1-3** committee-unreadable announce retry — parity with HEAD; changing it is a liveness
  experiment that belongs in a separate change.
- **Д-А1-8** no resting `Finalizing` — synchronous and correct.
- **Д-А1-12** `withhold_ack` — a real fix found by a test; keep.
- `ArtifactStore::insert` first-wins and the `PULL_MIN_INTERVAL`/inflight pull de-dup.
- `try_recompute` retrying each tick with an empty `want` (R-038) and the off-poly/heal retry —
  explicitly out of scope.
- `SatOut`/`Unrecoverable` keeping the key digest and continuing to pull (`needs_artifact`) — I4.
- Evidence as data beside the phase (not a `Conflict` variant) — matches Б and the design's split
  between two-log equivocation and two quorum artifacts.
- The `INTERVAL=30` unit-test geometry as a *test* choice (the concern is documentation, DA-12, not
  the value).
- Existing comments in `log_store.rs`, `dkg_oracle.rs`, `log_resolver.rs`, `dkg_agree.rs`,
  `node/src/dpos.rs` may stay until А2 rewrites those files (DA-17 records them).
