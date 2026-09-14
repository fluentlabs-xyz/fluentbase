# Independent review, round 2 — DKG dealer-log identity `(dealer, hash)` (PLAN row 5.3, заход Б)

Baseline: `git diff HEAD` in this workspace, HEAD `18573e76`, 13 modified files under
`crates/dpos/consensus/src/` (+2076/−580). Inputs read as INPUT, not evidence:
`dsh-input-task.md`, `dsh-input-round1-review.md` (D-01…D-18), `dsh-input-fix-round2.md`,
`dsh-input-journal.md` (section «Второй круг»). No cargo was run (reading review; the orchestrator
runs the gates). Commonware sources were read read-only under
`~/.cargo/git/checkouts/monorepo-27b478c9bb41d208/3c4e02c/`.

The workspace state matches the journal's round-2 md5 block exactly for all 13 files
(`md5sum` re-run: actor `bb958996…`, artifact `1b4bb2b5…`, ceremony `cc99b8c6…`, confirmations
`9abe5da7…`, dkg_agree `6e785bf0…`, dkg_engine `6566765f…`, dkg_msg `bdcdcf4e…`, log_resolver
`a710a05f…`, log_store `3d6ab40d…`, metrics `469b0690…`, plane `6fe24182…`, share_state
`c105c5b1…`, testbed/tests `d20bf723…`), so this review is of the exact artifact the journal
describes. `.claude/` is absent and gitignored (`.gitignore:47`), so D-06 is not assessable.

Confidence: `confirmed` = read directly in the code cited; `inferred` = follows from code read but
not executed/traced end-to-end.

---

## Part (A) — verdicts D-01…D-18

