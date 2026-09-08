SOLIDITY_CONTRACTS_DIR ?= ../../../solidity-contracts

.PHONY: case-growth case-quorum case-seed-continuity clean down golden logs regen-contracts \
        xp-up xp-down xp-status xp-e5 xp-e2 xp-byz \
        smoke-all smoke-base smoke-base-dry-run smoke-byzantine \
        smoke-cert-cascade smoke-cert-catchup smoke-cert-follow smoke-cert-keyless \
        smoke-crash-survivor \
        smoke-deferred smoke-epoch smoke-fault smoke-fault-dry-run smoke-follow-dry-run \
        smoke-full-restart smoke-liveness smoke-onchain-dry-run smoke-peers smoke-prod-dry-run \
        smoke-production-path smoke-rejump-signer smoke-sim smoke-sim-quick smoke-sim-ram \
        smoke-tx smoke-tx-cascade smoke-vrf smoke-vrf-boundary smoke-vrf-dkg-durability \
        smoke-vrf-dkg-halt smoke-vrf-dkg-live-heal smoke-vrf-dkg-restart-midwindow smoke-vrf-fault \
        smoke-vrf-rotation smoke-weighted-vrf up

regen-contracts:
	@test -d $(SOLIDITY_CONTRACTS_DIR) || \
	  (echo "ERROR: solidity-contracts not found at $(SOLIDITY_CONTRACTS_DIR); set SOLIDITY_CONTRACTS_DIR" && exit 1)
	cd $(SOLIDITY_CONTRACTS_DIR) && forge build
	cp $(SOLIDITY_CONTRACTS_DIR)/out/StakingPool.sol/StakingPool.json                       contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/FluentGovernance.sol/FluentGovernance.json             contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/MockBlendToken.sol/MockBlendToken.json                 contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/PrevRandaoProbe.sol/PrevRandaoProbe.json               contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/MockRollup.sol/MockRollup.json                         contracts/
	git -C $(SOLIDITY_CONTRACTS_DIR) rev-parse HEAD > contracts/.vendor-sha
	@echo "Vendored contracts updated; SHA recorded in contracts/.vendor-sha; remember to commit."

up:
	docker compose up --build

down:
	docker compose down -v

logs:
	docker compose logs -f

# Two-phase sequencer→DPoS migration smoke. ORDER MATTERS and the phases are NOT
clean:
	docker compose down -v --remove-orphans

# --- regression cases (each self-contained: brings up + tears down its own stack) ---
# Read-only default-stack suite on ONE bring-up: tx + epoch + vrf + vrf-boundary.
# These four never mutate consensus (no node stop/restart/jail), so they share a
# single sequencer→DPoS migration instead of four. Each remains runnable standalone via
# its own smoke-<case> target below. This is what run-all uses (replaces the three
# separate tx/epoch/vrf runs and adds boundary coverage).
smoke-base:
	@python3 -m dpos_harness case smoke-base $(ARGS)
# Recoverable DESTRUCTIVE default-stack suite on ONE bring-up: deferred + peers +
# vrf-fault + crash-survivor + full-restart, chained least→most invasive (each
# restores the stack before handing off; fail-fast). Each remains runnable
# standalone via its own smoke-<case> target below. Excludes smoke-liveness (its
# jail is unrecoverable). Heavy (~25-30 min).
smoke-fault:
	@python3 -m dpos_harness case smoke-fault $(ARGS)
smoke-tx:
	@python3 -m dpos_harness case smoke-tx $(ARGS)
smoke-epoch:
	@python3 -m dpos_harness case smoke-epoch $(ARGS)
smoke-peers:
	@python3 -m dpos_harness case smoke-peers $(ARGS)
smoke-liveness:
	@python3 -m dpos_harness case smoke-liveness $(ARGS)
smoke-crash-survivor:
	@python3 -m dpos_harness case smoke-crash-survivor $(ARGS)
smoke-byzantine:
	@python3 -m dpos_harness case smoke-byzantine $(ARGS)
