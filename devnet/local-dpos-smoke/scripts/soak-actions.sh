#!/usr/bin/env bash
# Production-soak churn menu + the PURE safety gate (sourced by case-soak.sh).
#
# Two clearly separated halves:
#   1. gate_accept() — a PURE function (NO docker/cast/network calls): reads only
#      its GA_* input variables and returns 0 (accept) / 1 (reject, reason in
#      GA_REASON). This is the soak's safety proof: the orchestrator populates the
#      GA_* inputs from live on-chain reads, the unit test populates them
#      synthetically (plan §8.4). Keeping the safety logic isolated and testable —
#      not buried in the loop — is the no-crutch payoff (plan D3).
#   2. act_* wrappers + the quorum-loss probe — these DO touch docker/cast; each
#      is a single-shot reuse of an existing case body (the loop owns restore
#      timing). They never decide safety; the gate already did.

# ---------------------------------------------------------------------------
# Pure set helpers (string = space-separated word set).
# ---------------------------------------------------------------------------
_count()  { local x=0 w; for w in $1; do x=$((x+1)); done; echo "$x"; }
_in_set() { local e="$1" w; for w in $2; do [[ "$w" == "$e" ]] && return 0; done; return 1; }
_intersect() { local a="$1" b="$2" out="" w; for w in $a; do _in_set "$w" "$b" && out+="$w "; done; echo "$out"; }

# Actions that add a TRANSIENT disruption (count toward rule 1). register_activate
# (new node) and delegate_shift (a tx) add none.
_adds_disruption() {
    case "$1" in
        graceful_stop_restart|sigkill_restart|cpu_throttle|dkg_midwindow_restart|\
        byzantine_equivocate|byzantine_forge_pk|liveness_jail) return 0 ;;
        *) return 1 ;;
    esac
}

