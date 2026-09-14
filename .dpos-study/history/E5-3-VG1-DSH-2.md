# Round-2 independent review — beacon p2p ingress (row 5.3, В+Г1), revision of decision 2

Scope: exactly `git diff HEAD` (base `a45c458d`, 8 files under `crates/`):
`beacon/{actor,ceremony,dkg_transport,mod}.rs`, `testbed/{stand,tests}.rs`,
`staking-reader/src/epoch_transition.rs`, `node/src/dpos.rs`.
Read-only: no cargo ran, no file was changed. Inputs `dsh-input-*.md` were read as INPUT;
every load-bearing claim below was re-checked in the tree and is marked `confirmed by code` or
`inferred`. `.claude/` docs are absent from this worktree, so doc findings cover only files present.

Assumptions (stated because no question can be asked):
- `T` = a frame's `ceremony_epoch`, `now` = receiver actor's `epoch_of(last_height)`
  (`actor.rs:822-827`, `:1324-1327`), `E_w` = the receiver's last `track`ed epoch,
  window primary = `{C[E_w-1], C[E_w], C[E_w+1]}` (`epoch_transition.rs:779-791`),
  secondary = active registry (`:778`). `d = T − now`.
- Unless a row says otherwise, steady state `E_w == now`. "continuing" = sender also in one of the
  window records; "new" = only in `C[T]` (hence only in the registry/secondary tier at the gate).
- Gate outcome: `Member → admit`, `Tracked → refuse secondary`, `Dropped → refuse untracked`,
  `classify == None → admit` (`consensus/src/dpos.rs:112-124`, `p2p/src/lib.rs:376-392`).
- Codes below: **G** = gate refusal, **A** = handled, **B** = `pending`, **D** = silent drop,
  **R:x** = actor `refuse` (`actor.rs:2189-2200`). HEAD behaviour is read from the diff's removed
  lines plus the orchestrator's facts; the diff is HEAD→final (round 1 + round 2 combined), so the
  removed `beacon_member` (`(self.committee_for)(epoch)...`) is HEAD's, not round 1's.

Round-2 shape confirmed by code: the actor holds no membership opinion. Its only epoch rule is
`epoch_is_actionable` = live ceremony ∪ `[now, now+2]` (`actor.rs:2165-2170`); the seat is the
consumer's — `DkgCeremony::has_seat` at the dispatch (`actor.rs:2264-2271`) and at the `pending`
drain (`actor.rs:1989-2011`), `committee_for(target)` in `on_confirm` (`actor.rs:1668-1676`).
`beacon_member`, `window_unset`, `not_member`, `ValidatorInputs.ingress_window` and the test window
helpers are gone (`grep beacon_member|window_unset|not_member|window_around` over `crates/` = 0).

---

## Part A — verdict for D-01 … D-21

