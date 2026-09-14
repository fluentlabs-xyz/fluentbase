# Independent review — round 1, beacon p2p ingress (row 5.3, В+Г1)

Scope: exactly `git diff HEAD` (6 files). Base `HEAD = a45c458d`.
Read-only review: no cargo ran (no build cache); every claim below cites a file:line I opened.
The brief (`dsh-input-task.md`) and the implementer's journal (`dsh-input-journal.md`) are INPUT; every
load-bearing claim was re-checked against code and is marked `confirmed by code` or `inferred`.
`.claude/` docs are absent from this worktree, so Q8 covers only in-diff comments/docstrings.

Assumptions (stated because nobody can answer questions):
- "sender epoch offset" in Q1 is read as `T − now`, where `T` is the frame's `ceremony_epoch`
  (`dkg_msg.rs:83-84`, `:105`) and `now` is the receiver actor's `epoch_of(last_height)`
  (`actor.rs:823-827`, `:1330`).
- Q1 assumes an honest sender that is a member of `committee[T]`, and (unless a row says otherwise)
  steady state `E_w == now`, i.e. the window the transition last recorded is `{now−1, now, now+1}`
  (`epoch_transition.rs:767-790`, `p2p/src/lib.rs:366-398`). The `E_w != now` cases are called out.
- HEAD behaviour is taken from the diff's removed lines plus the orchestrator's established facts.

---

## Q1 — Liveness: every frame type × sender epoch offset

Ingress order is the same for every body type: `BeaconMessage` decode (`actor.rs:2213-2218`), header
epoch parse (`:2228`), `epoch_is_actionable` (`:2231`, `:2148`), `beacon_member`
(`:2235`, `:2188-2194`), then body decode and `Confirm` interception (`:2254-2257`). So membership
is body-blind; only what happens *after* admission differs by body.

Codes: **A** = admitted and handled; **B** = admitted and buffered in `pending`; **R:x** = refused with
reason `x`; **D** = admitted then silently dropped (no ceremony, not bufferable, no counter).
Each cell is `HEAD → new`.

| body | d=−2 | d=−1 | d=0 | d=+1 | d=+2 |
|---|---|---|---|---|---|
| Commitment | live ceremony: A → **R:not_member**; else R:epoch → R:epoch | A/R:epoch → same | A / D → same | A / B → same | H=B → **R:not_member** |
| Share | same as Commitment | same | A / D → same | A / B → same | H=B → **R:not_member** |
| Ack | live: A → **R:not_member**; else D → R:epoch | A / D → same | A / D → same | A / D → same | H: D (no live ceremony) → **R:not_member** |
| Reveal | live: A → **R:not_member**; else D → R:epoch | A / D → same | A / D → same | A / D → same | H: D → **R:not_member** |
| Confirm | live: H=R:confirm_window → **R:not_member**; else R:epoch → same | H=R:confirm_window → same (`actor.rs:1659-1664`) | recorded → same | recorded → same | H=recorded → **R:not_member** |

Re-delivery of a refused frame (this is the load-bearing column):

| body | what re-delivers it |
|---|---|
| Commitment / Share | dealer leg `DkgCeremony::retransmit` every pre-seal tick (`ceremony.rs:660-697`, called `actor.rs:1427`). The lagging player stays in `unsent` because it never acks (`ceremony.rs:384-394`). For `d=+2` the dealer seals at `epoch_start(T)−20` (`actor.rs:1360-1375`), ~`EL−20` blocks after the receiver's window advances, so the resend lands. |
| Ack | player leg: a re-received dealing re-emits the cached ack (`ceremony.rs:643-652`). After the dealer seals, retransmit stops — a late Ack is then **not** re-delivered, but it is no longer needed (the dealer already revealed the point in its log). |
| Reveal | one-shot broadcast at seal (`ceremony.rs:699+`); recovered by the resolver by pinned hash once the artifact exists (`actor.rs:2358-2410`, `:2426-2440`), and by the agreement's own proposal-body pull (`actor.rs:2358-2370`). No re-broadcast. |
| Confirm | **no dedicated re-delivery.** `Confirmations::mint` is edge-triggered on width growth and skips a width `<= previous` (`confirmations.rs:186-197`); a full-width confirm minted once is not re-sent. The only fallback is a leader's proposal carrying it (`dkg_agree.rs:1160`; receive-side check `:1195-1280`). → **D-01**. |