# ---------------------------------------------------------------------------
# THE GATE (pure). Inputs (caller sets before calling; unit test sets directly):
#   GA_ACTION            action name (or "quorum_probe")
#   GA_VICTIM            victim node ("" for register_activate)
#   GA_N                 current LIVE committee size (getEpochCommittee(cur).length)
#   GA_DISRUPTED         currently-disrupted nodes (space-set, NOT incl victim)
#   GA_CHANGE_IMMINENT   1 iff committee(cur+1) != committee(cur)
#   GA_INCOMING          incoming committee (committee(cur+1)) members (space-set)
#   GA_SHARELESS         current projected shareless/verify-only count
#   GA_MIN_COMMITTEE     absolute floor (from SOAK_INITIAL_COMMITTEE, D4)
#   GA_REFILL            1 iff a register_activate refill is in flight
#   GA_PENDING_MIN       smallest projected committee size over {cur+1..cur+3}
#                        after tracked PENDING shrinks (== GA_N when none)
#   GA_LEADER            current top-stake leader node
#   GA_ROUND_OTHERS      disruptions already applied this storm round
#   GA_MEMBERSHIP_SETTLING 1 iff a committee-membership change (growth/refill join or a
#                        prior byzantine tombstone) is still settling (rule 4c serialize)
# Output: return 0 (accept) / 1 (reject); GA_REASON set on reject.
# Reject precedence mirrors plan §3.3 rules 1-6 (+ victim-availability + probe).
# ---------------------------------------------------------------------------
gate_accept() {
    GA_REASON=""

    # The marked quorum-loss probe (plan §3.4) is NOT gated by rule 1 (it
    # intentionally drops f+1). It only requires a STABLE committee.
    if [[ "$GA_ACTION" == "quorum_probe" ]]; then
        (( GA_CHANGE_IMMINENT == 1 )) && { GA_REASON="probe-needs-stable-committee"; return 1; }
        return 0
    fi

    # f from the live committee size — computed AFTER the probe early-return: the
    # probe path needs no f, so GA_N need not be set for the probe gate check
    # (only rule-1..6 below read f, and their caller always sets GA_N).
    local f=$(( (GA_N - 1) / 3 ))

    # Victim availability (plan §3.3 victim selection): never re-hit a disrupted
    # node; validator-0 is pinned honest (host RPC, pre-mortem §11.2).
    if [[ -n "$GA_VICTIM" ]]; then
        [[ "$GA_VICTIM" == "validator-0" ]] && { GA_REASON="victim-is-pinned-host-rpc"; return 1; }
        _in_set "$GA_VICTIM" "$GA_DISRUPTED" && { GA_REASON="victim-already-disrupted"; return 1; }
    fi

    local adds=0; _adds_disruption "$GA_ACTION" && adds=1

    # Rule 1 — transient quorum: the chain survives exactly f concurrent faults
    # (live = n-f = 2f+1 = quorum); reject the (f+1)-th. NOT ">= f".
    if (( adds == 1 )); then
        local cur; cur=$(_count "$GA_DISRUPTED")
        (( cur + 1 > f )) && { GA_REASON="rule1-transient-quorum ($((cur+1)) > f=$f)"; return 1; }
    fi

    # Rule 2 — DKG-window safety: in the E-1 window of a change, only
    # dkg_midwindow_restart may touch an INCOMING member, and never > f of them
    # (>f incoming pre-seal = terminal halt; plan §3.3 rule 2).
    if (( GA_CHANGE_IMMINENT == 1 )) && [[ -n "$GA_VICTIM" ]] && _in_set "$GA_VICTIM" "$GA_INCOMING"; then
        [[ "$GA_ACTION" != "dkg_midwindow_restart" ]] && {
            GA_REASON="rule2-dkg-window (only dkg_midwindow_restart may touch incoming in E-1)"; return 1; }
        local inc_dis; inc_dis=$(_count "$(_intersect "$GA_DISRUPTED" "$GA_INCOMING")")
        (( inc_dis + 1 > f )) && { GA_REASON="rule2-dkg-window (incoming disrupted $((inc_dis+1)) > f=$f)"; return 1; }
    fi

    # Rule 3 — shareless <= f. A down member of the incoming committee during E-1
    # is PREDICTABLY shareless on restart (missed the ceremony), so count it even
    # though its metric is unreadable while stopped (addresses critic dropped-#4).
    if (( adds == 1 )); then
        local sl="$GA_SHARELESS"
        if [[ "$GA_ACTION" == "dkg_midwindow_restart" ]] \
           || { (( GA_CHANGE_IMMINENT == 1 )) && [[ -n "$GA_VICTIM" ]] && _in_set "$GA_VICTIM" "$GA_INCOMING"; }; then
            sl=$(( sl + 1 ))
        fi
        (( sl > f )) && { GA_REASON="rule3-shareless ($sl > f=$f)"; return 1; }
    fi

    # Rule 4 — permanent floor: a liveness_jail permanently shrinks the committee.
    # Never below MIN_COMMITTEE, and only when a refill is in flight.
    if [[ "$GA_ACTION" == "liveness_jail" ]]; then
        (( GA_N - 1 < GA_MIN_COMMITTEE )) && { GA_REASON="rule4-min-committee (jail drops below $GA_MIN_COMMITTEE)"; return 1; }
        (( GA_REFILL != 1 )) && { GA_REASON="rule4-no-refill (jail needs a register_activate refill)"; return 1; }
    fi

    # Rule 4b — byzantine equivocation is a PERMANENT tombstone (un-recoverable:
    # AlreadySlashedForEquivocation). Never let it drop the LIVE committee below the
    # absolute floor; a spare refill restores the committee toward cap afterwards (E+3).
    # (A transient fault may dip to f; a permanent loss must leave a safe committee.)
    if [[ "$GA_ACTION" == "byzantine_equivocate" ]]; then
        (( GA_N - 1 < GA_MIN_COMMITTEE )) && { GA_REASON="rule4b-byzantine-floor (tombstone drops below $GA_MIN_COMMITTEE)"; return 1; }
        # Rule 4c — SERIALIZE membership changes: a byzantine tombstone is a committee-
        # membership change, and the on-CHANGE live DKG for the transitioning committee can
        # only QUALIFY if its members can all contribute valid dealings. Overlapping a
        # tombstone with a growth/refill join in flight makes committee[E] include a not-
        # ready joiner AND the byzantine/tombstoned member → the DKG under-qualifies → NO
        # usable shares → all verify-only → beacon DEADLOCK (observed seed cascadefull1
        # epoch 6). So don't equivocate while another membership change is still settling.
        (( GA_MEMBERSHIP_SETTLING == 1 )) && { GA_REASON="rule4c-serialize-membership (a growth/refill/tombstone is still settling)"; return 1; }
    fi

    # Rule 5 — pending projection: no committee in {cur+1..cur+3} may fall below
    # the floor after tracked PENDING shrinks land.
    (( GA_PENDING_MIN < GA_MIN_COMMITTEE )) && {
        GA_REASON="rule5-pending-projection (future committee $GA_PENDING_MIN < $GA_MIN_COMMITTEE)"; return 1; }

    # Rule 6 — leader-liveness (SOFT, WeightedVRF): don't drop the top-stake
    # leader concurrently with another disruption (false-positive-stall guard).
    if [[ -n "$GA_VICTIM" && "$GA_VICTIM" == "$GA_LEADER" ]] && (( GA_ROUND_OTHERS >= 1 )); then
        GA_REASON="rule6-leader-liveness (top-stake leader + concurrent disruption)"; return 1
    fi

    return 0
}

