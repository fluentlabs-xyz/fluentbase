#!/usr/bin/env bash
# Heavy-load blaster for the DPoS smoke harness — the load-GENERATION half of the
# execution-saturation observability pair (the measurement half is soak-invariants.sh's
# `_inv_exec_saturation` / `_inv_block_rate` watches: aim this at a live soak and read
# the exec-saturation / block-rate events off the soak timeline).
#
# Deploys the vendored SSTORE-heavy GasBurner fixture (contracts/GasBurner.{sol,json},
# harness-local, compiled with forge + vendored like PrevRandaoProbe.json) once, funds
# LH_SENDERS ephemeral keys from the harness's funded dev key, then runs per-sender
# async-send loops with locally-tracked nonces, paced so that
#     LH_SENDERS x rate x LH_TX_GAS ≈ LH_TARGET_GAS_FRACTION x block_gas_limit / 1 s block.
#
# Usage:   ./scripts/load-heavy.sh                        # supervise until SIGTERM/Ctrl-C
#          LH_DURATION=60 LH_SENDERS=1 ./scripts/load-heavy.sh   # bounded smoke-poke
#          LH_WAIT_MARKER_LOG=<soak.log> ./scripts/load-heavy.sh # gate on DeployStaking marker
#          ./scripts/load-heavy.sh stop [pidfile]         # clean pidfile-based shutdown
# LIFECYCLE (the sender-lifecycle rework, 2026-07-20 — retires the external
#   launch-load.sh/blaster-waiter.sh scratchpad glue): this ONE process is self-supervising.
#     start    — launch it (e.g. `setsid env … load-heavy.sh > lh.log 2>&1 &`); it writes a
#                pidfile (LH_PIDFILE, default beside LH_LOG or $TMPDIR/load-heavy.pid).
#     wait     — LH_WAIT_MARKER_LOG gates ALL funding behind the soak's first `[soak r<N> `
#                marker (DeployStaking-complete); funder key + RPC are acquired with bounded
#                retry/backoff INSIDE this script (no more die-on-first-miss zero-load runs).
#     supervise— it restarts any sender loop that dies (counter + exponential backoff) and
#                emits the `sent=/inflight=` heartbeat every LH_STATUS_EVERY.
#     stop     — `load-heavy.sh stop [pidfile]`: TERM the recorded process group, wait, KILL
#                -9 fallback, identity-verify by pid+etime, remove the pidfile.
#   NEVER pkill this process (self-match traps + orphaned cast children). Always use `stop`.
# Knobs:   LH_RPC (default http://localhost:8545 — the v0 host port)
#          LH_MODE (burn — default GasBurner SSTORE fill; or transfer — 21000-gas ETH
#            self-sends at a fixed modest aggregate rate for disk-backed/SSD soaks where
#            heavy exec+IO overruns the 1 s block budget)   LH_TPS (transfer only: 25 tx/s)
#          LH_SENDERS (4)   LH_TARGET_GAS_FRACTION (0.6)   LH_TX_GAS (3000000)
#          LH_DURATION (0 = unbounded, seconds)   LH_MAX_FEE (5000 wei)   LH_TIP (2 wei)
#          LH_FUND_WEI (default window x tx_gas x max_fee x 4, computed)
#          LH_STATUS_EVERY (30 s)   LH_FUNDER_KEY (else validator-0 /runtime/keys/funded.hex)
#          LH_WAIT_MARKER_LOG (soak log to gate on; unset = no gate)
#          LH_ACQUIRE_TIMEOUT (1800 s loud-FAIL cap)   LH_ACQUIRE_INTERVAL (15 s poll)
#          LH_PIDFILE / LH_LOG (pidfile path / its default anchor)
# Robust:  bounded startup acquisition (funder key + RPC + soak marker, retry every
#          LH_ACQUIRE_INTERVAL, loud FAIL after LH_ACQUIRE_TIMEOUT), a SUPERVISOR that
#          restarts dead senders (backoff, flap-reset), per-sender backpressure window
#          (LH_MAX_INFLIGHT, never past reth's per-account pool cap), EIP-1559 capped-fee
#          sends (LH_MAX_FEE/LH_TIP — self-regulating: over the cap the pool WAITS, no
#          repricing, no fee-bid feedback, no bankruptcy), nonce-gap re-sync from RPC,
#          RPC-down backoff, hard timeout on every cast RPC call + unconditional status
#          heartbeat; no single failed send ever kills a loop; pidfile-based clean stop.

set -euo pipefail

# ── pure seams (gate-testable without a chain — soak-gate-test.sh §20) ──────────────

# <block_gas_limit> <target_fraction> <senders> <tx_gas> → "<total_tps> <per_sender_interval_ms>"
# total tx/s so the fleet burns fraction x limit gas per 1 s block; interval = the per-sender
# pacing that yields that aggregate rate. Fails (echo "0 0", rc 1) on non-positive inputs.
lh_calibrate() {
    awk -v g="${1:-0}" -v f="${2:-0}" -v s="${3:-0}" -v t="${4:-0}" 'BEGIN {
        if (g <= 0 || f <= 0 || s <= 0 || t <= 0) { print "0 0"; exit 1 }
        tps = g * f / t
        printf "%.3f %d\n", tps, (s * 1000) / tps
    }'
}

