SOLIDITY_CONTRACTS_DIR ?= ../../../solidity-contracts

.PHONY: regen-contracts smoke smoke-swap up down logs clean \
        smoke-tx smoke-epoch smoke-peers smoke-liveness smoke-crash-survivor smoke-byzantine \
        smoke-gov-interval smoke-full-restart smoke-cert-follow smoke-deferred \
        smoke-vrf smoke-production-path smoke-all

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
	git -C $(SOLIDITY_CONTRACTS_DIR) rev-parse HEAD > contracts/.vendor-sha
	@echo "Vendored contracts updated; SHA recorded in contracts/.vendor-sha; remember to commit."

up:
	docker compose up --build

down:
	docker compose down -v

logs:
	docker compose logs -f

# Two-phase Tempo→DPoS migration smoke. ORDER MATTERS and the phases are NOT
# self-contained: `smoke` (phase1) brings the chain up as Tempo and LEAVES IT
# RUNNING; `smoke-swap` (phase2) takes over that running chain and migrates it to
# DPoS. Running `make smoke-swap` ALONE polls a chain that was never started and
# times out after 180s ("Tempo did not reach dposActivationBlock=64"). Always run
# them together:  make smoke && make smoke-swap
# (The `smoke-<case>` targets below are different — each is self-contained.)
smoke:
	@./scripts/assert-smoke.sh phase1

smoke-swap:
	@./scripts/assert-smoke.sh phase2

clean:
	docker compose down -v --remove-orphans

# --- regression cases (each self-contained: brings up + tears down its own stack) ---
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
# Full production path: runtime contract deploy + dynamic registration + governance
# activation + live committee rotation + liveness ejection. Long (~5-8 min),
# first-of-its-kind, and gated on the EIP-170 Staking-codesize fix; NOT in run-all.
smoke-production-path:
	@./scripts/case-production-path.sh
# Aggregate (CI entry point).
smoke-all:
	@./scripts/run-all.sh