`d ≥ +3` is refused `epoch` on both HEAD and new (`actor.rs:2148`, constant not raised — decision 1).

`E_w != now` caveat (confirmed by code): the window is keyed to the transition's tracked epoch, not to
`now`. At the instant a boundary block finalizes the transition tracks `E+1` while the actor's
`epoch_of(boundary)` is still `E` (`epoch_transition.rs:608-621`), so `E_w = now+1` and the window is
`{now, now+1, now+2}`; a `d=+2` frame is then **admitted**. Conversely, if the actor's clock is driven
ahead of local finalization by the non-finalized marshal ordering tip (`actor.rs:1322-1326`), `E_w` can
be `now−1`, and `d=+1` (the current ceremony) is refused. See D-05.

---

## Q2 — `window_unset`

`TrackedWindow::classify` returns `None` only before the first `record` (`p2p/src/lib.rs:376-390`);
`record` is called by `OracleHandle::track` (`p2p/src/lib.rs:314-322`) and by the stand's `TrackSink`
(`stand.rs:1498-1501`). `beacon_member` turns `None` into `Err("window_unset")` (`actor.rs:2188-2194`).

Production can be live with an unset window:
1. `build_beacon_plane` spawns the poller, then awaits `beacon::build`; the actor is spawned there and
   parks only on `geometry` (`plane.rs:840-856`). In each poll iteration the geometry is **published
   first** and `track_peers` runs **after** (`node/dpos.rs:1560-1600` then `:1625-1650`), so between
   `send_replace(Some(frozen))` and `track_peers` the actor can run with an unset window.
2. `track_peers` returns `Ok(None)` when `committee[epoch]` is empty at the anchor (`epoch_transition.rs:854-878`)
   and is retried only on the next finalized change (`node/dpos.rs:1625-1650`); the layer's `cold_start`
   also leaves the window unset when `snap.validators.is_empty()` (`epoch_transition.rs:612-621`).
   Until the retry succeeds, every beacon frame is refused `window_unset`.
3. Once set, the window is never cleared (`p2p/src/lib.rs:366-370`), so this is a cold-start /
   visibility-lag state, not a steady state.

What is lost while unset: Commitment/Share and Ack are re-delivered by the mechanisms above; Reveal is
recoverable; **Confirm is not re-minted** (`confirmations.rs:186-197`) — the Д-129 shape stated in the
`beacon_member` doc (`actor.rs:2174-2185`). In the stand this cannot happen because `cold_start`
(`stand.rs:2438-2443`) runs before the gate and actor are built (`:2806`, `:2817`); the new test
asserts `window_unset == 0` (`tests.rs:3217-3222`).

---

## Q3 — Is the stand's window production-shaped?

Same builder, per node: the stand runs the production `EpochTransition` per node (`stand.rs:2414-2431`)
and records through `TrackSink` *before* forwarding (`stand.rs:1498-1501`), exactly as
`OracleHandle::track` records before `Manager::track` (`p2p/src/lib.rs:314-322`); `assemble_tracked_peers`
is the same function (`epoch_transition.rs:767-790`). So the `{E−1,E,E+1}` shape is production's.

Where the stand leads/leaves out production:
- **Timing**: the stand fills the window at `cold_start`, before the beacon is built
  (`stand.rs:2438-2443` → `:2800-2820`). Production's first `track` can be deferred (Q2). The stand
  therefore cannot be red on `window_unset`; the new test's `window_unset == 0` is structural, not
  evidence (`tests.rs:3217-3222`).
- **Tombstones**: production's window is built with the tombstone predicate
  (`node/dpos.rs:1320-1325`, `p2p/src/lib.rs:359-364`); the stand passes a plain
  `TrackedWindow::default()` (`stand.rs:2412`, `:2806`) and the `TombstoneSet` it creates
  (`stand.rs:2916`) is never filled, so `Dropped`-by-tombstone never fires in the stand.
- Both are exactly the two cases where the stand can be green while production refuses (D-02, D-03, D-20).

---

## Q4 — `GatedReceiver` in the stand

- It wraps the receiver the actor reads: `bcr` is the `BEACON_CHANNEL` receiver (`stand.rs:2749`),
  wrapped at `stand.rs:2806`, passed as `beacon_channel.1` (`:2815`) and destructured by the plane into
  the actor's `receiver` (`plane.rs:603-611`, `:866-875`). Same for production
  (`node/dpos.rs:1892-1897`, `:1902`).
