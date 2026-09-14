# Independent review, round 1 — DKG dealer-log identity `(dealer, hash)` (PLAN row 5.3, заход Б)

Baseline: `git diff HEAD` in this workspace, HEAD `18573e76`, 13 modified files under
`crates/dpos/consensus/src/` (+1747/−559). Inputs read as INPUT, not evidence:
`dsh-input-task.md` (the brief) and `dsh-input-journal.md` (the implementer's journal).
No cargo was run; this is a reading review. Every claim below cites a `file:line` I opened.
Commonware sources were read read-only under
`~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c/`.

Notation for confidence: `confirmed` = read directly in the code cited; `inferred` = follows
from code I read but was not executed/traced end-to-end.

## Verdict summary

No BLOCKER found. The pinned-set agreement is untouched (`AgreedSet.pinned`,
`DkgProposal`, `PinnedLogs`/`derive`, `covers` are byte-identical), the wire decode is total,
and the `(dealer, hash)` key is applied consistently across the recording and fetch paths.
The substantive issues are: one explicit brief requirement (test `:3293` "remains a stop")
was overridden; the out-of-scope `ArtifactPull` change is a real production behaviour change
with an unbounded bookkeeping map; and the required `.claude/dpos_architecture/*` doc edits
are not present in (and gitignored from) this workspace, so doc-drift cannot be confirmed.

---

## Answers to the ten questions

### (1) Correctness of the key change — what moved, what was deliberately left

Moved to `(dealer, hash)` (all `confirmed`):

| Structure | Key now | Anchor |
|---|---|---|
| `DkgCeremony.signed_logs` | `BTreeMap<LogId, DealerReveal>` | `ceremony.rs:205` |
| `recorded` (`BTreeSet<PeerPubkey>`) | **deleted**, subsumed by `signed_logs` keys | `ceremony.rs:188-234` (no field) |
| `logs: Logs<..>` (write-only) | **deleted** | `ceremony.rs:188-234` |
| `DkgCeremony.equivocations` | `BTreeMap<PeerPubkey, DealerEquivocation>` (by dealer, deliberately) | `ceremony.rs:218` |
| `checked_serve_map` | `BTreeMap<LogId, _>` | `ceremony.rs:281-295` |
| `take_signed_logs` / `DealerLogStore` / `ServeMap` | `LogId` | `ceremony.rs:759`, `log_store.rs:72,116,132,140` |
| `Step.recorded_log` / `nondurable_logs` | `LogId` | `ceremony.rs:109`, `actor.rs:455` |
| `DkgLogKey` / `RecomputeState.want` | `(epoch,dealer,hash)` / `LogId` | `log_resolver.rs:61-64`, `actor.rs:165-168` |
| `recompute_scoped` scope | `BTreeMap<PeerPubkey,B256>` | `ceremony.rs:1192-1225` |
| `fetch_missing_logs`, `serve_log`, `ingest_log`, `ingest_recompute_log`, `dkg_agree::fetch_bodies` | exact `(dealer,hash)` | `actor.rs:2266-2282,2753-2763,2807,2860`; `dkg_agree.rs:1393-1406` |

Deliberately still dealer-keyed (all `confirmed`):

- `first_log: BTreeMap<PeerPubkey, B256>` (`ceremony.rs:210`) — the first-recorded hash,
  consumed by `signed_log_hash` (`:1016-1018`), `own_log_recorded` (`:739-741`), the
  re-broadcast on resume (`:971-975`) and `Player::resume`'s `log_map` (`:868,893,901`).
- `recorded_dkg_logs`/`DkgLogIndex` (idx→hash) and `Confirmations`: `publish_recorded_logs`
  inserts `signed_log_hash(pk)` (`actor.rs:1495-1505`), i.e. first-wins per seat.
- `pending_pub`/`pending_priv`/`unsent`/`emitted_acks` (dealer is the right key there).

Path where a log is still selected "by dealer alone": **the publish/confirm/propose path**
(`first_log` → `signed_log_hash` → `recorded_dkg_logs`). It is first-wins and stable. It does
not reintroduce a *recording* first-wins: the ceremony can hold and, crucially, *finalize over*
the pinned body even when its first body differs, because `scoped_pinned_logs` selects by exact
`(pk,hash)` (`ceremony.rs:1053`) and `recompute_scoped` filters by the pinned map
(`ceremony.rs:1218`). The published first hash is what makes the stand's tamper witness live
(below), so it is intentional; the consequence is D-11.

`checked_serve_map` is by `LogId` and serves **both** bodies of an equivocating dealer
(`ceremony.rs:289-292`), matching `serve_log`'s exact-body contract (`actor.rs:2753-2762`).

### (2) The ban: refused vs still accepted; can it be forged; does the pinned refetch still work

- **Refused:** a `DkgBody::Reveal` whose *signer* (not sender) is in `equivocations`
  returns an empty `Step` (`ceremony.rs:462-470`).
- **Still accepted:** resolver delivery `ingest_signed_log` explicitly bypasses the ban
  (`ceremony.rs:775-798`); journal replay bypasses it (see Q3); our own seal; and a
  banned dealer's `Commitment`/`Share`/`Ack` are still processed by `try_ack`/the `Ack` arm
  (`ceremony.rs:438-461`) — the ban is a *log-ingress* ban only (D-12).
- **Pinned body of a banned dealer still reaches the victim:** `fetch_missing_logs` asks
  `(e,D,pinned_hash)` (`actor.rs:2266-2282`), the holder serves the exact body or nothing
  (`actor.rs:2753-2762`), `ingest_signed_log` records it and (on the victim) creates the pair
  (`ceremony.rs:790-795`, `insert_log` `:496-508`), and finalization is by exact id
  (`ceremony.rs:1053`). This is the acceptance path and it is real (`confirmed`).
- **A single forged log cannot trigger the ban:** `insert_log` only creates
  `DealerEquivocation` on a *second* distinct hash for a dealer whose first body is still held
  (`ceremony.rs:491-511`), and both bodies must pass `SignedDealerLog::check`, which verifies
  the dealer's signature over the log transcript
  (`commonware .../bls12381/dkg.rs:1217-1225`). A log failing `check` is dropped before
  `insert_log` (`ceremony.rs:466-469,784-789`). `log_hash` includes the signature
  (`ceremony.rs:66-68`), and ed25519 is deterministic, so re-delivering the same body cannot
  manufacture a second hash. `confirmed`.

### (3) Journal and replay

- `DealerEquivocation(first,second)` carries **both full bodies** (`share_state.rs:419-423`),
  so replay reconstructs both `signed_logs` entries and the `equivocations` pair/ban. The
  pair is self-contained; no dependence on a separate `PeerLog` for the first body.
- **Live insert and replay both go through `insert_log`**: live via
  `record_checked_log` (`ceremony.rs:529-545`), replay via `shell.insert_log` in `resume`
  (`ceremony.rs:889-905`). Same dedup, same first-log rule, same pair-creation rule. The
  difference is only that replay ignores the returned `Inserted` (it does not re-journal),
  which is correct. `confirmed`.
- **Journal with `PeerLog` for the second hash and no pair record:** replay reconstructs the
  pair anyway — the first `PeerLog` yields `First`, the second yields `Equivocation`, and
  `equivocations` is populated (`ceremony.rs:496-508,898-905`). So the ban survives. This is
  only *unlikely* from a HEAD-written journal: HEAD's `record_checked_log` dropped the second
  valid log entirely (orchestrator's fact; the removed code confirms it), so HEAD never wrote
  two `PeerLog`s for one dealer. It **is** reachable in this change via
  `ingest_recompute_log`, which writes a plain `PeerLog` for a pinned body
  (`actor.rs:2861-2862`) after a failed/nonexistent pair write; so the replay-reconstruction
  is load-bearing, and the `share_state` doc claim that a journal without the pair "holds
  neither" is false (D-08).
- **Crash between the two writes:** the new writer emits the second body *inside the pair
  record* (one `append_journal`), so there is no two-write window; a torn tail is truncated at
  the record boundary by `load_journal` (`share_state.rs:599-619`) and cannot yield half a
  pair. A crash after the first `PeerLog` and before the pair leaves exactly one log and no
  evidence, which is correct. `confirmed`.

### (4) Wire

- Layout `u64_be(epoch) ‖ dealer(32) ‖ hash(32)` = 72 (`log_resolver.rs:83-95`); decode reads
  `u64`, `PeerPubkey`, then 32 bytes with `?` (`:100-109`) — **total, no panic on short input**;
  a 71-byte buffer errors. Tag byte unchanged: `TAG_LOG = 0`, `TAG_ARTIFACT = 1`,
  `TAG_SEED_RETIRED = 2` refused (`:139-177,198-241`). Boxing `Log(Box<DkgLogKey>)` is sound
  and preserves the tag/key bytes (`:198-221,226-241`). Codec test pins length, byte slices,
  round-trip, 71-byte rejection and hash-sensitive ordering (`log_resolver.rs:606-660`).
- No production construction of `DkgLogKey` with a zero/absent hash: the only non-test sites
  are `actor.rs:2274-2279` (pinned hash), `actor.rs:2311-2316` (want hash),
  `dkg_agree.rs:1396-1404` (proposal hash). `DkgLogKey`/`BeaconFetchKey` live in the private
  `mod log_resolver` (`beacon/mod.rs:81`), so this is not an external API change. `confirmed`.

### (5) `fetch_missing_logs` gating: what happens between seal and artifact

`fetch_missing_logs` now requires `self.agreed_pinned.get(e)` (`actor.rs:2245`); without an
artifact it issues nothing, and its `retain` therefore cancels any earlier keys for that epoch
(`:2327-2334`). The replacement pre-artifact path is: (a) gossip `Reveal`s, and (b) the
agreement's own `verify` → `PinnedDerive::Missing` → `dkg_agree::fetch_bodies`, which asks by
the **proposal's** hash (`dkg_agree.rs:1348-1361,1393-1408`) and delivers into the live
ceremony via the shared resolver. I could not construct a concrete regression: a live ceremony
exists only for `committee[epoch]` members, who are exactly the agreement participants, so the
`verify` fetch is available whenever the live finalize would need a body; the artifact itself is
pulled by the separate `ArtifactPull`/repair path. Residual risk (not a demonstrated bug):
before the artifact there is no by-hash fetch *outside* a live agreement instance, so the
window depends on the agreement running — D-17, `inferred`. The journal's own M2 result
(stand stays green with the live key zeroed) confirms the stand is healed through the
`fetch_bodies` leg, not through `fetch_missing_logs` (D-10).

### (6) Out-of-scope `ArtifactPull`

- `minter_to_ask` (`artifact.rs:1709-1730`): picks one `others[*i % len]` per pull, skips
  `self.me`, advances a wrapping counter; `None` when the committee is unreadable or `others`
  is empty → untargeted fetch; `NonEmptyVec::try_from(vec![target]).ok()`. Index arithmetic is
  in-bounds (`others.len() > 0` checked); `wrapping_add` cannot panic.
- Cursor reset: removed only on `Some(PullAnswer::Have)` (`:1774-1779`). The early
  local-store hit (`:1751-1753`) returns before the reset, so an epoch held via another path
  keeps its cursor entry (D-03).
- Committee unreadable → `None` → old untargeted behaviour. `others` empty → `None` →
  untargeted (self is filtered by the resolver, `fetcher.rs:241`). Resolver not tracking the
  target: `fetch_targeted` merely adds a target (`p2p/ingress.rs:86-97`,
  `engine.rs:232-252`, `fetcher.rs:519-529`), `get_eligible_peers` filters to tracked
  participants (`fetcher.rs:233-252`), so an untracked target yields no request and one
  `PULL_TIMEOUT`; the next pull advances the cursor. Note the resolver **accumulates** targets
  for an existing in-flight key rather than replacing them (`fetcher.rs:521-529`), but the
  exhausted pull cancels the key and clears targets (`engine.rs:255-263`,
  `fetcher.rs:534-543`), so the rotation does progress.
- `PULL_TIMEOUT` is per attempt and per epoch (`:1770-1773`, value 8 s at `:183`), but the
  cursor means worst-case rotation latency is ~`8 s × |committee|` (D-04).
- Mutex poisoning: `unwrap_or_else(PoisonError::into_inner)` (`:1723,1777`) resumes with
  possibly half-updated counters, which is safe here. Appropriate.
- Behaviour beyond the stated one: **yes** — `PlaneAcquire` (validator artifact acquisition)
  and the actor's `pull_artifact` / §5.4 heal all now target one committee member per attempt,
  because `ArtifactPull::new(..., Some(me))` is wired in production
  (`plane.rs:346`) and `AcquireArtifact`'s bound is narrowed (`plane.rs:415-421`). It is not
  test-only. `confirmed`.
- **Causal chain** ("removing the pre-agreement fetch exposed this defect"): supported in
  mechanism by the resolver's EMA ranking (`fetcher.rs:218-227`: a fast `NotYet` lowers a
  peer's estimate; untracked peers sit at `initial`) and by the old pre-agreement targeted
  fetches previously penalising the same neighbours. The specific 272× measurement and the
  C7/C8 red→green cannot be reproduced in a reading review; call this `inferred`, not refuted.

### (7) Tests

New/changed tests:

| Test | Location | Non-vacuous witness? |
|---|---|---|
| `a_two_log_dealers_victim_refetches_the_pinned_log_and_keeps_its_share` (rewritten from `:3117`) | `tests.rs:3123-3281` | Yes: `reveals_swapped==reveals_seen`, `log1_hash != log2_hash`, `both_logs_check`, and the victim's own `ShareConfirm` names the forged hash at the dealer seat (`:3132-3185`); plus `equivocation==1` only on node 0 and one WARN with both hashes (`:3229-3252`). The victim-minted assertion is tied to the refetch by the evidence count. |
| `a_two_log_dealer_that_also_withholds_its_partial_is_one_silent_node_within_f` (rewritten from `:3293`) | `tests.rs:3322-3398` | Yes for the tamper: `reveals_swapped`, hashes differ, `both_logs_check`, `schemes_withheld>=1`, `withhold_probe==Some((true,false))` (`:3332-3348`); `equivocation==1` on node 0 ties the crossing to the refetch (`:3389-3393`). It does **not** re-assert demotion metrics on the victim (minor, D-18). |
| `a_dealers_second_valid_log_is_evidence_and_a_ban_but_never_a_replacement` | `ceremony.rs:2295` | Yes: asserts pair order, first-hash stability, gossip ban, exact-hash resolver acceptance, and that pinning `log2` is `Missing` for a node holding only `log1`. |
| `a_replayed_journal_rebuilds_the_recorded_set_and_the_equivocation_pair` | `ceremony.rs:2436` | Yes: compares live vs replayed `signed_logs`/`first_log`/pair and the post-restart ban. |
| `a_held_body_that_is_not_the_pinned_one_is_refetched_by_the_pinned_hash` | `actor.rs:4754-4982` | Yes: exact in-flight key = pinned hash, `serve_log` exact, wrong-key delivery refused. |

Green-while-broken analysis: the **stand** acceptance test can be green with the live
`fetch_missing_logs` key broken (journal's M2; confirmed by the two independent fetch paths,
`dkg_agree.rs:1348-1361` vs `actor.rs:2266-2282`), so the stand is *not* the pin for Q5/Q1's
fetch-by-hash. The new actor unit test is the pin. The two rewritten tests are not vacuous.

Do the ~440 added test lines in `actor.rs`/`ceremony.rs` duplicate the stand tests? The
`actor.rs` test (≈230 lines) repeats the stand's two-log scenario at unit level, but covers the
live fetch key/serve/ingest exactness that the stand does not pin. Complementary with overlap,
not pure duplication (D-10).

### (8) Scope: every hunk outside the brief's file list, and unnamed behaviour changes

Brief's writable list: `beacon/{ceremony,actor,log_resolver,log_store,share_state,dkg_agree}.rs`,
their tests, `testbed/tests.rs`, `.claude/dpos_architecture/*.md`, the journal.

Outside that list, from `git diff --name-only`:

- `beacon/artifact.rs` — **behaviour change** (ArtifactPull rotation, `me`, `attempts`).
- `beacon/plane.rs` — **behaviour change** (wires `me`, narrows the `AcquireArtifact` bound).
- `beacon/metrics.rs` — new counter (arguably required by brief item 3, file not listed).
- `beacon/confirmations.rs`, `beacon/dkg_engine.rs`, `beacon/dkg_msg.rs` — comment-only.
- `.claude/dpos_architecture/*.md` — **absent** (`.claude` is missing and gitignored,
  `.gitignore:47`); `.dpos-study/history/E5-3-B.md` — absent. Journal claims these were
  updated; cannot verify here.

Behaviour changes not named in the brief: (a) ArtifactPull now targets one committee member for
**all** pulls (`artifact.rs:1764-1768`); (b) `agreed_pinned` is retained on an `adopt_share`
refusal instead of always removed (`actor.rs:1747-1761`); (c) the pre-agreement dealer-log
fetch is removed and gated on the artifact (`actor.rs:2245`); (d) `first_log` first-wins now
governs publish/confirm/propose; (e) `Player::resume`'s per-dealer log is the first-recorded
body.

### (9) Hygiene

- No `#[allow]` added; no `unwrap()`/`expect()`/`panic!` added on a production path (all added
  ones are inside `mod tests`/`clock_tests`, verified by diff grep; `minter_to_ask` uses
  `unwrap_or_else(PoisonError::into_inner)`, `fetch_missing_logs` keeps its pre-existing
  `.expect("checked Some above")` `actor.rs:2328`). `confirmed`.
- New `pub`: `DkgCeremony::{journal_record_for,holds,equivocation}` and
  `log_hash`/`LogId`/`DealerEquivocation` are `pub`/`pub(crate)` on `pub(crate)` types inside
  private modules (`beacon/mod.rs:68-86`), so nothing leaves the crate. `confirmed`.
- Dead code: `DkgCeremony.logs` and `recorded` are gone with no dangling `self.logs`/`.recorded`
  references; `recorded_dealers` is gone; `recorded_log_count` is `#[cfg(test)]` with no
  production reader (`ceremony.rs:704-707`). `Logs` type still used by
  `scoped_pinned_logs`/`recompute_scoped`. `confirmed`.
- Stale doc comments remain (D-05), and two new doc comments overstate the code (D-08, D-13).

### (10) Where this review is weakest (ranked)

1. The `ArtifactPull` rotation: I read the commonware resolver semantics but could not execute
   C7/C8 or measure the 272× claim; the causal chain is `inferred`.
2. Liveness between seal and artifact (Q5): I argued from reachability, not from a failing
   trace; no concrete regression found but not proven absent.
3. `Player::resume`/`first_log` interaction: I traced commonware `resume`/`finalize`
   (`dkg.rs:1726-1758,1804-1850`) and believe it is liveness-only, but did not exhaustively
   enumerate the crash/journal states.
4. The `attempts` leak magnitude (bounded per epoch, unbounded per process) is estimated, not
   measured.
5. The `.claude` docs: absent from this workspace, so the journal's doc-drift claim is
   unverifiable rather than disproven.

---

## Findings

| id | severity | file:lines | what is wrong | how I tried to refute it | confidence |
|---|---|---|---|---|---|
| D-01 | MODERATE | `crates/dpos/consensus/src/testbed/tests.rs:3324-3398` (brief item 5 vs `dsh-input-task.md:17,25`) | The brief requires `a_two_log_dealer_that_also_withholds_its_partial_stops_the_chain_silently` to remain a stop; it was rewritten (renamed) to expect the chain to cross 64. The causal mechanism is sound (victim mints ⇒ 3 honest partials = `quorum(4)`, `combined_scheme.rs:285-288`), but the explicitly required "stops silently" property is no longer tested. | Checked the new witnesses (`schemes_withheld`, `withhold_probe == Some((true,false))`, `equivocation==1` on node 0, all four `dkg_ceremony_ok==1`) — they establish the tamper and the refetch, so the test is not vacuous; tried to find a path where the witness could be green without the tamper and found none. The deviation is real. | confirmed (deviation), confirmed (mechanism) |
| D-02 | MODERATE | `beacon/artifact.rs:1709-1730,1764-1768`; `beacon/plane.rs:346,415-421` | Out-of-scope production behaviour change: every artifact pull now calls `fetch_targeted` with one committee member per attempt (validator `PlaneAcquire` and the live/heal `pull_artifact`), not just the C7/C8 stand tests. | Checked all `ArtifactPull::new`/`.pull` callers: only `plane.rs` in production; confirmed the new bound is on the real `AcquireArtifact` impl. Cannot refute that behaviour changed. | confirmed (behaviour), inferred (causal chain) |
| D-03 | MINOR | `beacon/artifact.rs:1674,1709-1730,1751-1753,1774-1779` | `attempts: HashMap<u64,usize>` is never pruned for epochs that never resolve (unlike `next_allowed`, pruned at `:1825`), and the early local-hit return at `:1751-1753` does not remove the entry; one entry per epoch ever pulled, unbounded over process life. | Looked for a `retain`/age-out on `attempts` — none; checked the doc claim "Cleared once the epoch is held" — only the `Have`-after-fetch path clears, not the store-hit path. | confirmed |
| D-04 | MINOR | `beacon/artifact.rs:1709-1730,1770-1773,183` | The rotation makes worst-case acquisition latency ~`PULL_TIMEOUT (8s) × |committee[epoch]|` when successive targets do not answer; with the cursor wrapping this can repeat. Each attempt is bounded, but the walk as a whole is not. | Confirmed `PULL_TIMEOUT` is per attempt, `PULL_MIN_INTERVAL` throttles between attempts; the resolver accumulates targets for in-flight keys but cancels on exhaustion, so this is real, not mitigated by parallel target expansion in the single-waiter case. | inferred |
| D-05 | MINOR | `beacon/metrics.rs:56`; `beacon/ceremony.rs:1033`, `beacon/ceremony.rs:1467-1470`; `beacon/confirmations.rs:37` | Stale doc comments still describe a per-dealer resolver fetch / per-dealer durability: metrics.rs "the resolver fetches per-DEALER"; ceremony.rs "the resolver fetches per-DEALER over the roster"; confirmations.rs "the exclusion is per-dealer" (it is now per `LogId`). The journal's claim that no stale doc comments remain is refuted. | Grepped `per-dealer|by dealer` across `beacon/`; these survived the change. Behaviour is correct; only the comments lie. | confirmed |
| D-06 | MODERATE | `.gitignore:47`; required `.claude/dpos_architecture/{00_preamble,05_identity_crypto,13_invariants_gotchas_rules}.md` (brief item 6) | The required in-change doc updates are not present and cannot be reviewed: `.claude/` does not exist in this workspace and is gitignored, so no doc edit can appear in `git diff`. The journal asserts they were made. Doc-drift was declared a review blocker by the brief. | `ls .claude` → missing; `git check-ignore -v` → `.gitignore:47:.claude`. Cannot distinguish "made but not exported" from "not made". | confirmed (workspace state), inferred (deliverable) |
| D-07 | MODERATE | `beacon/actor.rs:1733-1737`; `beacon/ceremony.rs:218,719-721` | The evidence pair lives only on the ceremony, which is removed at finalize, not at sweep; there is no actor-level evidence field. Brief item 3(a) asked for the pair in RAM "until sweep". After finalize only the journal + serve store keep both bodies. | Searched for any actor field holding the pair / any reader of `Step.equivocation` beyond the one-shot `note_equivocation` (`actor.rs:1612-1631`) — none. Not a safety bug, but a spec deviation for заход А. | confirmed |
| D-08 | MINOR | `beacon/share_state.rs:377-385,492-498`; `beacon/ceremony.rs:898-905` | The `DealerEquivocation` doc overclaims twice: (a) "a journal without it holds neither, and can never replay a pair it does not evidence" is false — replay reconstructs the pair/ban from two `PeerLog`s (reachable via `ingest_recompute_log`, `actor.rs:2861-2862`); (b) "both halves ... must name the same dealer under different hashes, else the record is dropped as a whole" — `parse_journal_inner` does not enforce this, and replay inserts the two bodies independently under their own checked keys. | Read the parse (`share_state.rs:492-505`) and the resume loop (`ceremony.rs:898-905`); the M3' mutation in the journal itself demonstrates the pair is rebuilt from two `PeerLog`s. | confirmed |
| D-09 | MINOR | `beacon/dkg_agree.rs:1348-1361`; `beacon/actor.rs:2266-2282` | The stand acceptance test does not pin the live `fetch_missing_logs` by-hash path: the victim is also healed by `verify`'s `fetch_bodies` by proposal hash, so the test can stay green with the live key corrupted (the journal's own M2). Only the new `actor.rs` unit test pins it. | Traced both fetch paths to the shared `ingest_log`; confirmed they use independent keys and either can supply the victim. Not a code bug, a coverage gap / answer to Q7. | confirmed |
| D-10 | MINOR | `beacon/ceremony.rs:868,893,901,971-975,1016-1018` | `Player::resume`'s `log_map` and the resume re-broadcast use the **first-recorded** body per dealer, which can be the non-pinned body (the victim's case). This affects only commonware's `MissingPlayerDealing` pre-check (`dkg.rs:1743-1754`) and resume liveness; finalize still re-checks against the pinned `Logs` (`dkg.rs:1817-1823`) and the heal scopes to pinned, so no wrong-share risk — but a resumed node can fail `resume` (or the finalize pre-check) on the wrong body and fall back to the heal. | Read commonware `Player::resume`/`finalize`; confirmed `resume` uses `logs` only for the missing-dealing check and `finalize` takes the fresh pinned `Logs`. Self-check gates adoption (`actor.rs:1069+`). | inferred |
| D-11 | MINOR | `beacon/actor.rs:2245`; `beacon/dkg_agree.rs:1348-1361` | Between seal and artifact, `fetch_missing_logs` fetches nothing; the only by-hash fetch is inside a live agreement instance's `verify`. A live ceremony whose node is somehow not driving an agreement instance gets no pre-artifact body recovery. No concrete failing scenario found. | Tried to construct a committee member with a live ceremony and no agreement instance: `announce_agreement_targets` (`actor.rs:975-1005`) announces every `dealing_closed` ceremony repeatedly, so the instance should exist; could not produce a clear hole. | inferred |
| D-12 | NIT | `beacon/ceremony.rs:436-475` | The ban applies to `DkgBody::Reveal` only; a proven equivocator's `Commitment`/`Share`/`Ack` are still dispatched (and journaled). For an already-acked dealer this is harmless (commonware `dealer_message` returns `None` and the cached ack is re-emitted), but the ban is not a general per-dealer ingress ban. | Read `handle` and commonware `Player::dealer_message` (`dkg.rs:1766-1788`); could not show harm, but the asymmetry with the stated "dealer locally banned" wording is real. | inferred |
| D-13 | NIT | `beacon/actor.rs:1505`; `beacon/confirmations.rs:16-21`; `beacon/dkg_agree.rs:376` | The comment "`publish_recorded_logs` never overwrites an entry" is not what the code does: `map.entry(..).or_default().insert(idx, hash)` overwrites and uses `is_none()` only for `grew`. Observable first-wins still holds because the source `signed_log_hash` is stable, so this is a comment/mechanism mismatch, not a bug. | Read `BTreeMap::insert` semantics and the only writer; confirmed the comment describes the effect, not the code. | confirmed |
| D-14 | NIT | `beacon/ceremony.rs:667-674` | `seal_dealings` calls `insert_log` directly and `let _` discards `Inserted::Equivocation`; a (theoretical) self-equivocation would set `equivocations[me]` but journal only `OwnSeal` and never emit the evidence/`note_equivocation`. Seeded deterministic dealing makes it unreachable. | Searched for any non-test path that could feed ourselves a second distinct valid log; found none (our log is deterministic per epoch, and a relayed copy is a `Duplicate`). | confirmed (code), inferred (unreachable) |
| D-15 | NIT | `beacon/actor.rs:2145,2812`; `beacon/ceremony.rs:537-540` | `note_equivocation` (WARN + counter) is called before `append_journal`, so the metric/WARN can fire for a pair that is never durable; the doc says the pair is "journaled as evidence". RAM ban is immediate and correct; durability is not. | Read both call sites; confirmed ordering. Crash would lose the ban while the counter stays incremented for the process. | confirmed |
| D-16 | NIT | `beacon/ceremony.rs:1049-1058` | `scoped_pinned_logs` looks up by `(pk,hash)` but then records the checked `cpk` without asserting `cpk == pk`; a corrupt map entry could inject a different dealer's log into the `Logs`. Unreachable because every insert keys by the checked signer (`ceremony.rs:290-292,512,894,902`), so this is only defense-in-depth. | Traced all writers of `signed_logs`; all use the `check`-returned key. | confirmed (code), inferred (unreachable) |
| D-17 | NIT | `beacon/testbed/tests.rs:3389-3393` | The rewritten second test asserts `dpos_dkg_dealer_equivocation_total == 1` only for node 0 and does not assert `0` on the other three, unlike the first test (`:3229-3235`); a stray double-prove on another node would not be caught here. | Compared the two tests' evidence assertions. | confirmed |
| D-18 | NIT | `beacon/ceremony.rs:759-761`; `beacon/actor.rs:1463-1466` | Journal bloat only: the second body is written both inside the pair and (on the `ingest_recompute_log` / nondurable-retry paths) as a `PeerLog` duplicate; replay dedups so it is correct, just redundant bytes. | Traced `journal_record_for` and the retry path; confirmed replay's `Duplicate` arm handles it. | confirmed |

