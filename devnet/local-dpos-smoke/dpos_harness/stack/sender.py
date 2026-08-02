"""sender.py — heavy-load blaster lifecycle. Python port of the reworked scripts/load-heavy.sh.

The load-GENERATION half of the execution-saturation observability pair (the measurement half is
the battery's exec-saturation / block-rate watches). Deploys the vendored SSTORE-heavy GasBurner,
funds LOAD_SENDERS ephemeral keys from the harness dev key, then runs per-sender async-send loops
with locally-tracked nonces, paced so LOAD_SENDERS×rate×LOAD_TX_GAS ≈ fraction×block_gas_limit / 1s.

LIFECYCLE (self-supervising, one process):
  start     — launch; writes a pidfile "<pid> <pgid> <start_epoch>".
  wait      — LOAD_WAIT_MARKER_LOG gates ALL funding behind that log's first run-started marker
              (DeployStaking-complete; incidents v51/v54/v55). Bounded retry/backoff. The marker
              PATTERN is LOAD_WAIT_MARKER_RE, defaulting to core.events.HEARTBEAT_MARKER_RE — this
              module holds no consumer's log format (C9).
  supervise — restart any sender that dies (counter + exponential backoff, flap-reset), emit the
              `sent=/inflight=` heartbeat every LOAD_STATUS_EVERY.
  stop      — pidfile-based: identity-verify by pid+etime, TERM the group, KILL -9 fallback.
  NEVER pkill this process (self-match traps + orphaned cast children) — always `stop`.

PORT-NOTE: bash tracked PIDs in an array + used `setsid` for a killable process group; the port
uses threads for the sender loops (one supervisor process) and a pidfile for cross-process stop, so
the pkill self-match hazard cannot exist (we own our own thread handles). The awk-based pure seams
below are byte-faithful arithmetic (§20 gate tests pin them).
"""

from __future__ import annotations

import json
import math
import os
import threading
import time

from ..core import events, proc, topology

# knobs (all keep their LOAD_* names + defaults)
LOAD_CAST_TIMEOUT = int(os.environ.get("LOAD_CAST_TIMEOUT", "15"))
LOAD_STATUS_TIMEOUT = int(os.environ.get("LOAD_STATUS_TIMEOUT", "5"))
LOAD_RESTART_BACKOFF_BASE = int(os.environ.get("LOAD_RESTART_BACKOFF_BASE", "2"))
LOAD_RESTART_BACKOFF_MAX = int(os.environ.get("LOAD_RESTART_BACKOFF_MAX", "60"))

_NONCE_ERR_RE = None


def lh_burner_bytecode():
    """The GasBurner deploy bytecode. LOAD_BURNER_BYTECODE overrides (tests stub it); otherwise read
    the vendored fixture's `.bytecode.object`, exactly as load-heavy.sh:448 does
    (`jq -r '.bytecode.object' contracts/GasBurner.json`). PORT-FIX: the port used to default this to
    "0x" and NEVER loaded the fixture — burn mode then `--create 0x`'d an empty contract, got no
    contractAddress, and aborted the whole load-blaster with a zero-load run (v61 a12)."""
    env = os.environ.get("LOAD_BURNER_BYTECODE", "")
    if env:
        return env
    # DEPTH-SENSITIVE: <smoke>/dpos_harness/stack/sender.py → three dirnames to the smoke dir.
    smoke_dir = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    path = os.path.join(smoke_dir, "contracts", "GasBurner.json")
    try:
        with open(path) as f:
            return json.load(f).get("bytecode", {}).get("object", "") or "0x"
    except (OSError, ValueError, KeyError):
        return "0x"


# ── pure seams (gate-testable without a chain — soak-gate-test.sh §20) ────────

def lh_calibrate(gas_limit, fraction, senders, tx_gas):
    """<block_gas_limit> <fraction> <senders> <tx_gas> → (total_tps, per_sender_interval_ms).
    Fleet burns fraction×limit gas per 1s block; interval = per-sender pacing for that aggregate.
    Raises ValueError (bash: echo "0 0", rc 1) on any non-positive input."""
    g, fr, s, t = float(gas_limit), float(fraction), float(senders), float(tx_gas)
    if g <= 0 or fr <= 0 or s <= 0 or t <= 0:
        raise ValueError("lh_calibrate: non-positive input")
    tps = g * fr / t
    # bash: printf "%.3f %d", tps, (s*1000)/tps  — integer truncation on the interval.
    return (round(tps, 3), int((s * 1000) / tps))