- Every `Beacon::Live` role is wrapped except `Role::AbsentBeacon`, which gets `absent(&ctx_i)` and no
  beacon at all (`stand.rs:2747`). Byzantine roles take the same `(Beacon::Live, _)` arm
  (`stand.rs:2748-2807`); only the *sender* is replaced under the feature (TwoRevealSender,
  `stand.rs:2769-2797`), so Byzantine receive paths are gated.
- The new test's `not_member` is process-wide: `counter_of` sums over nodes (`stand.rs:826-842`) and the
  metric carries only `channel`/`reason` labels (`consensus/src/dpos.rs:63-67`). Attribution to node 3
  rests on the control run (0) and the fixture (only node 3 is cut), not on a node label; the
  implementer states the same (`journal §0.9.3`). The assertion is only `not_member(cut) > not_member(control)`
  (`tests.rs:3210-3216`), not `== 3`.

---

## Q5 — `beacon_member`'s `Result<(), &'static str>`

Single non-test caller: `on_message` (`actor.rs:2235-2238`), which forwards the reason to `refuse`
(`:2200-2209`); the only other callers are the two unit tests (`:5576`, `:5596`, `:5611`). No caller
discards the reason. All four labels used (`epoch`, `window_unset`, `not_member`, `confirm_window`) are
reachable: `epoch` at `:2232`, `window_unset` when `classify` is `None`, `not_member` for a real
non-member / `Tracked` / `Dropped`, `confirm_window` at `:1661`. One imprecision: `classify` checks the
tombstone predicate before the window (`p2p/src/lib.rs:377-379`), so a tombstoned peer with an *unset*
window is labelled `not_member`, never `window_unset` (D-19). `Ingress::Tracked` and `Ingress::Dropped`
are collapsed into `not_member` (`actor.rs:2191-2193`), which is fine because the `members_only=true`
gate already dropped both (`consensus/src/dpos.rs:111-121`).

---

## Q6 — Resolver-exit `break`

The arm now `error!`s and `break`s (`actor.rs:913-935`). The channel's only sender is the
`LogHandler` moved into the resolver engine (`plane.rs:746-747`, `:781-791`, `:325-333`); the engine is
a supervised sibling (`plane.rs:1012-1016`) and the actor is another (`plane.rs:1012-1013`). The
supervisor returns on the first child exit (`plane.rs:215-235`), and the node's `supervise` treats any
overlay handle resolving as fatal: it cancels the shutdown token (`node/dpos.rs:614-655`, `:512`). So
`break` really is fatal. Nothing else in `run` holds a resource that must be flushed before return: the
write-back/artifact and pinned receiver arms only park (`actor.rs:940-953`), and `self`'s channels close
on drop. The `break` also fires during an ordinary shutdown, which is harmless. The unit
(`actor.rs:5748-5816`) keeps `heights` and the gossip receiver open, drops the resolver sender, and
selects on `run` vs a 5 s virtual timeout, so it proves exit rather than timeout (mutation M3 in the
journal). Inconsistency: `pinned_rx`/`artifacts_rx` closing still degrades silently
(`actor.rs:945`, `:951`), unlike the resolver arm (D-18).

---

## Q7 — `deque_size = 2`

The unit (`dkg_transport.rs:185-263`) builds the engine through the production `build_body_engine`
(`:109-150`, used in production at `dkg_engine.rs:312`), so it reads the real constant (`:147`). It
proves retention of two distinct digests and eviction on the third, i.e. only the deque bound; it does
not invoke `DkgAgree::propose`/nullify at all. The "leader re-proposes a **different** body after
nullify" claim is supported by code reading: `propose`'s uncertified arm calls `build_proposal`
(`dkg_agree.rs:1449-1460`), which loops `attempt_proposal` (`:1105-1130`), which snapshots the *current*
`local_set` and `confirms.covering` (`:1142-1175`); the digest is over the full encoding including
`logs` and `confirms` (`:448-460`, `:647-651`). So a log/confirmation landing between views changes the
digest. The unit only exercises the engine's own-send cache path, not the peer receive path (the
implementer notes this, `journal §0.9.5`); it also drops the engine handle rather than holding it
(D-17). Residual bound: a third distinct body evicts the first (`dkg_transport.rs:131-147`), which the
comment accepts as "past the measured envelope" (D-14).

---