No BLOCKER (no safety/liveness regression on the DKG path was demonstrated, no change to the
pinned-set agreement, no wire-decode panic).

---

## Leave as is

- **`AgreedSet.pinned` and the agreement protocol.** `DkgProposal`, `PinnedLogs::derive`,
  `ShareConfirm::covers`, `ConfirmPool` and the certification path are untouched
  (`dkg_agree.rs:180-241,1105-1174,1329-1371`); `verify` only clones the proposal set. This is
  the hard-stop the brief demanded and it holds. `confirmed`.
- **`(dealer,hash)` identity and `insert_log` as the single recording rule.** `LogId`
  (`ceremony.rs:59`), one `log_hash` (`:66`), one `insert_log` for live+replay
  (`:486,529,894,902`), and exact-id selection in `scoped_pinned_logs`/`recompute_scoped`
  (`:1053,1218`). The recording semantics are unified and the live/replay divergence the brief
  complained about is gone. `confirmed`.
- **The local ban's asymmetry.** Refusing gossip `Reveal`s while honouring exact-hash resolver
  deliveries is the correct shape: the ban must not block the agreed body. `confirmed`.
- **The wire change.** 72-byte fixed layout, unchanged tag, boxed `Log` variant, total decode,
  no zero-hash production key. I looked for a panic or a wildcard and found none. `confirmed`.
