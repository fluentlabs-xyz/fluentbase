#!/usr/bin/env bash
# soak-status.sh — one-screen, READ-ONLY status of the DPoS production soak.
#
# Answers "how is the soak doing?" without asking anyone: which log it read, whether a
# soak is running or ended (and if ended, passed or failed with the reason + bundle),
# how far it got, how fast it is going, how many invariant failures there are (the
# headline), which anomalies are active, whether the load blaster is alive, and host
# health.
#
# READ-ONLY BY CONSTRUCTION: it only reads files, /proc, and runs bounded `docker ps`
# style probes. It never stops, restarts, kills, or writes anything.
#
# NOTE ON `set -e`: deliberately NOT enabled. This is a diagnostic tool whose whole job
# is to degrade per-section ("n/a (reason)") when a probe is unavailable — an errexit
# abort halfway through the report would be the opposite of useful. `set -u` catches
# genuine typos. Because there is no errexit, the project's `(( cond )) && cmd` hazard
# (a false arithmetic test making a branch-tail return non-zero) cannot abort us; even
# so every conditional below is written as an explicit `if` to match house style.
#
# Usage:  scripts/soak-status.sh [LOGFILE] [--watch [SECONDS]]
set -u

VERSION="1.0"
SOAK_DATA_ROOT_DEFAULT="/mnt/storage/fluent-soak"
REPO_SMOKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." 2>/dev/null && pwd || echo "")"

usage() {
    cat <<'EOF'
soak-status.sh — one-screen, read-only status of the DPoS production soak.

USAGE
  scripts/soak-status.sh [LOGFILE] [--watch [SECONDS]]

ARGUMENTS
  LOGFILE            Soak log to read. If omitted, discovered automatically (see below).

OPTIONS
  --watch [SECONDS]  Re-render every SECONDS (default 10) until Ctrl-C.
  -h, --help         This help.

LOG DISCOVERY ORDER
  1. the LOGFILE argument
  2. $SOAK_LOG
  3. the stdout of the RUNNING soak process (/proc/<pid>/fd/1)
  4. the newest geo-soak.log / *soak*.log under /tmp/claude-*/ or the repo's
     devnet/local-dpos-smoke/
  Exits non-zero if none of these yields a readable log.

ENVIRONMENT
  SOAK_LOG           Explicit log path (overridden by the LOGFILE argument).
  SOAK_DATA_ROOT     Soak data root for the disk report (default /mnt/storage/fluent-soak).

Read-only: never kills, restarts, or modifies anything.
EOF
}

# ── tiny helpers ────────────────────────────────────────────────────────────────────
WIDTH="$( { tput cols 2>/dev/null || echo 100; } )"
if ! [[ "$WIDTH" =~ ^[0-9]+$ ]] || (( WIDTH < 60 )); then WIDTH=100; fi
rule() { printf '%*s\n' "$WIDTH" '' | tr ' ' '-'; }
# Truncate a line to the terminal width so one long reason cannot wrap the screen away.
clip() { cut -c "1-$WIDTH"; }
# Human-readable duration from seconds.
dur() {
    local s="${1:-0}"
    if ! [[ "$s" =~ ^[0-9]+$ ]]; then echo "n/a"; return 0; fi
    local d=$(( s / 86400 )) h=$(( (s % 86400) / 3600 )) m=$(( (s % 3600) / 60 ))
    if (( d > 0 )); then printf '%dd %dh %dm\n' "$d" "$h" "$m"
    elif (( h > 0 )); then printf '%dh %dm\n' "$h" "$m"
    else printf '%dm\n' "$m"; fi
}
# Count matching LINES, always emitting exactly one integer.
# NOTE: no `|| echo 0` here — `grep -c` already PRINTS "0" on no match and THEN exits 1, so a
# `|| echo 0` fallback appends a second "0" and yields the two-line string "0\n0", which then
# blows up every `(( ))` that consumes it. Validate the captured value instead.
_count1() {
    local n; n="$("$@" 2>/dev/null)"
    if [[ "$n" =~ ^[0-9]+$ ]]; then printf '%s' "$n"; else printf '0'; fi
}
countre() { _count1 grep -cE -- "$1" "$2"; }
countf()  { _count1 grep -cF -- "$1" "$2"; }

