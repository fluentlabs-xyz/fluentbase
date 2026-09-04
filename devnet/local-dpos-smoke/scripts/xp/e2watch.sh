#!/usr/bin/env bash
# Watch the stand after E2: is the chain alive, and if not, at which block and
# with which error.
set -uo pipefail
N=${1:-4}
for k in $(seq 1 40); do
  echo "--- t+$((k*6))s ---"
  "$(dirname "${BASH_SOURCE[0]}")/statusN.sh" "$N"
  alive=$(docker ps --format '{{.Names}}' | grep -c "xp$N-validator" || true)
  if [ "$alive" -le 1 ]; then echo "network down (alive=$alive)"; break; fi
  sleep 6
done
echo "=== fatal lines ==="
for i in $(seq 0 $((N-1))); do
  echo -n "validator-$i: "
  docker logs "xp$N-validator-$i-1" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' \
    | grep -c 'executor fatal error' || true
  docker logs "xp$N-validator-$i-1" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' \
    | grep 'did not succeed' | tail -1 | cut -c1-260
done
echo "=== jail/tombstone lines (should be NONE for a voluntary exit) ==="
docker logs "xp$N-validator-0-1" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' \
  | grep -ciE 'tombston|equivocat|ValidatorJailed' || true