## Q8 — In-diff doc drift (comments only; `.claude/` is absent)

- `INGRESS_LOOKAHEAD_EPOCHS` says the tracked window carries "`committee[now + 1]` at most"
  (`actor.rs:133-137`). False when `E_w = now+1` (boundary instant, `epoch_transition.rs:608-621`):
  the window is then `{now, now+1, now+2}` (D-09).
- `within_ingress_window`'s doc says a caller with no clock "must not call this with a `0` floor", but
  `epoch_is_actionable`/`is_bufferable` do exactly that via `height_now()` (`actor.rs:148-154`, `:2114`,
  `:2148`); the distinction only matters for `on_confirm` (D-08).
- `on_confirm`'s doc says "the membership check below is the whole bound" (`actor.rs:1601-1618`), but
  membership now runs *above* it and is itself window-based (`:2235`) (D-10).
- The `beacon_member` doc's justification cites "this actor's clock — already the max over every
  finalized-height feeder" (`actor.rs:134-137`); `on_height` lists a **non-finalized** ordering-tip
  feeder (`:1322-1326`) (D-05).
- Test name `a_beacon_frame_from_a_non_member_costs_no_committee_read_beyond_the_check` (`:5497`) no
  longer describes the assertion (now zero reads) (D-16).
- Out-of-diff but invalidated by the change: `assemble_tracked_peers`'s comment still says the beacon's
  per-epoch check reads `committee_for` (`staking-reader/src/epoch_transition.rs:735-746`) (D-21).
- `epoch_of`'s doc (`actor.rs:820-821`) is untouched and still accurate.

## Q9 — Hygiene

- No new `#[allow]` (`git diff | grep allow(` = none); the existing `too_many_arguments` on
  `DkgActor::new` (`actor.rs:625`) predates the extra parameter.
- No new `unwrap`/`expect` on a production path; all additions are inside `#[cfg(test)]`
  (`actor.rs:3141+`, `dkg_transport.rs:156+`, `testbed/tests.rs:3117+`).
- New public surface: the `pub ingress_window` field on `ValidatorInputs` (`plane.rs:529-537`) — needed
  so `node` and the stand can pass it; `within_ingress_window`/`INGRESS_LOOKAHEAD_EPOCHS` are
  `pub(crate)` (`actor.rs:146`, `:152`); `beacon_member`/`refuse` stay private. Nothing else widened.
- No dead code: the old `committee_for`-membership path is gone; `committee_for` remains for roster
  reads (`actor.rs:760, :973, :1560, :1665, :1746, :1900, :2383, :2436, :2545, :2703, :2752, :2988`),
  and it is the only `record_ingress_drop` site in `beacon/` (`:2208`).
- `git status` shows only the six intended files modified.

## Q10 — Where this review is weakest (ranked)

1. **No cargo run.** Every "green test" claim is the implementer's; I could only read. The new stand
   test's numbers (`not_member=3`) were not reproduced, and its threshold is not tight (D-12).
2. **`window_unset` in production (D-02/D-03).** I could not measure whether `track_peers` actually
   defers in practice; I read the ordering and the `Ok(None)` arm, not a live devnet.
3. **Window/actor-clock divergence (D-05).** Whether the ordering-tip feeder can outrun the transition's
   tracked epoch by a whole epoch is inferred from `on_height`, not measured.
4. **Confirm liveness (D-01).** Whether proposal-carried confirms really cover a permanently missing
   direct confirm in every view/leader schedule is inferred from `dkg_agree.rs`; I did not trace all
   vote-decide paths.
5. **Attribution inside the new stand test (D-11)** — process-wide counter, no node label.
6. **`deque_size` peer-receive path (D-13/D-14)** — not exercised by the unit.

---

## Findings

