# E5 · row 5.3, заход Д — final review (Opus 5, fresh context)

Date: 2026-09-15. Base `d8747086`, branch `djadjka/dpos-reth-2.2-squashed`. Object: the uncommitted
working tree — `crates/dpos/consensus/src/{beacon/actor.rs, beacon/metrics.rs, beacon/share_state.rs,
testbed/stand.rs, testbed/tests.rs}` (+599/−116 per `git diff HEAD --stat -- crates`) and the four
harness files under `devnet/local-dpos-smoke/dpos_harness/` (+37/−34). Paths without a prefix are
relative to `crates/dpos/consensus/src/`. Brief: `history/E5-prompts/5.3-D-review-final.md`.

Provenance: every `[KNOWN]` below is a file opened with `Read` or a command run in this session. The
tree under review is byte-identical to the orchestrator's gate snapshot: `md5sum -c gates/y2.md5` →
all five files `ЦЕЛ`, checked before the mutations and again after the last rollback. The journal
(`history/E5-3-D.md`), the dsh report (`history/E5-3-D-DSH-1.md`) and the comments in the code were
read as claims, not evidence. `history/E4-ORCHESTRATOR.md` was not opened.

**Verdict: COMMIT.** No BLOCKER. Two MINOR follow-ups outside the crate (F-01 harness prose/verdicts
in the prod-dkg smokes; F-06 doc line anchors drifted by 6–20 lines after round 3), one MINOR test
hygiene item (F-03), the rest NIT.

## §0 Direct answers

### 0.1 Gates `[KNOWN]` (read from `scratchpad/gates/y2-*.txt`, `y2-status.txt`)

| gate | command (from `y2-status.txt`) | result, verbatim |
|---|---|---|
| lib | `cargo test -p fluentbase-consensus --lib` | `test result: ok. 733 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 44.06s` |
| standf | `cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::` | `test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 684 filtered out; finished in 53.24s` |
| node | `cargo test -p fluentbase-node --lib` | `test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.57s` |
| reader | `cargo test -p fluentbase-staking-reader` | `test result: ok. 64 passed; 0 failed; …` and the doc-test line `0 passed; 0 failed; 1 ignored` |
| slasher | `cargo test -p fluentbase-consensus --test slasher_integration` | `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s` |
| clippy | `cargo clippy -p fluentbase-consensus -p fluentbase-node -p fluentbase-staking-reader --all-targets` | exit 0; two foreign warnings: `crates/dpos/staking-reader/src/epoch_transition.rs:3024` (`await_holding_lock`, test code) and `crates/node/src/dpos.rs:1985` (`large_enum_variant`); zero in `crates/dpos/consensus` |
| clippyf | `… --features dpos-devnet-byzantine` | `Finished \`dev\` profile … in 1.89s`, no warning lines |
| fmt | `cargo fmt --check` | 0 non-`Warning:` lines (the 329 lines are rustfmt's nightly-option warnings) |
| doc | `cargo doc -p fluentbase-consensus --no-deps` | `generated 57 warnings`: 50 private-item links, 6 `unresolved link`, 1 redundant target — the SAME breakdown as `x2-doc.txt` and `y1-doc.txt` (no new doc warning) |

Test-count delta, by name (`git diff HEAD -- crates | grep -E '^[-+]\s*fn '` and `#[test]` count +3/−0):
`--lib` 730 → 733 = +2 units in `beacon/actor.rs` (`an_absentee_restarted_after_the_seal_heals_its_share_from_the_reveals`
`:11995`, `an_absentee_heal_over_an_acked_dealing_is_unrecoverable_not_a_loop` `:12091`) + 1 stand test
in `testbed/tests.rs` (`an_absentee_restarted_after_the_seal_heals_its_share_and_signs` `:2030`, which
compiles without the feature: `testbed/mod.rs:84` gates only a sub-module, and `y2-lib.txt` lists it
`… ok`). Three renames, no count change: `recover_no_journal_at_the_seal_deadline_sits_out` →
`…_waits_for_the_artifact_to_heal` (`:11889`), `recover_torn_journal_at_the_seal_deadline_sits_out` →
`…_evicts_it_and_heals_as_a_player` (`:12245`), `at_a_zero_width_deal_window_a_missing_journal_at_the_deadline_sits_out`
→ `…_heals_not_deals` (`:13782`). stand 57 → 58 = the one stand test. Base numbers 730/57 read from
`gates/x2-lib.txt` / `x2-standf.txt`. The counts add up.

### 0.2 Smoke `[KNOWN]` (`scratchpad/smoke/live-heal-5.3D-y2.log`)

Verdict line, verbatim: `OK (smoke-vrf-dkg-live-heal): a member offline through its whole epoch-2 DKG
window PULLED the epoch's agreed artifact, recomputed its share from the dealers' public reveals (the
reveal-fallback path, first live coverage), left vote-only admission, and PRODUCED inside epoch 2 —
while the chain finalized throughout on the n-f=3 survivors and its prev_randao stayed byte-identical
to theirs`, then `exit=0`. The container log was not captured (`live-heal-5.3D-y2.validator-3.log`:
`No such container`).

The three validator-3 lines and what each proves:
1. `… beacon::actor: live DKG: demoted committee member detected — starting share recompute-heal epoch=2 want=4 pinned=4`
   — emitted only by `heal_over` (`actor.rs:2874-2879`); `want == pinned` ⇒ `held` (`parse_journal`,
   `:2868`; `log_store.rs:140-153` returns an empty map for `NoFile`/`Torn`) held none of the four
   pinned bodies ⇒ the journal was absent. The harness printed the heal line in the `fresh or heal`
   slot (`asserts_fault.py:684`), so `fresh` (`ceremony started`) was empty: no `start_fresh` ran.
   By code the only road to `heal_over` for a share-less member without a journal is `recover`'s
   `NoFile if past_seal` arm (`:3258-3266`) → `heal_or_acquire` (`:2890-2902`): the other two callers
   (`key_held_share` `:1807-1821` from `ArtifactForShare`, and `drive_finalization` `:2802-2846`)
   need either that same arm's `ArtifactForShare` or a live `Agreed` ceremony, which needs a `Present`
   journal or a pre-seal `start_fresh`. The base run (`live-heal-d8747086.log:132`) failed with
   «neither road». So the entry step is proven, indirectly (no WARN line captured).
2. `… live DKG: demoted committee member recomputed its share from the retained journal — share stored, promoting to Signer epoch=2`
   — `try_recompute` after `adopt_share` returned `Ok` (`:4042-4043`, `:4082-4086`): the share passed
   `validate_share_on_poly` (`share_on_artifact`, `:2067`, `:1835`) and was persisted + stored.
3. `… epoch_manager: promoted to Signer in-process: per-epoch BFT engine started epoch=Epoch(2)` —
   emitted only after `SignerVerdict::Signs(scheme)` and `spawn_engine` (`epoch_manager.rs:1237-1238`,
   `:1285-1295`); `Withheld` returns before the spawn (`:1252-1256`).

`PRODUCED in epoch 2 (producedAt=10 of blocksInEpoch=64)`: the harness reads the on-chain liveness
counter `producedAt(uint64,uint32)` / `blocksInEpoch` (`core/nodes.py:646-675`) after
`epoch_start(3) + K` (`asserts_fault.py:751-783`). It proves validator-3 was the round leader for 10
finalized blocks, which requires the spawned Signer engine (a verify-only member spawns none,
`epoch_manager.rs:1252-1256`). It does NOT prove its seed partial was load-bearing: 4 survivors ≥
quorum 3 of 5 finalize without it. The partial's validity is proven by the stand test instead (§0.7).

Round 3's diff on the live path: `metrics.rs:52-53,65,73,87` (`StallReason::SatOut`, `ALL` 10 → 9 —
`ALL` feeds the gauge HELP text in `register`, executed at startup) and `actor.rs:504-505` (`carries`
arm, executed on every `set_state`). Both removed code that was UNREACHABLE on rounds 1+2 (no producer
of `StallReason::SatOut` after `EpochState::SatOut` went in round 1: `git grep 'SatOut\|sat_out' --
crates` is empty; `enter` `:1365-1370` and `stall` never raise it). Everything else in round 3 is
comments, a test message and two test assertions. So: executed lines were touched, but the behaviour
on the smoke's path is identical by construction; a rerun is not needed to trust the result. If the
orchestrator wants a byte-identical evidence chain, rerun once before commit.