# <tx_gas> <base_gas> <gas_per_iter> → burn(n) iteration count filling 90% of the tx gas
# budget (10% headroom vs estimate drift so a send never ODs its own --gas-limit); min 1.
lh_burn_iters() {
    awk -v t="${1:-0}" -v b="${2:-0}" -v p="${3:-1}" 'BEGIN {
        n = int((0.9 * t - b) / p); if (n < 1) n = 1; print n }'
}

# Classify a failed `cast send` stderr: 0 = nonce-desync (re-sync the local nonce from the
# RPC and continue), 1 = anything else (RPC down / fee spike / revert → backoff path).
lh_is_nonce_error() {
    grep -qiE 'nonce too (low|high)|invalid nonce|already known|replacement transaction' <<<"${1:-}"
}

# Backpressure-window verdict: <next> <mined> <window> → "<reconciled_next> <send|skip>".
# The local nonce DERIVES from the mined head (never trails it — external reality wins) and
# the in-flight depth (next − mined) is capped at the window: at/over it the tick is SKIPPED
# with NO local nonce bump, so a runaway past the pool's per-account pending cap (reth: 16)
# is structurally impossible (2026-07-17 live forensics: blind pace-clock sends raced nonces
# past mined+16 → drain-paced gap=1 accounts + wedged accounts → pool starved to ~4-8
# executable txs, blocks at 8% instead of 87%).
lh_inflight_gate() {
    local n="${1:-0}" m="${2:-0}" w="${3:-1}"
    if (( n < m )); then n="$m"; fi   # an `if`, NOT a `&&`-tail: set -e kills a bare failed AND-list
    if (( n - m >= w )); then echo "$n skip"; else echo "$n send"; fi
}

# WEDGE FIX (geo-soak 2026-07-16): cast has NO client-side HTTP timeout — under a congested
# RPC (netem WAN + 60% gas) every sender AND the status printer hung simultaneously in
# unanswered requests (plus `cast receipt` without --async POLLS until a pending tx mines —
# the status-printer wedge). EVERY cast call that touches the RPC goes through a hard
# `timeout` now; a timed-out call degrades (one log line / "rpc-timeout" marker) and the loop
# continues — pacing is owned by the pace clock, never by RPC latency. `cast wallet …` is
# offline and stays bare. Defined above the source-guard so the dead-RPC behavior is testable
# without a chain (soak-gate-test.sh §20).
LH_CAST_TIMEOUT="${LH_CAST_TIMEOUT:-15}"          # hard deadline on every runtime cast RPC call
LH_STATUS_TIMEOUT="${LH_STATUS_TIMEOUT:-5}"       # shorter deadline for status reads (heartbeat stays near-interval)
LH_INIT_TIMEOUT="${LH_INIT_TIMEOUT:-60}"          # startup sync sends (funding/deploy wait for inclusion)
lh_cast() { timeout -k 2 "$LH_CAST_TIMEOUT" cast "$@"; }
lh_cast_status() { timeout -k 2 "$LH_STATUS_TIMEOUT" cast "$@"; }
lh_cast_init() { timeout -k 2 "$LH_INIT_TIMEOUT" cast "$@"; }

# ── lifecycle seams (sender-lifecycle rework 2026-07-20 — gate-testable, soak-gate-test.sh §20) ──

# Soak-gate predicate: true iff <logfile> already contains the first DeployStaking-complete
# marker `[soak r<N> ` — the ONLY safe signal that funding a tx won't corrupt the shared
# dev-EOA nonce (incidents v51/v54/v55: load launched on an RPC-up race BEFORE DeployStaking).
# Replaces the external blaster-waiter.sh grep-and-retry. Missing/unreadable log → false
# (keep waiting). The pattern mirrors soak-status.sh's progress grep.
LH_MARKER_RE='\[soak r[0-9]+ '
lh_log_has_marker() {
    local log="${1:-}"
    [[ -n "$log" && -r "$log" ]] || return 1
    grep -qE "$LH_MARKER_RE" "$log"
}

# Acquisition retry verdict: <elapsed_s> <timeout_s> → "retry" (rc 0) while within the bound,
# else "fail" (rc 1). The bounded, logged, retry-don't-die contract that replaces the
# die-on-first-miss funder read (2026-07-20: one missed read → 80 min of zero load).
lh_acquire_verdict() {
    local elapsed="${1:-0}" timeout="${2:-0}"
    if (( elapsed < timeout )); then echo retry; return 0; fi
    echo fail; return 1
}

# Supervisor restart backoff: <restart_count> → seconds to hold a repeatedly-dying sender
# before relaunch = min(BASE * 2^(n-1), MAX); n<=0 → 0 (first start is immediate). Exponential
# so a crash-looping sender backs off instead of hot-spinning; capped so a healthy fleet still
# recovers promptly (flap-reset in the supervisor zeroes n after a sender stays up).
LH_RESTART_BACKOFF_BASE="${LH_RESTART_BACKOFF_BASE:-2}"
LH_RESTART_BACKOFF_MAX="${LH_RESTART_BACKOFF_MAX:-60}"
lh_restart_backoff() {
    awk -v n="${1:-0}" -v base="$LH_RESTART_BACKOFF_BASE" -v cap="$LH_RESTART_BACKOFF_MAX" 'BEGIN{
        if (n <= 0) { print 0; exit }
        b = base * (2 ^ (n - 1)); if (b > cap) b = cap; printf "%d\n", b }'
}