def lh_burn_iters(tx_gas, base_gas, gas_per_iter):
    """burn(n) count filling 90% of the tx gas budget (10% headroom vs estimate drift); min 1.
    bash: n = int((0.9*t - b)/p); if n<1 n=1."""
    p = gas_per_iter if gas_per_iter else 1
    n = int((0.9 * float(tx_gas) - float(base_gas)) / float(p))
    return n if n >= 1 else 1


def lh_is_nonce_error(stderr: str) -> bool:
    """0 = nonce-desync (re-sync from RPC), 1 = anything else (RPC down / fee spike / revert)."""
    global _NONCE_ERR_RE
    if _NONCE_ERR_RE is None:
        import re
        _NONCE_ERR_RE = re.compile(
            r"nonce too (low|high)|invalid nonce|already known|replacement transaction",
            re.IGNORECASE)
    return bool(_NONCE_ERR_RE.search(stderr or ""))


def lh_inflight_gate(nxt, mined, window):
    """Backpressure-window verdict: <next> <mined> <window> → (reconciled_next, "send"|"skip").
    The local nonce DERIVES from the mined head (never trails it — external reality wins); at/over
    the window the tick is SKIPPED with NO local nonce bump, so a runaway past the pool's per-account
    pending cap is structurally impossible (2026-07-17 forensics)."""
    n, m, w = int(nxt), int(mined), int(window)
    if n < m:
        n = m
    return (n, "skip" if n - m >= w else "send")


def lh_acquire_verdict(elapsed, timeout):
    """<elapsed_s> <timeout_s> → "retry" while within the bound, else "fail" (raises). The bounded,
    logged, retry-don't-die contract that replaces die-on-first-miss (2026-07-20 zero-load incident)."""
    if int(elapsed) < int(timeout):
        return "retry"
    return "fail"


def lh_restart_backoff(n):
    """<restart_count> → seconds to hold a repeatedly-dying sender = min(BASE·2^(n-1), MAX); n<=0 → 0.
    Exponential so a crash-looper backs off; capped so a healthy fleet recovers promptly."""
    n = int(n)
    if n <= 0:
        return 0
    b = LOAD_RESTART_BACKOFF_BASE * (2 ** (n - 1))
    return int(min(b, LOAD_RESTART_BACKOFF_MAX))


def lh_proc_identity_matches(recorded_start, now, etimes, tol=2):
    """A live PID is OURS only if (now − its elapsed-seconds) lands on the pidfile's recorded start
    epoch (within tol of skew) — else the PID was recycled and MUST NOT be killed."""
    d = abs((int(now) - int(etimes)) - int(recorded_start))
    return d <= int(tol)


def lh_pidfile_parse(line):
    """Parse "<pid> <pgid> <start_epoch>" → (pid, pgid, start); raises ValueError on malformed /
    non-numeric / trailing-junk input (bash rc 1)."""
    parts = (line or "").split()
    if len(parts) != 3 or not all(p.isdigit() for p in parts):
        raise ValueError("malformed pidfile line")
    return int(parts[0]), int(parts[1]), int(parts[2])


def lh_status_line(mode, sent, inflight, cap, basefee="-", maxfee="-", sample="-", head="-"):
    """The SINGLE emitter of the `sent=<n> inflight=<n>/<cap>` string that soak-status.sh and
    status.py grep for. Format PINNED against that grep (soak-gate-test.sh §20) so the two can
    never drift."""
    return (f"load-heavy[status]: mode={mode} sent={sent} inflight={inflight}/{cap} "
            f"basefee={basefee}/max={maxfee} sample_receipt(last sender-0)={sample} head={head}")


def lh_marker_re():
    """The regex the funding gate matches in LOAD_WAIT_MARKER_LOG. PARAMETERIZED (C9): the
    blaster is framework layer and must not know any consumer's log format.
    LOAD_WAIT_MARKER_RE is the knob a consumer sets; the default is the framework event log's
    OWN heartbeat marker, which is the line the simulation writes — so the format has exactly one owner (core.events) and the blaster
    follows whatever that owner prints. Resolved per call, so an override set after import lands."""
    return os.environ.get("LOAD_WAIT_MARKER_RE") or events.HEARTBEAT_MARKER_RE


