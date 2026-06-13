#!/usr/bin/env bash
# smoke-all: run every regression case in sequence (CI entry point). Each case is
# self-contained (brings up + tears down its own stack). Non-zero exit if any case
# fails; prints a summary table. Set SMOKE_CASES to override the list.
set -uo pipefail
cd "$(dirname "$0")/.."

# Default suite = the cases green today. Excluded (run individually via
# SMOKE_CASES="..." or `make smoke-<case>`):
#   case-byzantine    — pending Phase 5 (devnet-byzantine Rust feature + Conflicter)
#   case-gov-interval — descoped: a ChainConfig setter change needs a full
#                       FluentGovernance propose/vote/execute flow (onlyFromGovernance)
CASES=(${SMOKE_CASES:-case-tx case-epoch case-peers case-liveness case-crash-survivor case-full-restart case-deferred case-cert-follow case-cert-cascade})

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