# ── log discovery ───────────────────────────────────────────────────────────────────
# Sets LOG + LOG_SRC (how it was found). Returns 1 if nothing readable was found.
SOAK_PID=""
discover_running_pid() {
    # The soak is `make smoke-soak` -> bash scripts/case-soak.sh. Bracketed patterns so a
    # shell whose argv happens to contain the string is not mistaken for the soak itself.
    local p
    for p in '[c]ase-soak\.sh' '[s]moke-soak'; do
        SOAK_PID="$(pgrep -f "$p" 2>/dev/null | head -1)"
        if [[ -n "$SOAK_PID" ]]; then return 0; fi
    done
    SOAK_PID=""
    return 1
}
discover_log() {
    LOG=""; LOG_SRC=""
    if [[ -n "${ARG_LOG:-}" ]]; then
        LOG="$ARG_LOG"; LOG_SRC="argument"
        if [[ -r "$LOG" ]]; then return 0; fi
        echo "soak-status: log '$LOG' (from argument) is not readable" >&2; return 1
    fi
    if [[ -n "${SOAK_LOG:-}" && -r "${SOAK_LOG:-}" ]]; then
        LOG="$SOAK_LOG"; LOG_SRC="\$SOAK_LOG"; return 0
    fi
    # Running soak's stdout. fd/1 may be a pipe (piped into tee) — only accept a real file.
    if [[ -n "$SOAK_PID" ]]; then
        local t
        t="$(readlink -f "/proc/$SOAK_PID/fd/1" 2>/dev/null || echo "")"
        if [[ -n "$t" && -f "$t" && -r "$t" ]]; then
            LOG="$t"; LOG_SRC="running soak pid $SOAK_PID (/proc/$SOAK_PID/fd/1)"; return 0
        fi
    fi
    # Newest candidate on disk.
    local newest
    newest="$( { find /tmp/claude-* "$REPO_SMOKE_DIR" -maxdepth 6 \
                   \( -name 'geo-soak.log' -o -name '*soak*.log' \) -type f 2>/dev/null; } \
                 | xargs -r ls -t 2>/dev/null | head -1 )"
    if [[ -n "$newest" && -r "$newest" ]]; then
        LOG="$newest"; LOG_SRC="newest *soak*.log on disk"; return 0
    fi
    return 1
}

# ── section: header + log ───────────────────────────────────────────────────────────
render_log_section() {
    local sz mt age
    sz="$( { du -h "$LOG" 2>/dev/null || echo '?'; } | awk '{print $1}')"
    mt="$(stat -c %Y "$LOG" 2>/dev/null || echo 0)"
    age=$(( NOW - mt )); if (( age < 0 )); then age=0; fi
    printf 'log      : %s\n' "$LOG"
    printf '           via %s | %s | last write %s ago\n' "$LOG_SRC" "$sz" "$(dur "$age")"
}