# bug-B committee-GROWTH regression (dpos_harness, deterministic): forces a committee
# GROWTH (register_activate → setActiveValidatorsLength) across an epoch boundary
# and asserts NO finalize-stall + NO "dkg_logs: an idx is out of committee[E+1]
# range" log line. The 2-epoch warm-up fix freezes the committee once so record and
# verify read the same slot. Self-contained: brings up its own minimal stack (6
# validators, epoch=32, cascade off) and tears it down (SIM_KEEP_UP=1 to keep it).
case-growth:
	@python3 -m dpos_harness case growth
# ZERO-OVERLAP boundary (FLU-1203): brings up 8 validators with a committee of 4,
# activates v4..v7 WITHOUT raising the cap, then out-stakes all four incumbents inside
# ONE selection epoch so the committee flips wholesale. Asserts the PREMISE first (all
# four seats flipped at one commit — a three-of-four flip fails HERE, not on liveness),
# then that the chain crossed the boundary and no leader skipped a view. Cannot run on
# the production-path stack: MIN_COMMITTEE_LENGTH=4 means zero overlap needs 8 keyed
# validators. Self-contained; tears down (SIM_KEEP_UP=1 to keep it).
case-turnover:
	@python3 -m dpos_harness case turnover
# Standalone BFT quorum-loss safety case: brings up a clean 7-member committee, stops f+1 to
# confirm finalization HALTS (quorum lost), restarts them + confirms EACH is back up, then asserts
# finalization RESUMES. Proves the f+1 stall-then-recover property the removed in-sim probe used to
# (unreliably) check. Self-contained; tears down (SIM_KEEP_UP=1 to keep it).
case-quorum:
	@python3 -m dpos_harness case quorum
# Standalone VRF seed-continuity case: brings up a clean 7-member committee, stops ONE member so
# its views nullify while quorum survives, then rebuilds the view→leader map from two INFO logs
# and asserts the leader AFTER a nullified view is NOT the pre-change fallback prediction. The
# assertion is two-sided: under the old binary that prediction held with certainty, so a single
# mismatch falsifies it. A positive control (view 1 of each epoch, which still takes the fallback)
# validates the offline predictor, so a broken predictor reads INCONCLUSIVE (2), never green.
# Self-contained; tears down (SIM_KEEP_UP=1 to keep it).
case-seed-continuity:
	@python3 -m dpos_harness case seed-continuity
# Build the "golden DPoS-active" snapshot ONCE (full boot → graceful flush → tar the /runtime
# volume, keyed by an image+contracts+topology invalidation hash). Then SIM_USE_GOLDEN=1
# case-growth RESTORES it and reaches DPoS-active in seconds instead of ~2-4 min. `make golden
# ARGS=--check` prints fresh/stale without building.
golden:
	@python3 -m dpos_harness stack golden $(ARGS)
# Additional NOW-tier cases (Phase 6).
smoke-full-restart:
	@python3 -m dpos_harness case smoke-full-restart $(ARGS)
# Trustless cert-follower, FOUR phases: subscribe-align + gap back-fill + tampered-cert reject
# (phase 3, refused at DECODE) + PK_epoch delivery (phase 4, FLU-1167). Phase 4 is the only one
# that reaches the SEED check: the follower obtains the epoch key over its cert upstream, stops
# taking vote-only admissions, and then refuses a certificate whose seed slot was CLEARED behind a
# still-valid multisig quorum — which a vote-only follower accepts. Heavy (~15-22 min: phases 1-3
# are ~10-12, phase 4 adds two key waits, an arming round-trip and two observation windows).
smoke-cert-follow:
	@python3 -m dpos_harness case smoke-cert-follow $(ARGS)