# Clean-stop identity check ("pid + etime"): a live PID is OURS only if (now − its
# elapsed-seconds) still lands on the pidfile's recorded start epoch (within <tol>s of clock
# skew) — otherwise the PID was recycled and MUST NOT be killed. <recorded_start> <now>
# <etimes> <tol>; rc 0 = match. Keeps `stop` from ever nuking an unrelated process.
lh_proc_identity_matches() {
    awk -v s="${1:-0}" -v now="${2:-0}" -v e="${3:-0}" -v tol="${4:-2}" 'BEGIN{
        d = (now - e) - s; if (d < 0) d = -d; exit (d <= tol) ? 0 : 1 }'
}

# Parse a supervisor pidfile line "<pid> <pgid> <start_epoch>" → echo the 3 validated fields,
# rc 1 on any malformed / non-numeric / trailing-junk input.
lh_pidfile_parse() {
    local pid pgid start rest
    read -r pid pgid start rest <<<"${1:-}"
    [[ "$pid" =~ ^[0-9]+$ && "$pgid" =~ ^[0-9]+$ && "$start" =~ ^[0-9]+$ && -z "$rest" ]] || return 1
    printf '%s %s %s\n' "$pid" "$pgid" "$start"
}

# The heartbeat line — the SINGLE emitter of the `sent=<n> inflight=<n>/<cap>` string that
# soak-status.sh's render_blaster_section greps for. Its format is pinned against that grep in
# soak-gate-test.sh §20 so the two can never drift. Args: <mode> <sent> <inflight> <cap>
# <basefee> <maxfee> <sample> <head>.
lh_status_line() {
    printf 'load-heavy[status]: mode=%s sent=%s inflight=%s/%s basefee=%s/max=%s sample_receipt(last sender-0)=%s head=%s\n' \
        "${1:-?}" "${2:-0}" "${3:-0}" "${4:-0}" "${5:--}" "${6:--}" "${7:--}" "${8:--}"
}

# Sourced (gate-test / other cases) → expose the pure seams only, run nothing.
if [[ "${BASH_SOURCE[0]:-$0}" != "$0" ]]; then return 0; fi

# ── pidfile + clean stop (runtime; NEVER pkill — self-match traps + orphaned cast children) ──

# Where the supervisor records itself: explicit LH_PIDFILE, else beside LH_LOG, else $TMPDIR.
lh_pidfile_path() {
    if [[ -n "${LH_PIDFILE:-}" ]]; then printf '%s' "$LH_PIDFILE"; return 0; fi
    if [[ -n "${LH_LOG:-}" ]]; then printf '%s/load-heavy.pid' "$(dirname "$LH_LOG")"; return 0; fi
    printf '%s/load-heavy.pid' "${TMPDIR:-/tmp}"
}

# Pidfile-based shutdown: parse → identity-verify by pid+etime (never kill a recycled PID) →
# TERM the recorded process group (or just the pid if we did not lead the group) → wait →
# KILL -9 fallback → verify gone → remove the pidfile.
lh_stop_pidfile() {   # <pidfile>
    local pf="$1" line pid pgid start now etimes target i
    if [[ ! -f "$pf" ]]; then echo "load-heavy stop: no pidfile at $pf (nothing to stop)"; return 0; fi
    line="$(cat "$pf" 2>/dev/null)"
    if ! read -r pid pgid start < <(lh_pidfile_parse "$line"); then
        echo "load-heavy stop: malformed pidfile $pf ('$line') — removing"; rm -f "$pf"; return 0
    fi
    now="$(date +%s)"
    # `|| true`: a dead pid makes `ps` exit non-zero, which under set -e + pipefail would
    # otherwise abort this function silently (leaving the stale pidfile un-removed).
    etimes="$(ps -o etimes= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
    if [[ ! "$etimes" =~ ^[0-9]+$ ]]; then
        echo "load-heavy stop: pid $pid not alive — removing stale pidfile $pf"; rm -f "$pf"; return 0
    fi
    if ! lh_proc_identity_matches "$start" "$now" "$etimes" 3; then
        echo "load-heavy stop: pid $pid alive but start-time mismatch (recorded=$start, now-etime=$(( now - etimes ))) — PID reused, NOT killing; removing stale pidfile $pf"
        rm -f "$pf"; return 0
    fi
    # Ours. Group-kill only when we lead the group (pgid==pid — the setsid launch); otherwise
    # a group TERM would also hit the parent shell, so fall back to the bare pid (its own trap
    # reaps its senders).
    if [[ "$pgid" == "$pid" ]]; then target="-$pgid"; else target="$pid"; fi
    echo "load-heavy stop: TERM $target (pid=$pid pgid=$pgid)"
    kill -TERM "$target" 2>/dev/null || true
    for (( i = 0; i < 20; i++ )); do
        if ! ps -p "$pid" >/dev/null 2>&1; then echo "load-heavy stop: stopped cleanly"; rm -f "$pf"; return 0; fi
        sleep 0.5
    done
    echo "load-heavy stop: pid $pid still alive after 10s — KILL -9 $target"
    kill -KILL "$target" 2>/dev/null || true
    sleep 1
    if ps -p "$pid" >/dev/null 2>&1; then
        echo "load-heavy stop: FAILED to kill pid $pid (check manually — do NOT pkill)" >&2; return 1
    fi
    echo "load-heavy stop: killed"; rm -f "$pf"; return 0
}

