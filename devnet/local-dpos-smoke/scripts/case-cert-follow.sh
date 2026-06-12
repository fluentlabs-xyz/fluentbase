#!/usr/bin/env bash
# smoke-cert-follow: a trustless `--cert-follow` node pulls finality certs from
# validator-0's `consensus` RPC, verifies each against the on-chain committee,
# and drives its own reth. Asserts three properties:
#   (1) subscribe-align  — the follower catches up + finalized-aligns with v0;
#   (2) gap back-fill     — stopped past >0 blocks then restarted, it catches up
#                           via `getFinalization` (persistent resume);
#   (3) tampered reject   — fed byte-flipped certs through a WS MITM, it makes
#                           ZERO finalized progress (verification is load-bearing).
#                           Cold-start succeeds (the anchor is trust-blind, authenticated
#                           transitively); the driver then rejects every tampered LIVE
#                           cert, so finalized never advances past the cold-start anchor.
# (3) needs the `cert-mitm` python sidecar; if it cannot start (offline pip), the
# negative phase is SKIPPED LOUDLY — the positive phases still gate the result.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

CF_COMPOSE=(-f docker-compose.yml -f docker-compose.dpos.yml -f docker-compose.cert-follow.yml)
CF_PORT=28545      # host → cert-follower reth RPC
TAMPER_PORT=38545  # host → cert-follower-tamper reth RPC

bring_up_dpos
trap tear_down EXIT

anchor=$(finalized_dec)
echo "smoke-cert-follow: DPoS converged; anchor finalized=$anchor"

# ── Phase 1: subscribe-align ────────────────────────────────────────────────
echo "smoke-cert-follow: starting cert-follower (ws://172.20.0.10:8546)"
docker compose "${CF_COMPOSE[@]}" up -d cert-follower

align_target=$(( anchor + EPOCH_INTERVAL ))   # at least one epoch boundary crossed
aligned=$(wait_follower_align "$CF_PORT" "$align_target" 180) || {
    echo "FAIL (smoke-cert-follow): cert-follower did not align with v0 past $align_target"
    docker compose "${CF_COMPOSE[@]}" logs --tail=200 cert-follower
    exit 1
}
echo "OK (phase 1 subscribe-align): cert-follower aligned with v0 at $aligned"

# ── Phase 2: gap back-fill (stop past >0, restart, catch up) ─────────────────
f1=$(printf '%d' "$(check_external "$CF_PORT" | cut -d'|' -f1)")
echo "smoke-cert-follow: stopping cert-follower at finalized=$f1"
docker compose "${CF_COMPOSE[@]}" stop --timeout 40 cert-follower
shutdown_flushed cert-follower || echo "  (warning) cert-follower did not exit cleanly (code 0); continuing"

# Let v0 advance well past f1 so the restart must back-fill a real gap.
gap_target=$(( f1 + EPOCH_INTERVAL ))
echo "  waiting for v0 to advance past $gap_target before restart"
wait_finalized_ge $(( gap_target + 1 )) 120 \
    || { echo "FAIL (smoke-cert-follow): v0 did not advance past $gap_target"; exit 1; }
f2=$(finalized_dec)

echo "  restarting cert-follower; must back-fill [$f1+1 .. $f2] via getFinalization"
docker compose "${CF_COMPOSE[@]}" start cert-follower
caught=$(wait_follower_align "$CF_PORT" "$f2" 180) || {
    echo "FAIL (smoke-cert-follow): cert-follower did not back-fill the gap to >= $f2"
    docker compose "${CF_COMPOSE[@]}" logs --tail=200 cert-follower
    exit 1
}
echo "OK (phase 2 gap back-fill): cert-follower caught up to $caught (>= f2=$f2)"

# ── Phase 3: tampered-cert rejection (negative) ─────────────────────────────
echo "smoke-cert-follow: starting cert-mitm + cert-follower-tamper (negative)"
docker compose "${CF_COMPOSE[@]}" up -d cert-mitm 2>/dev/null || true
# Wait for the MITM proxy to come up (pip install may need network).
mitm_up=""
deadline=$(( $(date +%s) + 90 ))
while (( $(date +%s) < deadline )); do
    if docker compose "${CF_COMPOSE[@]}" logs cert-mitm 2>/dev/null | grep -q 'cert-mitm: listening'; then
        mitm_up=1
        break
    fi
    sleep 3
done
if [[ -z "$mitm_up" ]]; then
    echo "SKIP (phase 3 tampered-reject): cert-mitm proxy did not start (offline pip / no python) \
— positive phases passed; run where the sidecar can install 'websockets' to exercise the negative."
    echo "OK (smoke-cert-follow): subscribe-align + gap back-fill verified"
    exit 0
fi

v0_before=$(finalized_dec)
docker compose "${CF_COMPOSE[@]}" up -d cert-follower-tamper
echo "  tamper-follower up; observing for 45s while v0 advances"
sleep 45
v0_after=$(finalized_dec)
tamper_h=$(check_external "$TAMPER_PORT" | cut -d'|' -f1)

(( v0_after > v0_before )) || { echo "FAIL (smoke-cert-follow): v0 stalled during tamper phase ($v0_before→$v0_after); cannot attribute follower stall to rejection"; exit 1; }

# A follower fed only byte-flipped certs must NOT finalize anything: reth's
# "finalized" stays unset (null) or genesis (0x0).
if [[ "$tamper_h" == "null" || "$tamper_h" == "0x0" ]]; then
    # Non-fatal observability: confirm the driver's live-tail verify is what rejected.
    # The finalized-progress assertion above is the load-bearing gate; this hint is
    # best-effort (log capture is not guaranteed).
    if docker compose "${CF_COMPOSE[@]}" logs --tail=400 cert-follower-tamper 2>/dev/null \
        | grep -qiE 'finalization cert FAILED BLS verification|dropping mismatched cert'; then
        echo "  (hint) rejection fired at the driver live-tail verify"
    fi
    echo "OK (phase 3 tampered-reject): tamper-follower made ZERO finalized progress (finalized=$tamper_h) while v0 advanced $v0_before→$v0_after"
else
    echo "FAIL (smoke-cert-follow): tamper-follower advanced finalized to $tamper_h despite byte-flipped certs — verification is NOT load-bearing!"
    docker compose "${CF_COMPOSE[@]}" logs --tail=200 cert-follower-tamper cert-mitm
    exit 1
fi

echo "OK (smoke-cert-follow): subscribe-align + gap back-fill + tampered-cert rejection all verified"
exit 0