def lh_log_has_marker(log_path, marker_re=None):
    """True iff <logfile> already contains the first run-started marker (see lh_marker_re) — the
    ONLY safe signal that funding a tx won't corrupt the shared dev-EOA nonce. Missing/unreadable
    → False (keep waiting)."""
    import re
    if not log_path or not os.access(log_path, os.R_OK):
        return False
    r = re.compile(marker_re or lh_marker_re())
    try:
        with open(log_path, errors="replace") as f:
            for ln in f:
                if r.search(ln):
                    return True
    except OSError:
        return False
    return False


# ── pidfile path + clean stop (NEVER pkill) ──────────────────────────────────

def lh_pidfile_path():
    if os.environ.get("LOAD_PIDFILE"):
        return os.environ["LOAD_PIDFILE"]
    if os.environ.get("LOAD_LOG"):
        return os.path.join(os.path.dirname(os.environ["LOAD_LOG"]), "load-heavy.pid")
    return os.path.join(os.environ.get("TMPDIR", "/tmp"), "load-heavy.pid")


def _etimes(pid):
    out = proc.read(["ps", "-o", "etimes=", "-p", str(pid)]).strip()
    return int(out) if out.isdigit() else None


def stop(pidfile=None):
    """Pidfile-based shutdown: parse → identity-verify by pid+etime (never kill a recycled PID) →
    TERM → wait → KILL -9 fallback → remove the pidfile."""
    pf = pidfile or lh_pidfile_path()
    if not os.path.isfile(pf):
        print(f"load-heavy stop: no pidfile at {pf} (nothing to stop)")
        return 0
    try:
        with open(pf) as f:
            pid, pgid, start = lh_pidfile_parse(f.read())
    except ValueError:
        print(f"load-heavy stop: malformed pidfile {pf} — removing")
        os.remove(pf)
        return 0
    et = _etimes(pid)
    if et is None:
        print(f"load-heavy stop: pid {pid} not alive — removing stale pidfile {pf}")
        os.remove(pf)
        return 0
    if not lh_proc_identity_matches(start, int(time.time()), et, 3):
        print(f"load-heavy stop: pid {pid} alive but start-time mismatch — PID reused, NOT killing")
        os.remove(pf)
        return 0
    target = -pgid if pgid == pid else pid
    print(f"load-heavy stop: TERM {target} (pid={pid} pgid={pgid})")
    try:
        os.kill(target, 15)
    except OSError:
        pass
    for _ in range(20):
        if _etimes(pid) is None:
            print("load-heavy stop: stopped cleanly")
            os.remove(pf)
            return 0
        time.sleep(0.5)
    print(f"load-heavy stop: pid {pid} still alive after 10s — KILL -9 {target}")
    try:
        os.kill(target, 9)
    except OSError:
        pass
    time.sleep(1)
    if _etimes(pid) is None:
        os.remove(pf)
        return 0
    print(f"load-heavy stop: FAILED to kill pid {pid} (check manually — do NOT pkill)")
    return 1


# ── runtime supervisor (thread-based sender loops) ────────────────────────────

