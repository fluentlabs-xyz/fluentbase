SOLIDITY_CONTRACTS_DIR ?= ../../../solidity-contracts

.PHONY: regen-contracts smoke smoke-swap up down logs clean \
        smoke-base smoke-fault smoke-tx smoke-epoch smoke-peers smoke-liveness smoke-crash-survivor smoke-byzantine \
        smoke-gov-interval smoke-full-restart smoke-cert-follow smoke-deferred \
        smoke-vrf smoke-weighted-vrf smoke-vrf-rotation smoke-byzantine-vrf smoke-vrf-fault smoke-vrf-boundary \
        smoke-vrf-dkg-liveness smoke-vrf-dkg-restart-midwindow \
        smoke-vrf-dkg-durability smoke-vrf-dkg-halt \
        smoke-production-path smoke-all

regen-contracts:
	@test -d $(SOLIDITY_CONTRACTS_DIR) || \
	  (echo "ERROR: solidity-contracts not found at $(SOLIDITY_CONTRACTS_DIR); set SOLIDITY_CONTRACTS_DIR" && exit 1)
	cd $(SOLIDITY_CONTRACTS_DIR) && forge build
	cp $(SOLIDITY_CONTRACTS_DIR)/out/Staking.sol/Staking.json                       contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/StakingDpos.sol/StakingDpos.json               contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/StakingEconomics.sol/StakingEconomics.json     contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/ChainConfig.sol/ChainConfig.json               contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/StakingPool.sol/StakingPool.json               contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/SystemReward.sol/SystemReward.json             contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/FluentGovernance.sol/FluentGovernance.json     contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/LivenessSlashing.sol/LivenessSlashing.json     contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/MockBlendToken.sol/MockBlendToken.json         contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/BLS12381Verifier.sol/BLS12381Verifier.json     contracts/
	cp $(SOLIDITY_CONTRACTS_DIR)/out/PrevRandaoProbe.sol/PrevRandaoProbe.json        contracts/
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
# Additional NOW-tier cases (Phase 6).
smoke-gov-interval:
	@./scripts/case-gov-interval.sh
smoke-full-restart:
	@./scripts/case-full-restart.sh
# Trustless cert-follower: subscribe-align + gap back-fill + tampered-cert reject.
smoke-cert-follow:
	@./scripts/case-cert-follow.sh
# Lean-follower serving + L1 trust root: MockRollup checkpoint align +
# follower→follower cascade + bogus-checkpoint fail-closed reject.
smoke-cert-cascade:
	@./scripts/case-cert-cascade.sh
# Deferred-execution observables: K-lag invariant, result commitment, EL-slowed validator.
smoke-deferred:
	@./scripts/case-deferred.sh
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
# converges to (PK_2, share), and AVOIDS the liveness slash a shareless node would
# suffer. Runs a TUNED config (epochBlockInterval=64 > missThreshold=50, felonyThreshold=1)
# so the slash is actually reachable — the default config (epoch 32 < missThreshold 50)
# cannot fire it (the smoke-vrf-dkg-liveness blind spot). Heavy (~6-8 min); NOT in run-all.
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
# Aggregate (CI entry point).
smoke-all:
	@./scripts/run-all.sh