# ── section: state (running / ended-passed / ended-failed) ──────────────────────────
render_state_section() {
    local failed passed
    failed="$(countf 'SOAK FAILURE BUNDLE' "$LOG")"
    passed="$(countf 'soak finished cleanly' "$LOG")"
    if [[ -n "$SOAK_PID" ]]; then
        local et
        et="$(ps -o etimes= -p "$SOAK_PID" 2>/dev/null | tr -d ' ')"
        if ! [[ "$et" =~ ^[0-9]+$ ]]; then et=""; fi
        printf 'state    : RUNNING (pid %s, up %s)\n' "$SOAK_PID" "$(dur "${et:-x}")"
        # A run can log a failure and still be tearing down; surface that immediately.
        if (( failed > 0 )); then
            printf '           NOTE: a SOAK FAILURE block is already present in this log (teardown in progress?)\n'
        fi
        return 0
    fi
    if (( failed > 0 )); then
        printf 'state    : ENDED - FAILED\n'
        local inv reason bundle seed
        inv="$(   grep -F 'broken invariant :' "$LOG" 2>/dev/null | tail -1 | sed 's/.*: *//')"
        reason="$(grep -F 'reason           :' "$LOG" 2>/dev/null | tail -1 | sed 's/^reason  *: *//')"
        seed="$(  grep -F 'seed             :' "$LOG" 2>/dev/null | tail -1 | sed 's/.*: *//')"
        # The bundle PATH is the `bundle:` line. Do NOT scrape a bundle-* token out of the
        # reason text: reasons routinely cite a HISTORICAL bundle as evidence, which is a
        # different directory from this run's.
        bundle="$(grep -E '^bundle: ' "$LOG" 2>/dev/null | tail -1 | sed 's/^bundle: *//')"
        printf '  invariant: %s\n' "${inv:-n/a}"
        # The reason is the single most useful line here, so WRAP it (indented) instead of
        # hard-clipping — a truncated reason is what sends people back to grepping the log.
        printf '  reason   : %s\n' "${reason:-n/a}" \
            | fold -s -w "$(( WIDTH - 2 ))" | sed '2,$s/^/             /'
        printf '  bundle   : %s\n' "${bundle:-n/a (no bundle: line)}"
        printf '  seed     : %s\n' "${seed:-n/a}"
    elif (( passed > 0 )); then
        printf 'state    : ENDED - PASSED\n'
        printf '  %s\n' "$(grep -F 'soak finished cleanly' "$LOG" 2>/dev/null | tail -1)"
    else
        printf 'state    : ENDED - INCOMPLETE (no failure block and no clean-finish line)\n'
        printf '           the run was interrupted (Ctrl-C / host reboot / OOM) or is mid-teardown\n'
    fi
}

