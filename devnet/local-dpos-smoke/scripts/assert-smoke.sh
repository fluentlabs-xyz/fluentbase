#!/usr/bin/env bash
# Pipeline 2 acceptance: two-phase smoke (mirrors prod sequencer→DPoS migration).
#
# phase1 — bring up validator-0 as Tempo sequencer + 3 followers + full-node;
#          all 5 align finalized > 0 within 60s. Chain stays UP on success
#          (so `phase2` can take over via compose override).
# phase2 — cold-restart validators 0-3 with --dpos via docker-compose.dpos.yml
#          override; all 5 align finalized > tempo_last within 60s; tear down.
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
    # Flush-gated Tempo→DPoS migration (sets PREV_FIN = anchor height).
    _migrate_to_dpos
    ;;
  *) echo "unknown phase: $PHASE"; exit 2 ;;
esac

# phase2 (DPoS cold-start at a high migration anchor) needs a longer window than
# phase1 (Tempo converges in seconds): the relative-epoch-0 anchor is ~block 64,
# and post-swap convergence + finalized-pointer advance takes longer.
if [[ "$PHASE" == "phase2" ]]; then WINDOW=120; else WINDOW=60; fi
DEADLINE=$(( $(date +%s) + WINDOW ))
v0="null|null"; v1="null|null"; v2="null|null"; v3="null|null"; fn="null|null"
while [[ $(date +%s) -lt $DEADLINE ]]; do
    v0=$(check_external 8545)
    v1=$(check_node docker compose exec -T validator-1)
    v2=$(check_node docker compose exec -T validator-2)
    v3=$(check_node docker compose exec -T validator-3)
    fn=$(check_external 18545)

    head_num="${v0%%|*}"
    if [[ "$head_num" != "null" && "$head_num" != "0x0" ]] \
       && [[ "$v0" == "$v1" && "$v1" == "$v2" && "$v2" == "$v3" && "$v3" == "$fn" ]]; then
        # phase2 guard: PREV_FIN may equal head_num if the cold-restart raced ahead
        # of the finalized-pointer advance; keep polling until it visibly advances.
        if [[ "$PHASE" == "phase2" ]] && [[ "$head_num" == "$PREV_FIN" ]]; then
            sleep 2; continue
        fi
        echo "OK ($PHASE): all 5 nodes at $v0"
        [[ "$PHASE" == "phase2" ]] && tear_down
        exit 0
    fi
    sleep 2
done

echo "FAIL ($PHASE): nodes did not converge within ${WINDOW}s"
echo "  validator-0: $v0"
echo "  validator-1: $v1"
echo "  validator-2: $v2"
echo "  validator-3: $v3"
echo "  full-node:   $fn"
[[ "$PHASE" == "phase2" ]] && echo "  PREV_FIN (Tempo last): $PREV_FIN"
docker compose logs --tail=200
tear_down
exit 1