if [[ "${1:-}" == stop ]]; then
    lh_stop_pidfile "${2:-$(lh_pidfile_path)}"
    exit
fi

# ── runtime ──────────────────────────────────────────────────────────────────────────

cd "$(dirname "$0")/.."
# shellcheck source=lib.sh
source scripts/lib.sh          # RPC default + rpc.sh transport helpers

RPC="${LH_RPC:-$RPC}"
LH_SENDERS="${LH_SENDERS:-4}"
LH_TARGET_GAS_FRACTION="${LH_TARGET_GAS_FRACTION:-0.6}"
LH_TX_GAS="${LH_TX_GAS:-3000000}"
LH_DURATION="${LH_DURATION:-0}"
LH_STATUS_EVERY="${LH_STATUS_EVERY:-30}"
LH_MAX_INFLIGHT="${LH_MAX_INFLIGHT:-12}"          # per-sender in-flight cap (< reth's 16 per-account pending cap)
LH_UNWEDGE_SKIPS="${LH_UNWEDGE_SKIPS:-10}"        # full-window skips with a frozen mined head between "stuck head" diagnostic logs
# Lifecycle (sender-lifecycle rework 2026-07-20).
LH_WAIT_MARKER_LOG="${LH_WAIT_MARKER_LOG:-}"      # soak log to gate funding behind the first `[soak r<N> ` marker (unset = no gate)
LH_ACQUIRE_TIMEOUT="${LH_ACQUIRE_TIMEOUT:-1800}"  # loud-FAIL cap on startup acquisition (funder key + RPC + marker), seconds
LH_ACQUIRE_INTERVAL="${LH_ACQUIRE_INTERVAL:-15}"  # acquisition retry poll interval, seconds
LH_SUPERVISE_TICK="${LH_SUPERVISE_TICK:-5}"       # supervisor loop cadence (dead-sender detection latency), seconds
LH_RESTART_HEALTHY_SECS="${LH_RESTART_HEALTHY_SECS:-120}"  # a sender up this long resets its restart counter (flap-reset)
# EIP-1559 FEE MODEL (wedge-3 fix, live forensics 2026-07-17): the legacy `--gas-price
# eth_gasPrice×3` bid formed a RUNAWAY FEEDBACK under saturation — each refresh re-bid 3× a
# base fee its own demand had climbed 12.5%/block (~87M wei peak), draining sender balances
# until their OWN peak-priced pending windows became unaffordable (frozen), and a legacy
# same-nonce replacement can only bid UP, so down-repricing was silently rejected (~530
# futile attempts/sender). 1559 is the self-regulator: effective price = baseFee+tip (never
# the cap), so when the base fee exceeds LH_MAX_FEE the txs simply WAIT in the pool and
# auto-mine as the fee decays — no bankruptcy, no repricing, no refresh loop.
LH_MAX_FEE="${LH_MAX_FEE:-5000}"                  # maxFeePerGas cap, wei (generous vs the 7-wei idle base fee)
LH_TIP="${LH_TIP:-2}"                             # maxPriorityFeePerGas, wei
# Load mode (SSD-soak fix 2026-07-17): `burn` (default — GasBurner SSTORE fill, saturates the
# block for the exec-overrun observability pair) or `transfer` (plain 21000-gas ETH self-sends
# at a FIXED modest aggregate rate — steady light load that never threatens the 1 s block
# budget on disk-backed runs where heavy GasBurner exec+IO overruns it). Transfer mode skips
# the GasBurner deploy/gas-calibration; sender funding and the 1559 fee caps are unchanged.
LH_MODE="${LH_MODE:-burn}"
case "$LH_MODE" in burn|transfer) ;; *) echo "FAIL (load-heavy): LH_MODE must be burn|transfer (got '$LH_MODE')" >&2; exit 1;; esac
# Per-sender funding: a FULL window at the worst-case (cap) price, ×4 safety margin —
# computed from the knobs so raising LH_MAX_FEE/LH_TX_GAS/LH_MAX_INFLIGHT scales it.
LH_FUND_WEI="${LH_FUND_WEI:-$(( LH_MAX_INFLIGHT * LH_TX_GAS * LH_MAX_FEE * 4 ))}"

# Funded dev key: env override, else the harness's canonical location (asserts.sh smoke-vrf
# pattern), else any live container matching validator-0 (a generated soak stack runs under
# its own compose project, so plain `docker compose exec` from this dir cannot see it).
lh_funder_key() {
    if [[ -n "${LH_FUNDER_KEY:-}" ]]; then printf '%s' "${LH_FUNDER_KEY}"; return 0; fi
    local k=""
    k=$(docker compose exec -T validator-0 cat /runtime/keys/funded.hex 2>/dev/null | tr -d '[:space:]') || true
    if [[ -z "$k" ]]; then
        local c
        c=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -m1 'validator-0') || true
        [[ -n "$c" ]] && k=$(docker exec "$c" cat /runtime/keys/funded.hex 2>/dev/null | tr -d '[:space:]') || true
    fi
    [[ -n "$k" ]] || { echo "FAIL (load-heavy): no funder key (set LH_FUNDER_KEY or run near a live stack)" >&2; return 1; }
    printf '0x%s' "${k#0x}"
}

