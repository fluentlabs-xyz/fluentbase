#!/usr/bin/env bash
# Pipeline 2 acceptance: two-phase smoke (mirrors prod sequencer→DPoS migration).
#
# phase1 — bring up validator-0 as the sequencer + 3 followers + full-node;
#          all 5 align finalized > 0 within 60s. Chain stays UP on success
#          (so `phase2` can take over via compose override).
# phase2 — cold-restart validators 0-3 with --dpos via docker-compose.dpos.yml
#          override; all 5 align finalized > sequencer_last within 60s; tear down.
#
# Shared RPC/convergence/migration helpers live in scripts/lib.sh.
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

PHASE="${1:?usage: $0 phase1|phase2}"

case "$PHASE" in
  phase1)
    docker compose up --build -d
    ;;
  phase2)
    # phase2 takes over the chain phase1 left running; it does NOT bring one up.
    # Fail fast with a clear message instead of polling a dead chain for 180s when
    # phase2 is run standalone (a common foot-gun — use `make smoke && make smoke-swap`).
    if [[ -z "$(docker compose ps -q 2>/dev/null)" ]]; then
      echo "FAIL (phase2): no chain is running — phase2 migrates the chain that"
      echo "  phase1 leaves up. Run phase1 first:  make smoke && make smoke-swap"
      exit 2
    fi
    # Flush-gated sequencer→DPoS migration (sets PREV_FIN = anchor height).
    _migrate_to_dpos
    ;;
  *) echo "unknown phase: $PHASE"; exit 2 ;;
esac

# phase2 (DPoS cold-start at a high migration anchor) needs a longer window than
# phase1 (the sequencer converges in seconds): the relative-epoch-0 anchor is ~block 64,
# and post-swap convergence + finalized-pointer advance takes longer.
if [[ "$PHASE" == "phase2" ]]; then WINDOW=120; else WINDOW=60; fi

# The reading set is EXACTLY `_read_sequencer_nodes`, so this reuses `_wait_aligned`
# rather than carrying its own copy of the cross-node compare. It predates
# `_wait_aligned` and hand-rolled the same defect the sweep is closing: five separate,
# non-atomic RPC round-trips were required to produce ONE byte-identical "height|hash",
# i.e. that five independently-advancing tips coincide, on a chain producing a block a
# second. That passes by luck; a node steadily one block behind fails every poll.
# `_wait_aligned` keeps the fork check at each node's OWN height and keeps the
# "null"/"0x0" rejection that stops an all-down or genesis-only poll from reading as
# convergence.
#
# The phase2 anchor guard becomes the FLOOR: `_wait_aligned` requires every head
# NUMERICALLY GREATER than it, which is strictly stronger than the old
# `head_num == PREV_FIN → keep polling` (that compared hex strings and only looked at
# validator-0). phase1 has no anchor, so it passes "" and gets the plain `> 0` genesis
# guard. PREV_FIN only exists after `_migrate_to_dpos`, hence the branch rather than a
# default expansion.
if [[ "$PHASE" == "phase2" ]]; then FLOOR="$PREV_FIN"; else FLOOR=""; fi

if ALIGNED=$(_wait_aligned "$WINDOW" "$FLOOR" _read_sequencer_nodes); then
    echo "OK ($PHASE): all 5 nodes at $ALIGNED"
    [[ "$PHASE" == "phase2" ]] && tear_down
    exit 0
fi

echo "FAIL ($PHASE): nodes did not converge within ${WINDOW}s"
mapfile -t LAST < <(_read_sequencer_nodes)
LABELS=(validator-0 validator-1 validator-2 validator-3 "full-node  ")
for i in "${!LABELS[@]}"; do echo "  ${LABELS[$i]}: ${LAST[$i]:-<no reading>}"; done
[[ "$PHASE" == "phase2" ]] && echo "  PREV_FIN (sequencer last): $PREV_FIN"
docker compose logs --tail=200
tear_down
exit 1