# ── section: progress + rate ────────────────────────────────────────────────────────
# Event lines look like:  "  [soak r651 invariant_fail] fin=27020 epoch=222 f=2 <msg>"
render_progress_section() {
    local last first round fin epoch f
    last="$(grep -E '^[[:space:]]*\[soak r[0-9]+ ' "$LOG" 2>/dev/null | tail -1)"
    if [[ -z "$last" ]]; then
        printf 'progress : n/a (no [soak r<N> ...] event lines in this log)\n'
        return 0
    fi
    round="$(sed -nE 's/.*\[soak r([0-9]+) .*/\1/p' <<<"$last")"
    # `fin=` then the FIRST `epoch=` after it is the state epoch (the start line carries a
    # second epoch= that is the epoch INTERVAL, not the current epoch).
    fin="$(  sed -nE 's/.*\bfin=([0-9]+).*/\1/p' <<<"$last")"
    epoch="$(sed -nE 's/.*\bfin=[0-9]+ epoch=([0-9]+).*/\1/p' <<<"$last")"
    f="$(    sed -nE 's/.*\bepoch=[0-9]+ f=([0-9]+).*/\1/p' <<<"$last")"
    printf 'progress : round %s | epoch %s | finalized %s | f=%s\n' \
        "${round:-?}" "${epoch:-?}" "${fin:-?}" "${f:-?}"

    # Elapsed: a running soak uses its process age; an ended one uses the log's
    # birth->last-write span (stat %W is available on this filesystem; 0 means unsupported).
    local birth mt elapsed=""
    birth="$(stat -c %W "$LOG" 2>/dev/null || echo 0)"
    mt="$(stat -c %Y "$LOG" 2>/dev/null || echo 0)"
    if [[ -n "$SOAK_PID" ]]; then
        elapsed="$(ps -o etimes= -p "$SOAK_PID" 2>/dev/null | tr -d ' ')"
    elif [[ "$birth" =~ ^[0-9]+$ ]] && (( birth > 0 && mt > birth )); then
        elapsed=$(( mt - birth ))
    fi
    if ! [[ "${elapsed:-}" =~ ^[0-9]+$ ]]; then elapsed=""; fi

    if [[ -n "$elapsed" ]]; then
        local started="n/a"
        if (( birth > 0 )); then started="$(date -d "@$birth" '+%Y-%m-%d %H:%M' 2>/dev/null || echo n/a)"; fi
        printf 'runtime  : %s (log opened %s)\n' "$(dur "$elapsed")" "$started"
    else
        printf 'runtime  : n/a (no process age and no usable log birth time)\n'
    fi

    # Rate. Prefer the harness's own windowed measurement (it already excludes teardown
    # gaps); also show the whole-run average, which is the honest long-horizon number.
    local rate_line rate="" bpm="" first_fin first_epoch avg_bpm="" eph=""
    rate_line="$(grep -oE 'rate=[0-9.]+ blk/s' "$LOG" 2>/dev/null | tail -1)"
    if [[ -n "$rate_line" ]]; then
        rate="${rate_line#rate=}"; rate="${rate% blk/s}"
        bpm="$(awk -v r="$rate" 'BEGIN{printf "%.0f", r*60}')"
    fi
    first="$(grep -E '^[[:space:]]*\[soak r[0-9]+ ' "$LOG" 2>/dev/null | head -1)"
    first_fin="$(  sed -nE 's/.*\bfin=([0-9]+).*/\1/p' <<<"$first")"
    first_epoch="$(sed -nE 's/.*\bfin=[0-9]+ epoch=([0-9]+).*/\1/p' <<<"$first")"
    if [[ -n "$elapsed" ]] && (( elapsed > 0 )) \
       && [[ "${fin:-}" =~ ^[0-9]+$ && "${first_fin:-}" =~ ^[0-9]+$ ]]; then
        avg_bpm="$(awk -v a="$first_fin" -v b="$fin" -v s="$elapsed" 'BEGIN{printf "%.0f", (b-a)*60/s}')"
    fi
    if [[ -n "$elapsed" ]] && (( elapsed > 0 )) \
       && [[ "${epoch:-}" =~ ^[0-9]+$ && "${first_epoch:-}" =~ ^[0-9]+$ ]]; then
        eph="$(awk -v a="$first_epoch" -v b="$epoch" -v s="$elapsed" 'BEGIN{printf "%.1f", (b-a)*3600/s}')"
    fi
    printf 'rate     : %s blk/s recent (~%s blk/min) | %s blk/min avg | %s epochs/h observed\n' \
        "${rate:-n/a}" "${bpm:-n/a}" "${avg_bpm:-n/a}" "${eph:-n/a}"
}

# ── section: the headline — invariant failures ──────────────────────────────────────
render_headline_section() {
    local nfail nbundle nwarn nWARN ntrip nleak
    nfail="$(countE_inv)"
    nbundle="$(countf 'SOAK FAILURE BUNDLE' "$LOG")"
    nwarn="$(countre '\[soak r[0-9]+ warn\]' "$LOG")"
    nWARN="$(countre '\[soak r[0-9]+ WARN\]' "$LOG")"
    ntrip="$(countf 'SHRANK' "$LOG")"
    nleak="$(countre 'leaked|leak-a|LEAK' "$LOG")"
    rule
    if (( nfail == 0 && nbundle == 0 )); then
        printf 'HEADLINE : 0 invariant failures\n'
    else
        printf 'HEADLINE : %s invariant failure(s), %s failure bundle(s)\n' "$nfail" "$nbundle"
    fi
    printf 'counts   : invariant_fail=%s  failure-bundle=%s  warn=%s  WARN=%s  tripwire/SHRANK=%s  leak=%s\n' \
        "$nfail" "$nbundle" "$nwarn" "$nWARN" "$ntrip" "$nleak"
    if (( nfail > 0 )); then
        printf 'last failure(s):\n'
        grep -E '\[soak r[0-9]+ invariant_fail\]' "$LOG" 2>/dev/null | tail -3 \
            | sed 's/^[[:space:]]*/  /' | clip
    fi
}
countE_inv() { countre '\[soak r[0-9]+ invariant_fail\]' "$LOG"; }