LH_TMP=$(mktemp -d "${TMPDIR:-/tmp}/load-heavy.XXXXXX")
LH_PIDS=()
lh_stop() {
    trap - TERM INT EXIT       # idempotent — no re-entry while we tear down
    local p
    for p in "${LH_PIDS[@]:-}"; do [[ -n "$p" ]] && kill "$p" 2>/dev/null || true; done
    LH_PIDS=()
    [[ -n "${PIDFILE:-}" ]] && rm -f "$PIDFILE" 2>/dev/null || true
    rm -rf "$LH_TMP" 2>/dev/null || true
    # Last (nothing runs after): sweep the whole process group so orphaned `timeout`/`cast`
    # grandchildren die too — but only if we lead it (setsid launch: pgid==our pid).
    if [[ "${SUPERVISOR_PGID:-}" == "$$" ]]; then kill -TERM "-$$" 2>/dev/null || true; fi
}
trap lh_stop TERM INT EXIT

# RPC readiness probe: a bounded eth_blockNumber that returns a plain integer.
lh_rpc_ready() {
    local n; n="$(lh_cast_status block-number --rpc-url "$RPC" 2>/dev/null)" || return 1
    [[ "$n" =~ ^[0-9]+$ ]]
}

# Bounded startup acquisition — the retry-don't-die contract. Polls, in order, the soak marker
# (if LH_WAIT_MARKER_LOG set), RPC readiness, and the funder key, logging each miss with its
# reason every LH_ACQUIRE_INTERVAL, LOUD-FAILing only after LH_ACQUIRE_TIMEOUT. On success sets
# FUNDER_KEY and sanity-checks its nonce is a readable integer before we ever send. Replaces
# both the die-on-first-miss funder read (2026-07-20 zero-load incident) and blaster-waiter.sh.
lh_acquire() {
    local start_ts elapsed reason addr fn
    start_ts="$SECONDS"; FUNDER_KEY=""
    while :; do
        elapsed=$(( SECONDS - start_ts )); reason=""
        if [[ -n "$LH_WAIT_MARKER_LOG" ]] && ! lh_log_has_marker "$LH_WAIT_MARKER_LOG"; then
            reason="soak marker '[soak r<N> ' not yet in $LH_WAIT_MARKER_LOG (DeployStaking incomplete — funding now would corrupt the shared dev-EOA nonce)"
        elif ! lh_rpc_ready; then
            reason="RPC $RPC not answering eth_blockNumber"
        elif ! FUNDER_KEY="$(lh_funder_key 2>/dev/null)"; then
            reason="funder key not readable (no LH_FUNDER_KEY and no live validator-0 funded.hex)"; FUNDER_KEY=""
        else
            addr="$(cast wallet address "$FUNDER_KEY" 2>/dev/null)" || addr=""
            fn="$(lh_cast_status nonce "$addr" --rpc-url "$RPC" 2>/dev/null)" || fn=""
            if [[ "$addr" == 0x* && "$fn" =~ ^[0-9]+$ ]]; then
                echo "load-heavy: acquisition OK after ${elapsed}s (funder=$addr nonce=$fn${LH_WAIT_MARKER_LOG:+, soak marker present})"
                return 0
            fi
            reason="funder $addr nonce not readable from $RPC yet (RPC half-up?)"; FUNDER_KEY=""
        fi
        if [[ "$(lh_acquire_verdict "$elapsed" "$LH_ACQUIRE_TIMEOUT")" != retry ]]; then
            echo "FAIL (load-heavy): startup acquisition timed out after ${elapsed}s (limit ${LH_ACQUIRE_TIMEOUT}s) — last reason: $reason" >&2
            return 1
        fi
        echo "load-heavy: waiting (${elapsed}s/${LH_ACQUIRE_TIMEOUT}s) — $reason; retry in ${LH_ACQUIRE_INTERVAL}s"
        sleep "$LH_ACQUIRE_INTERVAL"
    done
}