Harness diff (`git diff HEAD -- devnet`): `verdicts_fault.py:838-842` now matches `\bpinned=(\d+)`,
which is exactly what `heal_over` logs (`want = want.len(), pinned = pinned.len()`, `actor.rs:2876-2877`);
the two fields are matched independently (order-free) and the ANSI strip precedes the match. No other
reader of a renamed field remains: `grep -rn 'dealers=' dpos_harness --include='*.py'` hits only the
negative assertion `tests/test_smoke_fault_verdicts.py:606` (`… want=4 dealers=4` → `None`). The
`asserts_fault.py:664` comment the round-2 journal listed as «left» is in the diff and reads
`want == pinned`. `SHARE_LINE`, `HEAL_LINE`, `HEAL_START_LINE`, `CEREMONY_STARTED_LINE` untouched; the
`(4, 4)` reader was fed the verbatim failed line (`live-heal-5.3D-y1-reader-fail.log:132`) by the
round-2 implementer (journal, relay — not re-run here).

### 0.3 Safety of the new arms `[KNOWN]`

Path: `decide` (`:2943-2952`) → `recover` (`:3141`) → `Torn if past_seal` (`:3248-3257`, `evict_journal`
first) / `NoFile if past_seal` (`:3258-3266`) → `heal_or_acquire` (`:2890-2902`) → `Acquiring(Logs(heal_over))`
or `Acquiring(ArtifactForShare)`. Both return through `quiet` (`:3145`): empty `Vec<Outgoing>`, no
latch. `decide_window`'s `debug_assert!(dropped.is_empty())` (`:2926`) holds.

Outgoing sinks on the tick: `to_send` in `on_height` collects (a) `decide_window`'s `out` — empty here;
(b) the seal step — only for `EpochState::Dealing` (`:2242-2261`); (c) `confirmations.mint`
(`:2316`) — reads `recorded_dkg_logs` (`confirmations.rs:156-206`), which only `publish_recorded_logs`
grows and only from `self.ceremonies()` (`:2485-2520`); `EpochState::ceremony()` is `None` for both
new phases (`:379-387`), and the index is a fresh empty map at build (`plane.rs:673`), so nothing is
minted for the healing epoch; (d) `retransmit` — `ceremonies()` only (`:2362-2366`). On the resolver
path: `ingest_log` with no live ceremony goes to `ingest_recompute_log` (`:4216-4221`), which appends
a `PeerLog` and calls `try_recompute` (`:4248-4263`) — no `Outgoing`; the `Deliver` arm's
`confirmations.mint(AnyGrowth)` (`:4122-4123`) is again index-driven ⇒ nothing. `recompute_scoped`
discards the re-derived acks (`ceremony.rs:1373-1375`). `adopt_share` (`:2060-2109`) writes disk,
store, notify, counter — no send. On `on_message`, an epoch in either phase has no `ceremony_mut`
(`:3571`), so Commitment/Share bodies are buffered (`:3625-3655`) and drained-away by `decide`'s
non-dealing branch (`:2956-2961`) / `pending.retain` (`:2296-2305`); Ack/Reveal are dropped. **Nothing
is broadcast for the epoch: no dealing, no ack, no seal, no confirmation.**