| id | verdict | file:lines (current) | why | refutation attempt | confidence |
|---|---|---|---|---|---|
| D-01 | **REMOVED BY REVISION, correctly** | `actor.rs:1629-1676` (on_confirm), `:2247-2250` (interception), `:2189-2200` (refuse); `confirmations.rs:186-189` (mint memo); `node/dpos.rs:1894-1899` (gate) | The actor no longer refuses a `ShareConfirm` for `now+2` from a member of `C[now+2]`: `on_message` has no membership check, `on_confirm` reads `[now, now+2]` (`:1663`) then `committee_for(target)` (`:1668`) and `roster.position(from)` (`:1673`), so a continuing/new-but-window member's confirm is recorded exactly as on HEAD. The defect's old site (`beacon_member` between epoch gate and decode) is gone. The `mint` memo (`confirmations.rs:186-189`, `previous >= confirmed.len() ⇒ continue`) is still a fact, but it can now only cost a frame the GATE refuses — the pre-existing cell E-01, not the actor. | Tried to find a remaining actor-side refusal for `d=+2`: `epoch_is_actionable(now+2)` is true (`:2169`), `on_confirm`'s window contains it (`:1663`), the roster is `committee[target]` (same slot the minter signs against, `confirmations.rs:170-179`). None. Tried to re-open it through `last_height == None`: then the window is skipped (`:1662`) and only the roster binds, which is HEAD's behaviour. | confirmed by code (new site); inferred (residual is gate-only, E-01) |
| D-02 | **REMOVED BY REVISION, correctly** | `actor.rs:2202-2231` (`on_message`); `consensus/src/dpos.rs:112-115`; `node/dpos.rs:1320-1325`, `:1560-1652`, `:1894-1899` | There is no `window_unset` state any more: the gate admits when `classify == None` (`dpos.rs:113-115`), and the consumer binds by roster/seat with no window read (`actor.rs:2264-2271`, `:1673`). So the production ordering "publish geometry (`node/dpos.rs:1585`) then `track_peers` (`:1625-1652`)" can no longer make the actor refuse anything. | Tried to show the gate's `None` admission widens the attack surface: it does admit every sender pre-first-track, but `epoch_is_actionable` bounds the epoch (`:2169`) and the seat check bounds the sender (`:2267`), and HEAD's gate had the same `None` admission. | confirmed by code |
| D-03 | **REMOVED BY REVISION, correctly** | `testbed/tests.rs:3101-3184`; `testbed/stand.rs:2440-2445`, `:2887`; `epoch_transition.rs:614-630` | The stand still fills its window at `cold_start` (→ `track_and_trigger` → `sink.track`, `epoch_transition.rs:625-630`, `stand.rs:2471-2475`) before the gate exists (`stand.rs:2887`), but the assertion `window_unset == 0` is gone and there is no `window_unset` state to hide; the new test asserts gate refusals and the minted key instead. | Tried to find a path where the stand's window timing could still mask a production refusal: the only actor refusals are `epoch`, `confirm_window`, `no_seat`, none window-unset-derived. | confirmed by code |
| D-04 | **REMOVED BY REVISION, correctly** | `actor.rs:1909` (the only `committee_for` for a ceremony), `:2264-2271`; `ceremony.rs:191-195`, `:798-808`; `epoch_transition.rs:744-754` | The two actor-side membership sources are gone: `maybe_start` reads `committee_for(target)` only to build the ceremony (`:1909`), and the consumer's `has_seat` reads the roster the ceremony was built over (`ceremony.rs:195`, `:930`/`:460`), the same committed slot. | Tried to construct a disagreement between `maybe_start`'s roster and `has_seat`: both come from the same `next` argument (`actor.rs:1945`, `:1955`, `ceremony.rs:460`, `:930`). The remaining gate-vs-consumer disagreement is the pre-existing E-02 (window skips an uncommitted neighbour, `epoch_transition.rs:803-824`). | confirmed by code (actor side); inferred (gate residual) |
| D-05 | **REMOVED BY REVISION, correctly** | `actor.rs:124-152` (constant doc), `:1322-1327` (three feeders), `:2135`, `:2169`, `:1663` (window sites) | The clock divergence `now ↔ E_w` can no longer refuse a current-ceremony frame at the actor: no actor check is keyed to the window's epoch membership. The constant doc now correctly lists "the max over its three feeders, `fin + K`, the upstream cert frontier and the marshal's ordering tip (`on_height`)" (`:143-145`), matching `:1322-1327`. | Tried to find a new window-keyed refusal: `epoch_is_actionable` keys on `now`, not `E_w`; a `now = E_w+1` clock still leaves the sender's `T ∈ {now, now+1, now+2}` actionable. | confirmed by code |
| D-06 | **REMOVED BY REVISION, correctly** | `actor.rs:2165-2170` (live-ceremony short-circuit), `:2264-2271` (seat), `:1662-1666` (confirm still windowed) | A live ceremony below `now` still passes `epoch_is_actionable` via `ceremonies.contains_key` (`:2166-2167`), and a frame from a roster member is then handled (`has_seat`). | Tried to find a needed frame: such a ceremony is already sealed/post-deal (`ceremony.rs:795`, `dealing_closed`), so only Reveals matter and they are resolver-refetchable (`actor.rs:2342-2357`); confirms for an epoch below `now` are still refused `confirm_window` (`:1663-1664`), which the doc calls deliberate (`:1619-1628`). The gate may still refuse a rotated-out member (E-02), as on HEAD. | confirmed by code |
| D-07 | **FIXED** | `actor.rs:2189-2200` | `refuse` now logs `?now` (`:2190`, `:2195`), recovered from `last_height.map(epoch_of)`; `None` before the first tick. The round-1 form (`from`/`epoch`/`reason` only) hid this site's clock. | Tried to show `now` is redundant with `epoch`: `epoch` is the frame's target, `now` is the receiver's clock; a `confirm_window` refusal is only interpretable with both. | confirmed |
| D-08 | **FIXED** | `actor.rs:154-162` | The doc now states that `epoch_is_actionable`/`is_bufferable` read `height_now() == 0` (floor `[0,2]`, honest for them) and that only `on_confirm` reads `last_height` itself; matches `:2149-2150`, `:2132`, `:2169`. | Tried to find a caller still contradicting the doc: only the three named call sites exist (`:1663`, `:2135`, `:2169`). | confirmed |
| D-09 | **FIXED** | `actor.rs:124-152` | The "window carries `committee[now+1]` at most" claim and the whole window→membership rationale are gone; the constant is now about EPOCHS only (`:131-138`) and the "why 2 not 3" rationale names re-delivery/resolver fallbacks (`:140-151`). | Tried to find a residual window-size claim: `within_ingress_window` is `[now, now+2]` (`:163-164`), no `committee` mention. | confirmed |
| D-10 | **FIXED** | `actor.rs:1600-1613` | `on_confirm`'s doc now says this function is the CONSUMER, that `from` must hold a seat in `committee[target_epoch]` or be refused `no_seat`, and that the gate/epoch checks upstream ask nothing about seats. | Tried to read "below" as still implying an in-function membership check: the check is the new `roster.position(from)` (`:1673-1676`), and the doc names it. | confirmed |
| D-11 | **FIXED** | `testbed/tests.rs:3121-3129`, `:3150-3167`; `stand.rs:1076-1082`, `:1599-1600`, `:2015-2017` | Attribution is no longer "process-wide sum > 0": the test asserts the exact equality `(secondary, untracked) == (stray_dealer_sends[4], 0)` (`tests.rs:3161-3167`), with `stray_dealer_sends` counting the `(frame, recipient)` pairs the stray's `Sender::send` returned (`stand.rs:2825-2827`, `:2015-2017`). The stray is the only sender any gate can refuse, because every other node is in every record and node 4 is registry-only (`stand.rs:1633-1634`). | Tried to find another `secondary` emitter in the stand: `grep record_ingress_drop` gives only `slasher/gossip.rs:121` and `dpos.rs:122`; the stand wires `slasher_evidence: None` (`stand.rs:3033`). Tried to fake the count: any unrelated `secondary` would break the equality (red), not fake it. Residual fragility = E-04. | confirmed by code |
| D-12 | **FIXED** | `testbed/tests.rs:3161-3170` | Replaced the loose `not_member(cut) > not_member(control)` with the exact `(secondary, untracked) == (sent, 0)` and `no_seat == 0`, plus `sent > 0` and `others == 0` (`:3152-3157`). | Tried to show the equality can pass with the gate absent: M1 report `secondary=0` vs `sent=412` makes `:3161` fail; `no_seat=48` also fails `:3170`. | confirmed |
| D-13 | **RECORDED, correctly** | `dkg_transport.rs:185-264` (unit), `:109-147` (`build_body_engine`); `dkg_agree.rs:2243-2253`, `:2260-2283` | The unit still builds through the production `build_body_engine` (`:147` reads the real `deque_size`) and never calls `DkgAgree::propose`; the "different body after nullify" claim is code reading (`dkg_agree.rs:1425-1460`, `:1142-1170`, `:458`). The map permitted recording when the fixture exceeds 40 lines; the existing fixture (`dkg_agree.rs:2243-2253`) uses `MAX_SET_LEN` and would have to move to `dkg_agree.rs`, outside the write list. Recorded in the journal §В.2 D-13. | Tried to find a `<40`-line path: a real propose/nullify needs `agree_over`'s mailbox rewired to the production engine plus two `propose` calls with a log landing between; not a local edit. | confirmed by code (limitation) |
| D-14 | **RECORDED, correctly** | `dkg_transport.rs:131-147`, `:256-262`; `dkg_agree.rs:1142-1170` | The accepted bound ("a THIRD re-proposal evicts the first") is stated in the comment and pinned by the unit (`:256-262` asserts the first digest is gone, the second/third present). Adversarially repeated nullifies remain the unresolved bound; recorded as a registry candidate. | Tried to bound it from code: `propose` re-proposes the certified value verbatim when one exists (`:1443-1454`), so nullification is needed between distinct bodies; no code bound on nullifies. | inferred |
| D-15 | **FIXED** | `actor.rs` test module; `grep window_around\|record_window` over `crates/` = 0; new tests `actor.rs:5784-5882` | The `window_around`/`record_window` helper and all window passing into fixtures were deleted with the actor field. The new window unit builds state from `within_ingress_window` and real actor calls, not a `committee_for`-derived window. | Tried to find the old divergence (a test window recording `Some((center, empty))`): no helper and no `ingress_window` field remain in the actor. | confirmed |
| D-16 | **FIXED** | `actor.rs:5484` | Renamed to `a_confirmation_from_a_sender_with_no_seat_costs_one_roster_read_and_is_counted`; it measures exactly one roster read and the `no_seat` count (`:5473-5500`). | Tried to read the old name as still descriptive: "costs no committee read beyond the check" is now literally "one roster read" and is asserted. | confirmed |
| D-17 | **RECORDED, correctly** | `dkg_transport.rs:221`; checkout `runtime/src/utils/handle.rs:24-31`, `:128-139` | `drop(engine.start(channel))` still relies on `commonware_runtime::Handle` having no `Drop`. Verified in the pinned checkout: `Handle` is `{abort_handle, receiver, metric}` with no `impl Drop`; `abort()` is explicit (`handle.rs:107-118`) and `Future::poll` only reads the receiver (`:134-139`). Recorded, not changed. | Tried to show the test would be vacuous if `Drop` aborted: `mailbox.get` after `drop` returns `Some` in the green run, so the engine is alive; and the source confirms no `Drop`. | confirmed by code (checkout) |
| D-18 | **RECORDED, correctly** | `actor.rs:926-934` (resolver `break`), `:940-953` (pinned/artifacts park); `plane.rs:468-497` | The asymmetry is by design: the resolver is the only supervised sibling (its handle is in `spawn_supervisor`'s list, `plane.rs:1003-1009`), while `pinned_rx` closes with ordinary agreement-instance teardown and the artifact write-back `PARK`s specifically so its clean exit cannot cancel the node (`plane.rs:491-495`). Recorded in the journal §В.2 D-18. | Tried to find a second fatal arm: `artifacts_rx`'s sender is held by the parking write-back, `pinned_rx`'s by per-view instances; neither is fatal. | confirmed by code |
| D-19 | **REMOVED BY REVISION, correctly** | `actor.rs:2165-2170` (the only epoch check); `p2p/src/lib.rs:376-392` (classify order, unchanged) | Both labels (`window_unset`, `not_member`) are gone from the beacon; the actor does not call `classify` at all. `classify` still checks tombstones before the window (`p2p/src/lib.rs:377-379`), but its only beacon consumer is the gate, which reports `untracked`/`secondary` (`dpos.rs:112-124`). | Tried to find an actor path that still reads `Ingress`: `grep Ingress` in `beacon/actor.rs` = 0. `Ingress::member_of` remains used only by `slasher/gossip.rs:161`. | confirmed |
| D-20 | **PARTIALLY FIXED** | `stand.rs:2440-2445`, `:2996`, `:2410-2414`; `fakes.rs:1131-1146`; `tombstone.rs:24-25` | The requested form fix landed: the stand's window is now `TrackedWindow::default().with_tombstones(...)` over the same `TombstoneSet` that is moved into `OuterBuilder` (`:2440-2445`, `:2996`; `TombstoneSet` is `Arc<RwLock<..>>`, so the clone shares, `tombstone.rs:24-25`). But the set is `TombstoneSet::default()` and nothing fills it — `cfg.tombstoned` goes only to `FakeStaking::with_schedule` (`:2410-2414`), which merely flags validators (`fakes.rs:1131-1146`); the snapshot keeps the tombstoned validator in `primary`. So the stand-green / production-red consequence D-20 named is still unrepresented: the predicate can never fire. Implementer records the emptiness (journal §В.2 D-20). | Tried to show `FakeStaking` drops tombstoned validators from the snapshot: `fakes.rs:1131-1141` keeps them and only sets `validator.tombstoned`; `reader.rs:694-705` copies the flag through. So the predicate is load-bearing and empty. | confirmed by code |
| D-21 | **FIXED** | `epoch_transition.rs:744-754` | The stale sentence "the beacon's own per-epoch check reads the SAME write-once slot through `committee_for`" is replaced by: a skipped-record member is refused at the pre-decode gate until the next `track`, dealings re-send, and the seat is the consumer's check over the same slot. | The replacement's closing clause "so no second reading of membership can disagree with this one" still overclaims (gate window vs `committee_for`) — residual NIT = E-10; the specific D-21 claim (`committee_for` as the ingress reader) is gone. | confirmed |

No BLOCKER established. The nearest to one is E-01 (a new member's confirm refused at the gate,
no re-mint), which exists on HEAD and has the leader's proposal-carried confirms as a fallback.

---

## Part B — fresh review of the change as it stands

### B.1 Rebuilt frame × sender-epoch-offset table

Sender is an honest member of `C[T]`; steady state `E_w == now`; codes as in the header.

| body | d | sender | HEAD (production) | round 2 | re-delivery |
|---|---|---|---|---|---|
| Commitment/Share | +2 | continuing | B | B | dealer retransmit each pre-seal tick (`ceremony.rs` retransmit; bounded by the `now+2` window, `actor.rs:2133-2137`) |
| Commitment/Share | +2 | new | G | G | dealer retransmit until the receiver's transition tracks `T-1` (window gains `C[T]`, `epoch_transition.rs:779-791`); dealer seals at `start(T)-20`, after that track |
| Commitment/Share | +1 | continuing | live A / else B | live A / else B | retransmit if B; live path acked |
| Commitment/Share | +1 | new, K-gap (`E_w = now-1`) | G | G | retransmit after the next `track` (pre-existing 4.3-A) |
| Commitment/Share | 0 | member | live A / else D | live A / else D | retransmit while un-acked |
| Commitment/Share | −1 | member | live A / else R:epoch | same | none needed (dealing closed) |
| Commitment/Share | ≤ −2 | continuing, live | A | A | none needed (sealed) |
| Commitment/Share | ≤ −2 | rotated-out, live | G | G | none needed |
| Commitment/Share | ≥ +3 | continuing | R:epoch | R:epoch | dealing retransmit (R-126) |
| Ack | +1/0 | member, live | A | A | player re-emits cached ack on a dealer retransmit (`ceremony.rs:233-238`) |
| Ack | +2 | (vacuous) | D | D | — |
| Ack | −1/≤−2, live | member | A (gate permitting) | A | none needed |
| Ack/Reveal | any | no live ceremony | D | D | Reveal: resolver by pinned hash; Ack: none needed |
| Reveal | +1/0, live | member | A | A | resolver by pinned hash / proposal-body pull |
| Reveal | ≤ −2, live | continuing | A | A | resolver by pinned hash |
| Reveal | ≤ −2, live | rotated-out | G | G | resolver (gate-refused gossip; refetch still works) |
| Confirm | +2 | continuing | recorded | recorded | — |
| Confirm | +2 | new | G | G | **none**: `mint` edge-triggered (`confirmations.rs:186-189`); fallback proposal-carried (`dkg_agree.rs:1158`, `:1228-1251`) ⇒ **E-01** |
| Confirm | +1 | continuing, live | recorded | recorded | — |
| Confirm | +1 | new, K-gap | G | G | none (same fallback) |
| Confirm | 0 | member | recorded | recorded | — |
| Confirm | −1, live ceremony | member | R:confirm_window | R:confirm_window | none needed |
| Confirm | −1, no live ceremony | member | R:epoch | R:epoch | none needed |
| Confirm | ≤ −2, live | continuing | R:confirm_window | R:confirm_window | none needed |
| Confirm | ≥ +3 | member | R:epoch | R:epoch | none needed (R-126) |
| Confirm | in-window, not in `C[T]` | stranger | R:not_member | R:no_seat | — |
| any | any, window `None` | any | gate admits; actor `committee_for` | gate admits; consumer seat | regrouped, same outcomes |

**Where this differs from the implementer's table (journal §В.3):**
1. Rows it labels "член" for `d ≤ −1` (e.g. "Commitment/Share ≤ −2, live: A") are only A when the
   sender is *continuing* (window member). A rotated-out member is gate-refused on both HEAD and
   round 2 (`actor.rs:2264-2271` is never reached). The outcome still equals HEAD, so the
   "round 2 == HEAD on every cell" claim survives, but the cell annotation is optimistic.
2. `Confirm d=-1` is `R:epoch`, not `R:confirm_window`, unless a live ceremony for `now-1` exists:
   `on_message` runs `epoch_is_actionable` for *every* body before the `Confirm` interception
   (`actor.rs:2228-2230`, `:2247-2250`); `confirm_window` needs the live-ceremony short-circuit
   (`:2166-2167`). The implementer's row (and round 1's) collapses the two.