# ---------------------------------------------------------------------------
# Single-shot churn primitives (DO touch docker/cast). The orchestrator manages
# the DISRUPTED/PENDING bookkeeping AROUND these calls; they only effect the
# disruption + its restore. Each reuses an existing case body by mechanism.
# `_cid v` resolves the raw container id (the SIGKILL/throttle path that must
# bypass compose deps so a restart never re-runs genesis-init).
# ---------------------------------------------------------------------------
# Fail loud: clear message + non-zero (the caller fail_bundles).
_die() { echo "FAIL (soak-action): $*" >&2; return 1; }

# set -u SAFETY CONTRACT for every action: validate the required victim arg with
# `${1:?...}` (turns a cryptic set -u abort into a named error, and lets
# soak_selfcheck FLUSH it at startup), and short-circuit under SOAK_DRYRUN=1 (the
# self-check exercises the arg contracts + the function bodies' var-scoping WITHOUT
# touching docker). NOTE: never put a same-line `local a=$1 b=${a}` — bash evaluates
# all RHS of one `local` before assigning, so `${a}` is unbound under set -u (this is
# exactly the act_byzantine crash). Assign, THEN reference on a later line.
_cid() { docker compose ps -q "${1:?_cid: service required}" 2>/dev/null; }

act_graceful_stop()  { local v="${1:?act_graceful_stop: victim required}";  [[ -n "${SOAK_DRYRUN:-}" ]] && return 0; docker compose stop --timeout 40 "$v" >/dev/null 2>&1; shutdown_flushed "$v" || true; }
act_graceful_start() { local v="${1:?act_graceful_start: victim required}"; [[ -n "${SOAK_DRYRUN:-}" ]] && return 0; docker compose start "$v" >/dev/null 2>&1; }
act_sigkill_stop()   { local v="${1:?act_sigkill_stop: victim required}";   [[ -n "${SOAK_DRYRUN:-}" ]] && return 0; docker kill "$(_cid "$v")" >/dev/null 2>&1 || true; }
act_sigkill_start()  { local v="${1:?act_sigkill_start: victim required}";  [[ -n "${SOAK_DRYRUN:-}" ]] && return 0; docker start "$(_cid "$v")" >/dev/null 2>&1 || true; }
act_cpu_throttle()   { local v="${1:?act_cpu_throttle: victim required}";   [[ -n "${SOAK_DRYRUN:-}" ]] && return 0; docker update --cpus 0.15 "$(_cid "$v")" >/dev/null 2>&1 || true; }
act_cpu_restore()    { local v="${1:?act_cpu_restore: victim required}";    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0; docker update --cpus 4 "$(_cid "$v")" >/dev/null 2>&1 || true; }

