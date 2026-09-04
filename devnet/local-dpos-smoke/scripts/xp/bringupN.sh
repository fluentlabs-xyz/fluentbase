#!/usr/bin/env bash
# Bring stand xp<N> up through the sequencer -> DPoS migration. $1 = N.
set -euo pipefail
N=$1
XP="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$XP/out"
INTERVAL="${EPOCH_BLOCK_INTERVAL:-32}"
ACT="${DPOS_ACTIVATION_BLOCK:-$((INTERVAL*2))}"
RPC="http://localhost:$((28000+N))"
P=xp$N
C1="-f $XP/out/xp$N.yml"
C2="-f $XP/out/xp$N.yml -f $XP/out/xp$N.dpos.yml"

test -f "$XP/out/xp$N.yml" || { echo "no stand for N=$N — run: python3 $XP/genN.py $N"; exit 2; }

rpc() { curl -s -m 5 -X POST -H 'Content-Type: application/json' \
  --data "{\"jsonrpc\":\"2.0\",\"method\":\"$2\",\"params\":${3:-[]},\"id\":1}" "$1"; }
hexdec() { python3 -c "import sys;v=sys.stdin.read().strip();print(int(v,16) if v.startswith('0x') else -1)"; }
fin() { rpc "$1" eth_getBlockByNumber '["finalized",false]' | jq -r '.result.number // "0x0"' | hexdec; }

echo "== phase 1 (N=$N interval=$INTERVAL activation=$ACT) =="
docker compose -p $P $C1 up -d
deadline=$(( $(date +%s) + 500 ))
while :; do
  f=$(fin "$RPC" || echo -1)
  if [ "${f:-0}" -ge "$ACT" ] 2>/dev/null; then break; fi
  if [ "$(date +%s)" -gt "$deadline" ]; then echo "TIMEOUT activation, finalized=$f"; exit 3; fi
  sleep 2
done
echo "sequencer finalized=$f"

echo "== flush =="
VS=""; for i in $(seq 0 $((N-1))); do VS="$VS validator-$i"; done
attempt=1
while :; do
  # shellcheck disable=SC2086
  docker compose -p $P $C1 stop --timeout 90 $VS
  bad=0
  for i in $(seq 0 $((N-1))); do
    code=$(docker inspect -f '{{.State.ExitCode}}' "$P-validator-$i-1")
    echo "attempt $attempt validator-$i exit=$code"
    [ "$code" = "0" ] || bad=1
  done
  if [ "$bad" = 0 ]; then break; fi
  if [ "$attempt" -ge 3 ]; then echo "flush gate failed"; exit 3; fi
  attempt=$((attempt+1))
  # shellcheck disable=SC2086
  docker compose -p $P $C1 start $VS; sleep 25
done
ANCHOR=$f

echo "== phase 2 (--dpos) =="
# shellcheck disable=SC2086
docker compose -p $P $C2 up -d --force-recreate $VS
deadline=$(( $(date +%s) + 400 ))
while :; do
  ok=1
  n=$(fin "$RPC" || echo -1)
  if [ "${n:-0}" -gt "$ANCHOR" ] 2>/dev/null; then :; else ok=0; fi
  if [ "$ok" = 1 ]; then break; fi
  if [ "$(date +%s)" -gt "$deadline" ]; then echo "TIMEOUT dpos converge"; exit 3; fi
  sleep 3
done
echo "DPoS live past anchor=$ANCHOR (finalized=$n)"
echo "$ANCHOR" > "$XP/out/anchor-xp$N.txt"