A node that DID seal before crashing: `start_fresh` (`:3315`) is reachable only from the `Torn`
(`:3267-3278`) and `NoFile` (`:3279-3282`) arms that are guarded by `if past_seal` falling through,
i.e. `!past_seal`; the seal in `on_height` fires at `height >= epoch_start(e) − DKG_MARGIN_BLOCKS`
(`:2247`), the same predicate as `past_seal` (`:3236-3237`), so a sealed node always restarts with
`past_seal = true` and can neither re-deal nor re-seal (no ceremony exists to seal). `evict_journal`
(`:1208-1210`, `share_state.rs:642-649`) removes only `beacon-dkgjournal-e<E>.bin`; the share lives in
`file_for` (`share_state.rs:653-659`, separate file) and is checked first in `recover` (`:3187-3201`).
A `Torn` file is by definition one whose FIRST record fails to decode and the parse loop `break`s
there (`share_state.rs:608-637`), so `parse_journal` / `serve_log` serve nothing from it either way
(`log_store.rs:146-153`) — nothing readable is destroyed. Residual: F-02 (a journal that is intact but
undecryptable in THIS process is also `Torn`, `share_state.rs:528-531`).

### 0.4 Termination `[KNOWN]`

Acked-then-lost absentee: every pinned body lands (`want` empties only on a DURABLE append,
`:4253-4260`), `try_recompute` loads `Present` (`:3971-3972`), sets `attempted` (`:3989`),
`Player::resume` returns `MissingPlayerDealing` (`dkg.rs:1741-1755` — a log holding a valid ack of
`me` that the replay did not provide; and `finalize` `dkg.rs:1818-1823` catches the same), the arm at
`:4006-4022` logs ONE `warn!`, increments `dkg_share_unrecoverable`, sets `Unrecoverable{key: Some}`
and latches `Stalled{Unrecoverable}` (`stall` is latched per `(epoch, reason)`, `:1478-1484`). From
there: `needs_artifact()` is false for `key: Some` (`:417-427`) ⇒ no pull; `fetch_missing_logs`
matches only `Agreed`/`Dealing{agreed}`/`Acquiring(Logs)` (`:3701-3726`) ⇒ no fetch, and its
`retain` cancels the epoch's in-flight keys (`:3765-3768`); `try_recompute` filters on
`Acquiring(Logs)` (`:3948-3956`) ⇒ no re-run. Unit `an_absentee_heal_over_an_acked_dealing_is_unrecoverable_not_a_loop`
asserts the phase, the counter (1), the empty store and an empty in-flight set after two ticks
(`:12185-12221`). **Exactly once, no loop, no per-tick line.**

Fetch bound: `want` keys are re-issued every tick (`:3748-3757`, `:3769-3771`) — dedupe of in-flight
keys and retry cadence belong to the resolver (comment `:3671-3672`, `log_resolver.rs` not read —
`[LIKELY]`); the hard bound is the slot's lifetime: `sweep_epoch_state` drops `e + JOURNAL_RETENTION_EPOCHS < now`
(`:2154-2170`) and the next `fetch_missing_logs` `retain` cancels its keys. `attempted` bounds the
crypto (once per input change: re-armed by a durable body `:4261` or a persist failure `:4064`), not
the fetch.