# One sender: fire-and-forget --async EIP-1559 sends (maxFee=LH_MAX_FEE, tip=LH_TIP; cast:
# `--gas-price` = maxFeePerGas once `--priority-gas-price` is given) under the
# lh_inflight_gate BACKPRESSURE WINDOW (wedge-2 fix). No gas-price machinery at all
# (wedge-3 fix): a capped-fee tx never needs repricing — when the base fee climbs past the
# cap the pending window WAITS and auto-mines on decay, which surfaces here as a frozen
# mined head + full window (skip ticks). That state is logged as a diagnostic, never
# "fixed" by re-sends: a same-nonce replacement must bid ≥ old×1.1 (up-only), so futile
# down-repricing is exactly the old wedge. Never exits on a failed send; every RPC call
# hard-timeout'd (wedge-1 fix).
lh_sender_loop() {   # $1=idx $2=private-key $3=sleep-seconds(string)
    local idx="$1" key="$2" pace="$3" addr out rc sent=0
    local mined=0 pending next inflight verdict mn stall=0
    addr=$(cast wallet address "$key")
    mn=$(lh_cast nonce "$addr" --rpc-url "$RPC" 2>/dev/null) || mn=""
    if [[ "$mn" =~ ^[0-9]+$ ]]; then mined="$mn"; fi
    pending=$(lh_cast nonce "$addr" --block pending --rpc-url "$RPC" 2>/dev/null) || pending=""
    [[ "$pending" =~ ^[0-9]+$ ]] || pending="$mined"
    # Inherited stuck window (pending > mined at START — impossible for a fresh-key fleet,
    # the normal case): likely legacy peak-priced txs from a pre-1559 run. Displacing them
    # needs a same-nonce bid ≥ old×1.1, which is exactly the unaffordable-rebid wedge — log
    # LOUDLY and let window accounting hold from the pending head instead of futile re-sends.
    if (( pending > mined )); then
        echo "load-heavy[sender-$idx]: $(( pending - mined )) inherited PENDING txs at start (mined=$mined pending=$pending) — likely stale legacy/peak-priced; replacement needs >=1.1x their bid. MANUAL TOPUP OR BASE-FEE DECAY REQUIRED; holding window accounting from the pending head, NOT re-sending."
    fi
    next="$pending"
    while :; do
        # Backpressure window: poll the mined head, derive the send-or-skip verdict.
        mn=$(lh_cast nonce "$addr" --rpc-url "$RPC" 2>/dev/null) || mn=""
        if [[ "$mn" =~ ^[0-9]+$ ]]; then
            if (( mn > mined )); then stall=0; fi   # head advanced → not stuck
            mined="$mn"
        fi
        read -r next verdict < <(lh_inflight_gate "$next" "$mined" "$LH_MAX_INFLIGHT") || verdict="skip"
        inflight=$(( next - mined ))
        echo "$inflight" > "$LH_TMP/inflight-$idx"
        if [[ "$verdict" == skip ]]; then
            stall=$(( stall + 1 ))
            if (( stall >= LH_UNWEDGE_SKIPS )); then
                # Window full AND mined frozen: base fee above LH_MAX_FEE (self-regulating
                # wait — clears on decay) or an inherited stuck head (see start log).
                # Diagnostic only; re-sending cannot help (up-only replacement rule).
                echo "load-heavy[sender-$idx]: window full, mined frozen $stall ticks — head-of-line waits (base fee > LH_MAX_FEE=$LH_MAX_FEE self-regulation, or an inherited stuck tx: topup/decay required)"
                stall=0
            fi
            sleep "$pace"; continue
        fi
        rc=0
        if [[ "$LH_MODE" == transfer ]]; then
            # Light mode: a 21000-gas ETH self-transfer (1 wei to own address), same 1559 caps.
            out=$(lh_cast send "$addr" --value 1 \
                    --rpc-url "$RPC" --private-key "$key" --nonce "$next" \
                    --gas-limit 21000 \
                    --gas-price "$LH_MAX_FEE" --priority-gas-price "$LH_TIP" --async 2>&1) || rc=$?
        else
            out=$(lh_cast send "$BURNER" "burn(uint256)" "$BURN_N" \
                    --rpc-url "$RPC" --private-key "$key" --nonce "$next" \
                    --gas-limit "$LH_TX_GAS" \
                    --gas-price "$LH_MAX_FEE" --priority-gas-price "$LH_TIP" --async 2>&1) || rc=$?
        fi
        if (( rc == 0 )); then
            next=$(( next + 1 )); sent=$(( sent + 1 ))
            echo "$sent"  > "$LH_TMP/sent-$idx"
            echo "$out"   > "$LH_TMP/last-$idx"     # tx hash (--async prints it bare)
        elif (( rc == 124 || rc == 137 )); then
            echo "load-heavy[sender-$idx]: rpc-timeout on send (>${LH_CAST_TIMEOUT}s) — skipping tick"
        elif lh_is_nonce_error "$out"; then
            pending=$(lh_cast nonce "$addr" --block pending --rpc-url "$RPC" 2>/dev/null) || pending=""
            if [[ "$pending" =~ ^[0-9]+$ ]]; then next="$pending"; fi   # re-derive from the pool's pending head
        else
            sleep 3                                  # RPC down / revert — back off, keep going
        fi
        sleep "$pace"
    done
}

echo "load-heavy: mode=$LH_MODE RPC=$RPC senders=$LH_SENDERS fraction=$LH_TARGET_GAS_FRACTION tx_gas=$LH_TX_GAS duration=${LH_DURATION:-0}s${LH_WAIT_MARKER_LOG:+ gate=$LH_WAIT_MARKER_LOG}"

# 0) acquisition GATE: block here (bounded, logged) until the soak marker + RPC + funder key
#    are all ready. Sets FUNDER_KEY. NOTHING below funds a tx before this returns.
lh_acquire || exit 1