# DKG mid-window restart: a graceful stop+start during the E-1 window so the
# victim restarts pre-finalize and resumes its share from the on-disk journal
# (recoverable path; reuse of case-vrf-dkg-restart-midwindow). The gate has
# already confirmed (rule 2) the victim is an incoming member and <= f.
act_dkg_midwindow_restart() {
    local v="${1:?act_dkg_midwindow_restart: victim required}"
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    docker compose stop --timeout 5 "$v" >/dev/null 2>&1 || true
    docker compose start "$v" >/dev/null 2>&1 || true
}

# Liveness-jail: stop the victim at epoch start so it accrues > MISS_THRESHOLD
# misses and the slasher jails it (drop at E+2 — reuse of case-production-path).
# Permanent until a P5 un-jail; no restore pair (the orchestrator may later
# un-jail + re-activate it as a fresh registration).
act_liveness_jail_begin() {
    local v="${1:?act_liveness_jail_begin: victim required}"
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    docker compose stop --timeout 40 "$v" >/dev/null 2>&1 || true
}

# Byzantine: restart the victim under a generated single-service env overlay
# carrying FLUENT_DPOS_BYZANTINE=<mode>. `mode` ∈ {equivocate, forge-beacon-pk}.
# Requires the dpos-devnet-byzantine image (case-soak asserts it parses first).
# The orchestrator DPOS_CONVERGE_EXCLUDEs the victim (it runs no executor).
act_byzantine() {
    local v="${1:?act_byzantine: victim required}"
    local mode="${2:?act_byzantine: mode required}"
    local overlay="docker-compose.soak.byz-${v}.gen.yml"   # SEPARATE line: ${v} now set
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    : "${COMPOSE_FILE:?act_byzantine: COMPOSE_FILE must be set (phase B)}"
    cat >"$overlay" <<EOF
services:
  ${v}:
    environment:
      FLUENT_DPOS_BYZANTINE: "${mode}"
EOF
    # Stay on the COMPOSE_FILE seam (NO -f, which would make compose ignore the env
    # and re-introduce the wrong-project hazard): append the overlay to the current
    # COMPOSE_FILE (the phase-B dpos scope) for just this recreate, so the bare-call
    # pp_* helpers keep resolving the same project afterwards.
    COMPOSE_FILE="${COMPOSE_FILE}:$overlay" docker compose up -d --force-recreate "$v" >/dev/null 2>&1 || true
}

# Restore a byzantine victim to HONEST: drop its env overlay and recreate it from
# the plain COMPOSE_FILE (the phase-B dpos scope, WITHOUT the byz overlay), so the
# next launch carries no FLUENT_DPOS_BYZANTINE. The restore pair for the recoverable
# byzantine_forge_pk action (the honest C-gate rejected its forged PK; once honest it
# rejoins). byzantine_equivocate has NO restore (permanent tombstone) so this is never
# called for it.
act_byzantine_restore() {
    local v="${1:?act_byzantine_restore: victim required}"
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    : "${COMPOSE_FILE:?act_byzantine_restore: COMPOSE_FILE must be set (phase B)}"
    rm -f "docker-compose.soak.byz-${v}.gen.yml" 2>/dev/null || true
    docker compose up -d --force-recreate "$v" >/dev/null 2>&1 || true
}