| id | verdict | file:lines (current) | why | refutation attempt | confidence |
|---|---|---|---|---|---|
| D-01 | RECORDED, correctly (accepted deviation) | `testbed/tests.rs:3325-3402` | The brief required `…stops_the_chain_silently`; the test was renamed to `a_two_log_dealer_that_also_withholds_its_partial_is_one_silent_node_within_f` (`:3325`) and now expects the chain to cross 64 (`crossed`, `:3352,3364-3371`). The mechanism is the arithmetic `n−1 = 3 = quorum(4)`, and both tamper witnesses survive (`reveals_swapped` `:3334`, `both_logs_check` `:3339`, `schemes_withheld>=1` `:3340-3343`, `withhold_probe==Some((true,false))` `:3344-3349`). The orchestrator accepted this as a Д-nn deviation; the round-2 journal records it and the run. | Tried to read the rewrite as silently dropping the required property: the old verdict is still printed in the branch line (`:3353-3363`) and the new verdict has a non-vacuous tamper witness. The rewrite does drop one witness the first test keeps (`reveals_seen==reveals_swapped`, see E-06), but the verdict change itself is honestly recorded. | confirmed (code/tests); inferred (run numbers) |
| D-02 | RECORDED, correctly (accepted deviation) | `artifact.rs:1725-1751,1765-1790`; `plane.rs:346` | Out-of-brief behaviour: `ArtifactPull::minter_to_ask` now returns one rotating committee member (`:1725-1751`) and `pull` calls `fetch_targeted` (`:1787-1790`); production wires `Some(me)` (`plane.rs:346`), so both the validator `PlaneAcquire` (`plane.rs:415-451`) and the actor's live/heal pull (`plane.rs:366-392`) rotate. Journal records the causal chain (C7 272× `NotYet`). | Checked every `ArtifactPull::new`/`pull` caller (`grep`): only `plane.rs:346` is production; the follower's `TransportAcquire` (`artifact.rs:832-884`) is a different seam, so "all pulls" means the validator plane, not the follower transport. The deviation is real. | confirmed |
| D-03 | FIXED | `artifact.rs:1673,1681-1688,1700-1706,1772-1775,1796-1798,1841-1856` | Old form: `next_allowed: HashMap<u64,SystemTime>` plus a separate `attempts: HashMap<u64,usize>`; `attempts` was never pruned and the early local hit returned before any removal (round-1 D-03). New form: ONE `slots: HashMap<u64,PullSlot{next_allowed,cursor}>` (`:1673,1682-1688`); `forget` runs on the early store hit (`:1772-1775`) and on `Have` (`:1796-1798`); the single retention rule is `slots.retain(...)` in `throttle` (`:1849`). The `attempts` map is gone, and the cursor cannot outlive its slot because it is a field of it. | `grep attempts artifact.rs` → none; traced every `slots` mutator: `forget` (`:1701-1706`), `minter_to_ask` entry (`:1742-1745`), `throttle` retain+entry (`:1849-1853`). | confirmed |
| D-04 | RECORDED, but fixable | `artifact.rs:176,183,1725-1751,1849` | Recorded as a boundary, not a defect, with numbers n=4→24 s, n=100→792 s. Those numbers are the **member** case: `minter_to_ask` skips `me` (`:1727-1731`), so the walk is `|committee|−1`. | The non-member acquisition path (`acquire_mint_artifacts`, `actor.rs:2615-2618` calls `pull(e)` at `:2618`) has `me ∉ C[E]`, so `others` is all `n`; the worst case is `PULL_TIMEOUT × n` = 32 s / 800 s, not 24 s / 792 s. The recorded bound understates by one target for that path (E-03). | confirmed (code); inferred (latency) |
| D-05 | PARTIALLY FIXED | fixed: `metrics.rs:56`, `ceremony.rs:1072,1526`, `confirmations.rs:37-39`; residual: `actor.rs:754,2213-2214` | The four cited comments are corrected by concrete edits: `metrics.rs:56` now says "the resolver fetches a `(dealer, hash)` of the roster"; `ceremony.rs:1072` "the resolver fetches exactly those, by hash"; `ceremony.rs:1526` "the resolver fetches `(dealer, hash)` over the roster"; `confirmations.rs:37-39` "the exclusion is per recorded log (`(dealer, hash)` …)". | The same class survives at two uncited lines: `actor.rs:754` "which owns the per-dealer durability gate" and `actor.rs:2213-2214` "gated on per-dealer durability in `publish_recorded_logs`: a dealer named in `nondurable_logs`". The gate is `BTreeSet<LogId>` (`:456`) checked as `(pk,hash)` (`:1515`). The journal's `grep "per-DEALER"` claim is true but narrower than the class. | confirmed |
| D-06 | SKIPPED — artifact of the review method | `.gitignore:47` | `.claude/` does not exist (`ls .claude` → missing) and is gitignored (`git check-ignore -v .claude` → `.gitignore:47:.claude`), as the orchestrator stated, so no doc edit can appear in `git diff HEAD`. Not evidence of a defect. | Cannot be refuted or confirmed in this workspace. | confirmed (workspace state) |
| D-07 | FIXED | `actor.rs:505-512,540,1227-1229,1640-1667,2014-2019`; tests `actor.rs:4799-5053,5060-5155` | Old form: the pair lived only on the ceremony and died at finalize (the ceremony is `remove`d on `Ok`, `actor.rs:1769-1773`). New form: actor-level `equivocations: BTreeMap<u64, BTreeMap<PeerPubkey, DealerEquivocation>>` (`:512`), written after the journal append (`note_equivocation`, `:1640-1667`, called at `:2190` and `:2860`), copied from a resumed ceremony (`:2014-2019`), swept with the epoch (`:1229`). It stores only dealer + two hashes (`ceremony.rs:75-78`), not bodies. | Searched for another production reader: the field has production writers + `retain`; the only explicit readers are the tests, which is the recorded intent for заход А. The pair test asserts it survives finalize and dies at sweep (`actor.rs:5028-5051`); the restart test asserts it comes back (`:5117-5152`). | confirmed |
| D-08 | FIXED | `share_state.rs:377-390`; `ceremony.rs:923-940` | (a) The doc no longer overclaims "a journal without the pair holds neither": it now states that both bodies as plain `PeerLog`s re-prove the pair (`share_state.rs:381-384`), which is exactly what `resume` does through `insert_log` (`ceremony.rs:941-947`, `:496-508`). (b) The "dropped as a whole" claim is now true in code: the pair arm accepts only two halves that `check` as the same dealer under different hashes (`ceremony.rs:925-933`), else it warns and records neither (`:934-939`). | Re-read `parse_journal_inner` (`share_state.rs:497-505`): the codec still enforces nothing, and the doc now says so explicitly (`:388-389`). The M3′ shape (a `PeerLog` instead of the pair) still replays a pair, as documented. | confirmed |
| D-09 | RECORDED, correctly | `testbed/tests.rs:3259-3269`; `actor.rs:4799-5053` | The stand acceptance test is healed by two independent by-hash legs, so it stays green with the live `fetch_missing_logs` key corrupted (the journal's M2). The gap is real: the stand exercises `verify → fetch_bodies` (`dkg_agree.rs:1354-1361`) as well as the live key. The mitigation is the actor unit test `a_held_body_that_is_not_the_pinned_one_is_refetched_by_the_pinned_hash` (`:4799`), which pins the exact in-flight key `(2, dealer1, h1)` (`:4962-4966`), exact serving (`:4977-4988`) and exact ingest (`:4990-5017`). | Confirmed both legs converge on the same `ingest_signed_log` (`actor.rs:2851`, `dkg_agree.rs:1354-1361`); the limitation is recorded, not hidden. | confirmed |
| D-10 | RECORDED, correctly | `ceremony.rs:889,914,929-932,943-944,1013-1027` | Deferred with a precise description. `Player::resume` is still fed the FIRST-recorded body per dealer (`:914,929,944,956`). The `>30 lines` justification is accurate: `DkgCeremony::resume` has 21 textual call sites (`grep` `DkgCeremony::resume(`: `ceremony.rs` ×9, `actor.rs` ×12), and adding a `pinned` parameter also needs the per-dealer "best body" selection. | Read commonware `Player::resume`/`finalize` (`dkg.rs:1726-1758,1804-1825`): `logs` is used only for the `MissingPlayerDealing` check; `finalize` re-checks against the passed `Logs`. Liveness-only, as recorded; effort estimate holds. | confirmed (code); confirmed (count) |
| D-11 | RECORDED, correctly | `actor.rs:2289`; `dkg_agree.rs:1348-1361` | Between seal and artifact `fetch_missing_logs` issues nothing: the loop is gated on `self.agreed_pinned.get(e)` (`actor.rs:2289`). The pre-artifact by-hash path is the agreement's `verify` → `Missing` → `fetch_bodies` (`dkg_agree.rs:1348-1361`). The journal records this and the residual for А. | Tried to build a live ceremony without an agreement instance: `announce_agreement_targets` re-probes every closed ceremony each tick (`actor.rs:989-1014`) and retries a failed send (`:1007-1012`), so no clear hole; recorded as inferred, not fixed. | inferred |
| D-12 | REJECTED, correctly | `ceremony.rs:436-476` (esp. `462-470`) | No code change, and the rejection of the DEFECT claim is substantially right: the ban is a gossip-log-ingress ban; a second log exists only after the dealer sealed; `Player::dealer_message` is first-wins on `view` (commonware `dkg.rs:1766-1770`), so a re-sent dealing cannot replace an already-acked one. | Adversarially: a proven equivocator's `Commitment`/`Share` still overwrite `pending_pub`/`pending_priv` latest-wins (`ceremony.rs:438-445`) for a player that never acked; that player can accept the second polynomial and then fail the pinned self-check, losing its share (E-10). But an ingress ban could not reliably prevent it (the commitment can arrive before the second log proves the equivocation), so it is not a reason to reject the rejection. | confirmed (code); inferred (residual) |
| D-13 | FIXED | `actor.rs:1518-1528`; `confirmations.rs:16-21` | Old form: `map.entry(..).or_default().insert(idx, hash)` overwrote in place while the comment claimed "never overwrites". New form: `if let Entry::Vacant(seat) = map.entry(*e).or_default().entry(idx as u8)` (`:1523-1528`), so a seat is published once and `grew` is set only on insert. This is what `confirmations.rs`'s width memo (`previous >= confirmed.len()`, `:186-189`) and the stand witness `claimed(0)==log2` (`tests.rs:3170-3176`) require. | Checked whether a seat could legitimately need to change: `signed_log_hash` is first-wins and stable (`ceremony.rs:491-494,1059-1061`), and a second ceremony for the same epoch cannot start (`maybe_start` runs only for `now+1`, `actor.rs:1374`). | confirmed |
| D-14 | FIXED | `ceremony.rs:666-690` | Old form discarded the result of `insert_log` (`let _ = …`), so a self-equivocation would silently set `equivocations[me]` without evidence. New form names it: `debug_assert!(matches!(inserted, Inserted::First), "…seeded dealer self-equivocated")` (`:677-680`) plus a release `error!` (`:681-687`) and an explanatory comment. | Checked reachability: the dealer is `take`n once (`:662`), pre-seal resume re-derives the identical seeded polynomial (`init_dealer`, `:326-357`; `dealer_seed_rng`, `:310-315`), and nobody holds our log before the seal broadcast; no path found. | confirmed (code); inferred (unreachable) |
| D-15 | FIXED | `actor.rs:2189-2190,2856-2860,1640-1666` | Old order was `note_equivocation` then `append_journal`. New order is `let durable = self.append_journal(...)` then `self.note_equivocation(epoch, step.equivocation.as_ref(), durable)` at both the gossip site (`:2189-2190`) and the resolver site (`:2856-2860`); the WARN carries `evidence="journaled"|"nondurable"` (`:1662`). | Re-read both sites; `Step.equivocation` is set exactly once per pair (`ceremony.rs:537-541`), and `note_equivocation` is not called on resume (the pair is copied at `:2014-2019`), so the counter fires once. | confirmed |
| D-16 | FIXED | `ceremony.rs:1104-1118` | Old form looked up `(pk,hash)` then recorded the checked `cpk` without comparing to `pk`. New form: `Some((cpk, log)) if cpk == *pk => logs.record(...)`; otherwise `tracing::error!(… "recorded-log map corrupt …")` and `missing.push(*idx)` (`:1105-1118`). | Confirmed every writer keys by the `check`-returned signer (`:290-292,512,894,902`), so the mismatch is unreachable; the guard makes corruption explicit. Its one consequence — a corrupt entry is "not held" in `pinned_ready` but `holds()` still returns true, so `fetch_missing_logs` cannot refill it — is E-04. | confirmed |
| D-17 | FIXED | `testbed/tests.rs:3390-3397` | The second acceptance test now asserts `dpos_dkg_dealer_equivocation_total == Some(if i==0 {1.0} else {0.0})` on all four nodes, matching the first test (`:3229-3235`). | Compared the two tests' evidence blocks. | confirmed |
| D-18 | RECORDED, correctly | `ceremony.rs:550-562`; `actor.rs:1455-1484,2891-2917` | The redundancy is real: `journal_record_for` returns the pair only when `id.1 == pair.second` (`ceremony.rs:552-559`); the nondurable retry of the FIRST half therefore appends a `PeerLog` (`actor.rs:1469-1476`), and the heal path always appends a plain `PeerLog` (`actor.rs:2907`), while the live path writes the second body only inside the pair (`ceremony.rs:537-540`). Replay dedups via `Inserted::Duplicate` (`ceremony.rs:488-489`). | Confirmed no correctness impact (same bytes, deduped). A fix would need the heal path to know the pair, but it has no live ceremony, so leaving it recorded is right. | confirmed |

---

## Part (B) — fresh review of the change as it now stands

### Re-asked questions (1)–(10)

**(1) Any path that selects a log by dealer alone, reintroducing first/last-wins.**
No path selects a *finalizable* body by dealer alone. The dealer-keyed maps are: `first_log`
(`ceremony.rs:210`) → only the published index / `ShareConfirm` / `own_log_recorded` / resume
re-broadcast (`:761,1013-1027,1059-1061`); `Player::resume`'s `log_map` (`:889,914,929-932,943-944`)
→ only commonware's `MissingPlayerDealing` pre-check (`dkg.rs:1726-1758`), D-10; `pinned_by_dealer`
(`actor.rs:261-269`) → a deterministic idx→dealer translation of the pinned set; and
`recompute_scoped`'s `log_map.insert(pk, log)` (`ceremony.rs:1274-1277`) which is reached only
after filtering by the exact pinned hash, so the "last-wins" can only rewrite identical bytes.
Finalizable selection is exact-id everywhere: `scoped_pinned_logs` (`:1096`), `derive_pinned`
(`:1182`), `finalize_over_pinned` (`:1224`), `holds` (`:728-730`), `signed_log` (`:769-771`),
`serve_log` (`actor.rs:2797-2807`). `signed_logs` is never iterated one-per-dealer. **Confirmed.**

**(2) The ban's exact refusal set; does a pinned log of a banned dealer always reach the ceremony.**
The refusal is exactly `DkgBody::Reveal` whose `check`-returned signer is in the ceremony's
`equivocations` (`ceremony.rs:466-470`). Not refused: resolver `ingest_signed_log`
(`:804-819`), replay (`:898-950`), own seal (`:661-714`), and a banned dealer's
`Commitment`/`Share`/`Ack` (`:438-461`). A pinned log reaches the ceremony: `fetch_missing_logs`
asks `(e, dealer, pinned_hash)` (`actor.rs:2310-2325`) for a dealer whose held body is a different
hash (`holds` is false, `:2315`); `serve_log` answers the exact body or nothing
(`:2797-2807`); `ingest_signed_log` accepts only the exact `(key.dealer,key.hash)`
(`ceremony.rs:804-815`); the second body becomes the pair (or `Another` if already proven,
`:496-512`); `scoped_pinned_logs` selects the exact id (`:1096`). **Confirmed.**

**(3) Pair replay whole-or-nothing; a journal with a `PeerLog` of the second hash but no pair.**
Whole-or-nothing is implemented at `ceremony.rs:923-940`: both halves must `check` as the same
dealer under different hashes, else a WARN and neither body is recorded. A bare `PeerLog` of the
second hash goes through the same `insert_log` rule (`:941-947`): if the first body is also in the
journal (as a `PeerLog`, `OwnSeal` or pair), the pair and the ban are reconstructed
(`:496-508`); if the first body is absent, the second becomes the dealer's FIRST log and there is
no ban — correct, since one body alone is not evidence. The codec checks nothing
(`share_state.rs:497-505`), as the doc now states (`:388-389`). One asymmetry: the cold serve path
does not apply the whole-or-nothing rule (E-02). **Confirmed.**

**(4) Wire decode totality and the boxed key.**
`DkgLogKey::read_cfg` reads `u64`, `PeerPubkey`, then `<[u8;32]>::read` (`log_resolver.rs:100-109`);
the ed25519 public-key reader itself uses the array reader (commonware `ed25519/scheme.rs:164-178`),
and the array reader errors on short input (`codec/src/types/primitives.rs:158-167`). Short input at
71 bytes is rejected by the test (`log_resolver.rs:638-641`). `BeaconFetchKey::read_cfg` matches the
two tags and refuses the retired tag 2 and unknown tags (`:226-241`); boxing changes no bytes
(`:198-221`). Layout is `u64 ‖ dealer(32) ‖ hash(32)` = 72 (`:83-95`). **Confirmed.**

**(5) `ArtifactPull` with the merged `slots` map.**
Lifecycle: `throttle` retains slots with `next_allowed + PULL_TIMEOUT > now` (`artifact.rs:1849`),
then creates/claims the epoch's slot and bumps `next_allowed = max(prev, now) + PULL_MIN_INTERVAL`
(`:1850-1856`); `minter_to_ask` lazily creates a slot if `throttle` did not (`:1742-1745`), picks
`others[cursor % len]` and post-increments the cursor (`:1746-1747`). `forget` sites: early local
store hit (`:1772-1775`) and delivered `Have` (`:1796-1798`). Growth is bounded by the single
`retain` (one entry per epoch pulled within the last `PULL_MIN_INTERVAL+PULL_TIMEOUT` window, plus
the current), and a held artifact removes its epoch immediately. Unreadable committee or
`others.is_empty()` → `None` → untargeted `resolver.fetch` (`:1787-1790`), which also **clears** any
accumulated targets in commonware (resolver `p2p/engine.rs:233-251`), so a transient committee flap
resets the rotation for that key. `PULL_TIMEOUT × (n−1)` is the worst case only when `me ∈ C[E]`;
for a non-member it is `× n` (E-03). The rotation-cursor lifetime and the `me`-skip test gap are
E-04/E-05. **Confirmed (code); inferred (latency).**

**(6) `Entry::Vacant` first-wins by seat in `publish_recorded_logs`.**
First-wins by seat is the right local rule given `confirmations.rs`: the width memo treats the index
as a set that never changes an entry in place (`confirmations.rs:16-21,186-193`), and the source
`signed_log_hash` is itself first-wins and stable (`ceremony.rs:491-494,1059-1061`), so the code
change only makes the invariant explicit rather than relying on the source. It **can** keep the
non-pinned hash in the shared index, and it deliberately does: the victim's own `ShareConfirm` names
`log2` where the network pinned `log1` (`testbed/tests.rs:3170-3176`). That does not pin the
non-pinned hash on-chain: `AgreedSet.pinned` comes from the certified artifact
(`actor.rs:949,973`), and `rejects_structurally` requires every carried confirmation to
`covers(&proposal.logs)` (`dkg_agree.rs:1243-1251`), so the victim's non-covering confirmation is
excluded from any pinned-set proposal. The residual is liveness-only and pre-existing: a victim
leader proposes its first-wins (non-pinned) set, its peers park on the unheld body and the view
times out. **Confirmed.**

**(7) The D-12 rejection.**
Right for the defect claim. The ban's asymmetry is intentional and does not lose the agreed body;
the only path where a broader ban could matter (a not-yet-acked player accepting a second
polynomial's dealing) is not reliably reachable by a log-ingress ban and is caught by the
`adopt_share` self-check. See D-12 and E-10.

**(8) Tests: green-while-broken configurations.** See D-09 (stand green with the live fetch key
zeroed; the actor unit test is the pin), E-05 (the rotation test passes `None`, so the production
`me`-skip is untested), E-06 (the second stand test omits `reveals_seen == reveals_swapped`), and
D-13 (the `Entry::Vacant` rule is pinned only by the byzantine stand test; no default-`--lib` unit
test asserts it). The two ceremony unit tests and the actor unit test are not vacuous: M1/M2/M3 in
the journal turn them red, and the pair-record *shape* is pinned by
`a_dealers_second_valid_log_is_evidence_and_a_ban_but_never_a_replacement`
(`ceremony.rs:2398-2402`), not only by replay.

**(9) Hygiene and dead code.**
No `#[allow]` added; no production `unwrap`/`expect`/`panic!` added — every added `expect`/`panic!`
in the diff is inside `mod tests`/`clock_tests` (verified by scanning the added lines of
`git diff HEAD`). `DkgCeremony::{holds,signed_log,equivocation,equivocations,journal_record_for}`
are used in production (`actor.rs:2014,2315,2760,2806,2859`), `recorded_log_count` is `#[cfg(test)]`
(`ceremony.rs:719-722`), and the actor's `equivocations` field has production writers plus the sweep
and test readers. The only hygiene residue is the stale "per-dealer durability" wording (E-01).
**Confirmed.**

**(10) Where this review is weakest — ranked.** See the last section.

### Findings (Part B)

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| E-01 | MINOR | `actor.rs:754`, `actor.rs:2213-2214` | D-05-class stale doc comments survive: `with_recorded_logs` says `publish_recorded_logs` "owns the per-dealer durability gate", and the gossip path says the index is "gated on per-dealer durability … a dealer named in `nondurable_logs` is excluded". The gate is per `(dealer, hash)` (`nondurable_logs: BTreeSet<LogId>` `:456`; check `set.contains(&(pk.clone(), hash))` `:1515`). | Grepped the whole beacon tree for `per-dealer durability`; only these two survive. `grep "per-DEALER"` is 0, which is why the fix round's check missed them. Behaviour is correct; only the comments lie. | confirmed |
| E-02 | MINOR | `ceremony.rs:289-293` vs `ceremony.rs:923-940`; doc `share_state.rs:377-389` | The cold serve store and replay disagree on pair validity. `checked_serve_map` flat-maps `logs_in(DealerEquivocation(first,second))` and inserts each half that `check`s, keyed by the checked signer (`:289-293`); `resume` instead drops a pair whole unless both halves are the same dealer under different hashes (`:923-940`). So a corrupt/tampered pair whose halves name two dealers is not recorded by the ceremony but IS served by a cold-restarted node. | Tried to turn this into a forgery: no, both halves are `check`-valid and `checked_serve_map` keys by the checked signer (`:290-292`), so the served body is always a genuine log of the key it is filed under; the divergence is only that a body the resumed ceremony would not hold is still servable. Bounded by the retention window. | confirmed (code); inferred (impact) |
| E-03 | MINOR | `artifact.rs:1727-1731`; `actor.rs:2615-2618` | The recorded D-04 worst case `PULL_TIMEOUT × (|committee|−1)` undercounts the non-member path. `minter_to_ask` filters out `me` only when `me` is in the committee (`:1727-1731`); `acquire_mint_artifacts` pulls for epochs the node is NOT a member of (`:2615-2618`), so `others` is all `n` and the worst case is `8 s × n` (32 s at n=4, 800 s at n=100). | Read both pull consumers; the live/heal pull is gated on committee membership reads, the acquisition pull is explicitly the non-member leg (`actor.rs:2555-2571`). The member-case numbers in the journal are correct; the bound as recorded is not the global worst case. | confirmed (code); inferred (latency) |
| E-04 | MINOR | `ceremony.rs:1104-1118`; `ceremony.rs:728-730`; `actor.rs:2315` | The D-16 guard creates a "present but not held" state that the fetch path cannot repair. A body filed under the wrong signer is reported `missing` by `scoped_pinned_logs` (`:1116-1118`), so `pinned_ready` is `all_held=false`; but `holds()` still returns true for that exact `(pk,hash)` because `signed_logs` contains it (`:728-730`), so `fetch_missing_logs` skips the key (`actor.rs:2315`) and the ceremony defers forever. | Checked reachability: every writer keys by the `check`-returned signer (`:290-292,512,894,902`), so a mismatched entry is unreachable by construction; if it ever occurred, the old code would have recorded the foreign body under the seat (also wrong). This is a latent wedge only. | confirmed (code); inferred (unreachable) |
| E-05 | MINOR | `artifact.rs:2746`; `artifact.rs:1727-1731` | The rotation unit test constructs `ArtifactPull::new(context, fetching, None)`, so the `me`-skip branch — the exact production configuration (`plane.rs:346` passes `Some(me)`) and the fix for the measured C8 9-block lag — is not exercised by any unit test. A regression in the filter would be caught only by the stand C7/C8 (byzantine feature). | Grepped all `ArtifactPull::new` sites: tests pass `None`; the only `Some(me)` is `plane.rs:346`. | confirmed |
| E-06 | MINOR | `testbed/tests.rs:3334` vs `testbed/tests.rs:3138` | The second rewritten acceptance test asserts only `reveals_swapped >= 1`, not `reveals_seen == reveals_swapped` as the first test does (`:3138`). A wrapper that swapped the addressee's reveal but also forwarded the original (so the victim holds both logs even without the refetch) would still satisfy `reveals_swapped>=1`, `log1!=log2`, `both_logs_check`, `schemes_withheld`, `withhold_probe` and the crossing, weakening the tie between the verdict and the refetch. The first test's equality is the stronger witness. | Compared the two tests' witness blocks; the second still has `withhold_probe == Some((true,false))` and `dkg_ceremony_ok==1`, but the swap-completeness witness is absent. | confirmed |
| E-07 | MINOR | `artifact.rs:1772-1785`; `artifact.rs:1650-1658` | A `Have` delivered during the throttle sleep is lost for the current call. `pull` checks the local store (`:1772`), then `throttle` may sleep up to `PULL_MIN_INTERVAL` (`:1776`), and only then registers the waiter (`:1778-1785`). `ArtifactBridge::wake` removes and answers only the waiters present at delivery time (`:1650-1658`), so the current pull waits the full `PULL_TIMEOUT` and may return `NotYet`/`None` even though the store now holds the artifact; the next pull's local hit recovers it. | The store-insert and wake happen together in `deliver` (`:1578-1602`), and no path re-checks the store after the throttle; traced the ordering. Pre-existing, but the rotation makes the wasted attempt more likely than the old immediate untargeted fetch. | inferred |
| E-08 | NIT | `actor.rs:1523-1524` | `publish_recorded_logs` calls `map.entry(*e).or_default()` for every live epoch with a readable committee, creating an empty inner map even when no seat is inserted (`signed_log_hash` returns `None`, `:1509-1511`). `grew` stays false, so nothing wakes on it, and the sweep removes it, but a transient empty per-epoch entry exists. | Read the loop: the `or_default()` precedes the `Vacant` check, so the inner map is created eagerly. No correctness impact. | confirmed |
| E-09 | NIT | `actor.rs:2906-2917` | `ingest_recompute_log` returns `true` ("fetch satisfied") even when `append_journal` fails; it keeps the id in `want` (`:2911-2914`), so `fetch_missing_logs` re-issues the key next tick. The intermediate state is "resolver believes the key is done, `want` believes it is not", costing one wasted round trip. | Traced the return value against the resolver's `deliver` semantics (`log_resolver.rs:398-407`): `true` clears the fetch; the re-issue is the recovery. Correct but not free. | inferred |
| E-10 | MINOR | `ceremony.rs:438-445`; commonware `cryptography/src/bls12381/dkg.rs:1766-1770` | The ban does not cover `Commitment`/`Share`, and both `pending_pub`/`pending_priv` and the buffered `PendingDealings` are latest-wins per sender (`ceremony.rs:439,443`). A player that never acked can accept a second polynomial's dealing from a proven equivocator, put it in `view`, and then finalize over the PINNED log while using the wrong `view` point (commonware `finalize` prefers `view` over the log reveal, `dkg.rs:1826-1846`); the resulting share fails the `adopt_share` self-check, so no fork — but the share is lost. | Confirmed `dealer_message` is first-wins once `view` holds the dealer (`dkg.rs:1768-1770`), so an acked player is safe; the window is only for a never-acked player. Also confirmed the ban cannot reliably close it: the second commitment can arrive before the second log proves the equivocation. This is the D-12 residual, reported rather than re-litigated. | confirmed (code); inferred (exploitability) |
| E-11 | NIT | `actor.rs:2014-2019` | On resume, the actor copies the replayed pair with `.entry(target).or_default().extend(ceremony.equivocations().clone())`, which silently OVERWRITES an existing `(epoch,dealer)` entry and emits neither the WARN nor the metric — so a pair proven live and then re-resumed (no path found today) would lose its own recorded order/observability. | Checked `resume_from_journal` callers: only `maybe_start`, which does not re-run for a live epoch (`actor.rs:1819`), so double-resume is unreachable. Observability-only. | inferred |
| E-12 | NIT | `ceremony.rs:923-933` | The pair validity check enforces same dealer + different hashes but not that the record's order matches the order the hashes were first recorded. `resume` inserts `first` then `second`, so a reordered/tampered journal would set `first_log` to the record's first half and could flip which hash this node proposes/confirms. | The writer always emits `(first_log, new_hash)` in that order (`ceremony.rs:537-540`) and the append-only journal preserves order, so only disk tampering reaches it; the shared index/confirmations would then diverge from the network's pin, which the agreement's `covers` check already refuses. Not reachable in the threat model. | inferred |
| E-13 | NIT | `log_resolver.rs:613-666` vs `log_resolver.rs:198-221` | The codec test pins `DkgLogKey` at 72 bytes and byte slices, but the boxed `BeaconFetchKey::Log` arm's total encoded size (tag + 72 = 73) and tag placement are not asserted; only round-trip equality is (`the_shared_key_space_separates_its_two_subjects`, `:487-511`). A future regression in `BeaconFetchKey::write`/`encode_size` for the boxed arm would still round-trip within the same binary. | Read both tests; the layout test does not wrap the key in a `BeaconFetchKey`. Byte-level cross-version compatibility is claimed by the doc (`:58-59`) but not pinned at the envelope level. | confirmed |
| E-14 | NIT | `actor.rs:2282,2371-2375` | The `unreadable` set preserves ALL in-flight keys for an epoch whose committee read fails this tick (`resolver.retain(move |key| wanted.contains(key) || unreadable.contains(&key.epoch))`). If the committee read flaps, keys the ceremony no longer wants (a body swept from `want`, or a seat removed from the pinned set) stay alive for the whole flap. | `unreadable` is populated only for epochs still in `self.ceremonies` / `recompute_pending` (`:2283,2293,2334,2346`), so it is not attacker-controlled and dies with the epoch; bounded by the retention window. No leak beyond the window. | inferred |

---

## Leave as is

- **`AgreedSet.pinned` and the agreement protocol.** Untouched: `AgreedSet.pinned` is `idx→hash`
  (`actor.rs:572-574`), `on_artifact` is first-wins (`:958-973`), `DkgProposal`/`derive`/`covers`
  are unchanged (`dkg_agree.rs:1329-1372,1218-1252`). The hard-stop holds.
- **`(dealer,hash)` identity and the single recording rule.** `LogId` (`ceremony.rs:59`), one
  `log_hash` (`:66`), one `insert_log` for seal/gossip/resolver/replay (`:486-514`), exact-id
  selection in `scoped_pinned_logs`/`derive_pinned`/`finalize_over_pinned` (`:1096,1182,1224`).
  Live/replay divergence is gone.
- **The local ban's asymmetry.** Refusing gossip `Reveal` while honouring exact-hash resolver
  deliveries is the correct shape: the ban must not block the agreed body (`ceremony.rs:462-470`,
  `:796-799`). The residual (E-10) does not justify a broader ban.
- **The wire change.** 72-byte fixed layout, unchanged tags, boxed `Log` arm, total decode, no
  zero-hash production key (`log_resolver.rs:83-109,198-241`; `actor.rs:2310-2325`,
  `dkg_agree.rs:1393-1408`). No panic or wildcard found.
- **The rewritten acceptance tests' witnesses and the new unit tests.** Both stand tests retain a
  real tamper witness and the new `dkg_ceremony_ok`/equivocation assertions; the three added unit
  tests (`ceremony.rs:2353,2494`; `actor.rs:4799,5060`) pin the identity, the ban, replay, the live
  fetch key and the evidence lifetime. The E-05/E-06 gaps are witnesses, not blockers.
- **`ArtifactPull`'s merged `slots` map and `NonEmptyVec::try_from(vec![target]).ok()`.** The map is
  bounded by one retention rule, `forget` covers both held paths, the index arithmetic is guarded by
  `others.is_empty()`, and `wrapping_add` cannot panic (`artifact.rs:1673-1751,1834-1861`).
- **`recorded_log_count` under `#[cfg(test)]`** (`ceremony.rs:719-722`) and the removal of the
  write-only `Logs` field and `recorded` set: no dangling references remain.
- **Mutex-poisoning handling** in `slots`/`waiters` (`artifact.rs:1703-1705,1736-1739,1781-1785`):
  resuming with `PoisonError::into_inner` is appropriate for a monotone cursor.

## Where this review is weakest (ranked)

1. **No gate was run.** All behavior claims (crossing 64, quorum arithmetic, timing) are read from
   code and the journal; D-01/D-17 verdicts and the D-02 causal chain are `inferred` from the
   journal's verbatim runs.
2. **The `ArtifactPull` rotation's real behaviour.** I read the callers, the test and commonware's
   target accumulation (`resolver/src/p2p/engine.rs:226-251`) but did not exercise the resolver;
   E-03/E-05/E-07 are code-level, not measured.
3. **`Player::resume`/`finalize` interaction with the first-body `log_map` (D-10).** I read
   commonware's `resume`/`finalize` but did not enumerate crash/journal states; the liveness-only
   classification is `inferred`.
4. **Journal replay under exotic orders.** I traced the writer's orders and the reachable duplicate
   shapes but did not exhaustively enumerate corrupt/torn journals (E-02/E-12 are reachability
   arguments, not fuzz results).
5. **The corrupt-map consequence (E-04).** It depends on an invariant ("every writer keys by the
   checked signer") I verified textually at five sites but not with an adversarial mutation.