# FLU-1202's live gate, and the only case whose PRECONDITION is the point. A `--cert-follow`
# node enters a beacon-active epoch with NO key (it can obtain PK_epoch only by fetching the
# epoch's agreed artifact, and that fetch is triggered by a certificate that already went through
# the inlet), so its first certificates are admitted on the multisig quorum ALONE. The case
# refuses to conclude anything until it has read `dpos_cert_vote_only_admissions_total >= 1` off
# the follower's reth recorder; then the key arrives and the counter must go FLAT over a window
# in which v0 advances and the follower itself finalizes — the late key taking effect through the
# scheme the node already had, with no `apply_pin`. The surviving below-frontier repair sweep is
# ruled out by ORDER, not by absence: it fires for exactly this epoch (~30 s after the adoption,
# live), so the case requires only that the node's own artifact fetch spoke first.
# One honest follower, nothing poisoned. Moderate (~6-9 min); IS in run-all (`cli.py` SUITE).
smoke-cert-keyless:
	@python3 -m dpos_harness case smoke-cert-keyless $(ARGS)
# Lean-follower serving + L1 trust root: MockRollup checkpoint align +
# follower→follower cascade + bogus-checkpoint fail-closed reject.
smoke-cert-cascade:
	@python3 -m dpos_harness case smoke-cert-cascade $(ARGS)
# Tx WRITE PATH across the sentry cascade: a tx submitted to L3 (reaches ONLY L2)
# is relayed via reth devp2p tx-gossip to a hidden validator's proposer, mined,
# finalized, and synced back to L3. Privacy invariant + fail-loud monitor checked.
smoke-tx-cascade:
	@python3 -m dpos_harness case smoke-tx-cascade $(ARGS)
# Deferred-execution observables: K-lag invariant, result commitment, EL-slowed validator.
smoke-deferred:
	@python3 -m dpos_harness case smoke-deferred $(ARGS)
# Cert-budget catch-up fix (task dpos-cert-budget-catchup-recover-stall): DETERMINISTIC
# gate that a GRACEFULLY-restarted validator falling behind during catch-up PARKS on a
# cert-lagged deferred block (event-driven, no budget/shutdown) and rejoins -- NO
# "finalization cert unavailable within budget ... shutting down" ERROR, NO
# consensus_exit. Fails loud if the cert-park path is never exercised (false pass).
# Tuned config (epochBlockInterval=64 -> re_jump_threshold=64; down-gap 40). The victim
# runs its catch-up window under CERT_CATCHUP_DELAY_MS (300) of tc-netem egress delay --
# depth is NOT the lever, the derive-vs-repair ratio is, and at zero RTT the fetch always
# wins on a LAN so the park would fire at no gap. Set CERT_CATCHUP_DEEP=1 to also exercise
# the gap-gated re-jump. Heavy (~8-10 min).
smoke-cert-catchup:
	@python3 -m dpos_harness case smoke-cert-catchup $(ARGS)
# Threshold randomness beacon: prev_randao = H(verified threshold seed) under DPoS.
smoke-vrf:
	@python3 -m dpos_harness case smoke-vrf $(ARGS)
# Stake-weighted VRF leader election: skew validator-0's genesis stake 9x and assert
# it proposes proportionally more blocks (the heavy validator leads ~75% of views vs
# ~8.3% each for the three equal-stake peers), while the chain stays live.
smoke-weighted-vrf:
	@python3 -m dpos_harness case smoke-weighted-vrf $(ARGS)
# Live DKG beacon ROTATION: a committee change forces a fresh DKG → a rotated
# on-chain group key; beacon relives from the new epoch's first block; a stable
# epoch carries the key forward (no on-chain commit). Runtime forge deploy +
# dynamic registration; same prerequisites as smoke-production-path. Long (~6-9 min).
smoke-vrf-rotation:
	@python3 -m dpos_harness case smoke-vrf-rotation $(ARGS)
# (`smoke-byzantine-vrf` was here — RETIRED. It drove a committee STAYER to forge a
# PK_E at a change-epoch boundary; the epoch key has left `OrderBlock`, so no block
# asserts a key, the forgery is inexpressible, and `--dpos.byzantine forge-beacon-pk`
# now fails loud at DPoS start. Equivocation is still covered by `smoke-byzantine`.)
# Full production path: runtime contract deploy + dynamic registration + governance
# activation + live committee rotation + liveness ejection. Long (~5-8 min),
# first-of-its-kind, and gated on the EIP-170 Staking-codesize fix; NOT in run-all.
smoke-production-path:
	@python3 -m dpos_harness case smoke-production-path $(ARGS)