- **The rewritten acceptance tests' witnesses.** Both retain a real tamper witness
  (`reveals_swapped`, distinct hashes, `both_logs_check`, `withhold_probe`, and the victim's
  forged-hash `ShareConfirm`), and the new verdict follows from the code. Whatever the
  process decision on D-01, the tests are not vacuous. `confirmed`.
- **The new actor unit test** (`actor.rs:4754-4982`). It is the only pin for the live
  fetch-by-hash key, serve/ingest exactness and the victim-side evidence; it is worth its
  length even though it overlaps the stand scenario. `confirmed`.
- **`recorded_log_count` under `#[cfg(test)]`** (`ceremony.rs:704-707`). No production reader
  remains after the `fetch_missing_logs` rewrite, so gating it is appropriate. `confirmed`.
- **Removing the write-only `Logs` field** and `recorded` (`ceremony.rs:188-234`). No dangling
  references; `Logs` is still used where it is needed. `confirmed`.
- **`NonEmptyVec::try_from(vec![target]).ok()` and the `others.is_empty()` guard** in
  `minter_to_ask` (`artifact.rs:1716-1729`). Both are safe and fall back to the previous
  untargeted behaviour rather than failing. `confirmed`.
- **Mutex-poisoning handling** in the new `attempts` accesses (`artifact.rs:1723,1777`).
  Resuming with `PoisonError::into_inner` is appropriate for a monotone cursor. `confirmed`.