class Sender:
    """The self-supervising blaster. `run()` acquires (marker+RPC+funder), funds senders, deploys
    GasBurner (burn mode) or skips it (transfer mode), then supervises N sender threads emitting the
    heartbeat. Cast shell-outs are bounded by `timeout` exactly as the bash lh_cast wrapper."""

    def __init__(self):
        self.rpc = os.environ.get("LOAD_RPC", os.environ.get("RPC", topology.DEFAULT_RPC_URL))
        self.senders = int(os.environ.get("LOAD_SENDERS", "4"))
        self.fraction = float(os.environ.get("LOAD_TARGET_GAS_FRACTION", "0.6"))
        self.tx_gas = int(os.environ.get("LOAD_TX_GAS", "3000000"))
        self.duration = int(os.environ.get("LOAD_DURATION", "0"))
        self.status_every = int(os.environ.get("LOAD_STATUS_EVERY", "30"))
        self.max_inflight = int(os.environ.get("LOAD_MAX_INFLIGHT", "12"))
        self.mode = os.environ.get("LOAD_MODE", "burn")
        self.max_fee = int(os.environ.get("LOAD_MAX_FEE", "5000"))
        self.tip = int(os.environ.get("LOAD_TIP", "2"))
        self.wait_marker_log = os.environ.get("LOAD_WAIT_MARKER_LOG", "")
        self.wait_marker_re = lh_marker_re()
        self.acquire_timeout = int(os.environ.get("LOAD_ACQUIRE_TIMEOUT", "1800"))
        self.acquire_interval = int(os.environ.get("LOAD_ACQUIRE_INTERVAL", "15"))
        self.supervise_tick = int(os.environ.get("LOAD_SUPERVISE_TICK", "5"))
        if self.mode not in ("burn", "transfer"):
            raise SystemExit(
                f"FAIL (load-heavy): LOAD_MODE must be burn|transfer (got '{self.mode}')")
        self._stop = threading.Event()
        self._sent = [0] * self.senders
        self._inflight = [0] * self.senders
        self.tx_gas = int(os.environ.get("LOAD_TX_GAS", "3000000"))
        self.fund_wei = os.environ.get("LOAD_FUND_WEI", "100000000000000000")
        self.funder_key = os.environ.get("LOAD_FUNDER_KEY", "")
        self.burn_n = None
        self.burner = ""
        self._pace_s = 0.0
        # write-side exec seam (shared with the rest of the port; stubbed in tests).
        self.p = proc.Runner(dry=bool(os.environ.get("LOAD_DRY")))

    # ── pure command builders (oracle-pinned by the unit tests) ───────────────
    def fund_cmd(self, addr):
        """The per-sender funding tx: `cast send <addr> --value <LOAD_FUND_WEI> --rpc-url <RPC>
        --private-key <FUNDER_KEY>` (load-heavy.sh:439)."""
        return ["cast", "send", addr, "--value", str(self.fund_wei), "--rpc-url", self.rpc,
                "--private-key", self.funder_key]

    def send_cmd(self, addr, key, nxt):
        """One sender tx, keyed on the locally-tracked nonce (load-heavy.sh:385-393). Fire-and-
        forget --async EIP-1559 (gas-price=LOAD_MAX_FEE, priority=LOAD_TIP) with a gas cap —
        WITHOUT --async cast blocks on the receipt and the fleet can never reach its target rate;
        without the caps a 1559 fee spike silently drops every send. transfer: a 21000-gas ETH
        self-send; burn: BURNER.burn(BURN_N) at the LOAD_TX_GAS cap."""
        caps = ["--gas-price", str(self.max_fee), "--priority-gas-price", str(self.tip), "--async"]
        if self.mode == "transfer":
            return ["cast", "send", addr, "--value", "1", "--rpc-url", self.rpc,
                    "--private-key", key, "--nonce", str(nxt), "--gas-limit", "21000", *caps]
        return ["cast", "send", self.burner, "burn(uint256)", str(self.burn_n),
                "--rpc-url", self.rpc, "--private-key", key, "--nonce", str(nxt),
                "--gas-limit", str(self.tx_gas), *caps]

    # ── live surface (funding + deploy + supervised send) ─────────────────────
    def _rpc_ready(self):
        """lh_rpc_ready (load-heavy.sh:296): a bounded eth_blockNumber that returns a plain int."""
        out = self.p.run(["cast", "block-number", "--rpc-url", self.rpc], note="rpc-ready")
        return out.strip().isdigit()

    def _resolve_funder_once(self):
        """lh_funder_key (load-heavy.sh:268), ONE non-raising attempt: LOAD_FUNDER_KEY, else read
        validator-0's /runtime/keys/funded.hex (compose exec, then a docker-ps fallback). Returns
        the 0x-normalized key or "" on a miss — the acquire loop retries a miss, it does NOT abort
        (the stack/funder aren't up the instant the sender launches). Strips only the "0x" PREFIX
        (bash `${k#0x}`), never leading zero nibbles (LOW)."""
        if self.funder_key:
            return "0x" + self.funder_key.removeprefix("0x")
        k = self.p.run(["docker", "compose", "exec", "-T", topology.RUNTIME_MOUNT_HOST,
                        "cat", "/runtime/keys/funded.hex"], note="funder-key").strip()
        if not k:
            c = self.p.run(["docker", "ps", "--format", "{{.Names}}"], note="funder-ps")
            name = next((ln for ln in c.splitlines()
                         if topology.RUNTIME_MOUNT_HOST in ln), "")
            if name:
                k = self.p.run(["docker", "exec", name, "cat", "/runtime/keys/funded.hex"],
                               note="funder-key2").strip()
        return ("0x" + k.removeprefix("0x")) if k else ""

    def _acquire(self):
        """lh_acquire (load-heavy.sh:307): the ONE bounded startup retry loop — poll, in order, the
        LOAD_WAIT_MARKER_LOG run-started marker (pattern: LOAD_WAIT_MARKER_RE), RPC readiness,
        and the funder key, logging each miss with its reason every LOAD_ACQUIRE_INTERVAL. The
        LOUD zero-load abort is reserved for LOAD_ACQUIRE_TIMEOUT EXHAUSTION — never a t=0 preflight (the operational defect: launched
        against a just-started sim, the funder key hasn't materialized yet). On success sets
        self.funder_key + returns True; returns False if `stop`ped mid-wait."""
        if self.p.dry:
            self.funder_key = self.funder_key or "0xFUNDER"
            return True
        start = time.time()
        while True:
            elapsed = time.time() - start
            reason = ""
            if self.wait_marker_log and not lh_log_has_marker(self.wait_marker_log,
                                                              self.wait_marker_re):
                reason = (f"run-started marker /{self.wait_marker_re}/ not yet in "
                          f"{self.wait_marker_log} (DeployStaking incomplete — funding now would "
                          "corrupt the dev-EOA nonce)")
            elif not self._rpc_ready():
                reason = f"RPC {self.rpc} not answering eth_blockNumber"
            else:
                key = self._resolve_funder_once()
                if key:
                    self.funder_key = key
                    print(f"load-heavy: acquisition OK after {int(elapsed)}s (funder ready"
                          f"{', run-started marker present' if self.wait_marker_log else ''})",
                          flush=True)
                    return True
                reason = ("funder key not readable (no LOAD_FUNDER_KEY, no live "
                          f"{topology.RUNTIME_MOUNT_HOST} funded.hex)")
            if lh_acquire_verdict(elapsed, self.acquire_timeout) == "fail":
                raise SystemExit(
                    f"FAIL (load-heavy): startup acquisition timed out after {int(elapsed)}s "
                    f"(limit {self.acquire_timeout}s) — last reason: {reason}; refusing to start a "
                    "zero-load run")
            print(f"load-heavy: waiting ({int(elapsed)}s/{self.acquire_timeout}s) — {reason}; "
                  f"retry in {self.acquire_interval}s", flush=True)
            if self._stop.wait(self.acquire_interval):
                return False

    def _new_sender_keys(self):
        keys = []
        for _ in range(self.senders):
            out = self.p.run(["cast", "wallet", "new", "--json"], note="wallet-new")
            try:
                import json
                w = json.loads(out) if out else []
            except Exception:
                w = []
            # `cast wallet new --json` returns a JSON ARRAY (load-heavy.sh:438: `.[0].private_key`),
            # never an object — a `.get` on the list silently produced empty keys and zero load (F14).
            entry = w[0] if isinstance(w, list) and w else (w if isinstance(w, dict) else {})
            keys.append((entry.get("address", ""),
                         entry.get("private_key", entry.get("privateKey", ""))))
        return keys

    def _fund(self, keys):
        for addr, _ in keys:
            self.p.run(self.fund_cmd(addr), note="fund-sender")

    def _deploy_burner(self):
        """burn mode only: deploy the vendored GasBurner once (`cast send --private-key <FUNDER>
        --rpc-url <RPC> --create <bytecode> --json`); compute burn(n) filling 90% of tx gas."""
        if self.mode != "burn":
            return
        bc = lh_burner_bytecode()
        out = self.p.run(["cast", "send", "--private-key", self.funder_key, "--rpc-url", self.rpc,
                          "--create", bc, "--json"], note="deploy-burner")
        try:
            import json
            self.burner = json.loads(out).get("contractAddress", "") if out else ""
        except Exception:
            self.burner = ""
        if self.mode == "burn" and not self.burner and not self.p.dry:
            raise SystemExit("FAIL (load-heavy): GasBurner deploy failed (no contractAddress) — "
                             "burn mode cannot send without a burner; refusing a zero-load run")
        base_gas = int(os.environ.get("LOAD_BURN_BASE_GAS", "21000"))
        per_iter = int(os.environ.get("LOAD_BURN_GAS_PER_ITER", "20000"))
        self.burn_n = lh_burn_iters(self.tx_gas, base_gas, per_iter)

    def _block_gas_limit(self):
        """GAS_LIMIT (load-heavy.sh:423): the live block gas limit, for the pacing calibration."""
        out = self.p.run(["cast", "block", "latest", "--field", "gasLimit", "--rpc-url", self.rpc],
                         note="gas-limit")
        try:
            s = out.strip()
            return int(s, 16) if s.startswith("0x") else int(s)
        except (ValueError, AttributeError):
            return int(os.environ.get("LOAD_BLOCK_GAS_LIMIT", "50000000"))

    def run(self):
        """Live supervisor. Acquire (marker → RPC → funder, bounded) → fund senders → deploy GasBurner
        (burn) → calibrate the per-sender pacing (SENDERS×rate×tx_gas ≈ fraction×gas_limit)
        → spawn one send THREAD per sender (locally-tracked nonce, backpressure gate, paced) → emit
        the `sent=/inflight=` heartbeat every LOAD_STATUS_EVERY. Bounded by LOAD_DURATION (0 = until
        `stop`). Cast calls go through the proc seam (recorded/stubbed in tests)."""
        import json  # noqa: F401 — used by helpers
        with open(lh_pidfile_path(), "w") as f:
            f.write(f"{os.getpid()} {os.getpgrp()} {int(time.time())}")
        # _acquire owns funder resolution now (marker → RPC → funder, bounded; abort only on
        # timeout exhaustion) — no t=0 pre-resolve that aborts before the stack is up.
        if not self._acquire():
            return 0
        keys = self._new_sender_keys()
        self._fund(keys)
        self._deploy_burner()
        gas_limit = self._block_gas_limit()
        try:
            _tps, ivl_ms = lh_calibrate(gas_limit, self.fraction, self.senders, self.tx_gas)
        except ValueError:
            ivl_ms = 1000
        self._pace_s = max(ivl_ms / 1000.0, 0.0)
        deadline = time.time() + self.duration if self.duration > 0 else None
        threads = [threading.Thread(target=self._sender_loop, args=(i, addr, key, deadline),
                                    daemon=True)
                   for i, (addr, key) in enumerate(keys)]
        for t in threads:
            t.start()
        self._heartbeat_loop(deadline)
        self._stop.set()
        for t in threads:
            t.join(timeout=self.supervise_tick + 1)
        return 0

    def _sender_loop(self, i, addr, key, deadline):
        """One sender's paced fire-and-forget loop: derive the local nonce from the mined head
        (external reality wins), skip at the backpressure window, send, sleep the calibrated pace."""
        nxt = 0
        while not self._stop.is_set():
            mined = self.nonce(addr)
            nxt, verdict = lh_inflight_gate(nxt, mined, self.max_inflight)
            self._inflight[i] = max(nxt - mined, 0)
            if verdict == "send":
                self.p.run(self.send_cmd(addr, key, nxt), note="sender-send")
                nxt += 1
                self._sent[i] += 1
            if deadline and time.time() >= deadline:
                break
            if self._stop.wait(self._pace_s):
                break

    def _heartbeat_loop(self, deadline):
        """Emit the SINGLE pinned `sent=/inflight=` status line (lh_status_line) every
        LOAD_STATUS_EVERY seconds — the string soak-status.sh / status.py grep for."""
        while not self._stop.is_set():
            if self._stop.wait(max(self.status_every, 1)):
                break
            print(lh_status_line(self.mode, sum(self._sent), sum(self._inflight),
                                 self.max_inflight * self.senders), flush=True)
            if deadline and time.time() >= deadline:
                break

    def nonce(self, addr):
        out = self.p.run(["cast", "nonce", addr, "--rpc-url", self.rpc], note="sender-nonce")
        return int(out) if out.isdigit() else 0