# Beacon under fault: f=1 validator down (beacon survives via n−f quorum) + the
# downed node restarts and catches up the gap with verified prev_randao (A1/B3/B4).
# Heavy (~4-5 min); NOT in run-all.
smoke-vrf-fault:
	@python3 -m dpos_harness case smoke-vrf-fault $(ARGS)
# Beacon across an epoch boundary on a stable committee: prev_randao stays active
# + node-agreed across the boundary, on-chain key carries forward (F1/F2). Heavy
# (~3-4 min); NOT in run-all.
smoke-vrf-boundary:
	@python3 -m dpos_harness case smoke-vrf-boundary $(ARGS)
# LIVE-EPOCH KEY DELIVERY (FLU-1166). This target was `smoke-vrf-dkg-liveness` and it
# asserted the OPPOSITE: a member offline through its whole epoch-2 DKG window misses the
# ceremony and now obtains the epoch key ANYWAY — it pulls epoch 2's agreed artifact,
# recomputes its share from the dealers' public reveals (the reveal-fallback path, whose
# only coverage anywhere in the repo this is), leaves vote-only certificate admission, and
# PRODUCES inside epoch 2. The vote-only-exit leg is a COUNTER since FLU-1202
# (`dpos_seed_verify_ok_total` going 0 -> non-zero): the `epoch scheme upgraded to PINNED` log
# edge it used to grep has no emitter any more, because a scheme holds no key material and reads
# the key store live, so there is no pin transition left to log. The chain-liveness half (finalizing on the n−f=3 survivors,
# byte-identical prev_randao) is still asserted; it is no longer the point. ONE victim and
# never two: absentees must stay ≤ f, or every honest dealer withholds its whole log on
# TooManyReveals and the ceremony fails outright — a different test. Tuned genesis
# (epochBlockInterval=64) for two reasons: the recovered member needs room to be elected before
# the epoch ends, and the victim must be stopped before the DEAL phase opens one interval after
# the migration anchor. Production credit is read only AFTER epoch 2 ends — the counter climbs
# until then, and sampling it at the moment of recovery reads a coin toss.
# Heavy (~8-10 min); IS in run-all (`cli.py` SUITE).
smoke-vrf-dkg-live-heal:
	@python3 -m dpos_harness case smoke-vrf-dkg-live-heal $(ARGS)
# Durable mid-window DKG restart: a committee[2] member restarts AFTER its epoch-2 seal
# but BEFORE the boundary; it RESUMES its ceremony from the on-disk journal + pull,
# converges to (PK_2, share), and then PRODUCES in epoch 2 — which a shareless node
# cannot. Production is the load-bearing signal now that the liveness slash and its
# felony ladder are deleted; the old tuning existed only to make that slash reachable.
# Still runs a TUNED config (epochBlockInterval=64) for a generous DKG window.
# Heavy (~6-8 min); NOT in run-all.
smoke-vrf-dkg-restart-midwindow:
	@python3 -m dpos_harness case smoke-vrf-dkg-restart-midwindow $(ARGS)
# Consolidated live-DKG mid-window-restart durability suite on ONE rotation-stack
# bring-up, each scenario on its own DKG ceremony, recoverable phases first: post-seal
# fleet recovery + consensus-quorum stall (G1, subsumes the single-victim midwindow
# baseline) on the epoch-2 ceremony -> torn-journal sit-out (G5) on the first driven
# rotation. Runtime forge deploy + governance-driven rotation; needs foundry. Long
# (~12-18 min); NOT in run-all (like smoke-vrf-rotation / smoke-production-path).
smoke-vrf-dkg-durability:
	@python3 -m dpos_harness case smoke-vrf-dkg-durability $(ARGS)
