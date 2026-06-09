#!/usr/bin/env bash
# smoke-full-restart: stop ALL 4 validators (verify each persisted, exit 0), restart them, and
# assert the network reconverges from the persisted finalized head — i.e. DPoS
# cold-restart from disk works for the whole set, not just the migration anchor.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

bring_up_dpos
trap tear_down EXIT

pre=$(baseline_height)
echo "smoke-full-restart: stopping all 4 validators at finalized=$pre"
docker compose stop --timeout 40 "${VALS[@]}"
for v in "${VALS[@]}"; do
    shutdown_flushed "$v" || { echo "FAIL (smoke-full-restart): $v did not exit cleanly (code 0) on shutdown"; exit 1; }
done
echo "  all persisted (exit 0); restarting"
docker compose start "${VALS[@]}"

# reconverge: all 5 align finalized at >= pre (resume from persisted head).
deadline=$(( $(date +%s) + 120 ))
while (( $(date +%s) < deadline )); do
    v0=$(check_external 8545); v1=$(check_node docker compose exec -T validator-1)
    v2=$(check_node docker compose exec -T validator-2); v3=$(check_node docker compose exec -T validator-3)
    fn=$(check_external 18545); head="${v0%%|*}"
    if [[ "$head" != "null" && "$head" != "0x0" \
          && "$v0" == "$v1" && "$v1" == "$v2" && "$v2" == "$v3" && "$v3" == "$fn" ]] \
       && (( $(printf '%d' "$head") >= pre )); then
        echo "OK (smoke-full-restart): all 5 reconverged at $v0 (>= pre=$pre) after full stop/start"
        exit 0
    fi
    sleep 2
done
echo "FAIL (smoke-full-restart): network did not reconverge after full restart"
docker compose logs --tail=200
exit 1
