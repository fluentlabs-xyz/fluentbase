#!/usr/bin/env bash
# smoke-all: run every regression case in sequence (CI entry point). Each case is
# self-contained (brings up + tears down its own stack). Non-zero exit if any case
# fails; prints a summary table. Set SMOKE_CASES to override the list.
set -uo pipefail
cd "$(dirname "$0")/.."

# Default suite = the cases green today. `case-base` bundles the read-only
# default-stack cases (tx + epoch + vrf + vrf-boundary) onto ONE bring-up — it
# replaces the former separate case-tx/case-epoch/case-vrf entries and adds
# vrf-boundary coverage. Excluded (run individually via SMOKE_CASES="..." or
# `make smoke-<case>`):
#   case-byzantine     — pending Phase 5 (equivocation Conflicter Rust side absent)
#   case-byzantine-vrf — long (~8-10 min), runs the rotation stack, needs foundry +
#                        the `dpos-devnet-byzantine` image feature
#   case-gov-interval  — descoped: a ChainConfig setter change needs a full
#                        FluentGovernance propose/vote/execute flow (onlyFromGovernance)
# `case-fault` bundles the recoverable DESTRUCTIVE default-stack cases
# (deferred + peers + vrf-fault + crash-survivor + full-restart) onto ONE bring-up,
# so it replaces those four separate entries here (and adds vrf-fault coverage).
CASES=(${SMOKE_CASES:-case-base case-fault case-liveness case-cert-follow case-cert-cascade case-tx-cascade})

declare -A RESULT
fail=0
for c in "${CASES[@]}"; do
    echo "==================== $c ===================="
    if ./scripts/"$c".sh; then
        RESULT[$c]="PASS"
    else
        RESULT[$c]="FAIL"
        fail=1
    fi
    # defensive teardown between cases (each case also traps tear_down, but ensure a
    # crashed case leaves nothing behind for the next one).
    docker compose down -v --remove-orphans >/dev/null 2>&1 || true
done

echo "==================== summary ===================="
for c in "${CASES[@]}"; do printf "  %-22s %s\n" "$c" "${RESULT[$c]:-MISSING}"; done
(( fail == 0 )) && echo "ALL SMOKE CASES PASSED" || echo "SOME SMOKE CASES FAILED"
exit "$fail"