# >f PRE-SEAL TERMINAL halt (G2) — the negative control for smoke-vrf-dkg-durability's
# post-seal recovery (same 2-of-5 kill, opposite seal state -> permanent boundary
# freeze). Its OWN bring-up (plan §6.2 fallback: a second deterministic committee
# change is not cleanly drivable on the 6-key equal-stake stack), driving the halt on
# the first rotation C_r1. Kills 2 committee stayers PRE-seal + keeps them down ->
# (beacon_for_epoch)(E_new)=None -> proposer skips the boundary view forever -> the
# chain freezes permanently; asserts the sustained no-progress + the 'skipping propose'
# log, then tears down. Needs foundry. Long (~10-14 min); NOT in run-all.
smoke-vrf-dkg-halt:
	@python3 -m dpos_harness case smoke-vrf-dkg-halt $(ARGS)
# TODO(deferred, after-forked-staking-state-harness): E2 wrong-key cert-follower
# smoke (`smoke-vrf-wrongkey` + docker-compose.cert-follow-wrongkey.yml) — a
# follower with a WRONG beacon key must derive a DIVERGENT mixHash (order.digest()
# fallback), the negative of E1. The naive `--dpos.beacon-sharing-path` lever is
# VACUOUS: a cert-follower reads PK_E from on-chain getEpochBeaconKey, not
# sharing-path (see cert_follow/mod.rs::scheme_at). A real divergence needs a
# doctored on-chain key (forked staking state) or a test-only wrong-key injection
# flag (a production code smell), so this is deferred. The core property
# (unverifiable PK_E ⇒ order.digest() fallback ⇒ divergence) is already unit-covered
# by derive.rs::resolve_prev_randao_uses_verified_seed_else_falls_back, and the
# forged-PK_E *attacker* angle is covered by the byzantine beacon smoke. Approved
# deferred by user 2026-06-19.
# Long-running production survivability SIMULATION (task dpos_production_soak_smoke).
# ONE unified beacon-ON dynamic profile: a GENERATED N-validator stack, seeded
# stochastic churn (stop/kill/throttle/liveness-jail/register/rotate/byzantine/
# DKG-mid-window-restart) interleaved with clean epochs under tx load, a
# DKG-window-aware safety gate (so any unexpected stall is a real bug), a
# continuous invariant battery, and a replayable failure bundle. Needs foundry +
# the dpos-devnet-byzantine image.
#
# These run the PYTHON simulation (`dpos_harness/sim`). The bash `scripts/case-soak.sh` they
# used to invoke is gone; the knobs it read as SOAK_* are SIM_* here, and the blaster's LH_* are
# LOAD_*.
#
# Long / unbounded; NOT in run-all. Replay a failure from its logged seed:
#   SIM_SEED=<seed> SIM_VALIDATORS=<n> SIM_EPOCH_INTERVAL=<i> make smoke-sim
#
# SIM_DATA_ROOT: relocate ALL runtime validator data (reth MDBX + static files +
# consensus/marshal journals + keys — the shared /runtime volume) off the system NVMe
# onto the separate disk mounted at /mnt/storage, so sim I/O stops hammering the system
# disk. Opt-in in the harness (empty default = historical docker-managed volume on /);
# the value lives HERE in the launcher. Override on the CLI, or `SIM_DATA_ROOT= make
# smoke-sim` to opt out.
SIM_DATA_ROOT ?= /mnt/storage/fluent-sim
smoke-sim:
	@SIM_DATA_ROOT='$(SIM_DATA_ROOT)' python3 -m dpos_harness sim run
# RAM-backed run: the data root on /dev/shm (already a tmpfs — no sudo/mount needed) so sim
# I/O never touches the SSD. Runs typically die before RAM would fill; if one doesn't, the
# data-root fill watchdog warns at SIM_DATA_FILL_WARN_PCT and hard-fails at
# SIM_DATA_FILL_FAIL_PCT with honest attribution (tmpfs full, not a product failure).
smoke-sim-ram:
	@SIM_DATA_ROOT=/dev/shm/fluent-sim python3 -m dpos_harness sim run