# 1) per-sender pacing. burn: derive the aggregate tps from the live block gas limit so the
#    fleet fills fraction x limit gas per 1 s block. transfer: a FIXED aggregate LH_TPS
#    (default 25 tx/s) — steady light load, decoupled from the block gas budget. Both feed
#    the same interval = senders x 1000 / tps → PACE → per-sender sleep + inflight window.
if [[ "$LH_MODE" == burn ]]; then
    # block gas limit from the live head (block time is the harness's 1 s).
    GAS_LIMIT=$(( $(rpc_post_url "$RPC" "$(rpc_body eth_getBlockByNumber '["latest",false]')" | jq -r '.result.gasLimit') ))
    (( GAS_LIMIT > 0 )) || { echo "FAIL (load-heavy): cannot read block gasLimit from $RPC"; exit 1; }
    read -r LH_TPS LH_IVL_MS < <(lh_calibrate "$GAS_LIMIT" "$LH_TARGET_GAS_FRACTION" "$LH_SENDERS" "$LH_TX_GAS")
else
    LH_TPS="${LH_TPS:-25}"
    LH_IVL_MS=$(awk -v s="$LH_SENDERS" -v tps="$LH_TPS" 'BEGIN{ if (tps <= 0) { print 0; exit } printf "%d", (s * 1000) / tps }')
    (( LH_IVL_MS > 0 )) || { echo "FAIL (load-heavy): LH_TPS must be > 0 (got '$LH_TPS')"; exit 1; }
fi
(( LH_IVL_MS >= 50 )) || echo "WARN (load-heavy): per-sender interval ${LH_IVL_MS}ms < 50ms — cast send round-trips will under-shoot the target; raise LH_TX_GAS/LH_SENDERS (burn) or lower LH_TPS/LH_SENDERS (transfer)"

# 2) fund ephemeral senders from the dev key (sync sends → the funder nonce self-paces).
#    FUNDER_KEY was acquired + nonce-sanity-checked by lh_acquire above.
declare -a SENDER_KEYS=()
for (( i = 0; i < LH_SENDERS; i++ )); do
    w=$(cast wallet new --json)
    k=$(jq -r '.[0].private_key' <<<"$w"); a=$(jq -r '.[0].address' <<<"$w")
    lh_cast_init send "$a" --value "$LH_FUND_WEI" --rpc-url "$RPC" --private-key "$FUNDER_KEY" >/dev/null \
        || { echo "FAIL (load-heavy): funding sender $i timed out/failed (RPC congested? raise LH_INIT_TIMEOUT)"; exit 1; }
    SENDER_KEYS+=("$k")
    echo "load-heavy: sender $i = $a funded ($LH_FUND_WEI wei)"
done

# 3) burn only: deploy the vendored GasBurner once (PrevRandaoProbe.json deploy pattern), then
#    empirically calibrate burn(n) — transfer mode's 21000-gas self-sends need neither.
if [[ "$LH_MODE" == burn ]]; then
    BURNER_BC=$(jq -r '.bytecode.object' contracts/GasBurner.json)
    BURNER=$(lh_cast_init send --private-key "$FUNDER_KEY" --rpc-url "$RPC" --create "$BURNER_BC" --json | jq -r '.contractAddress') || BURNER=""
    [[ "$BURNER" == 0x* ]] || { echo "FAIL (load-heavy): GasBurner deploy failed"; exit 1; }
    echo "load-heavy: GasBurner deployed at $BURNER"

    # empirical gas calibration (two estimates → per-iteration slope + intercept; immune to
    # gas-schedule drift). Probe points stay SMALL (16/116, Δ100 ≈ 2.6M gas) — the fluent
    # node's eth_estimateGas has an rWASM fuel cap and a big probe (e.g. n=1016 ≈ 22.6M)
    # fails with `EVM error: OutOfFuel` (observed live). Falls back to the SSTORE-schedule
    # constants on estimate failure.
    E1=$(lh_cast estimate "$BURNER" "burn(uint256)" 16  --rpc-url "$RPC" 2>/dev/null) || E1=""
    E2=$(lh_cast estimate "$BURNER" "burn(uint256)" 116 --rpc-url "$RPC" 2>/dev/null) || E2=""
    if [[ "$E1" =~ ^[0-9]+$ && "$E2" =~ ^[0-9]+$ ]] && (( E2 > E1 )); then
        GAS_PER_ITER=$(( (E2 - E1) / 100 )); GAS_BASE=$(( E1 - 16 * GAS_PER_ITER ))
    else
        GAS_PER_ITER=22300; GAS_BASE=50000
    fi
    BURN_N=$(lh_burn_iters "$LH_TX_GAS" "$GAS_BASE" "$GAS_PER_ITER")
fi

PACE=$(awk -v m="$LH_IVL_MS" 'BEGIN{printf "%.3f", m / 1000}')
if [[ "$LH_MODE" == burn ]]; then
    echo "load-heavy: gas_limit=$GAS_LIMIT target=${LH_TPS} tx/s total, burn($BURN_N)/tx (base=$GAS_BASE per_iter=$GAS_PER_ITER), per-sender pace ${PACE}s"
else
    echo "load-heavy: transfer mode — 21000-gas ETH self-sends, target ${LH_TPS} tx/s total, per-sender pace ${PACE}s"
fi