`Err(other)` from `recompute_scoped` with empty `want`: `:4024-4038` — one `warn!` + `Stalled{HealFailed}`,
`attempted` stays `true`, no fetch (`want` empty) ⇒ non-terminal but silent and bounded by the sweep.
Reachability for an absentee whose committee read matches the peers': `info_for` is deterministic
(`ceremony.rs:247-260`, `previous = None` ⇒ `reckon`'s reshare branch is out); `Player::new` fails only
for a non-member (excluded at `:3222`); `select`/`DkgFailed` cannot fail over a pinned set the
network's `observe` accepted under the same `Info`; a committee read that DIFFERS from the peers' makes
`signed.check(&info)` fail in `ingest_recompute_log` (`:4246-4267`) ⇒ `false` ⇒ `want` never empties
⇒ this arm is not reached. So unreachable for the absentee `[LIKELY]` (by branch enumeration, not by a
run — same as the journal §0.4).

### 0.5 Artifact-less entry `[KNOWN]`

`apply_artifact`'s `ArtifactForShare` arm (`:1740-1751`) → `key_held_share` (`:1792-1823`) → `store.get
== None` → `Acquiring(Logs(heal_over))`, `stalled: None` (`:1816-1821`) — the same constructor call
`heal_or_acquire`'s `Some(set)` arm makes (`:2897-2899`) with `AgreedSet::of(proposal)` as input; the
only difference is the moment `parse_journal` runs (on artifact arrival instead of at decision), and
for an evicted/absent journal both read empty. So yes, identical. `needs_artifact()` includes
`ArtifactForShare` (`:420-423`) ⇒ `drive_acquisition` pulls every tick (`:3838-3849`) and latches
`Stalled{NoArtifact}` once `e <= now`. A committee read failure in `apply_artifact` keeps the phase
(`:1746-1750`); the store still holds the artifact, so `reconcile_with_store` re-applies it on the next
tick (`:3876-3888`, `held_digest` is `None` for `ArtifactForShare` `:401-413`) — a retry, not a stall;
the latch (if `e <= now`) misnames the cause as a missing artifact (F-11, pre-existing shape).

### 0.6 The other cells `[KNOWN]`

`(Torn, h < seal)` `:3267-3278` and `(NoFile, h < seal)` `:3279-3282`: unchanged, `evict_journal` +
`start_fresh`. `(Present, h ≥ seal)` `:3242-3246`: unchanged, `resume_from_journal(.., !past_seal, ..)`.
`decidable_epochs` (`:618`) and `decide_window` (`:2914-2928`) untouched: an epoch jumped past is
decided while inside `[now − R, now + 1]`. Non-member: `:3222-3229` returns `KeyOnly` / `Acquiring(ArtifactForKey)`
BEFORE `load_journal` (`:3241`) — never the heal. The `past_boundary` → `ArtifactForCeremony` rule
lives in the `state = match (ceremony.dealing_closed(), agreed, past_boundary)` after the journal match
(`:3284-3291`); the two new arms `return` before it, so it applies to `Present` (and the pre-seal
starts) only. `drive_acquisition`'s `Sealed`-entered edge (`:3818-3837`) matches `Sealed` only.

### 0.7 Tests and mutations

(a) Stand `testbed::tests::an_absentee_restarted_after_the_seal_heals_its_share_and_signs` (`tests.rs:2030-2192`) `[KNOWN]`.
Deletion is asserted: `assert!(path.exists(), …)` then `remove_file(..).expect(..)` for both
`beacon-dkgjournal-e2.bin` and `beacon-share-e2.bin` (`:2098-2103`). Inputs to `recover(2)` on the
replay: no journal, no share, clock `first.heights[3] = 90 ≥ SEAL_2 = 44` (`:2033`, `:2093-2096`;
`EPOCH_LEN = 32` `:1088`, `DKG_MARGIN_BLOCKS = 20` `actor.rs:123`), artifact PRESENT in node 3's
store (phase 1 asserts `artifact_on_every_node(&first, &all, 2)` `:2060` and `signable[3] ∋ 2`
`:2088-2092`, i.e. node 3 keyed in-process during phase 1 — the journal's §0.9 п. 1 admission). So the
stand covers the `Some(set)` arm of `heal_or_acquire` (`heal_over` at decision); the `ArtifactForShare`
arm is covered end-to-end only by the live smoke (§0.2: validator-3's store was empty at restart, the
case asserts `dpos_dkg_artifact_pull_ok_total ≥ 1`, `asserts_fault.py:668-670`) and, up to the
artifact's arrival, by the unit `recover_no_journal_at_the_seal_deadline_waits_for_the_artifact_to_heal`.

«Signs» is proven by liveness after node 1 is cut at 102 (`:2109-2118`): the run must reach
`min_height_of([0,2,3]) ≥ 130` — 28 blocks with exactly `{0, 2, 3}` connected, committee 4, f = 1,
quorum 2f+1 = 3, so every one of node 3's votes must count. A verify-only node casts none:
`epoch_manager.rs:1252-1256` — `SignerVerdict::Withheld` → `soft_enter` → `return` without
`spawn_engine`; `share_probe`/`signer_scheme` withhold without a share (`surface.rs:2258-2295`,
`:2331-2359`). And a vote only counts with a VALID partial: `combined_scheme.rs:348-351`
(`Some(value) => o.verify_partial(round, attestation.signer, &value), None => false`, comment `:340`
«t == consensus quorum»). So `!timed_out` (`:2147-2154`) + `metric(3, dkg_ceremony_ok_total) == 1`
(`:2164-2168`) + `signable[3] ∋ 2..=last` (`:2174-2181`, the production `Beacon::can_participate`
read, `stand.rs:1994-2003`) prove node 3 signed with a share on the polynomial. Can it pass with the
heal broken? No — M1 and M2 below park it at 102 with `signable[3] = [0, 1]`.

(b) The three inverted units assert the new cell: `…_waits_for_the_artifact_to_heal` — phase
`acquiring_artifact_for_share`, no ceremony, no journal written, latches empty then exactly
`{NoArtifact}` after the boundary, pull asked (`:11904-11938`); `…_evicts_it_and_heals_as_a_player` —
`acquiring_logs`, `load_journal == NoFile`, `want.len() == pinned.len()` (`:12257-12283`);
`…_heals_not_deals` — `acquiring_artifact_for_share`, no ceremony, no journal, no latch before the
boundary (`:13807-13822`). `node0_absent_epoch2_artifact` (`:11945-11988`) builds a real pinned set:
`mint_committee_logs_at(&keys[1..], ..)` (`:8751-8804`) runs a full dealing exchange among nodes
1..3 only (node 0 has no ceremony, its direct messages are dropped at `:8780`), and the fixture
self-checks `DealerLogSummary::Ok{reveals}` contains `me0` for EVERY log (`:11967-11974`), then
`observe`s the outcome from those three logs (`:11978-11983`). The acked unit
(`:12091-12223`) takes the four sealed logs out of `node0_pre_seal_journal_full_sealed`
(`:9812-9876`, a complete exchange, so every dealer holds node 0's ack) — it does NOT self-verify the
withholding and cannot tell `MissingPlayerDealing` from `OffPolynomial` (F-03).

(c) Mutations `[KNOWN]` — `cargo test -p fluentbase-consensus --lib beacon::actor::clock_tests` (93
tests) and `cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine an_absentee_restarted_after_the_seal_heals_its_share_and_signs`,
`CARGO_BUILD_JOBS=12`, sequential; backup `scratchpad/review-mut/actor.rs.orig`
(md5 `6bd26c36987e3f9f9ebb343697ee5582` = `y2.md5`); rollback by `cp`, md5 re-checked after each;
outputs in `scratchpad/review-mut/m{1,2,3}-{lib,stand}.txt`.

- **M1** — `NoFile if past_seal` returns `quiet(EpochState::Unrecoverable { key })` instead of
  `heal_or_acquire` (md5 `215741f6c1c56a07991e2168e6961aa1`). Units: `test result: FAILED. 89 passed; 4 failed`
  — `at_a_zero_width_…_heals_not_deals`, `recover_no_journal_…_waits_for_the_artifact_to_heal`,
  `an_absentee_heal_over_an_acked_dealing_…`, `an_absentee_restarted_…_from_the_reveals`. Stand:
  `(B4″) … phase2 heights=[102, 102, 102, 102] … signable=[…, [0, 1]] ceremony_ok=[…, Some(0.0)] …
  cell_lines=1` → `panicked at crates/dpos/consensus/src/testbed/tests.rs:2147:5` →
  `test result: FAILED. 0 passed; 1 failed`. **Red, as expected.** Rollback md5 `6bd26c36…`.
- **M2** — `heal_over`: `let want: BTreeSet<LogId> = BTreeSet::new();` (md5
  `05289f6d1b323a6107a568d4fbb0cced`). Units: `FAILED. 91 passed; 2 failed` —
  `recover_torn_journal_…_evicts_it_and_heals_as_a_player`, `an_absentee_restarted_…_from_the_reveals`.
  Stand: `phase2 heights=[102, 102, 102, 102] … signable=[…, [0, 1]] ceremony_ok=[…, Some(0.0)]` →
  `tests.rs:2147:5` → `FAILED. 0 passed; 1 failed`. **Red.** Rollback md5 `6bd26c36…`.
- **M3** (own choice) — drop `self.evict_journal(epoch)` in the `Torn if past_seal` arm (md5
  `b4464cfd0efd472fff4509611aca4f17`). Units: `FAILED. 92 passed; 1 failed` —
  `recover_torn_journal_at_the_seal_deadline_evicts_it_and_heals_as_a_player` (`:12272`, the
  `load_journal == NoFile` assertion). Stand: `test result: ok. 1 passed` — **green**: the stand
  takes the `NoFile` arm. What this names: the torn-eviction is covered by ONE assertion in one unit;
  no test drives the `Torn` arm through fetch → `try_recompute`, so the failure mode the eviction
  prevents (appended bodies after a torn prefix are never read, `share_state.rs:610-637` ⇒
  `try_recompute` sees `Torn` forever ⇒ `Stalled{HealFailed}` park, `actor.rs:3971-3981`) is asserted
  by reading, not by a test. Rollback; `md5sum -c gates/y2.md5` → all five `ЦЕЛ`.

### 0.8 `SatOut` removal `[KNOWN]`

Every `match` on `EpochState` is exhaustive (the crate compiles with `-D warnings` under clippy,
`y2-clippy.txt`; the arms: `name` `:361-376`, `ceremony`/`ceremony_mut` `:379-397` (wildcard),
`held_digest` `:401-414` (wildcard), `needs_artifact` `:417-427`, `carries` `:484-507`, `enter`
`:1365-1370` (wildcard), `apply_artifact` `:1708-1765` (wildcard)). `StallReason::SatOut`, the
`sat_out` label and the `carries` arm are gone (`metrics.rs:52-53, 65, 73, 87`; `actor.rs:504`);
`git grep -n 'SatOut\|sat_out' -- crates devnet .claude` → only `devnet/…/asserts_prod_dkg.py:187,284,355,665`
(`assert_sat_out_torn`, a harness FUNCTION NAME reading `TORN_LINE`, not the metric — F-01) and the
gitignored `.claude` docs, all marked DELETED/REVISED. No producer, no reader.

### 0.9 Hygiene `[KNOWN]`

No `#[allow]`; every added `unwrap`/`expect`/`panic!` is inside `mod clock_tests` or `testbed::tests`
(`git diff | grep`); no `unsafe`; the only new `pub` is `Outcome::signable` on the `pub(super)`
test-only struct (`stand.rs:916`) and `StallReason::ALL`'s arity. The two new `warn!` (`:3249-3254`,
`:3259-3264`) run once per epoch DECISION: `decide` returns early on `epochs.contains_key`
(`:2944`) and `recover` inserts the slot through `enter` (`:2952`); the committee read (`:3218`)
precedes both, so an unreadable committee retries without reaching them. The `heal_over` INFO is once
per entry (`held_digest` is `Some` afterwards, so `apply_artifact` returns at `:1692-1697`).