| id | severity | file:lines | what is wrong | how I tried to refute it | confidence |
|---|---|---|---|---|---|
| D-01 | SERIOUS | `beacon/actor.rs:2235-2238`, `:2188-2194`; `beacon/confirmations.rs:186-197` | A `ShareConfirm` whose target is `now+2` (sender one epoch ahead) is now refused `not_member` and has **no dedicated re-delivery**: `mint` is edge-triggered on width growth and skips `<= previous`, so a full-width confirm is never re-sent. The lagging receiver's entry bar can permanently undercount that member. | Tried to show it is re-minted: `mint` is called per tick with `AnyGrowth` (`actor.rs:1395`), but it returns early at `confirmations.rs:187-188`; tried to show the leader's proposal covers it (`dkg_agree.rs:1160`, `:1253-1275`), which is a fallback but not a re-delivery of the frame. | confirmed by code (refusal + no re-mint); inferred (liveness consequence) |
| D-02 | SERIOUS | `node/dpos.rs:1560-1600`, `:1625-1650`; `beacon/actor.rs:2188-2194` | Production can have the actor live with an unset window: geometry is published before `track_peers` in the same poll, and `track_peers`/`cold_start` can leave the window unset (`epoch_transition.rs:612-621`, `:854-878`). Every frame in that state is refused `window_unset`, and a Confirm then is lost (D-01). | Tried to show geometry publication implies a readable committee in the same poll (the state at `fin` is executed because `freeze_geometry` succeeded), which makes the gap microseconds; the implementer did not measure it. | confirmed by code (state reachable); inferred (duration/impact) |
| D-03 | SERIOUS | `testbed/stand.rs:2408-2419`, `:2438-2443`, `:2806`; `testbed/tests.rs:3217-3222` | The stand window is filled at `cold_start` **before** the gate and actor exist, so the stand cannot exercise the production cold-start `window_unset` race; `window_unset == 0` is structural. The stand can be green where production refuses. | Tried to find a stand path where the actor runs before `cold_start`: `cold_start` is awaited at `stand.rs:2438` and the beacon is built at `:2800`, so there is none. | confirmed by code |
| D-04 | MODERATE | `beacon/actor.rs:1900`, `:2188-2194`; `staking-reader/src/epoch_transition.rs:798-812` | Two sources still coexist: `maybe_start` reads the ceremony roster from `committee_for`, ingress reads the window. `assemble_tracked_peers` **skips** an uncommitted neighbour record, so if `C[now+1]` is absent at the tracking anchor but `committee_for(now+1)` resolves, the node deals while all peer frames for that ceremony are refused `not_member`. | Tried to show both read the same committed slot and `C[E+1]` is committed an epoch ahead (comment at `epoch_transition.rs:735-746`), so disagreement is genesis-era/visibility-lag only — I could not rule that out. | inferred |
| D-05 | MODERATE | `beacon/actor.rs:1322-1326`, `:2159-2180`, `:2188-2194`; `staking-reader/src/epoch_transition.rs:608-621` | The membership window is keyed to the transition's tracked epoch `E_w`, the actor's `now` is `max` over three feeders including the **non-finalized** marshal ordering tip. If `now` outruns `E_w` by one epoch (boundary not yet finalized / catch-up), the current ceremony's `now+1` frames are refused `not_member`; the doc claims the clock is "the max over every finalized-height feeder", which the code contradicts. | Tried to show `fin+K` tracks the ordering scale so the divergence is a few blocks only, and retransmit heals it; a sustained ≥1-epoch lead would need a stalled finality, which I could not exclude analytically. | inferred |
| D-06 | MODERATE | `beacon/actor.rs:2188-2194`, `:2148`; `beacon/ceremony.rs:660-669` | A still-live ceremony for an epoch `≤ now−2` has its frames refused `not_member` (window min `now−1`) where HEAD accepted them through `committee_for`. | Tried to find a needed frame: such a ceremony is already sealed (`dealing_closed`, `ceremony.rs:661`), so no dealings/Acks are still needed, and Reveals are resolver-refetchable; only the label changes for Confirm. | confirmed by code (behaviour); inferred (no loss) |
| D-07 | MINOR | `beacon/actor.rs:1659-1664`, `:2200-2209` | The `on_confirm` refusal's debug line lost the old `now` field (the old site logged `now` and `target_epoch`, the new `refuse` logs `from`/`epoch`/`reason`). Operability regression only. | Tried to recover `now` from the remaining fields: `epoch` is the target, `last_height` is not in the line; it is recoverable only by cross-referencing. | confirmed |
| D-08 | MINOR | `beacon/actor.rs:148-154`, `:2114`, `:2148` | `within_ingress_window`'s doc forbids being called with a `0` floor, but `epoch_is_actionable` and `is_bufferable` call it with `height_now()` == 0 before the first tick. Doc vs code drift (behaviour itself is unchanged). | Tried to read the doc as "must not treat 0 as a real clock", which is what `on_confirm` does; the function itself is called with 0 anyway. | confirmed |
| D-09 | MINOR | `beacon/actor.rs:124-146`; `dpos/p2p/src/lib.rs:366-398`; `staking-reader/src/epoch_transition.rs:608-621` | The constant's doc says the tracked window carries `committee[now+1]` at most. It carries `{E_w−1,E_w,E_w+1}`, and `E_w = now+1` at every boundary instant (`epoch_transition.rs:608-621`), so it can carry `committee[now+2]`. | Tried to show `E_w <= now` always: `epoch_transition.rs:612-614` explicitly bootstraps `epoch_e+1` on a boundary-aligned resume, so no. | confirmed |
| D-10 | MINOR | `beacon/actor.rs:1601-1618`, `:2235` | `on_confirm`'s doc says the membership check "below" is the whole bound; membership now runs above it and is itself the window (`beacon_member`). Stale rationale. | Tried to read "below" as the body-decode order inside `on_confirm`; no membership check remains inside `on_confirm`. | confirmed |
| D-11 | MINOR | `testbed/tests.rs:3154-3162`; `testbed/stand.rs:826-842` | The new test's `not_member` is a process-wide sum with no node label; attribution to node 3 is inference from the control and the fixture, not from the metric. | Tried to find a per-node label: `record_ingress_drop` has only `channel`/`reason` (`consensus/src/dpos.rs:63-67`). | confirmed (method); inferred (attribution) |
| D-12 | MINOR | `testbed/tests.rs:3210-3216` | The liveness assertion is only `not_member(cut) > not_member(control)` (`3 > 0`); any unrelated source of `not_member` on any node would satisfy it, and a timing shift that removes the lag would fail it. No exact-count or per-node assertion. | Tried to bound the other sources: only node 3 lags, control is 0, and gate refusals use `secondary`/`untracked` (`consensus/src/dpos.rs:117-121`), so currently sound. | confirmed |
| D-13 | MINOR | `beacon/dkg_transport.rs:185-263`; `beacon/dkg_agree.rs:1449-1460`, `:1105-1175`, `:448-460` | The deque unit uses the production engine config but never exercises the real `DkgAgree::propose`/nullify re-proposal; the "different body after nullify" claim rests on code reading, not on the test. | Tried to find a call to `propose` in the test: none; it calls `Mailbox::broadcast` with hand-built bodies. | confirmed |
| D-14 | MINOR | `beacon/dkg_transport.rs:123-147`; `beacon/dkg_agree.rs:1425-1471` | `deque_size = 2` retains only two bodies per sender; a third distinct re-proposal evicts the first, so a peer parked on the first digest can no longer fetch it. The comment accepts this as "past the measured envelope", but an adversary able to force repeated nullifications can reach it. | Tried to bound it with the measurement claim (one body in all precondition shapes) and the accepted map decision; nullification is not adversarially bounded in code I could read. | inferred |
| D-15 | MINOR | `beacon/actor.rs:3151-3169`, e.g. `:4446` | The test helper builds the window from `committee_for`, not from `EpochTransition::assemble_tracked_peers`; on a transient `committee_for → None` it records `Some((center, empty))`, which production never does (production skips the track). A test that flips a `readable` flag afterward then classifies every peer as `Dropped` (not `window_unset`), diverging from production. | Tried to find a current test that depends on it: the transient-`None` tests (`:4435`, `:4632`) exercise `serve_log`, not ingress, so it is latent. | confirmed by code |
| D-16 | NIT | `beacon/actor.rs:5497` | Test name still says "costs no committee read **beyond the check**", but the membership check now costs zero reads (assertion at `:5583-5588`). | Tried to justify the name as "the feature's own read"; the docstring was updated, the name was not. | confirmed |
| D-17 | NIT | `beacon/dkg_transport.rs:221` | `drop(engine.start(channel))` relies on commonware `Handle` having no `Drop` (the plane says so at `beacon/plane.rs:200-207`); if that ever changed, the engine would be aborted and the test vacuous/flaky. | Tried to confirm from the checkout: the plane's comment asserts no `Drop`; I did not re-open the runtime source. | inferred |
| D-18 | NIT | `beacon/actor.rs:926-935` vs `:940-953` | The resolver arm is fatal while the adjacent `pinned_rx`/`artifacts_rx` arms still degrade silently (`None => pinned_rx = None`), so a closed agreement/artifact channel is not fatal. Inconsistent exit policy inside one loop. | Tried to read it as intentional (resolver is the documented "gossip-only" degradation, decision 4); the pin/artifact seams have no supervisor handle argument in the diff. | confirmed |
| D-19 | NIT | `beacon/actor.rs:2188-2194`; `dpos/p2p/src/lib.rs:376-390` | `classify` checks tombstones before the window, so a tombstoned sender with an *unset* window is labelled `not_member`, never `window_unset`; and `Tracked`/`Dropped` both collapse to `not_member`. Diagnostic granularity only. | Tried to find a path where `window_unset` still fires for such a peer: none, but the gate drops tombstones first in production anyway. | confirmed |
| D-20 | MINOR | `testbed/stand.rs:2912-2919`, `:2806`; `node/dpos.rs:1320-1325` | The stand's window has no tombstone predicate (it is `TrackedWindow::default()`), while production's is `with_tombstones`. The stand's `TombstoneSet` is never filled, so a tombstoned committee member is admitted in the stand but dropped at production's gate (reason `untracked`) — a second stand-green/production-red gap. | Tried to show `cfg.tombstoned` fills the set: it goes to `FakeStaking.with_schedule` (`stand.rs:2387`) and does not populate `TombstoneSet::default()` (`:2916`). | confirmed by code |
| D-21 | MINOR | `staking-reader/src/epoch_transition.rs:735-746` | Stale comment left by the change: `assemble_tracked_peers` justifies skipping an absent neighbour record with "the beacon's own per-epoch check reads the SAME write-once slot through `committee_for`, so an epoch with no record has no members to admit either". After this diff the beacon's per-epoch ingress check reads the tracked window (`beacon_member`), not `committee_for`; if `committee_for` resolves a record the window skipped, the reassurance is false (this is the mechanism behind D-04). | Tried to read the sentence as still true because both ultimately derive from the committed slot; the sentence names `committee_for` as the ingress reader, which the diff removed. | confirmed |

