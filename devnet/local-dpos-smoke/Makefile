SOLIDITY_CONTRACTS_DIR ?= ../../../solidity-contracts

.PHONY: regen-contracts smoke smoke-swap up down logs clean \
        smoke-base smoke-fault smoke-tx smoke-epoch smoke-peers smoke-liveness smoke-crash-survivor smoke-byzantine \
        smoke-full-restart smoke-cert-follow smoke-cert-cascade smoke-tx-cascade smoke-deferred smoke-cert-catchup \
        smoke-vrf smoke-weighted-vrf smoke-vrf-rotation smoke-byzantine-vrf smoke-vrf-fault smoke-vrf-boundary \
        smoke-vrf-dkg-liveness smoke-vrf-dkg-restart-midwindow \
        smoke-vrf-dkg-durability smoke-vrf-dkg-halt \
        smoke-production-path smoke-sim smoke-sim-ram smoke-sim-quick smoke-all \
        case-growth case-quorum case-seed-continuity golden \
        smoke-base-py smoke-tx-py smoke-epoch-py smoke-vrf-py smoke-vrf-boundary-py \
        smoke-base-dry-run \
        smoke-fault-py smoke-deferred-py smoke-peers-py smoke-crash-survivor-py \
        smoke-full-restart-py smoke-vrf-fault-py smoke-vrf-dkg-liveness-py \
        smoke-fault-dry-run \
        smoke-cert-follow-py smoke-cert-cascade-py smoke-tx-cascade-py \
        smoke-follow-dry-run \
        smoke-liveness-py smoke-byzantine-py smoke-cert-catchup-py smoke-rejump-signer \
        smoke-vrf-dkg-restart-midwindow-py smoke-onchain-dry-run \
        smoke-production-path-py smoke-vrf-rotation-py smoke-vrf-dkg-halt-py \
        smoke-vrf-dkg-durability-py smoke-byzantine-vrf-py smoke-prod-dry-run

regen-contracts:
	@test -d $(SOLIDITY_CONTRACTS_DIR) || \
	  (echo "ERROR: solidity-contracts not found at $(SOLIDITY_CONTRACTS_DIR); set SOLIDITY_CONTRACTS_DIR" && exit 1)
	cd $(SOLIDITY_CONTRACTS_DIR) && forge build
	cp $(SOLIDITY_CONTRACTS_DIR)/out/Staking.sol/Staking.json                               contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/StakingDpos.sol/StakingDpos.json                       contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/StakingEconomics.sol/StakingEconomics.json             contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/StakingRewards.sol/StakingRewards.json                 contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/BlendReserve.sol/BlendReserve.json                     contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/ChainConfig.sol/ChainConfig.json                       contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/StakingPool.sol/StakingPool.json                       contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/SystemReward.sol/SystemReward.json                     contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/FluentGovernance.sol/FluentGovernance.json             contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/ProductionLiveness.sol/ProductionLiveness.json         contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/MockBlendToken.sol/MockBlendToken.json                 contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/BLS12381Verifier.sol/BLS12381Verifier.json             contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/PrevRandaoProbe.sol/PrevRandaoProbe.json               contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/SimplexEvidenceDecoder.sol/SimplexEvidenceDecoder.json contracts/
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
# self-contained: `smoke` (phase1) brings the chain up as the sequencer and LEAVES IT
# RUNNING; `smoke-swap` (phase2) takes over that running chain and migrates it to
# DPoS. Running `make smoke-swap` ALONE polls a chain that was never started and
# times out after 180s ("sequencer did not reach dposActivationBlock=64"). Always run
# them together:  make smoke && make smoke-swap
# (The `smoke-<case>` targets below are different — each is self-contained.)
smoke:
	@./scripts/assert-smoke.sh phase1

smoke-swap:
	@./scripts/assert-smoke.sh phase2

clean:
	docker compose down -v --remove-orphans

# --- regression cases (each self-contained: brings up + tears down its own stack) ---
# Read-only default-stack suite on ONE bring-up: tx + epoch + vrf + vrf-boundary.
# These four never mutate consensus (no node stop/restart/jail), so they share a
# single sequencer→DPoS migration instead of four. Each remains runnable standalone via
# its own smoke-<case> target below. This is what run-all uses (replaces the three
# separate tx/epoch/vrf runs and adds boundary coverage).
smoke-base:
	@./scripts/case-base.sh