Added comment lines (`git diff HEAD -- crates | grep -cE '^\+\s*//'` = 57). By the rule (a *why* the
code cannot show, one line where one does; no narration, no document/finding/history references):
- Narrating/none: `:1759` «An unrecoverable member still verifies with the key.» (an existing comment
  with `sat-out /` cut — borderline narration, pre-existing shape).
- Cites a document/finding/brief: none. The word «(self-checked)» (`:11944`) and «(B4″)» in the
  `eprintln!` label (`tests.rs:2122`, a string, not a comment — F-05) are the only leftovers of the
  brief's vocabulary.
- History: none left; the `INTERVAL` doc (`:4451-4455`) and the retention/latch test docs
  (`:8173-8179`, `:8237-8240`, `:8256`, `:10657-10659`) are pre-existing comments with the symbol
  renamed in place.
- Multi-line where one would do: the `recover` doc paragraph (`:3128-3133`, six lines) states the
  reveal rule and the eviction reason — the one real *why* of the change; acceptable as the rule
  doc of `recover`. `JournalLoad` docs (`share_state.rs:560-567`, `:576-580`, `:586-591`) are
  rewrites of existing docs; `:562-565` keeps the stale «re-dealing fresh would draw new `OsRng`
  randomness» sentence (the dealer has been seeded since `actor.rs:3311-3314`) inside a paragraph
  this change edited (F-09).
