#!/usr/bin/env bash
# head/finalized/hash of every node of stand xp<N>. $1 = N.
set -uo pipefail
N=$1; SUB="172.$((20+N)).0"
q() { docker exec "xp$N-validator-0-1" sh -c \
  "curl -s -m 5 -X POST -H 'Content-Type: application/json' --data '{\"jsonrpc\":\"2.0\",\"method\":\"$2\",\"params\":$3,\"id\":1}' http://$1" 2>/dev/null; }
for i in $(seq 0 $((N-1))); do
  ip="$SUB.$((10+i))"
  fin=$(q "$ip:8545" eth_getBlockByNumber '["finalized",false]')
  head=$(q "$ip:8545" eth_blockNumber '[]' | jq -r '.result // "?"')
  st=$(docker inspect -f '{{.State.Status}}({{.State.ExitCode}})' "xp$N-validator-$i-1" 2>/dev/null)
  printf 'validator-%-2s %-14s head=%-8s finalized=%-8s hash=%s\n' "$i" "$st" "$head" \
    "$(echo "$fin" | jq -r '.result.number // "?"')" "$(echo "$fin" | jq -r '.result.hash // "?"' | cut -c1-18)"
done
