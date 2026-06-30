#!/usr/bin/env bash
# Pure unit test for gate_accept (plan §8.4) — NO live chain, NO docker. Feeds
# synthetic (committee, incoming, DISRUPTED, PENDING, phase) and asserts verdicts.
# This is the soak's safety proof: the gate logic is isolated and provable in
# isolation. Run: ./scripts/soak-gate-test.sh   (also wired as `make smoke-soak-gate`).
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=/dev/null
source scripts/soak-actions.sh

PASS=0 FAIL=0
# Reset all GA_* inputs to a benign baseline before each case.
_reset() {
    GA_ACTION="graceful_stop_restart" GA_VICTIM="validator-1" GA_N=7
    GA_DISRUPTED="" GA_CHANGE_IMMINENT=0 GA_INCOMING="" GA_SHARELESS=0
    GA_MIN_COMMITTEE=4 GA_REFILL=0 GA_PENDING_MIN=7 GA_LEADER="validator-9" GA_ROUND_OTHERS=0
    GA_MEMBERSHIP_SETTLING=0
}
# expect <accept|reject> <case-name>
expect() {
    local want="$1" name="$2" got
    if gate_accept; then got=accept; else got=reject; fi
    if [[ "$got" == "$want" ]]; then
        PASS=$((PASS+1)); printf '  ok   %-48s %s\n' "$name" "${GA_REASON:+($GA_REASON)}"
    else
        FAIL=$((FAIL+1)); printf '  FAIL %-48s want=%s got=%s %s\n' "$name" "$want" "$got" "${GA_REASON:+($GA_REASON)}"
    fi
}

echo "== gate_accept unit test (§8.4) =="

# 1. f-concurrent at n=4 ACCEPTED (the liveness boundary: f=1, 0 disrupted → 1 == f)
_reset; GA_N=4; GA_DISRUPTED=""; expect accept "f-concurrent@n4 (1st fault, f=1)"

# 2. f+1-concurrent REJECTED (n=4, f=1, one already down → 2nd is f+1)
_reset; GA_N=4; GA_DISRUPTED="validator-2"; expect reject "f+1-concurrent@n4 (rule1)"

# 3. n=7 (f=2): 2 concurrent ACCEPTED, 3rd REJECTED
_reset; GA_N=7; GA_DISRUPTED="validator-2"; expect accept "2-concurrent@n7 (f=2)"
_reset; GA_N=7; GA_DISRUPTED="validator-2 validator-3"; expect reject "3-concurrent@n7 (rule1)"

# 4. >f-incoming-during-E-1 REJECTED (non-dkg action touching incoming in change window)
_reset; GA_CHANGE_IMMINENT=1; GA_INCOMING="validator-1 validator-2 validator-3"
GA_VICTIM="validator-1"; GA_ACTION="sigkill_restart"; expect reject ">f-incoming-E1 non-dkg (rule2)"

# 5. dkg_midwindow_restart ALLOWED in E-1 when incoming-disrupted stays <= f
_reset; GA_N=7; GA_CHANGE_IMMINENT=1; GA_INCOMING="validator-1 validator-2 validator-3 validator-4 validator-5 validator-6 validator-7"
GA_VICTIM="validator-1"; GA_ACTION="dkg_midwindow_restart"; GA_DISRUPTED=""; expect accept "dkg_midwindow in E-1 (<=f incoming)"
# ... but a SECOND incoming dkg restart that pushes incoming-disrupted past f is REJECTED (f=2 here, 2 already)
_reset; GA_N=7; GA_CHANGE_IMMINENT=1; GA_INCOMING="validator-1 validator-2 validator-3"
GA_VICTIM="validator-3"; GA_ACTION="dkg_midwindow_restart"; GA_DISRUPTED="validator-1 validator-2"; expect reject "dkg_midwindow >f incoming (rule2)"

# 6. Shareless cap (rule 3): already f shareless, a dkg restart would make f+1
_reset; GA_N=7; GA_ACTION="dkg_midwindow_restart"; GA_VICTIM="validator-1"; GA_SHARELESS=2; expect reject "shareless f+1 (rule3, f=2)"

# 7. jail below MIN_COMMITTEE REJECTED
_reset; GA_N=4; GA_ACTION="liveness_jail"; GA_VICTIM="validator-1"; GA_REFILL=1; expect reject "jail@n4 below MIN_COMMITTEE=4 (rule4)"
# jail with refill above the floor ACCEPTED
_reset; GA_N=7; GA_ACTION="liveness_jail"; GA_VICTIM="validator-1"; GA_REFILL=1; expect accept "jail@n7 with refill (rule4 ok)"
# jail WITHOUT a refill in flight REJECTED
_reset; GA_N=7; GA_ACTION="liveness_jail"; GA_VICTIM="validator-1"; GA_REFILL=0; expect reject "jail without refill (rule4)"

# 7b. byzantine_equivocate (PERMANENT tombstone) below MIN_COMMITTEE REJECTED; above floor ACCEPTED
_reset; GA_N=4; GA_ACTION="byzantine_equivocate"; GA_VICTIM="validator-1"; expect reject "byzantine@n4 below MIN (rule4b)"
_reset; GA_N=7; GA_ACTION="byzantine_equivocate"; GA_VICTIM="validator-1"; expect accept "byzantine@n7 above floor (rule4b ok)"
# 7c. byzantine_equivocate while a membership change is still settling REJECTED (rule4c serialize); clear → accept
_reset; GA_N=7; GA_ACTION="byzantine_equivocate"; GA_VICTIM="validator-1"; GA_MEMBERSHIP_SETTLING=1; expect reject "byzantine while membership settling (rule4c)"
_reset; GA_N=7; GA_ACTION="byzantine_equivocate"; GA_VICTIM="validator-1"; GA_MEMBERSHIP_SETTLING=0; expect accept "byzantine when settled (rule4c ok)"

# 8. pending projection (rule 5): a future committee would drop below the floor
_reset; GA_N=7; GA_PENDING_MIN=3; GA_MIN_COMMITTEE=4; expect reject "pending projection < floor (rule5)"

# 9. quorum-probe ACCEPTED in stable epoch, REJECTED when a change is imminent
_reset; GA_ACTION="quorum_probe"; GA_CHANGE_IMMINENT=0; expect accept "quorum-probe stable (§3.4)"
_reset; GA_ACTION="quorum_probe"; GA_CHANGE_IMMINENT=1; expect reject "quorum-probe during change (rejected)"

# 10. leader-liveness (rule 6): drop top-stake leader concurrently with another → REJECT
_reset; GA_N=10; GA_VICTIM="validator-9"; GA_LEADER="validator-9"; GA_ROUND_OTHERS=1; expect reject "leader + concurrent (rule6)"
# leader alone (no other this round) ACCEPTED
_reset; GA_N=10; GA_VICTIM="validator-9"; GA_LEADER="validator-9"; GA_ROUND_OTHERS=0; expect accept "leader alone (rule6 ok)"

# 11. victim availability: validator-0 pinned, and an already-disrupted victim
_reset; GA_VICTIM="validator-0"; expect reject "victim validator-0 pinned"
_reset; GA_VICTIM="validator-2"; GA_DISRUPTED="validator-2"; expect reject "victim already disrupted"

# 12. register_activate adds NO disruption → allowed even at the rule-1 ceiling
_reset; GA_N=4; GA_ACTION="register_activate"; GA_VICTIM=""; GA_DISRUPTED="validator-2"; expect accept "register_activate at rule1 ceiling"

echo "== $PASS passed, $FAIL failed =="
(( FAIL == 0 ))