- Test/fixture docs: one-liners at `:11935`, `:11944`, `:12089`, `:12243`, `tests.rs:2028`,
  `stand.rs:915`, `:1579`, and the three-line `:13778-13780` — within the rule.

### 0.10 Docs `[KNOWN]` (`.claude` is gitignored, `.gitignore:47`; read from disk)

`00_preamble.md:6-31` — a new `verified-against` entry dated 2026-09-15 for 5.3-Д with the
mechanism, the commonware anchors and the test list; `:293-294`, `:299` — the А1 entry amended
«[`SatOut` DELETED …]» / «[REVISED …]». `08_…md:1023` (`ArtifactForShare` row, «ALSO the post-seal
absentee»), `:1025` (`Acquiring(Logs)` row), `:1028` (the struck `SatOut` row), `:1029`
(`Unrecoverable` row), `:1107-1116` (`StallReason` list of 9, «the two terminals»), `:1173` (renamed
test), `:2164-2167` (residual prose REVISED). `DPOS_ARCHITECTURE_CHANGELOG.md:16-27` — the 5.3-Д
entry. `grep -rn 'SatOut\|sat out\|sits out' .claude/dpos_architecture` → every hit is either the
DELETED/REVISED marker, the struck row, or `08_…md:2154` «the member sits out that epoch» in the
accepted-residual paragraph about a body NO peer holds — still true (that node stays `Acquiring(Logs)`
until the sweep; it does not sign). Line anchors in the new entry drifted after round 3's comment
trimming (F-06): e.g. `recover` `:3149` → 3141, `Torn if past_seal` `:3256` → 3248, `evict` `:3263` →
3255, `NoFile if past_seal` `:3266` → 3258, `heal_or_acquire` `:2896` → 2890, `heal_over` `:2868` →
2863, `key_held_share` `:1797` → 1792 and `:1821-1826` → 1816-1821, `try_recompute` `:4013` →
4006-4021; tests `:11898` → 11889, `:12008` → 11995, `:12105` → 12091, `:12261` → 12245, `:13802` →
13782, `tests.rs:2033` → 2030. Symbol anchors are all current.

### 0.11 dsh report (`history/E5-3-D-DSH-1.md`)

| id | verdict | evidence |
|---|---|---|
| D-01 | CONFIRMED as prose/semantics drift, runtime consequence NOT observed | `verdicts_rotation.py:562` `TORN_LINE` is a prefix of the new WARN (`actor.rs:3251`) ⇒ `evaluate_torn_sitout` still fires; `evaluate_no_re_deal` (`:1067-1078`) holds — no `start_fresh` post-seal; `evaluate_shareless` (`:1080-1084`) reads `SHARE_LINE` (the LIVE finalize line, `asserts_prod_dkg.py:116-124`), which a heal never writes; only `evaluate_did_not_promote` (`asserts_prod_dkg.py:415-419`, `:670-673`) turns red if the torn victim never acked a dealer — it is torn after the «journal present» gate (`:625-634`), i.e. after it dealt and received dealings, so it acked ⇒ `Unrecoverable` ⇒ green `[LIKELY]`; the halt case's victims have no artifact to heal over ⇒ `ArtifactForShare` forever ⇒ green. F-01. |
| D-02 | CONFIRMED and FIXED in round 2 | `verdicts_fault.py:838-842` `\bpinned=`; live `y2` run green. |
| D-03 | CONFIRMED | §0.7(a): the stand covers `Some(set)` only; the smoke covers `ArtifactForShare` live. F-10. |
| D-04 | CONFIRMED (MINOR) | `share_state.rs:528-531` (`encrypted journal record but no seal key available` ⇒ `Torn`); old code kept the file. F-02. |
| D-05 | CONFIRMED, fixed in round 3 | `metrics.rs:65` `ALL: [StallReason; 9]`; grep empty. |
| D-06 | CONFIRMED, fixed in round 3 | `actor.rs:2957`, `:3782-3794`, `:4449-4458`; `share_state.rs:560-591`, `:631-632`. |
| D-07 | CONFIRMED, fixed | `tests.rs:2069-2073` message now matches the assertion. |
| D-08 | CONFIRMED, fixed | `tests.rs:2068-2085` asserts seats via `committee_seats(1, 4)`. |
| D-09 | CONFIRMED (NIT) | `tests.rs:2120`; the substring is in the old warn too and the grep is not node-filtered. F-04. |
| D-10 | REJECTED for this workspace | `.claude/…` exists on disk here (gitignored, `.gitignore:47`); the doc edits are present (§0.10). |
| D-11 | CONFIRMED (NIT, pre-existing shape) | `actor.rs:1746-1750`, `:3844-3848`. F-11. |
| D-12 | CONFIRMED, fixed in round 3 | no design-cell / history reference remains in an added comment (§0.9). |
| D-13 | CONFIRMED (NIT, pre-existing) | `actor.rs:4060-4066` re-arms on `PersistFailed`; `adopt_share` logs ERROR per attempt (`:2081-2091`). Bounded by the sweep. F-12. |