# 5) SUPERVISOR: launch the fleet, write the pidfile, own the foreground — restart any sender
#    that dies (counter + exponential backoff, flap-reset after a healthy run) and emit the
#    heartbeat every LH_STATUS_EVERY. The old version had NO supervision: a silently-dead
#    sender just stopped contributing load with nobody restarting it.
declare -a SENDER_RESTARTS=() SENDER_STARTED=() SENDER_NEXTTRY=()
lh_launch_sender() {   # <idx> — (re)launch and record its pid + start tick
    local idx="$1"
    lh_sender_loop "$idx" "${SENDER_KEYS[$idx]}" "$PACE" &
    LH_PIDS[$idx]=$!
    SENDER_STARTED[$idx]=$SECONDS
}
for (( i = 0; i < LH_SENDERS; i++ )); do
    SENDER_RESTARTS[$i]=0; SENDER_NEXTTRY[$i]=0
    lh_launch_sender "$i"
done

# Pidfile: "<pid> <pgid> <start_epoch>" — the clean-stop contract (see `stop` subcommand).
PIDFILE="$(lh_pidfile_path)"
SUPERVISOR_PGID="$(ps -o pgid= -p $$ 2>/dev/null | tr -d ' ')"
[[ "$SUPERVISOR_PGID" =~ ^[0-9]+$ ]] || SUPERVISOR_PGID=$$
printf '%s %s %s\n' "$$" "$SUPERVISOR_PGID" "$(date +%s)" > "$PIDFILE"
echo "load-heavy: $LH_SENDERS sender loops up (pids: ${LH_PIDS[*]}); supervisor pid=$$ pgid=$SUPERVISOR_PGID pidfile=$PIDFILE"
echo "load-heavy: STOP WITH 'scripts/load-heavy.sh stop $PIDFILE' (or SIGTERM the supervisor) — NEVER pkill (self-match traps + orphaned cast children)"

DEADLINE=$(( LH_DURATION > 0 ? SECONDS + LH_DURATION : 0 ))
LAST_STATUS=0
while (( DEADLINE == 0 || SECONDS < DEADLINE )); do
    # a) supervise: restart dead senders with counter + backoff (flap-reset once a sender has
    #    stayed up LH_RESTART_HEALTHY_SECS so a single late death never inherits a huge backoff).
    for (( i = 0; i < LH_SENDERS; i++ )); do
        if kill -0 "${LH_PIDS[$i]}" 2>/dev/null; then continue; fi   # alive
        if (( SECONDS - SENDER_STARTED[i] >= LH_RESTART_HEALTHY_SECS )); then SENDER_RESTARTS[$i]=0; fi
        if (( SECONDS < SENDER_NEXTTRY[i] )); then continue; fi       # still in backoff
        SENDER_RESTARTS[$i]=$(( SENDER_RESTARTS[i] + 1 ))
        bo="$(lh_restart_backoff "${SENDER_RESTARTS[$i]}")"
        SENDER_NEXTTRY[$i]=$(( SECONDS + bo ))
        echo "load-heavy: sender-$i died — restart #${SENDER_RESTARTS[$i]}, next backoff ${bo}s"
        lh_launch_sender "$i"
    done

    # b) heartbeat every LH_STATUS_EVERY (the supervisor ticks faster than that for snappy
    #    restarts; the status line stays on its own cadence).
    if (( SECONDS - LAST_STATUS >= LH_STATUS_EVERY )); then
        LAST_STATUS=$SECONDS
        total=0; infl=0
        for (( i = 0; i < LH_SENDERS; i++ )); do
            v=$(cat "$LH_TMP/sent-$i" 2>/dev/null) || v=0
            if [[ "$v" =~ ^[0-9]+$ ]]; then total=$(( total + v )); fi   # `if`, not `&&`-tail (set -e)
            v=$(cat "$LH_TMP/inflight-$i" 2>/dev/null) || v=0
            if [[ "$v" =~ ^[0-9]+$ ]]; then infl=$(( infl + v )); fi
        done
        basefee=$(lh_cast_status base-fee --rpc-url "$RPC" 2>/dev/null) || basefee="rpc-timeout"
        [[ -n "$basefee" ]] || basefee="rpc-timeout"
        # HEARTBEAT: prints EVERY interval no matter what — all chain reads here are hard-
        # timeout'd and degrade to markers, so an external log-staleness watchdog only fires
        # when this script is genuinely dead, never when the RPC is merely slow. `cast receipt`
        # MUST carry --async: without it cast POLLS until the tx mines — sampling the newest
        # (likely still-pending) --async send, that poll was the status-printer wedge.
        sample="-"
        h=$(cat "$LH_TMP/last-0" 2>/dev/null) || h=""
        if [[ "$h" == 0x* ]]; then
            rc=0
            sample=$(lh_cast_status receipt --async "$h" status --rpc-url "$RPC" 2>/dev/null | tr -d '[:space:]') || rc=$?
            if (( rc == 124 || rc == 137 )); then sample="rpc-timeout"
            elif (( rc != 0 )) || [[ -z "$sample" ]]; then sample="pending"; fi
        fi
        head_now=$(lh_cast_status block-number --rpc-url "$RPC" 2>/dev/null) || head_now="rpc-timeout"
        [[ -n "$head_now" ]] || head_now="rpc-timeout"
        lh_status_line "$LH_MODE" "$total" "$infl" "$(( LH_SENDERS * LH_MAX_INFLIGHT ))" \
            "$basefee" "$LH_MAX_FEE" "$sample" "$head_now"
    fi
    sleep "$LH_SUPERVISE_TICK"
done
echo "load-heavy: duration reached — stopping senders"