# Recoverable DESTRUCTIVE default-stack suite on ONE bring-up: deferred + peers +
# vrf-fault + crash-survivor + full-restart, chained least→most invasive (each
# restores the stack before handing off; fail-fast). Each remains runnable
# standalone via its own smoke-<case> target below. Excludes smoke-liveness (its
# jail is unrecoverable). Heavy (~25-30 min).
smoke-fault:
	@./scripts/case-fault.sh
smoke-tx:
	@./scripts/case-tx.sh
smoke-epoch:
	@EPOCH_MIN_CROSS=1 ./scripts/case-epoch.sh
smoke-peers:
	@./scripts/case-peers.sh
smoke-liveness:
	@./scripts/case-liveness.sh
smoke-crash-survivor:
	@./scripts/case-crash-survivor.sh
smoke-byzantine:
	@./scripts/case-byzantine.sh
# bug-B committee-GROWTH regression (dpos_harness, deterministic): forces a committee
# GROWTH (register_activate → setActiveValidatorsLength) across an epoch boundary
# and asserts NO finalize-stall + NO "dkg_logs: an idx is out of committee[E+1]
# range" log line. The 2-epoch warm-up fix freezes the committee once so record and
# verify read the same slot. Self-contained: brings up its own minimal stack (6
# validators, epoch=32, cascade off) and tears it down (SIM_KEEP_UP=1 to keep it).
case-growth:
	@python3 -m dpos_harness case growth
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
	@./scripts/case-full-restart.sh
# Trustless cert-follower: subscribe-align + gap back-fill + tampered-cert reject.
smoke-cert-follow:
	@./scripts/case-cert-follow.sh
# Lean-follower serving + L1 trust root: MockRollup checkpoint align +
# follower→follower cascade + bogus-checkpoint fail-closed reject.
smoke-cert-cascade:
	@./scripts/case-cert-cascade.sh
# Tx WRITE PATH across the sentry cascade: a tx submitted to L3 (reaches ONLY L2)
# is relayed via reth devp2p tx-gossip to a hidden validator's proposer, mined,
# finalized, and synced back to L3. Privacy invariant + fail-loud monitor checked.
smoke-tx-cascade:
	@./scripts/case-tx-cascade.sh
# Deferred-execution observables: K-lag invariant, result commitment, EL-slowed validator.
smoke-deferred:
	@./scripts/case-deferred.sh
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
	@./scripts/case-cert-catchup.sh
# Threshold randomness beacon: prev_randao = H(verified threshold seed) under DPoS.
smoke-vrf:
	@./scripts/case-vrf.sh
# Stake-weighted VRF leader election: skew validator-0's genesis stake 9x and assert
# it proposes proportionally more blocks (the heavy validator leads ~75% of views vs
# ~8.3% each for the three equal-stake peers), while the chain stays live.
smoke-weighted-vrf:
	@./scripts/case-weighted-vrf.sh
# Live DKG beacon ROTATION: a committee change forces a fresh DKG → a rotated
# on-chain group key; beacon relives from the new epoch's first block; a stable
# epoch carries the key forward (no on-chain commit). Runtime forge deploy +
# dynamic registration; same prerequisites as smoke-production-path. Long (~6-9 min).
smoke-vrf-rotation:
	@./scripts/case-vrf-rotation.sh
# Byzantine beacon: a committee STAYER forges a PK_E at the change-epoch boundary;
# the honest share-holders reject it at the "C" gate so it never finalizes, and an
# honest leader crosses the boundary with the real key (SAFETY + LIVENESS). Runs on
# the rotation stack (the forge only fires on a real committee change); requires the
# image built with the `dpos-devnet-byzantine` feature. Long (~8-10 min), needs
# foundry; NOT in run-all (like smoke-byzantine / smoke-production-path).
smoke-byzantine-vrf:
	@./scripts/case-byzantine-vrf.sh
# Full production path: runtime contract deploy + dynamic registration + governance
# activation + live committee rotation + liveness ejection. Long (~5-8 min),
# first-of-its-kind, and gated on the EIP-170 Staking-codesize fix; NOT in run-all.
smoke-production-path:
	@./scripts/case-production-path.sh