### 0.12 Hard-stop

Nothing requires a change in `DECISIONS.md`. П-9 says «`NoFile` после дедлайна ⇒ не стартовать» and
lists `Recompute` among the automaton's states (`DECISIONS.md:92`) — both hold: no dealing is started
post-seal, and the recompute is the state entered. П-2/П-3/Д-1/Д-3/Д-6/Д-7/Д-9 are not touched (the
share still passes `validate_share_on_poly` before adoption, П-3; the chain's `changed` bit still
decides, Д-7 `:3203-3217`). The deviation Д-5.3Д-1 is from the DESIGN's §5.2/§5.4 cells
(`E5-BEACON-DESIGN.md:612`, `:616`, `:637` — «(Torn, h ≥ seal) ⇒ SatOut; (NoFile, h ≥ seal) ⇒ SatOut»,
«при/после seal ⇒ SatOut(E) явное») and is owner-ratified (journal §0.7); R-036's object — a second,
differently-acked log — cannot be produced by the heal (§0.3). No BLOCKER the design does not answer.

### 0.13 Verdict

**COMMIT.** Not read: `ceremony.rs` beyond `info_for`/`finalize_over_pinned`/`recompute_scoped`
(`DkgCeremony::start`/`resume`/`handle` internals), `log_resolver.rs` (in-flight dedupe and retry
cadence — relied on the `fetch_missing_logs` doc and the dsh trace), `artifact.rs`/`pull_artifact`,
`dkg_agree.rs`, `stand.rs` beyond the diff (`replay`/`partition`/`Runner::from(checkpoint)` — the
fresh-registry claim rests on the M1 run's `ceremony_ok=[…, Some(0.0)]` in phase 2, which could not
be 0 if metrics carried over), `asserts_prod_dkg.py` outside the quoted ranges, `verdicts_rotation.py`
outside `:555-1090`, the harness unit tests beyond the diff, `E5-BEACON-DESIGN.md` beyond `:608-640`,
the base gate logs beyond the `x2` counts and doc-warning breakdown, and (per the brief)
`E4-ORCHESTRATOR.md`.

### 0.14 Weakest — ranked

1. The smoke's proof that the NEW arm fired is indirect: no container log survived; it rests on
   `want == pinned` + no `ceremony started` + the base run's «neither road» + the code's only road
   (§0.2). A rerun with `docker logs` captured before teardown would close it.
2. F-01: the prod-dkg smokes' runtime behaviour under the new cell is inferred from the harness gates
   and the victims' timing, not run.
3. The resolver's dedupe/retry bound (§0.4) is read from `fetch_missing_logs`'s doc and the dsh
   trace, not from `log_resolver.rs`.
4. The stand's replay reconstructing every node with fresh metrics is inferred from a mutation run's
   counters, not from reading `Stand::replay`.
5. The `Err(other)` unreachability for an absentee is by branch enumeration (`[LIKELY]`), as in the
   journal.

## §1 Findings