# ── section: active anomalies ───────────────────────────────────────────────────────
# Each row is "<label>|<grep -E pattern>". Benign-by-design rows are marked so a healthy
# run does not read as alarming (e.g. "warm-debt cleared" is a RECOVERY, not a problem).
render_anomalies_section() {
    local rows row label pat n any=0
    rows=(
        "dkg-deferral|dkg-deferral"
        "promote blocked/no-host|promote_blocked|promote_deferred_no_host|nohost"
        "promote warming (deferred)|promote_deferred_warming"
        "stalled_with_peers|stalled_with_peers"
        "carry_forward_refused|carry_forward_refused"
        "marker_reject|marker_reject"
        "self-partition|self_partition|SELF_PARTITIONED"
        "warm-debt HELD (bad)|warm-debt-stall|warm-debt HELD"
        "battery-coverage|battery-coverage|battery_coverage"
        "result divergence|result-diverg|result_diverg|SafetyHalt"
        "spec-lag degraded|spec_lag_degraded_rate"
        "in-flight reconcile|in-flight RECONCILE"
    )
    printf '\nanomalies (0 = not seen):\n'
    for row in "${rows[@]}"; do
        label="${row%%|*}"; pat="${row#*|}"
        n="$(countre "$pat" "$LOG")"
        if (( n > 0 )); then any=1; fi
        printf '  %-26s %s\n' "$label" "$n"
    done
    # Recoveries, shown separately so they are never mistaken for faults.
    printf '  %-26s %s  (benign: recoveries)\n' "warm-debt cleared" "$(countf 'warm-debt cleared' "$LOG")"
    if (( any == 0 )); then printf '  (none of the watched anomaly classes appear in this log)\n'; fi
}

# ── section: load blaster ───────────────────────────────────────────────────────────
render_blaster_section() {
    local pids nproc status lhlog
    # Bracketed pattern: never match the pgrep/our own argv (the harness's documented
    # self-match trap; here it would only mis-COUNT, but the discipline is the same).
    pids="$(pgrep -f '[l]oad-heavy\.sh' 2>/dev/null || true)"
    nproc="$(printf '%s' "$pids" | grep -c . 2>/dev/null)"; if ! [[ "$nproc" =~ ^[0-9]+$ ]]; then nproc=0; fi
    lhlog="$( { find /tmp/claude-* "$REPO_SMOKE_DIR" "$(dirname "$LOG")" -maxdepth 4 \
                  -name '*load-heavy*.log' -type f 2>/dev/null; } \
                | xargs -r ls -t 2>/dev/null | head -1 )"
    status=""
    if [[ -n "$lhlog" && -r "$lhlog" ]]; then
        status="$(tail -n 400 "$lhlog" 2>/dev/null | grep -oE 'sent=[0-9]+ inflight=[0-9]+(/[0-9]+)?' | tail -1)"
    fi
    # Distinguish "no log at all" from "log found but it carries no status line" — the old
    # single fallback string claimed "no load-heavy log found" while printing the log path.
    local note
    if [[ -z "$lhlog" ]]; then note="no load-heavy log found"
    elif [[ -z "$status" ]]; then note="log found but no sent=/inflight= status line in its tail"
    else note="$status"; fi
    if (( nproc > 0 )); then
        printf 'blaster  : RUNNING (%s proc) | %s\n' "$nproc" "$note"
    else
        printf 'blaster  : not running (0 proc) | %s\n' "$note"
    fi
    if [[ -n "$lhlog" ]]; then printf '           log %s\n' "$lhlog"; fi
}

