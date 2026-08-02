#!/usr/bin/env bash
# smoke-weighted-vrf (standalone): verify STAKE-WEIGHTED VRF leader election end-to-end.
#
# validator-0's genesis stake is skewed HEAVY_STAKE_MULT× (default 9) the others, so on
# the 4-validator stack the weighted elector should make it lead ~75% of views vs ~8.3%
# each for the three equal peers (shares 9:1:1:1). The larger skew gives a far stronger
# signal-to-noise (heavy/light ≈ 9×), so a HALVED window keeps higher confidence. We tally
# per-validator
# "dpos: proposing order block" logs — FluentApp::propose is leader-gated, so it fires
# exactly once per block a node proposes — over a clean POST-activation window, and assert
# validator-0 proposes a clear weighted plurality while the chain stays live (finalization
# advancing ⇒ quorum agreement). With the default HEAVY_STAKE_MULT=1 this is byte-identical
# to smoke-vrf (equal stakes ⇒ uniform), so the skew is the whole point of the case.
set -euo pipefail
cd "$(dirname "$0")/.."
export HEAVY_STAKE_MULT="${HEAVY_STAKE_MULT:-9}"
# shellcheck source=lib.sh
source "$(dirname "$0")/lib.sh"

PROPOSE_LOG='dpos: proposing order block'
WINDOW="${WEIGHTED_VRF_WINDOW:-100}"

bring_up_dpos
trap '[ -n "${SMOKE_KEEP_UP:-}" ] || tear_down' EXIT

# Settle clearly past the DPoS activation block (default 64 = 2×interval) so the tally
# window is pure weighted-VRF — pre-activation blocks are not produced by this elector.
wait_finalized_ge 90 240 \
    || { echo "FAIL: chain did not reach finalized height 90 (post-activation) in time"; exit 1; }

# START counts (cumulative docker-log greps), then run the measurement window.
declare -a S E D
for i in 0 1 2 3; do S[$i]=$(log_count "validator-$i" "$PROPOSE_LOG"); done
win_start=$(finalized_dec)
echo "  [weighted-vrf] window start finalized=$win_start; HEAVY_STAKE_MULT=$HEAVY_STAKE_MULT; waiting +$WINDOW blocks"
wait_finalized_ge $(( win_start + WINDOW )) $(( WINDOW * 2 + 120 )) \
    || { echo "FAIL: chain did not finalize +$WINDOW blocks (LIVENESS) within window"; exit 1; }
for i in 0 1 2 3; do E[$i]=$(log_count "validator-$i" "$PROPOSE_LOG"); done

# Per-validator proposals produced during the window.
total=0
for i in 0 1 2 3; do
    D[$i]=$(( E[$i] - S[$i] ))
    total=$(( total + D[$i] ))
    echo "  validator-$i proposed ${D[$i]} blocks in window"
done
(( total > 0 )) || { echo "FAIL: no proposals logged in window — proposer hook missing?"; exit 1; }

# Heavy = validator-0 (HEAVY_STAKE_MULT× stake); light_max = busiest of the equal peers.
heavy=${D[0]}
light_max=${D[1]}
for i in 2 3; do
    if (( D[$i] > light_max )); then light_max=${D[$i]}; fi
done

# Expected (mult=9): heavy 9/12=75%, each light 1/12≈8.3% ⇒ heavy ≈ 9× a light. Assert heavy
# is the strict plurality with a safe ≥1.5× margin (heavy*2 ≥ light_max*3, integer-exact) so
# binomial variance over the window cannot flip it. Fail loud otherwise.
if (( heavy > light_max )) && (( heavy * 2 >= light_max * 3 )); then
    echo "PASS: weighted leader election — validator-0 (${HEAVY_STAKE_MULT}× stake) proposed $heavy vs light-max $light_max (≥1.5×) over $total proposals"
else
    echo "FAIL: validator-0 did not propose a weighted plurality (heavy=$heavy, light_max=$light_max, total=$total) — weighting not effective"
    exit 1
fi