| id | severity | file:lines | what is wrong | refutation attempt | confidence |
|---|---|---|---|---|---|
| F-01 | MINOR | `devnet/…/cases/smoke/asserts_prod_dkg.py:187-202, 340-360, 405-425, 655-680`; `verdicts_rotation.py:1047-1090` | The durability (phase 3) and halt smokes still assert and describe the deleted «sit-out» semantics for a post-seal `Torn` victim (`assert_sat_out_torn`, `evaluate_torn_sitout`, `evaluate_shareless`, docstrings «sits out E_new permanently», the halt case's «cannot complete it — the recompute needs `JournalLoad::Present` and a torn journal never becomes that» at `:668-671` is now false: the torn file is evicted and the heal runs). Under the new code the victim heals (never acked) or ends `Unrecoverable` (acked). | `TORN_LINE` (`verdicts_rotation.py:562`) is a prefix of the new WARN (`actor.rs:3251`); no re-deal post-seal; `evaluate_shareless` reads the live-finalize `SHARE_LINE`, not `HEAL_LINE`; the victim is torn after the «journal present» gate so it acked ⇒ `Unrecoverable` ⇒ `evaluate_did_not_promote` green; the halt victims have no artifact ⇒ `ArtifactForShare` forever ⇒ green. Runtime green `[LIKELY]`; not run. | prose drift confirmed by code; runtime inferred |
| F-02 | MINOR | `actor.rs:3248-3257`; `share_state.rs:528-531, 576-580` | Post-seal `Torn` now EVICTS. `Torn` also covers an intact journal that is merely undecryptable in this process (wrong keystore mode ⇒ `encrypted journal record but no seal key available`). The old cell kept the file, so a later restart with the right key could `resume`; now the records are gone and an acked player ends `Unrecoverable` for that epoch. | Pre-seal already evicted on `Torn`; the share file is separate and checked first; the records are unreadable to this process anyway; operator error, one epoch, verify-only is safe. | confirmed by code; scenario frequency unknown |
| F-03 | MINOR | `actor.rs:12091-12223` (`an_absentee_heal_over_an_acked_dealing_is_unrecoverable_not_a_loop`) | The «acked» fixture does not self-verify that the pinned logs hold node 0's ack and no reveal of it (the absent fixture does, `:11967-11974`), and the assertions (`unrecoverable`, `dkg_share_unrecoverable == 1`, `dkg_ceremony_ok == 0`) cannot tell the `MissingPlayerDealing` arm (`:4006-4022`) from the `OffPolynomial` arm (`:4044-4058`) — both set the same phase and counter. A fixture drift into a reveal would pass through `keyed`, so the test would go red, but a drift into an off-polynomial share would still pass as «the acked case». | `node0_pre_seal_journal_full_sealed` (`:9812-9876`) is a complete exchange, so every log holds the ack; the outcome is computed by the canonical `finalize_over_pinned`, so off-polynomial is not reachable today. | confirmed by code |
| F-04 | NIT | `tests.rs:2120, 2143-2146` | The anti-vacuity guard greps a substring present in the old `SatOut` WARN as well and is not filtered to node 3 (`logs_containing` is cluster-wide). It proves the cell was hit by SOMEONE. | Only node 3 can hit it (0..2 hold share files ⇒ `share_held` path); `!timed_out` + `ceremony_ok == 1` + `signable` carry the load, and M1/M2 show they do. | confirmed by code |
| F-05 | NIT | `tests.rs:2121-2142` | `eprintln!` labelled `(B4″)` (a brief iteration marker) and dumping EVERY stand log line (`warns=`) on every run — noise in `--nocapture` output; the other stand tests in the file do not do this. | Diagnostic only; harmless when captured. | confirmed by code |
| F-06 | NIT | `.claude/dpos_architecture/00_preamble.md:10-31`; `08_…md:1023, 1025, 1029` | Line anchors in the 5.3-Д `verified-against` entry and the 08 rows are 6–20 lines stale after round 3's comment trimming (list in §0.10). Symbol anchors are correct. | The project rule makes SYMBOL drift the blocker; line drift is the WIP-branch norm and the anchors still land inside the right functions. | confirmed by reading |
| F-07 | NIT | `actor.rs:2874-2879` | The heal's INFO says «demoted committee member detected» — for the post-seal absentee nothing was demoted; the wording now names one of three entries. | The harness pins the text (`HEAL_START_LINE`); a rename is a harness+code change for no behaviour. | confirmed by code |
| F-08 | NIT | `share_state.rs:560-567` | The `JournalLoad` doc this change rewrote still opens with «re-dealing fresh would draw new `OsRng` randomness → a divergent commitment» — stale since the dealer is seeded from key + epoch (`actor.rs:3311-3314`); the reason a torn journal must not re-deal post-seal is the possibly-broadcast log, which the new tail states. | Pre-existing sentence; the new tail is correct. | confirmed by code |
| F-09 | NIT | `actor.rs:1759` | «An unrecoverable member still verifies with the key.» is narration of the arm below it. | An existing comment with two words cut; matches the file's baseline. | confirmed by code |
| F-10 | MINOR | `actor.rs:1740-1751, 1816-1821`; `tests.rs:2030-2192` | No in-crate test drives the smoke's exact road end to end: `NoFile if past_seal` with NO artifact → `ArtifactForShare` → artifact arrives → `key_held_share(None)` → heal → `Keyed`. The unit stops before the artifact; the stand starts with the artifact held. | The `None` arm of `key_held_share` is pre-existing and one line; the live smoke is green on exactly this road; M1 covers the arm's entry. | confirmed by code |
| F-11 | NIT | `actor.rs:1746-1750, 3844-3848` | With the artifact held but `committee_for` failing, the phase stays `ArtifactForShare`, the `NoArtifact` latch misnames the cause and a pull is spent on a local read race. | Pre-existing shape; `reconcile_with_store` re-applies next tick; the pull is throttled. | confirmed by code |
| F-12 | NIT | `actor.rs:4060-4066`, `:2081-2091` | `PersistFailed` re-arms `attempted`, so `adopt_share` logs ERROR every tick while the disk fails; newly reachable from the absentee heal. | The documented §5.4 retry, identical on the pre-existing heal paths, bounded by the sweep. | confirmed by code |

## Leave as is

1. The heal broadcasts nothing (§0.3): both new phases have no ceremony, `quiet` returns no
   `Outgoing`, the confirmation index is fed only by live ceremonies, `recompute_scoped` drops its
   acks, `adopt_share` has no network side.
2. A sealed node cannot re-seal or re-deal: `start_fresh` only behind `!past_seal`, and the seal
   predicate equals `past_seal`; `evict_journal` on a `Torn` file removes nothing readable and never
   the share.
3. The acked-then-lost absentee is `Unrecoverable` exactly once with one WARN, no fetch, no re-run;
   the never-acked absentee keys over the reveals (`dkg.rs:1838-1846`) — both proven by units that
   go red under M1/M2.
4. The `Torn` eviction before the heal is necessary, not cosmetic: a body appended after a torn prefix
   is never read (`share_state.rs:610-637`), so without it `try_recompute` would park on
   `Stalled{HealFailed}` forever (`actor.rs:3971-3981`).
5. Non-members never reach the arms (`:3222-3229`); the other five cells of the restart table are
   untouched; the `past_boundary` rule stays on `Present`.
6. `EpochState::SatOut` and `StallReason::SatOut` deleted rather than kept as a dead label — the
   right call for an undeployed surface (no reader anywhere).
7. The stand test is non-vacuous: real temp share dir, files asserted present before deletion, fresh
   processes on replay, quorum arithmetic that needs node 3's valid partial (`combined_scheme.rs:348-351`).
8. The harness reader fix is exact and pinned by a negative test (`dealers=` → `None`).
9. `Outcome::signable` through the production `Beacon::can_participate` — the same probe that printed
   `NoUsableShare` in the failed smoke; the right observable.

Substantial, outside the row (one line): the stand cannot replay a node that crossed a boundary
keyless (journal §0.8, σ-hole) — that gap, not the heal, is why the fixture keys node 3 in phase 1
and deletes its files by hand; worth its own row if the `ArtifactForShare` road is to be stand-covered.