# Beacon under fault: f=1 validator down (beacon survives via n−f quorum) + the
# downed node restarts and catches up the gap with verified prev_randao (A1/B3/B4).
# Heavy (~4-5 min); NOT in run-all.
smoke-vrf-fault:
	@./scripts/case-vrf-fault.sh
# Beacon across an epoch boundary on a stable committee: prev_randao stays active
# + node-agreed across the boundary, on-chain key carries forward (F1/F2). Heavy
# (~3-4 min); NOT in run-all.
smoke-vrf-boundary:
	@./scripts/case-vrf-boundary.sh
# DKG-liveness negative edge (NO reshare): a member offline during its epoch-2 DKG
# window misses the ceremony, is shareless for that epoch, and SITS OUT the seed
# quorum while the chain stays live on the n−f survivors; it re-derives prev_randao
# from the cert seed on restart. Heavy (~4-5 min); NOT in run-all.
smoke-vrf-dkg-liveness:
	@./scripts/case-vrf-dkg-liveness.sh
# Durable mid-window DKG restart: a committee[2] member restarts AFTER its epoch-2 seal
# but BEFORE the boundary; it RESUMES its ceremony from the on-disk journal + pull,
# converges to (PK_2, share), and then PRODUCES in epoch 2 — which a shareless node
# cannot. Production is the load-bearing signal now that the liveness slash and its
# felony ladder are deleted; the old tuning existed only to make that slash reachable.
# Still runs a TUNED config (epochBlockInterval=64) for a generous DKG window.
# Heavy (~6-8 min); NOT in run-all.
smoke-vrf-dkg-restart-midwindow:
	@./scripts/case-vrf-dkg-restart-midwindow.sh
# Consolidated live-DKG mid-window-restart durability suite on ONE rotation-stack
# bring-up, each scenario on its own DKG ceremony, recoverable phases first: post-seal
# fleet recovery + consensus-quorum stall (G1, subsumes the single-victim midwindow
# baseline) on the epoch-2 ceremony -> torn-journal sit-out (G5) on the first driven
# rotation. Runtime forge deploy + governance-driven rotation; needs foundry. Long
# (~12-18 min); NOT in run-all (like smoke-vrf-rotation / smoke-byzantine-vrf /
# smoke-production-path).
smoke-vrf-dkg-durability:
	@./scripts/case-vrf-dkg-durability.sh
# >f PRE-SEAL TERMINAL halt (G2) — the negative control for smoke-vrf-dkg-durability's
# post-seal recovery (same 2-of-5 kill, opposite seal state -> permanent boundary
# freeze). Its OWN bring-up (plan §6.2 fallback: a second deterministic committee
# change is not cleanly drivable on the 6-key equal-stake stack), driving the halt on
# the first rotation C_r1. Kills 2 committee stayers PRE-seal + keeps them down ->
# (beacon_for_epoch)(E_new)=None -> proposer skips the boundary view forever -> the
# chain freezes permanently; asserts the sustained no-progress + the 'skipping propose'
# log, then tears down. Needs foundry. Long (~10-14 min); NOT in run-all.
smoke-vrf-dkg-halt:
	@./scripts/case-vrf-dkg-halt.sh
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
# These run the PYTHON simulation (`dpos_harness/sim`). The bash `scripts/case-soak.sh`
# they used to invoke is superseded and is deleted next; the knobs it read as SOAK_*
# are SIM_* here, and the blaster's LH_* are LOAD_*.
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

# --- the PORTED read-only smoke cases (dpos_harness, P7 chunk 1) --------------------
# Same four assertion bodies as scripts/asserts.sh, same static stack, same bring-up. These run
# ALONGSIDE the bash targets above rather than replacing them: the port is only proven once both
# have been run against the same stack and their verdicts compared, and that comparison needs
# both to stay runnable. `smoke-base-py` is the one to compare against `smoke-base`.
#
# Every target also accepts a dry run (`ARGS=--dry-run`), which walks the whole choreography with
# canned readings and prints the command transcript without touching docker.
smoke-base-py:
	@python3 -m dpos_harness case smoke-base $(ARGS)
smoke-tx-py:
	@python3 -m dpos_harness case smoke-tx $(ARGS)
smoke-epoch-py:
	@EPOCH_MIN_CROSS=1 python3 -m dpos_harness case smoke-epoch $(ARGS)