# ~5-8 min CI sanity gate (4 nodes, short epoch; byzantine off).
smoke-sim-quick:
	@SIM_QUICK=1 SIM_DATA_ROOT='$(SIM_DATA_ROOT)' python3 -m dpos_harness sim run

# --- the DRY-RUN aggregates -----------------------------------------------------------------
# One transcript per family, docker-free. They were the fidelity oracle against the bash argv;
# with the bash gone what they check is CHEAPER and still worth a target: that every case in the
# family still walks its whole choreography end to end, with the right commands in the right
# order, in about a second and without a daemon. `make case-<name> ARGS=--dry-run` does one case;
# these do a family.
#
# The read-only five.
smoke-base-dry-run:
	@for c in smoke-tx smoke-epoch smoke-vrf smoke-vrf-boundary smoke-base; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# The destructive seven. ORDER matters here as much as the commands: every one of these breaks
# the stack and puts it back, and a restore issued before the reading it brackets is a different
# test that still passes.
smoke-fault-dry-run:
	@for c in smoke-deferred smoke-peers smoke-vrf-fault smoke-crash-survivor \
	          smoke-full-restart smoke-vrf-dkg-live-heal smoke-fault; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# The follower three. Each runs ALONE — never chained: cert-follow leaves a man-in-the-middle
# poisoning validator-0's certificate plane, cert-cascade leaves two per-run contract addresses in
# the compose environment, and tx-cascade widens the sentry's trusted-peer set. cert-follow now
# leaves a SECOND poisoned stream too (`cert-mitm-seed`, armed and clearing seed slots).
# Its phases 3 and 4b SKIP (exit 0) when their sidecar cannot pip-install `websockets`, so a green
# run on an offline host proves the positive phases only — look for `SKIP (phase 3
# tampered-reject)` / `SKIP (phase 4b cleared-seed reject)` in the output. Phase 4a is unaffected
# by either skip: the honest follower talks to validator-0 directly.
#
# Here the `-f` overlay array and the exported
# ${MOCK_ROLLUP_ADDR}/${BOGUS_ROLLUP_ADDR} are part of the evidence: an export printed after the
# `up` that interpolates it is a follower pointed at the zero address.
smoke-follow-dry-run:
	@for c in smoke-cert-follow smoke-cert-cascade smoke-tx-cascade; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# --- smoke-rejump-signer ---------------------------------------------------------------------
# Post-jump boundary seeding (task 2026_07_30__post_jump_boundary_block_gate): a seated
# validator that falls far enough behind to RE-JUMP must be SIGNING again inside the LANDING
# epoch -- promote line naming epoch_of(landing), and a real proposal in that epoch. Before the
# fix it promoted one epoch LATE (the E-1 boundary block sat below the marshal floor, where
# repair starts at floor+1 and nothing re-fetches it), so the member was verify-only -- no
# proposals, no votes -- for the whole landing epoch, and NO case reddened.
# Tuned config (epochBlockInterval=64 -> re-jump gate 64; down-gap 160, which is inside the band
# (64, 1024) between the steady-state and cold-start jump gates -- the GAP selects the path, not
# the restart). The victim produces nothing for two full epochs at that gap. That used to take
# the liveness slash; the slash and its felony ladder are deleted and the successor tier ships
# flag-off, so the gap has no on-chain consequence and the committee stays four.
# Override BOUNDARY_VICTIM / BOUNDARY_GAP. Heavy (~9-11 min: ~5-6 min to the DKG-safe point at
# interval=64, then the cycle plus up to one epoch of restart-window steering).
smoke-rejump-signer:
	@python3 -m dpos_harness case smoke-rejump-signer $(ARGS)