# ---------------------------------------------------------------------------
# The marked quorum-loss-and-recover probe (plan §3.4 / Q1). Drops f+1 committee
# members SIMULTANEOUSLY in a STABLE epoch, asserts a correct stall WITH NO
# unsafe finalization, then restores and asserts recovery. Owns its own pass/
# fail; the continuous finalize invariant is SUSPENDED for its window.
# Caller provides: SOAK_VALS (node array), the EXPECTED_STALL flag setter, and
# the event-log writer (soak_event). Returns 0 pass / 1 fail (reason echoed).
# ---------------------------------------------------------------------------
run_quorum_probe() {
    local n="${1:?run_quorum_probe: committee size required}" victims=("${@:2}")  # f+1 to drop
    local f=$(( (n - 1) / 3 ))
    local need=$(( f + 1 )) v p1 p2 settle window recover_wait

    # set-u flush (soak_selfcheck): touch every external var the body reads in
    # arithmetic so a missing one aborts loudly at startup, then bail (the live body
    # sleeps + waits on finalization, far too slow for a <1s dry-run).
    if [[ -n "${SOAK_DRYRUN:-}" ]]; then
        : "$f" "$need" "$RESULT_LAG_K" "$SOAK_VALIDATORS" "${EXPECTED_STALL:-}"
        settle=$(( 5 + 2 * RESULT_LAG_K )); window=$(( 8 + 2 * RESULT_LAG_K )); recover_wait=$(( 180 + SOAK_VALIDATORS * 30 ))
        return 0
    fi

    (( ${#victims[@]} == need )) || { echo "probe: expected $need victims, got ${#victims[@]}"; return 1; }

    EXPECTED_STALL=1                       # suspends invariant 1 (read by soak-invariants)
    soak_event probe_begin "stopping ${need} (=f+1) committee members: ${victims[*]}"
    for v in "${victims[@]}"; do act_graceful_stop "$v"; done

    # Let any in-flight (<=K-deep) finalizations DRAIN, then confirm a PLATEAU over a
    # window: with f+1 down, no new quorum can form, so finalized must stop advancing.
    # Checking a plateau (p2 == p1) — NOT "advanced past base" — avoids a false safety
    # flag from the K-deep pipeline draining the instant after the stop. All block-based
    # (a quorum-loss stall manifests in a few blocks at any epoch length).
    settle=$(( 5 + 2 * RESULT_LAG_K )); window=$(( 8 + 2 * RESULT_LAG_K ))
    sleep "$settle"; p1=$(finalized_dec); sleep "$window"; p2=$(finalized_dec)
    if (( p2 > p1 )); then
        EXPECTED_STALL=0
        soak_event probe_end "DID NOT STALL: finalized still advancing ${p1}->${p2} with f+1 down (quorum not lost / stop ineffective)"
        echo "probe: chain did not stall after dropping f+1=${need} (${p1}->${p2})"
        return 1
    fi
    soak_event probe_begin "stall confirmed (finalized plateau at ${p2}); restoring"

    # Restore + assert finalization RESUMES past the plateau. Recover deadline SCALES
    # with N (the f+1 graceful-stopped nodes reload their persisted share + reconnect).
    for v in "${victims[@]}"; do act_graceful_start "$v"; done
    recover_wait=$(( 180 + SOAK_VALIDATORS * 30 ))
    if wait_finalized_ge "$(( p2 + 1 ))" "$recover_wait"; then
        EXPECTED_STALL=0
        soak_event probe_end "recovered: finalized resumed past ${p2}"
        return 0
    fi
    EXPECTED_STALL=0
    soak_event probe_end "RECOVERY FAILURE: finalized did not resume past ${p2} in ${recover_wait}s"
    echo "probe: recovery — chain did not resume after restoring ${victims[*]}"
    return 1
}

# ---------------------------------------------------------------------------
# Committee-growth / rotation actions (reuse case-production-path ABIs verbatim;
# read STAKING_RT/TOKEN/GOV_ADDR exported by the bring-up). Single-shot; their
# committee effect lands at E+3 (the orchestrator records it in PENDING).
# ---------------------------------------------------------------------------
# Register + activate a joiner AND RAISE THE COMMITTEE CAP so it enters as an
# ADDITIONAL member (4→5→…→N), not merely rotates a weaker one. The committee is the
# on-chain top-k where k=activeValidatorsLength; without the cap raise the committee
# stays at the initial size and growth is a silent no-op (the bug). Every
# success-critical cast is FAIL-LOUD (`_die ... || return 1`) — a failed growth must
# be VISIBLE (the caller fail_bundles), never a fictional "APPLIED".
# Fail-loud `cast send`. CRITICAL: `cast send` returns exit 0 even when the tx
# REVERTS on-chain (foundry gotcha — the exit code reflects submission, not the
# receipt status). A bare `cast send … || _die` therefore SILENTLY swallows a
# revert: that is exactly what turned a reverted registerValidator into a
# baffling NotPendingValidator at the later (gas-estimating) activate. This
# wrapper (a) checks the receipt `.status` and (b) on revert replays the call
# read-only with `cast call` (which DOES surface the revert reason) so the
# failure is LOCATED and REASONED, not a phantom downstream error.
# Usage: soak_send <label> <to> <sig> [args…] --private-key <key>
soak_send() {
    local label="$1"; shift
    local out st
    out=$(cast send --json --rpc-url "$RPC" "$@" 2>&1) \
        || { _die "$label: cast send error: $(printf '%s' "$out" | tail -1)"; return 1; }
    st=$(jq -r 'if type=="object" then (.status // .receipt.status // "") else "" end' <<<"$out" 2>/dev/null)
    if [[ "$st" != "0x1" && "$st" != "1" ]]; then
        local reason; reason=$(cast call --rpc-url "$RPC" "$@" 2>&1 | tail -1)
        _die "$label: tx REVERTED on-chain (status=$st): $reason"; return 1
    fi
    return 0
}

soak_register_activate() {
    local idx="${1:?soak_register_activate: joiner idx required}" raise_cap="${2:-1}"
    : "${TOKEN:?soak_register_activate: TOKEN unset}"
    : "${STAKING_RT:?soak_register_activate: STAKING_RT unset}"
    : "${CHAIN_CONFIG_RT:?soak_register_activate: CHAIN_CONFIG_RT unset}"
    : "${GOV_ADDR:?soak_register_activate: GOV_ADDR unset}"
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    local addr key ck bls_pub bls_pop peer cap new_cap st gotcap
    addr=$(pp_owner_addr "$idx"); key=$(pp_owner_key "$idx")
    [[ "$addr" == 0x* ]] || { _die "register: no owner addr for joiner idx $idx"; return 1; }
    # FRESH registration — the proven production-path add-validator sequence. Joiners
    # idx>=SOAK_INITIAL_COMMITTEE are deliberately NOT in the deploy's initialValidators
    # (case-soak passes INITIAL_VALIDATORS=v0..v$((INITIAL-1))), so each starts NotFound on-chain
    # and gets a clean register→Pending→activate→Active→delegate lifecycle — exactly like the
    # external v5 join in case-production-path. NEXT_JOINER increments monotonically so each idx
    # is registered AT MOST ONCE. Every step is FAIL-LOUD via soak_send (revert-checked) AND
    # post-asserts the on-chain lifecycle invariant (ValidatorStatus: NotFound=0,Active=1,
    # Pending=2,Jail=3) so a swallowed revert can never masquerade as a later phantom error.
    soak_send "approve(stake) v$idx" "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "1000000000000000000" \
        --private-key "$key" || return 1
    soak_send "registerValidator v$idx" "$STAKING_RT" "registerValidator(address,uint16,uint256)" "$addr" 0 "1000000000000000000" \
        --private-key "$key" || return 1
    st=$(pp_validator_status "$addr"); [[ "$st" == "2" ]] \
        || { _die "register v$idx ($addr): status=$st after registerValidator (want 2=Pending)"; return 1; }
    ck=$(pp_consensus_keys "$idx")
    bls_pub=$(jq -r .blsPubkeyUncompressed <<<"$ck"); bls_pop=$(jq -r .blsPoPUncompressed <<<"$ck"); peer=$(jq -r .peerPubkey <<<"$ck")
    soak_send "setConsensusKeys v$idx" "$STAKING_RT" "setConsensusKeys(address,bytes,bytes,bytes32)" "$addr" "$bls_pub" "$bls_pop" "$peer" \
        --private-key "$key" || return 1
    # STAKE-WEIGHTED quorum (FluentGovernance: 2/3 of total delegated stake over the
    # ACTIVE set). Vote EVERY container owner-key (0..VAL_CONTAINERS-1) with VOTE_ALL: the
    # currently-active subset (wherever it sits — growth prefix, or a post-tombstone/refill
    # gap) casts 100% of the voting supply "for"; not-yet-active / tombstoned indices cast
    # 0-weight (or revert) harmlessly via --async. This is robust for BOTH growth (active =
    # a prefix) AND refill (active = a non-prefix set after tombstones), unlike the old
    # PP_GOV_VOTERS=idx prefix which under-votes once spares fill non-prefix slots.
    PP_GOV_VOTERS="$SOAK_VAL_CONTAINERS" PP_GOV_VOTE_ALL=1 \
    pp_gov_action "$STAKING_RT" "$(cast calldata 'activateValidator(address)' "$addr")" "activate-$idx" \
        || { _die "gov activateValidator v$idx"; return 1; }
    st=$(pp_validator_status "$addr"); [[ "$st" == "1" ]] \
        || { _die "activate v$idx ($addr): status=$st after activateValidator (want 1=Active)"; return 1; }
    soak_send "approve(delegate) v$idx" "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "2000000000000000000" \
        --private-key "$key" || return 1
    soak_send "delegate v$idx" "$STAKING_RT" "delegate(address,uint256)" "$addr" "2000000000000000000" \
        --private-key "$key" || return 1
    # GROWTH (raise_cap=1): raise activeValidatorsLength by 1 (capped at the committee
    # target) so the new validator is an ADDITIONAL slot. REFILL (raise_cap=0): cap is
    # UNCHANGED — a permanent loss (byzantine tombstone) already freed a slot, so the new
    # validator just fills it (committee returns to cap). EffBal lands at E+3 either way.
    if (( raise_cap == 1 )); then
        cap=$(printf '%d' "$(pp_chainconfig_call 'getActiveValidatorsLength()(uint32)' 2>/dev/null || echo 0)")
        (( cap > 0 )) || { _die "read activeValidatorsLength"; return 1; }
        new_cap=$(( cap + 1 )); (( new_cap <= SOAK_VALIDATORS )) || new_cap="$SOAK_VALIDATORS"
        PP_GOV_VOTERS="$SOAK_VAL_CONTAINERS" PP_GOV_VOTE_ALL=1 \
        pp_gov_action "$CHAIN_CONFIG_RT" "$(cast calldata 'setActiveValidatorsLength(uint32)' "$new_cap")" "grow-cap-$new_cap" \
            || { _die "gov setActiveValidatorsLength($new_cap)"; return 1; }
        gotcap=$(printf '%d' "$(pp_chainconfig_call 'getActiveValidatorsLength()(uint32)' 2>/dev/null || echo 0)")
        (( gotcap == new_cap )) || { _die "grow-cap v$idx: activeValidatorsLength=$gotcap after setActiveValidatorsLength (want $new_cap)"; return 1; }
        echo "  register_activate v$idx: committee cap ${cap}->${new_cap} (EffBal lands @E+3)"
    else
        echo "  refill v$idx: registered+activated WITHOUT cap-raise (fills a tombstone-freed slot; lands @E+3)"
    fi
    return 0
}

# Delegate >1e18 (from the deployer) to the victim's owner addr to shift EffBal
# and force a committee ROTATION at E+3 (reuse of case-production-path delegate).
soak_delegate_shift() {
    local v="${1:?soak_delegate_shift: victim required}"
    : "${TOKEN:?soak_delegate_shift: TOKEN unset}"
    : "${STAKING_RT:?soak_delegate_shift: STAKING_RT unset}"
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    local idx="${v##*-}" addr key
    addr=$(pp_owner_addr "$idx"); key=$(pp_owner_key 0)
    # Rotation PRESSURE, not success-critical growth: a failed delegate just means no
    # rotation this round (tolerable) — so `|| true` here, unlike register's fail-loud.
    cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "3000000000000000000" \
        --rpc-url "$RPC" --private-key "$key" >/dev/null 2>&1 || true
    cast send "$STAKING_RT" "delegate(address,uint256)" "$addr" "3000000000000000000" \
        --rpc-url "$RPC" --private-key "$key" >/dev/null 2>&1 || true
}

# P5 un-jail → re-activate (the restore path for a liveness_jail victim). There is NO
# `unjail()` ABI — the jail auto-expires after validatorJailEpochLength epochs, after
# which re-activation (gov) + re-delegation re-enters the committee. Best-effort
# (`|| true`): a re-activate that races the exact expiry epoch just retries the cycle
# next time the validator is jailed; the invariant battery still proves the chain stays
# live. Node is brought back online first so it can rejoin once re-selected.
soak_unjail_reactivate() {
    local v="${1:?soak_unjail_reactivate: victim required}"
    : "${TOKEN:?soak_unjail_reactivate: TOKEN unset}"
    : "${STAKING_RT:?soak_unjail_reactivate: STAKING_RT unset}"
    [[ -n "${SOAK_DRYRUN:-}" ]] && return 0
    local idx="${v##*-}" addr key
    addr=$(pp_owner_addr "$idx"); key=$(pp_owner_key "$idx")
    docker compose start "$v" >/dev/null 2>&1 || true
    cast send "$TOKEN" "approve(address,uint256)(bool)" "$STAKING_RT" "2000000000000000000" \
        --rpc-url "$RPC" --private-key "$key" >/dev/null 2>&1 || true
    cast send "$STAKING_RT" "delegate(address,uint256)" "$addr" "2000000000000000000" \
        --rpc-url "$RPC" --private-key "$key" >/dev/null 2>&1 || true
    # Same stake-weighted quorum scaling as soak_register_activate: vote with the whole
    # currently-registered set (NEXT_JOINER = the index past the last registered joiner).
    # The victim itself is still jailed at vote time → 0-weight, dropped from supply; the
    # other active owners carry quorum. Best-effort (|| true): a missed un-jail just retries.
    PP_GOV_VOTERS="${NEXT_JOINER:-$SOAK_INITIAL_COMMITTEE}" PP_GOV_VOTE_ALL=1 \
    pp_gov_action "$STAKING_RT" "$(cast calldata 'activateValidator(address)' "$addr")" "unjail-reactivate-$idx" || true
}

# Startup set-u + arg-contract self-check (the MECHANISM): FLUSH latent unbound /
# missing-arg / same-line-local traps BEFORE a multi-hour run rather than crashing
# hours in. Runs every action through its REAL arg signature in SOAK_DRYRUN mode (no
# docker) so any `${:?}` contract or same-line-local trap aborts loudly at startup —
# the act_byzantine bug would have fired HERE. Also asserts the success-critical
# globals the actions need are set. Call AFTER soak_bring_up (globals exported).
soak_selfcheck() {
    : "${TOKEN:?selfcheck: TOKEN unset}"; : "${STAKING_RT:?selfcheck: STAKING_RT unset}"
    : "${CHAIN_CONFIG_RT:?selfcheck: CHAIN_CONFIG_RT unset}"; : "${GOV_ADDR:?selfcheck: GOV_ADDR unset}"
    : "${COMPOSE_FILE:?selfcheck: COMPOSE_FILE unset}"
    local rep="${SOAK_VALS[1]:-validator-1}"
    SOAK_DRYRUN=1
    act_graceful_stop "$rep"; act_graceful_start "$rep"
    act_sigkill_stop "$rep";  act_sigkill_start "$rep"
    act_cpu_throttle "$rep";  act_cpu_restore "$rep"
    act_dkg_midwindow_restart "$rep"
    act_liveness_jail_begin "$rep"
    act_byzantine "$rep" equivocate; act_byzantine "$rep" forge-beacon-pk
    act_byzantine_restore "$rep"
    soak_register_activate "${SOAK_INITIAL_COMMITTEE:-4}"
    soak_delegate_shift "$rep"
    soak_unjail_reactivate "$rep"
    run_quorum_probe 7 validator-1 validator-2   # dry: flushes its var-scoping (RESULT_LAG_K/SOAK_VALIDATORS/EXPECTED_STALL)
    SOAK_DRYRUN=
    echo "  self-check: all actions pass the set-u + arg-contract dry-run"
}