smoke-vrf-py:
	@python3 -m dpos_harness case smoke-vrf $(ARGS)
smoke-vrf-boundary-py:
	@python3 -m dpos_harness case smoke-vrf-boundary $(ARGS)
# The transcript of all five, docker-free — the fidelity oracle against the bash argv.
smoke-base-dry-run:
	@for c in smoke-tx smoke-epoch smoke-vrf smoke-vrf-boundary smoke-base; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# --- the PORTED DESTRUCTIVE smoke cases (dpos_harness, P7 chunk 2) ------------------
# Same six assertion bodies as scripts/asserts-fault.sh, same static stack, same bring-up. Like
# chunk 1 these run ALONGSIDE the bash targets above rather than replacing them — the port is
# proven only once both have run against the same stack and their verdicts have been compared.
# `smoke-fault-py` is the one to compare against `smoke-fault`.
#
# Every one of these BREAKS the stack and puts it back: the restore is part of the assertion, so
# a comparison run must check that each case left the chain healthy, not only that it printed OK.
smoke-fault-py:
	@python3 -m dpos_harness case smoke-fault $(ARGS)
smoke-deferred-py:
	@python3 -m dpos_harness case smoke-deferred $(ARGS)
smoke-peers-py:
	@python3 -m dpos_harness case smoke-peers $(ARGS)
smoke-crash-survivor-py:
	@python3 -m dpos_harness case smoke-crash-survivor $(ARGS)
smoke-full-restart-py:
	@python3 -m dpos_harness case smoke-full-restart $(ARGS)
smoke-vrf-fault-py:
	@python3 -m dpos_harness case smoke-vrf-fault $(ARGS)
smoke-vrf-dkg-liveness-py:
	@python3 -m dpos_harness case smoke-vrf-dkg-liveness $(ARGS)
# The transcript of all seven, docker-free — the fidelity oracle against the bash argv. For these
# the ORDER of the transcript matters as much as the argv: a restore issued before the reading it
# brackets is a different test that still passes.
smoke-fault-dry-run:
	@for c in smoke-deferred smoke-peers smoke-vrf-fault smoke-crash-survivor \
	          smoke-full-restart smoke-vrf-dkg-liveness smoke-fault; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# --- the PORTED FOLLOWER smoke cases (dpos_harness, P7 chunk 3) ---------------------
# Same three case bodies as scripts/case-cert-follow.sh, case-cert-cascade.sh and
# case-tx-cascade.sh, same static stack, same per-case compose overlay. Like chunks 1 and 2 these
# run ALONGSIDE the bash targets above rather than replacing them.
#
# Each runs ALONE — never chained. cert-follow leaves a man-in-the-middle poisoning validator-0's
# certificate plane, cert-cascade leaves two per-run contract addresses in the compose
# environment, and tx-cascade widens the sentry's trusted-peer set.
#
# Comparing these against their bash originals needs one thing the other two chunks did not:
# cert-follow's phase 3 SKIPS (exit 0) when the `cert-mitm` sidecar cannot pip-install
# `websockets`. A run on a host with no network therefore proves the two POSITIVE phases only —
# check the output for `SKIP (phase 3 tampered-reject)` before recording the comparison.
smoke-cert-follow-py:
	@python3 -m dpos_harness case smoke-cert-follow $(ARGS)
smoke-cert-cascade-py:
	@python3 -m dpos_harness case smoke-cert-cascade $(ARGS)
smoke-tx-cascade-py:
	@python3 -m dpos_harness case smoke-tx-cascade $(ARGS)
# The transcript of all three, docker-free. Here the `-f` overlay array and the exported
# ${MOCK_ROLLUP_ADDR}/${BOGUS_ROLLUP_ADDR} are part of the evidence: an export printed after the
# `up` that interpolates it is a follower pointed at the zero address.
smoke-follow-dry-run:
	@for c in smoke-cert-follow smoke-cert-cascade smoke-tx-cascade; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# --- the PORTED ON-CHAIN smoke cases (dpos_harness, P7 chunk 4) ---------------------