# The transcript of all four, docker-free. Here the `export` lines are part of the evidence: they
# must precede the `docker compose up --build -d` that genesis-init runs under, and the header's
# `interval=` / `activation=` must show the TUNED values — a tuned interval that reached compose
# but not the profile would give the case a 64-block stack and 32-block epoch arithmetic.
smoke-onchain-dry-run:
	@for c in smoke-liveness smoke-byzantine smoke-cert-catchup \
	          smoke-vrf-dkg-restart-midwindow; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# The production-path four. These are the ONLY targets here that run on the PRODUCTION-PATH devnet: six validators, a
# BARE genesis, and a staking cluster deployed at RUNTIME by `forge script DeployStaking` whose
# addresses are DISCOVERED from the deploy manifest. So they need foundry (forge/cast) on the host
# and a solidity-contracts checkout at $$SOLIDITY_CONTRACTS_DIR, they take 6-18 minutes each, and
# none of them is in run-all.
#
# Each runs ALONE and never chained: every one leaves a rotated (and for production-path, a
# permanently shrunk + jailed) committee behind, and smoke-vrf-dkg-halt leaves the chain
# DELIBERATELY and PERMANENTLY halted.
# The production-path four, docker-free. Here the
# 14-phase bring-up dominates the diff, so read it in two halves: the phases up to `DPoS chain live
# past anchor` must match lib.sh's pp_bring_up_rotation, and everything after must match the case
# script.
smoke-prod-dry-run:
	@for c in smoke-vrf-rotation smoke-vrf-dkg-halt smoke-vrf-dkg-durability \
	          smoke-production-path; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# Aggregate (CI entry point).
smoke-all:
	@python3 -m dpos_harness case all

# --- scripts/xp: the SAME stand at an arbitrary N ------------------------------------------
# The default stack is hardwired to 4 validators + a full-node, which is exactly the size at
# which R-111/R-112 are indistinguishable from any other death: one exit or one tombstone takes
# the visible population to 3 and every node dies. Anything about committee size needs N as a
# knob, so scripts/xp generates the same two-phase compose pair for any N under its own docker
# project (xp<N>) and its own /24, and carries the scenarios EXPERIMENTS.md part 5 was run with.
#
# The image is the ordinary `fluent-dpos-smoke:local` — build it once via `make up` (or any
# case) before the first xp bring-up; genN.py only writes compose files.
#
#   make xp-up N=6      generate + bring up through the sequencer->DPoS migration
#   make xp-e5 N=6      E5: honest exits one at a time until the chain stops
#   make xp-e2 N=6      E2: ONE honest full exit (undelegate) by validator XP_IDX (default N-1)
#   make xp-byz N=6     E1: equivocation on XP_BYZ (default: the last validator); needs N>=5
#   make xp-status N=6  head/finalized/hash per node
#   make xp-down N=6    tear the stand down with its volume
N ?= 6
XP_IDX ?= $(shell expr $(N) - 1)
XP_BYZ ?= $(shell expr $(N) - 1)
XP := scripts/xp

xp-up:
	@python3 $(XP)/genN.py $(N)
	@$(XP)/bringupN.sh $(N)
	@$(XP)/statusN.sh $(N)

xp-status:
	@$(XP)/statusN.sh $(N)

xp-e5:
	@python3 $(XP)/exits.py $(N)

xp-e2:
	@$(XP)/e2exit.sh $(N) $(XP_IDX)
	@$(XP)/e2watch.sh $(N)

# Refuses at N<5 inside byz_overlay.sh/genN.py: at N=4 this stops being an equivocation case
# and becomes R-111 by another writer (R-112). XP_ALLOW_SMALL_BYZ=1 to do it anyway.
xp-byz:
	@python3 $(XP)/inc.py $(N) $(XP_BYZ) xp-byz-n$(N)

xp-down:
	@docker compose -p xp$(N) -f $(XP)/out/xp$(N).yml -f $(XP)/out/xp$(N).dpos.yml down -v \
	  --remove-orphans 2>/dev/null || docker compose -p xp$(N) -f $(XP)/out/xp$(N).yml down -v \
	  --remove-orphans