BLOCKER count: none established to the standard "an honest node **needs** the frame with **no** re-delivery
path". D-01/D-02 are the nearest; both have a fallback (proposal-carried confirms; the frame is a
Confirm only), so they are SERIOUS rather than BLOCKER. D-03/D-20 are stand-vs-production gaps but the
production side needs a narrow race (D-02) or tombstoned peers (D-20) to matter.

---

## Leave as is

- The three-site consolidation onto `within_ingress_window` (`actor.rs:152-154`, `:2114`, `:2148`,
  `:1660`) is exact: `is_bufferable` correctly keeps its own `epoch > now` on top
  (`:2112-2115`), `epoch_is_actionable` keeps the live-ceremony short-circuit (`:2145-2149`), and
  `on_confirm` keeps the `last_height = None` exemption (`:1659`). Do not "unify" those away.
- `INGRESS_LOOKAHEAD_EPOCHS = 2` and the refusal to add `now+3` (`actor.rs:146`, `:133-145`): the
  window would move a `now+3` frame from `epoch` to `not_member` and nothing else, and no `now+3`
  body has a delivery path that `now+2` does not.
- `beacon_member` reading only `ingress_window` (`:2188-2194`), with `committee_for` reserved for the
  ceremony roster and confirmation verification — the intended one-source design; keep it.
- The single `refuse` site (`:2196-2209`) with the shared `dpos_ingress_dropped_total` family
  (`consensus/src/dpos.rs:63-67`): R-070 is respected, no registry merge.
- The resolver-exit `break` (`:926-935`) and the supervisor chain that makes it fatal
  (`plane.rs:1012-1016`, `node/dpos.rs:614-655`): the `error!` names the chain and the decision is the
  orchestrator's; keep. The unit does prove exit (not timeout).
- The stand wiring: per-node `TrackedWindow` recorded by the node's own production `EpochTransition`
  before forwarding (`stand.rs:1460`, `:1498-1501`, `:2412-2419`), `GatedReceiver(.., "beacon", true)`
  on the same receiver the actor reads (`:2806`, `:2815`), and the same window in `ValidatorInputs`
  (`:2817`). This is the production shape; keep it.
- `deque_size = 2` unchanged, with its rationale brought to the code fact and pinned by a unit built
  through `build_body_engine` (`dkg_transport.rs:123-147`, `:170-263`). The constant is the
  orchestrator's; keep.
- No `#[allow]`, no production-path `unwrap`/`expect`, no gratuitous `pub`: keep that discipline.