# Same four case bodies as scripts/case-liveness.sh, case-byzantine.sh, case-cert-catchup.sh and
# case-vrf-dkg-restart-midwindow.sh. Like chunks 1-3 these run ALONGSIDE the bash targets above
# rather than replacing them.
#
# Each runs ALONE and none of them restores what it broke, so never chain them: byzantine can
# leave a TOMBSTONED validator and a permanently shrunk committee, and the last two bring up a
# TUNED GENESIS (64-block epochs) which is a different chain, not a different phase of the same
# one.
#
# The tuning reaches genesis-init through docker-compose.yml's ${EPOCH_BLOCK_INTERVAL} /
# ${DPOS_ACTIVATION_BLOCK} interpolation, exactly as the bash exports do —
# so a `-py` run and its bash original build the same chain. cert-catchup keeps bash's
# default-from-env spelling: EPOCH_BLOCK_INTERVAL=<n> overrides the 64, and EPOCH_INTERVAL follows.
smoke-liveness-py:
	@python3 -m dpos_harness case smoke-liveness $(ARGS)
smoke-byzantine-py:
	@python3 -m dpos_harness case smoke-byzantine $(ARGS)
smoke-cert-catchup-py:
	@python3 -m dpos_harness case smoke-cert-catchup $(ARGS)
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
smoke-vrf-dkg-restart-midwindow-py:
	@python3 -m dpos_harness case smoke-vrf-dkg-restart-midwindow $(ARGS)
# The transcript of all four, docker-free. Here the `export` lines are part of the evidence: they
# must precede the `docker compose up --build -d` that genesis-init runs under, and the header's
# `interval=` / `activation=` must show the TUNED values — a tuned interval that reached compose
# but not the profile would give the case a 64-block stack and 32-block epoch arithmetic.
smoke-onchain-dry-run:
	@for c in smoke-liveness smoke-byzantine smoke-cert-catchup \
	          smoke-vrf-dkg-restart-midwindow; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# --- the PORTED PRODUCTION-PATH smoke cases (dpos_harness, P7 chunk 5b) -------------
# Same five case bodies as scripts/case-production-path.sh, case-vrf-rotation.sh,
# case-vrf-dkg-halt.sh, case-vrf-dkg-durability.sh and case-byzantine-vrf.sh. Like chunks 1-4 these
# run ALONGSIDE the bash targets above rather than replacing them.
#
# These five are the ONLY targets here that run on the PRODUCTION-PATH devnet: six validators, a
# BARE genesis, and a staking cluster deployed at RUNTIME by `forge script DeployStaking` whose
# addresses are DISCOVERED from the deploy manifest. So they need foundry (forge/cast) on the host
# and a solidity-contracts checkout at $$SOLIDITY_CONTRACTS_DIR, they take 6-18 minutes each, and
# none of them is in run-all.
#
# Each runs ALONE and never chained: every one leaves a rotated (and for production-path, a
# permanently shrunk + jailed) committee behind, smoke-vrf-dkg-halt leaves the chain DELIBERATELY
# and PERMANENTLY halted, and smoke-byzantine-vrf needs the image built with the
# `dpos-devnet-byzantine` cargo feature.
smoke-production-path-py:
	@python3 -m dpos_harness case smoke-production-path $(ARGS)
smoke-vrf-rotation-py:
	@python3 -m dpos_harness case smoke-vrf-rotation $(ARGS)
smoke-vrf-dkg-halt-py:
	@python3 -m dpos_harness case smoke-vrf-dkg-halt $(ARGS)
smoke-vrf-dkg-durability-py:
	@python3 -m dpos_harness case smoke-vrf-dkg-durability $(ARGS)
smoke-byzantine-vrf-py:
	@python3 -m dpos_harness case smoke-byzantine-vrf $(ARGS)
# The transcript of all five, docker-free — the fidelity oracle against the bash argv. Here the
# 14-phase bring-up dominates the diff, so read it in two halves: the phases up to `DPoS chain live
# past anchor` must match lib.sh's pp_bring_up_rotation, and everything after must match the case
# script. For smoke-byzantine-vrf the header's `overlays=` must name docker-compose.byzantine-vrf.yml
# and the cold-restart `up` must carry all THREE compose files.
smoke-prod-dry-run:
	@for c in smoke-vrf-rotation smoke-vrf-dkg-halt smoke-vrf-dkg-durability \
	          smoke-byzantine-vrf smoke-production-path; do \
	  echo "===== $$c ====="; python3 -m dpos_harness case $$c --dry-run; done

# Aggregate (CI entry point).
smoke-all:
	@./scripts/run-all.sh