# ── section: host health ────────────────────────────────────────────────────────────
render_host_section() {
    local ram droot
    ram="$(free -g 2>/dev/null | awk '/^Mem:/{printf "%s/%s GB used (%d%%)", $3, $2, ($2>0? $3*100/$2 : 0)}')"
    printf 'host     : RAM %s\n' "${ram:-n/a}"
    df -h / 2>/dev/null | awk 'NR==2{printf "           disk /                    %s used of %s (%s free)\n", $5, $2, $4}'
    droot="${SOAK_DATA_ROOT:-$SOAK_DATA_ROOT_DEFAULT}"
    if [[ -d "$droot" ]]; then
        df -h "$droot" 2>/dev/null \
            | awk -v d="$droot" 'NR==2{printf "           disk %-20s %s used of %s (%s free)\n", d, $5, $2, $4}'
    else
        printf '           disk %-20s n/a (path does not exist)\n' "$droot"
    fi
    local ndock
    # Same grep -c caveat as _count1; a missing/hung docker leaves a non-numeric value → n/a.
    ndock="$(timeout 3 docker ps -q 2>/dev/null | grep -c . 2>/dev/null)"
    if ! [[ "$ndock" =~ ^[0-9]+$ ]]; then ndock="n/a (docker unavailable or timed out)"; fi
    printf '           docker %s container(s) running\n' "$ndock"
}

# ── render ──────────────────────────────────────────────────────────────────────────
render() {
    NOW="$(date +%s)"
    printf 'soak-status v%s  —  %s\n' "$VERSION" "$(date '+%Y-%m-%d %H:%M:%S')"
    rule
    render_log_section
    render_state_section
    printf '\n'
    render_progress_section
    render_headline_section
    render_anomalies_section
    printf '\n'
    render_blaster_section
    render_host_section
}

# ── argv ────────────────────────────────────────────────────────────────────────────
ARG_LOG=""; WATCH=0; WATCH_SECS=10
while (( $# > 0 )); do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        --watch)
            WATCH=1
            if [[ "${2:-}" =~ ^[0-9]+$ ]]; then WATCH_SECS="$2"; shift; fi
            ;;
        -*) echo "soak-status: unknown option '$1' (try --help)" >&2; exit 2 ;;
        *)  if [[ -z "$ARG_LOG" ]]; then ARG_LOG="$1"; else
                echo "soak-status: unexpected extra argument '$1' (try --help)" >&2; exit 2
            fi ;;
    esac
    shift
done

discover_running_pid || true
if ! discover_log; then
    # An explicit-but-unreadable argument already reported itself precisely; the generic
    # "where I looked" block would only add noise to a path the user chose deliberately.
    if [[ -n "$ARG_LOG" ]]; then exit 1; fi
    echo "soak-status: no soak log found." >&2
    echo "  looked at: the LOGFILE argument, \$SOAK_LOG, the running soak's /proc/<pid>/fd/1," >&2
    echo "  and the newest geo-soak.log / *soak*.log under /tmp/claude-*/ and $REPO_SMOKE_DIR." >&2
    echo "  Pass one explicitly:  scripts/soak-status.sh /path/to/geo-soak.log" >&2
    if [[ -n "$SOAK_PID" ]]; then
        echo "  (a soak IS running as pid $SOAK_PID, but its stdout is a pipe, not a file --" >&2
        echo "   re-run the soak with '| tee somewhere.log', or pass the log path.)" >&2
    fi
    exit 1
fi

if (( WATCH == 1 )); then
    while :; do
        clear 2>/dev/null || true
        render
        printf '\n(--watch %ss; Ctrl-C to stop)\n' "$WATCH_SECS"
        sleep "$WATCH_SECS" || break
        discover_running_pid || true
    done
else
    render
fi