3. `d ≥ +3` from a *new* sender is gate-refused (`G`), not `R:epoch`, because the gate precedes the
   actor; the implementer's `R:epoch` holds only for continuing senders.

No cell that a needed frame relies on is "nothing": the only no-re-delivery cell is confirm-from-new
(E-01), and round 2 introduces none — every round-2 outcome equals HEAD's.

### B.2 Findings

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| E-01 | **SERIOUS** (pre-existing, not introduced) | `node/dpos.rs:1894-1899`, `consensus/src/dpos.rs:112-124`; `confirmations.rs:186-189`; `dkg_agree.rs:1158`, `:1228-1251` | A full-width `ShareConfirm` for a target whose signer is a genuine `C[T]` member but NOT in the receiver's last-tracked window primary (a rotated-in/new member, `epoch_transition.rs:779-791`) is refused at the pre-decode gate as `secondary`/`untracked`. `Confirmations::mint` is edge-triggered on width growth and skips `previous >= confirmed.len()` (`confirmations.rs:186-189`), so the frame is never re-sent; the receiver's entry bar can permanently undercount that member. This is the one no-re-delivery cell; it exists on HEAD (the gate is 4.3-A). | Tried to close it with the leader's proposal: `attempt_proposal` embeds `self.confirms.covering(...)` (`dkg_agree.rs:1158`) and the receiver validates the proposal's confirm list (`:1228-1251`), so a lagging member can still be counted if some leader received the direct confirm. Whether some leader always does is not traced end-to-end; a leader on the same lagging clock is not covered. | confirmed by code (refusal + no re-mint); inferred (liveness impact) |
| E-02 | **MODERATE** (pre-existing) | `node/dpos.rs:1560-1652`, `:1894-1899`; `epoch_transition.rs:803-824`; `actor.rs:1322-1327`, `:2165-2170` | The "one source" is really two: the gate reads the transition's assembled window, the consumer reads `committee_for`. They can disagree when the transition SKIPS an uncommitted neighbour record (`epoch_transition.rs:809-817`) or when the actor clock (non-finalized ordering tip, `actor.rs:1322-1327`) outruns `E_w`; the gate then refuses a genuine member before the more accurate consumer can accept it. Round 2 did not create this (HEAD had the same gate), but it did not remove it, and the constant's "one source" prose (`actor.rs:131-138`) overstates. | Tried to show the window always carries the same records as `committee_for`: `push_neighbour_committee` deliberately omits an empty read (`:811-817`), so no. Tried to bound by time: the next `track` carries the record and dealings re-send, but confirms do not (E-01). | confirmed by code (mechanism); inferred (duration/impact) |
| E-03 | **MODERATE** | `stand.rs:2410-2414`, `:2440-2445`, `:2996`; `fakes.rs:1131-1146`; `reader.rs:694-705` | The stand's new tombstone predicate is connected to a `TombstoneSet` that nothing ever fills (`TombstoneSet::default()`, `stand.rs:2440`); `cfg.tombstoned` is routed only to `FakeStaking::with_schedule` (`:2410-2414`), which sets a flag on the validator (`fakes.rs:1135-1139`) but keeps it in the snapshot. So a "tombstoned" stand node remains a window `Member` and is admitted, while production would drop it `untracked`. Form matches production; the stand-green/production-red consequence D-20 named is not actually closed. | Tried to show the flag removes the peer from the tracked set: `assemble_tracked_peers` copies `snap.validators` wholesale (`epoch_transition.rs:783-787`) and `reader.rs:694-705` carries `tombstoned` through as a field. So no. | confirmed by code |
| E-04 | **MINOR** | `testbed/tests.rs:3121-3127`, `:3161-3167`; `stand.rs:842-859`, `:3033` | The new stand test's `beacon(reason)` uses `counter_of` with only a `reason` filter; `counter_of` sums across channels and nodes (no channel label attached, `stand.rs:842-859`). The actor-side helper does filter `channel="beacon"` (`actor.rs:5437-5457`). Today only the beacon gate emits `secondary` (evidence ingest is `slasher_evidence: None`, `stand.rs:3033`; the other ingress site uses `"epoch"`, `slasher/gossip.rs:166`), so the exact equality is sound; if evidence ingress is ever wired, the assertion breaks (safe) and unrelated emitters are indistinguishable. | Tried to find a second `secondary` source: `grep record_ingress_drop` over `crates/` gives only `slasher/gossip.rs:121` and `dpos.rs:122`; the stand wires no evidence path. | confirmed by code |
| E-05 | **MINOR** | `actor.rs:2207-2236`, `:1633-1640`, `:1668-1670`, `:1678-1685`; `consensus/src/dpos.rs:112-124` | `refuse` is the single *counted policy* refusal, but several ingress drops remain uncounted and mostly unlogged: outer/header/body decode failures return silently (`:2207-2236`; the gate logs nothing either, `dpos.rs:112-124`), the envelope/signed epoch mismatch is `debug!` only (`:1633-1640`), an unreadable roster returns silently (`:1668-1670`), and a `pool.record` failure is `debug!` only (`:1678-1685`); a within-window Ack/Reveal with no live ceremony also falls through uncounted. A metric reader cannot separate "malformed" from "nothing to do". Pre-existing. | Tried to read the map's "one refusal site" as covering all drops: it names the three membership/window refusals (`epoch`, `window_unset`/`not_member`, `confirm_window`); decode errors and "no ceremony" are not refusals of a seat. | confirmed by code |
| E-06 | **MINOR** (latent, pre-existing) | `actor.rs:2259-2271` vs `ceremony.rs:510-514`; `ceremony.rs:798-808`; `actor.rs:2288-2292` | `has_seat` binds `from` for every ceremony body, including `Reveal`, where the consumer (`ceremony.rs:510-514`: `signed.check(&self.info)` returns the SIGNER `pk`) binds the dealer. A Reveal relayed by a peer outside the ceremony roster is refused `no_seat` before the signature check; the actor's own comment contemplates such relays ("a peer may relay another dealer's valid Reveal", `actor.rs:2288-2292`). No in-tree relayer exists (the dealer broadcasts its own; `handle` emits nothing for a Reveal), and HEAD's `beacon_member` was equally strict, so this is latent. | Tried to find a relay path: `DkgBody::Reveal` handling emits nothing (`ceremony.rs:510-514`) and the only seal-broadcast is the dealer's own; p2p `from` is authenticated. | confirmed by code (path); inferred (reachability) |
| E-07 | **MINOR** (out-of-diff doc invalidated by the change) | `consensus/src/dpos.rs:73-78` | The `GatedReceiver` doc still says the per-epoch committee half "needs the frame's own epoch and therefore lives at each channel's own entry (`beacon::actor::on_message`, `slasher::gossip::ingest_batch`)". Round 2 removed exactly that from `beacon::actor::on_message`: the beacon's epoch half is now the pure `[now, now+2]` cost gate (`actor.rs:2165-2170`) and the seat is the consumer's (`:2264-2271`). Same class as D-21 but in a file outside the write list. | Tried to read the sentence as still true for evidence: `slasher/gossip.rs:160-164` does keep `ingress.member_of(epoch)`, so only the beacon half is stale — but the doc names both sites. | confirmed by code |
| E-08 | **MINOR** | `actor.rs:1673-1676`, `:2264-2271`; `consensus/src/dpos.rs:116-120`; journal §В.8 п.3 | The journal's claim that `no_seat` is unreachable in production "while the gate works" (prod value always 0) is false: the gate admits a sender that is a `Member` of ANY of the three window epochs (`dpos.rs:116-120`), while `on_confirm` requires the sender in `committee_for(target_epoch)` (`actor.rs:1673-1676`); a window member that signs for a different epoch (Byzantine; honest `mint` cannot, `confirmations.rs:170-179`) is refused `no_seat`. The same holds for the live-dispatch check. So the label is live in production against cross-epoch traffic, not only "gate removed/relay". | Tried to show an honest sender can never be in the window but not the target roster for a body it emits: for dealings/acks the sender is in `committee_for(epoch)` by construction; only a malicious cross-epoch confirm reaches `no_seat`. That still refutes "always 0". | confirmed by code |
| E-09 | **MINOR** | `actor.rs:1989-2011`, `:2322-2338`, `:1379-1384` | Round 2 lets a within-window dealing from a sender NOT in the target epoch's committee occupy a `pending` slot until `maybe_start` drains it and refuses `no_seat` (`:1989-2011`) or the per-tick `retain` evicts it (`:1379-1384`); HEAD refused it at `beacon_member` before buffering. The slot is per-sender and epoch-evicted, so it is bounded (the journal's M1 measurement is the un-gated budget), but it is a behaviour change: a stranger's frame now costs a map insert per epoch. | Tried to show the buffer is unbounded: the outer map is visitor-keyed and the `retain` drops every `e <= now` or dealing-closed epoch each tick (`:1379-1384`); with the gate, only window members can insert. | confirmed by code |
| E-10 | **NIT** | `epoch_transition.rs:746-754` | The rewritten comment's closing clause "the seat ... through `committee_for` — so no second reading of membership can disagree with this one" overclaims: the gate reads the transition's assembled window, which SKIPS an uncommitted neighbour (`:803-824`), while the consumer reads `committee_for`; those two readings can disagree (the gate's is stricter). The paragraph's earlier sentence already states the gate refusal, so the clause is at least self-undercutting. | Tried to read "this one" as the consumer's `committee_for` reading and "second reading" as the ceremony's `Info`, both the same slot: true for the ceremony, but the clause follows a sentence about the gate. | confirmed by code; inferred (severity) |
| E-11 | **NIT** (test coupling) | `testbed/tests.rs:3150-3157`; `stand.rs:1633-1634` | The exact equality `(secondary, untracked) == (sent, 0)` is coupled to node 4 being in the active registry (hence `Tracked`→`secondary`). It currently is (`PeerSet::AllNodes => pks.clone()`), so the test is correct; a config change that dropped node 4 from the registry would flip it to `untracked` and fail for a fixture reason, not a gate defect. | Tried to show `AllNodes` could exclude node 4: `stand.rs:1633-1634` includes all keys, and the test also asserts the four members send 0 (`tests.rs:3153-3157`). | confirmed by code |

### B.3 Answers to the nine re-asks

1. **Table rebuilt** in B.1; differences from the implementer's are the three items below the table
   (rotated-out `d ≤ −1` rows are gate-refused, `Confirm d=−1` without a live ceremony is
   `R:epoch`, `d ≥ +3` new senders are `G`). The only needed frame with no re-delivery is E-01.
2. **`has_seat` binding**: the roster is the right single binding for every ceremony body because
   `info_for` passes the SAME committee as dealers and players (`ceremony.rs:247-260`, Mode B), so
   dealing/ack/reveal all reduce to "in `committee[epoch]`". A legitimate ceremony body cannot be
   `no_seat`-refused by the actor: `maybe_start` only builds a ceremony when `committee_for(target)`
   is `Some` (`actor.rs:1924-1936`) and the ceremony stores that exact set (`ceremony.rs:460`,
   `:930`), including player-only resume (the roster is `next`, `actor.rs:2070-2077`). Confirms:
   `on_confirm` reads the target roster and the minter signs against the same slot
   (`confirmations.rs:170-179`). Acks bind the sender==player==dealer under Mode B. The drain applies
   the same `c.has_seat` (`:1997`) as the live dispatch (`:2267`). Caveats: Reveal binds the sender
   while the consumer binds the signer (E-06, latent), and cross-epoch confirms make `no_seat`
   live (E-08).
3. **Stand gate**: the window is recorded in `TrackSink::track` before forwarding (`stand.rs:1516-1519`),
   the same "record before register" order as production (`p2p/src/lib.rs:314-321`), and it is filled
   at `cold_start` (`epoch_transition.rs:625-630` → `stand.rs:2471-2475`). `with_tombstones` shares
   the `OuterBuilder`'s `TombstoneSet` (E-03 caveat: it is empty). Every `Beacon::Live` role takes the
   gated arm; `AbsentBeacon` has no beacon (`stand.rs:2782-2783`, `:2887`). The `StrayDealer` test
   DOES depend on the gate: with it absent, `secondary=0` against `sent>0` fails `tests.rs:3161` and
   `no_seat>0` fails `:3170` (journal M1). The assertion also cannot be faked by a consumer refusal
   for the same reason.
4. **`refuse` single site**: the only `record_ingress_drop` in `beacon/` is `actor.rs:2199`; all five
   refusal sites go through it (E-05 lists the remaining uncounted decode/roster/record drops, which
   are not membership refusals; `dpos.rs:122` is the gate's own count).
5. **Resolver `break`**: `resolver_rx` is `Some` only when wired (`plane.rs:736-737`, `:854`), and its
   only sender is moved into the resolver engine (`plane.rs:325-342`); the engine is a supervised
   child (`plane.rs:1001-1009`) whose supervisor resolving makes the node's `supervise` cancel the
   shutdown token (`node/dpos.rs:614-655`, `:797`). Nothing else in `run` holds unsaved state
   (`append_journal` is synchronous, `actor.rs:855-872`); the drop of `self` closes the actor's
   channels. Benign `None` is reachable only during shutdown (engine aborted) — the `Option::None`
   birth case parks (`actor.rs:316-319`, `:892`). Verified no second holder of the sender.
6. **Re-exports**: `beacon/mod.rs:167` gates the whole `testing` module with `#[cfg(test)]`; the
   ungated `DkgCeremony/DkgBody/DkgMsg/BeaconMessage` additions (`:199-206`) are `pub(crate)` inside
   it, so nothing widens the crate's public surface. Concrete gating: only `CommitteeFor`, `info_for`,
   `DealerReveal` remain behind `dpos-devnet-byzantine` (`:197-198`).
7. **Test quality**: all five new/rewritten tests have falsifiers that a real mutation breaks, and I
   found no way to be green with the property broken — except E-04/E-11's attribution coupling
   (fixture/observation, not the property) and the fact that the window unit cannot distinguish a
   site re-implementing `[now, now+2]` locally from the shared function (semantically equivalent).
   The resolver unit proves exit, not the fatal supervisor chain; the deque unit proves the bound,
   not that `propose` rebuilds (D-13), and its `drop(engine.start)` premise is verified.
8. **Hygiene/dead code**: nothing orphaned. `ValidatorInputs.ingress_window` is gone; the node's
   `ingress_window` still feeds the evidence gate (`node/dpos.rs:1814-1820`) and the beacon gate
   (`:1896`); the stand's `TrackSink.window`/`stray_sends` are used (`:1519`, `:2826`, `:3423`);
   `Ingress::member_of` still has its slasher caller (`slasher/gossip.rs:161`); no new `#[allow]` and
   every added `unwrap`/`expect` is inside `#[cfg(test)]`.
9. **Weakest parts, ranked**: (1) no cargo run — every green/red claim and every metric count is the
   implementer's; (2) E-01's liveness consequence (proposal fallback not traced); (3) E-02's clock/
   window direction (I read the ordering, did not measure the K-gap); (4) E-03 (I read that nothing
   fills the `TombstoneSet`, not a live stand run); (5) E-04 attribution; (6) E-06 Reveal relay
   reachability.

---

## Leave as is

- **`refuse` as the single counted site** (`actor.rs:2189-2200`) with the shared
  `dpos_ingress_dropped_total` family (`consensus/src/dpos.rs:63-67`, R-070). Do not split the
  registry; do not add a second counter for the gate.
- **`within_ingress_window` as the one pure rule** (`actor.rs:163-165`) with `is_bufferable` keeping
  its own `epoch > now` on top (`:2133-2137`) and `epoch_is_actionable` keeping the live-ceremony
  short-circuit (`:2165-2170`): the superset + per-site refinement is correct and the new unit pins
  it.
- **`on_confirm`'s `last_height == None` exemption and the deliberate `confirm_window` asymmetry**
  (`actor.rs:1649-1667`, `:1619-1628`): a confirmation is one-shot, a dealing is not.
- **The resolver `break`** (`actor.rs:926-934`) and its supervision chain (`plane.rs:1001-1009`,
  `node/dpos.rs:614-655`, `:797`): the arm is the only fatal one, the error line names the chain, and
  the unit proves exit rather than timeout. The `pinned_rx`/`artifacts_rx` park arms stay as they are
  (D-18).
- **The stand wiring**: per-node `TrackedWindow` recorded before forwarding (`stand.rs:1516-1519`),
  `GatedReceiver(.., "beacon", true)` on the same receiver the actor reads (`:2887`, `:2897`),
  `with_tombstones` over the `OuterBuilder`'s set (`:2440-2445`, `:2996`) — the production shape
  (the empty set is E-03, not a reason to rewrite this).
- **`DkgCeremony::has_seat`** (`ceremony.rs:798-808`) via the roster stored at start/resume
  (`:191-195`, `:460`, `:930`) rather than a fresh committee read at the frame: it is the right
  consumer-side binding under Model B and it removes the transient-read refusal.
- **The new `Role::StrayDealer` and its test** (`stand.rs:661-671`, `:2786-2830`;
  `tests.rs:3101-3184`) and the exact `(secondary, untracked) == (sent, 0)` attribution: this is the
  first stand execution of the pre-decode gate, and the test is red under M1.
- **`deque_size = 2` with the production-engine unit** (`dkg_transport.rs:123-147`, `:185-264`):
  the constant is the orchestrator's; the unit reads the real one and its premise is verified.
- **`#[cfg(test)]`-scoped `testing` re-exports** (`beacon/mod.rs:167-206`): the ungating of
  `DkgCeremony`/`DkgBody`/`DkgMsg`/`BeaconMessage` is test-only and does not widen the public surface.
- **No `#[allow]`, no production-path `unwrap`/`expect`, no gratuitous `pub`**: keep that discipline.
