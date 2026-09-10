#!/usr/bin/env python3
"""The committee-floor smoke: the population drops below MIN_COMMITTEE_LENGTH and the
commit REFUSES, so the chain stops advancing (R-111 / R-112, `.dpos-study/REGISTER.md`;
E1/E2/E5 in `.dpos-study/EXPERIMENTS.md` part 5).

    floor_halt_case.py <N> exit  [--victim I]     one honest owner withdraws its own stake
    floor_halt_case.py <N> decay [--exits K]      K honest exits, one per epoch, tail first
    floor_halt_case.py <N> byz   [--victim I]     one equivocator is tombstoned (N=4 needs
                                                  XP_ALLOW_SMALL_BYZ=1, set here)
    options: --reuse (the xp<N> stand is already up)  --keep-up (do not tear down)

What this case asserts. On the first block of the epoch after the exit/jail the node commits
`committee[E+3]` from a selection of V < MIN_COMMITTEE_LENGTH members. The contract refuses:
`commitEpochCommittee` reverts `CommitteeTooSmall(V, MIN_COMMITTEE_LENGTH)`
(`contracts/staking/src/consensus.rs`, `ERR_COMMITTEE_TOO_SMALL = 0x0a87ec8d`). That call is a
PRE-EXECUTION system call, so `EvmAheadCommit::commit_epoch` (node `crates/node/src/evm.rs`)
turns the revert into a `BlockExecutionError` carrying the message

    commitEpochCommittee(epoch E+3) did not succeed: Revert { … output: 0x0a87ec8d<V><MIN> }

and the block at the boundary is never executed. The chain does not advance past it.

Anti-vacuity: the REFUSAL PAYLOAD is required, not merely a stalled chain. A stall has many
causes and every one of them would pass a liveness-only gate; the two 32-byte words behind
`0x0a87ec8d` must be exactly the population this case removed to and MIN_COMMITTEE_LENGTH,
which no other failure produces. The refused EPOCH is read off that line rather than predicted,
and where validator-0 still answers RPC, `nextEpochToCommit()` must still be that epoch — the
cursor never advanced, i.e. the commit really did not take effect.

ГИПОТЕЗА (not verified in this session, and the reason the halt gate is written as
"did not advance" rather than "every container exited"): whether the boundary block is
refused during BUILDING on the leader (no block is ever produced, every node stays up and
stalls) or during VALIDATION on the followers (containers take the fatal branch and exit)
was not reproduced here. Both are a halt; this case pins the halt and the payload and
reports the container states rather than requiring one of the two shapes.

Stand: `genN.py` + `bringupN.sh` (the same stand `EXPERIMENTS.md` part 5 used). Set
`XP_CONTRACTS_DIR` to bring the stand up against a different contracts directory.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
SMOKE = HERE.parent.parent
# The fatal markers and the tombstone marker come from the SMOKE HARNESS, not from a second
# copy here: `smoke-byzantine` reads the same lines, and two copies of "which line means a
# node took the fatal branch" drift the moment one node message is reworded. Import-only —
# `verdicts_onchain` is a pure module (no docker, no RPC).
sys.path.insert(0, str(SMOKE))
from dpos_harness.cases.smoke import verdicts_onchain as vo   # noqa: E402

OUT = HERE / "out"
STAKING = "0x0000000000000000000000000000000000520011"
MIN_COMMITTEE_LENGTH = 4          # the floor both sides declare (R-117: reader.rs / consts.rs)
ANSI = re.compile(r"\x1b\[[0-9;]*m")

FATAL_LINES = vo.FATAL_LINES
SEVER_LINE = vo.TOMBSTONE_SEVER_LINE

#: `contracts/staking/src/consts.rs::ERR_COMMITTEE_TOO_SMALL`, read off the contract itself
#: (`derive_keccak256_id!("CommitteeTooSmall(uint256,uint256)")`) and pinned here as a literal.
#: `revert_with` writes it big-endian and appends the two ABI words, so the node's `{other:?}`
#: of the reverted `ExecutionResult` carries selector+args verbatim as one hex run.
ERR_COMMITTEE_TOO_SMALL = "0a87ec8d"
_REFUSAL_RE = re.compile(
    r"commitEpochCommittee\(epoch (\d+)\) did not succeed.*?"
    r"0x" + ERR_COMMITTEE_TOO_SMALL + r"([0-9a-fA-F]{64})([0-9a-fA-F]{64})")

RC_PASS, RC_FAIL, RC_USAGE, RC_ERROR = 0, 1, 2, 3


def sh(*a, check=False, timeout=180, env=None):
    r = subprocess.run(list(a), capture_output=True, text=True, timeout=timeout, env=env)
    if check and r.returncode != 0:
        raise RuntimeError(f"{' '.join(a[:6])}… rc={r.returncode}: {r.stderr.strip()[:400]}")
    return r.stdout.strip()


class Stand:
    def __init__(self, n, interval, act):
        self.n, self.interval, self.act = n, interval, act
        self.project = f"xp{n}"
        self.sub = f"172.{20 + n}.0"
        self.rpc = f"http://localhost:{28000 + n}"
        self.full_rpc = f"http://localhost:{29000 + n}"
        self.files = ["-f", str(OUT / f"xp{n}.yml"), "-f", str(OUT / f"xp{n}.dpos.yml")]

    def container(self, i):
        return f"{self.project}-validator-{i}-1"

    def compose(self, *tail, extra_files=()):
        argv = ["docker", "compose", "-p", self.project, *self.files]
        for f in extra_files:
            argv += ["-f", f]
        return sh(*argv, *tail, check=True, timeout=600)

    def state(self, i):
        return sh("docker", "inspect", "-f", "{{.State.Status}}/{{.State.ExitCode}}",
                  self.container(i))

    def running(self, i):
        return self.state(i).startswith("running")

    def logs(self, i):
        r = subprocess.run(["docker", "logs", self.container(i)], capture_output=True, text=True)
        return ANSI.sub("", r.stdout + r.stderr)

    def rpc_call(self, url, method, params):
        body = json.dumps({"jsonrpc": "2.0", "method": method, "params": params, "id": 1})
        req = urllib.request.Request(url, data=body.encode(),
                                     headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=5) as resp:
                return json.loads(resp.read()).get("result")
        except Exception:  # noqa: BLE001 — an unreachable node reads as None, never as 0
            return None

    def finalized(self, url=None):
        blk = self.rpc_call(url or self.rpc, "eth_getBlockByNumber", ["finalized", False])
        if not blk or not blk.get("number"):
            return None
        return int(blk["number"], 16), blk["hash"]

    def cast_call(self, sig, *args):
        return sh("cast", "call", "--rpc-url", self.rpc, STAKING, sig, *map(str, args))

    def epoch_of(self, b):
        return (b - self.act) // self.interval

    def addresses(self):
        return json.loads(sh("docker", "exec", self.container(0), "cat",
                             "/runtime/addresses.json"))["validators"]

    def owner_key(self, i):
        return "0x" + sh("docker", "exec", self.container(0), "cat",
                         f"/runtime/keys/owner-{i}.hex")


def refusal_lines(log_text: str):
    """`(target_epoch, eligible, floor)` for every committee-floor REFUSAL in `log_text`.

    Matched on the node's own message plus the contract's revert payload, so a line that says
    the commit failed for any OTHER reason does not parse into this list at all."""
    out = []
    for ln in (log_text or "").splitlines():
        m = _REFUSAL_RE.search(ln)
        if m:
            out.append((int(m.group(1)), int(m.group(2), 16), int(m.group(3), 16)))
    return out


def bring_up(stand: Stand):
    env = dict(os.environ, EPOCH_BLOCK_INTERVAL=str(stand.interval),
               DPOS_ACTIVATION_BLOCK=str(stand.act))
    sh(sys.executable, str(HERE / "genN.py"), str(stand.n), check=True, env=env)
    alt = os.environ.get("XP_CONTRACTS_DIR")
    if alt:
        p = OUT / f"xp{stand.n}.yml"
        text = p.read_text().replace(f"{SMOKE}/contracts:/contracts:ro", f"{alt}:/contracts:ro")
        assert f"{alt}:/contracts:ro" in text, "contracts bind-mount not found in the compose"
        p.write_text(text)
        print(f"  contracts dir overridden: {alt}", flush=True)
    r = subprocess.run([str(HERE / "bringupN.sh"), str(stand.n)], env=env)
    if r.returncode != 0:
        raise RuntimeError(f"bringupN.sh {stand.n} failed rc={r.returncode}")


def wait_dpos_ready(stand: Stand, budget_s=600):
    """All N nodes run under --dpos and finalized is climbing (the gate `exits.py` learned
    the hard way: an actuation on the sequencer chain is a different experiment)."""
    last = None
    deadline = time.time() + budget_s
    while time.time() < deadline:
        alive = all(stand.running(i) for i in range(stand.n))
        dpos = all("--dpos" in sh("docker", "inspect", "-f", "{{json .Config.Cmd}}",
                                  stand.container(i)) for i in range(stand.n))
        fin = stand.finalized()
        if alive and dpos and fin and fin[0] > stand.act:
            if last is not None and fin[0] > last:
                return fin[0]
            last = fin[0]
        time.sleep(4)
    raise RuntimeError("stand never reached a climbing DPoS phase")


def snapshot(stand: Stand, nodes):
    snap = {}
    for i in nodes:
        text = stand.logs(i)
        snap[i] = {
            "fatal": {m: text.count(m) for m in FATAL_LINES},
            "refusals": refusal_lines(text),
        }
    return snap


def undelegate_all(stand: Stand, idx):
    """E2's actuation: `undelegate(validator, self_stake)` signed by the validator's own
    owner key — an ordinary permissionless transaction. Returns (tx block, epoch)."""
    v = stand.addresses()[idx]
    key = stand.owner_key(idx)
    owner = sh("cast", "wallet", "address", "--private-key", key)
    amt = stand.cast_call("getValidatorDelegation(address,address)(uint256)", v, owner).split()[0]
    out = sh("cast", "send", "--rpc-url", stand.rpc, "--private-key", key, "--legacy",
             STAKING, "undelegate(address,uint256)", v, amt, timeout=120)
    status = next((l.split()[1] for l in out.splitlines() if l.startswith("status")), "")
    blk = next((int(l.split()[1]) for l in out.splitlines() if l.startswith("blockNumber")), None)
    if status != "1" or blk is None:
        raise AssertionError(f"undelegate({v}, {amt}) from validator-{idx} did not succeed: "
                             f"status={status!r} block={blk}\n{out[-600:]}")
    print(f"  validator-{idx} ({v}) withdrew self-stake {amt} in block {blk} "
          f"(epoch {stand.epoch_of(blk)}) — status 1", flush=True)
    return blk, stand.epoch_of(blk)


def enable_equivocation(stand: Stand, idx):
    env = dict(os.environ, XP_ALLOW_SMALL_BYZ="1")
    overlay = sh(str(HERE / "byz_overlay.sh"), str(stand.n), str(idx), check=True, env=env)
    stand.compose("up", "-d", "--force-recreate", f"validator-{idx}", extra_files=(overlay,))
    print(f"  equivocation enabled on validator-{idx} via {overlay}", flush=True)


def wait_sever(stand: Stand, observer, budget_s=420):
    """The tombstone reached an honest node: it logged the severance with the jail epoch."""
    deadline = time.time() + budget_s
    while time.time() < deadline:
        for ln in stand.logs(observer).splitlines():
            if SEVER_LINE in ln:
                m = re.search(r"\bepoch=(\d+)", ln)
                return int(m.group(1)) if m else None, ln.strip()
        time.sleep(4)
    return None, ""


def wait_past(stand: Stand, target, honest, budget_s, label):
    """Finalized on validator-0 reaches `target` while every honest node stays up. Used by the
    `decay` CONTROL, where the population is still at or above the floor and the chain must
    keep going. A node that exits during the wait ends it immediately with its fatal line."""
    deadline = time.time() + budget_s
    last_print = 0
    while time.time() < deadline:
        for i in honest:
            if not stand.running(i):
                lines = [ln for ln in stand.logs(i).splitlines()
                         if any(m in ln for m in FATAL_LINES)]
                raise AssertionError(
                    f"[{label}] validator-{i} is {stand.state(i)} before finalized reached "
                    f"{target} — the fatal branch: "
                    + (lines[-1][:300] if lines else "(no fatal line)"))
        fin = stand.finalized()
        if fin and fin[0] >= target:
            return fin[0]
        if time.time() - last_print > 20:
            print(f"    [{label}] finalized={fin[0] if fin else None} target={target}", flush=True)
            last_print = time.time()
        time.sleep(3)
    raise AssertionError(f"[{label}] finalized did not reach {target} within {budget_s}s "
                         f"(now {stand.finalized()})")


def wait_refusal(stand: Stand, honest, before, eligible, budget_s, label):
    """The first fresh committee-floor refusal carrying this case's payload, on any honest node.

    Returns `(node, (epoch, eligible, floor))`, or `(None, None)` on timeout. Matched on the
    PAYLOAD, never on a predicted epoch — see `main` for why the predicted one cannot be trusted
    in `byz` mode. Returns as soon as ONE node has it: the refusal is what stops the block, so
    whichever node ran the system call first is the one that can have written it. The per-node
    accounting is `refusal_evidence` below, on the full budget."""
    deadline = time.time() + budget_s
    last_print = 0
    while time.time() < deadline:
        for i in honest:
            fresh = refusal_lines(stand.logs(i))[len(before[i]["refusals"]):]
            hit = [t for t in fresh
                   if (t[1], t[2]) == (int(eligible), MIN_COMMITTEE_LENGTH)]
            if hit:
                return i, hit[0]
        if time.time() - last_print > 20:
            print(f"    [{label}] waiting for a CommitteeTooSmall({eligible}, "
                  f"{MIN_COMMITTEE_LENGTH}) refusal; finalized={stand.finalized()}", flush=True)
            last_print = time.time()
        time.sleep(3)
    return None, None


def refusal_evidence(stand: Stand, honest, before, eligible):
    """Which honest nodes wrote a refusal carrying this case's payload, and which epoch they
    agree it refused.

    Returns `(problems, witnesses, epoch)`. A node that never ran the boundary block writes
    nothing and is reported as such rather than as a mismatch — `problems` is raised for a node
    whose refusal carries the WRONG numbers, for "nobody refused at all", and for witnesses that
    do not agree on one epoch."""
    problems, witnesses, epochs = [], [], set()
    for i in honest:
        fresh = refusal_lines(stand.logs(i))[len(before[i]["refusals"]):]
        if not fresh:
            continue
        good = [t for t in fresh
                if (t[1], t[2]) == (int(eligible), MIN_COMMITTEE_LENGTH)]
        if not good:
            tgt, elig, floor = fresh[-1]
            problems.append(
                f"validator-{i}: committee[{tgt}] was refused with "
                f"CommitteeTooSmall({elig}, {floor}), but this case removed the population "
                f"to {eligible} against a floor of {MIN_COMMITTEE_LENGTH}")
            continue
        witnesses.append(i)
        epochs |= {t[0] for t in good}
    if not witnesses:
        problems.append(
            f"no honest node logged 'commitEpochCommittee(epoch N) did not succeed' with a "
            f"0x{ERR_COMMITTEE_TOO_SMALL} payload of ({eligible}, {MIN_COMMITTEE_LENGTH}) — the "
            "chain may have stopped for some other reason, so this run is not evidence that the "
            "floor was reached")
        return problems, witnesses, None
    if len(epochs) > 1:
        problems.append(f"honest nodes refused DIFFERENT epochs ({sorted(epochs)}) — they did "
                        "not stop at one boundary")
    return problems, witnesses, min(epochs)


def unexpected_fatals(stand: Stand, honest, before, witnesses):
    """Fatal lines written since the snapshot by a node that did NOT witness the refusal.

    A node dying at the boundary is the EXPECTED outcome here, and it dies loudly: the refusal
    rides in on `did not succeed`, and the executor and the outer engine then write their own
    markers, which carry no payload at all. Scoring those as strays failed the case on exactly
    the outcome it exists to observe. Every node that executes the boundary block runs the system
    call, so a node that died WITHOUT the refusal line died of something else — that, and only
    that, is the finding."""
    problems = []
    for i in honest:
        if i in witnesses:
            continue
        text = stand.logs(i)
        for m in FATAL_LINES:
            fresh = [ln for ln in text.splitlines() if m in ln][before[i]["fatal"][m]:]
            if fresh:
                problems.append(f"validator-{i} took a fatal branch WITHOUT the floor refusal — "
                                f"{fresh[-1][:300]}")
    return problems


def did_not_advance(stand: Stand, fatal, seconds=60):
    """Finalized stayed BELOW the boundary block across the window, on validator-0 and on the
    full node. `None` from either RPC is the halt in its harshest shape (the node is gone), not
    an unread value, so it satisfies the gate; a reading AT OR PAST `fatal` refutes it."""
    problems = []
    for label, url in (("validator-0", stand.rpc), ("full-node", stand.full_rpc)):
        fin = stand.finalized(url)
        if fin and fin[0] >= fatal:
            problems.append(f"{label} finalized {fin[0]} >= the boundary block {fatal} — the "
                            f"commit that had to be refused went through")
    time.sleep(seconds)
    for label, url in (("validator-0", stand.rpc), ("full-node", stand.full_rpc)):
        fin = stand.finalized(url)
        if fin and fin[0] >= fatal:
            problems.append(f"{label} finalized {fin[0]} >= the boundary block {fatal} after a "
                            f"{seconds}s window — the chain advanced past the floor")
    return problems


def cursor_stuck(stand: Stand, target):
    """`nextEpochToCommit()` is still `target`: the contract's own statement that the commit
    did not take effect. Skipped, not failed, when no node answers — an unreachable RPC is the
    halt itself, and reading it as a mismatch would fail the case for passing."""
    out = stand.cast_call("nextEpochToCommit()(uint64)")
    if not out:
        return [], None
    try:
        cur = int(out.split()[0])
    except (ValueError, IndexError):
        return [], None
    if cur != target:
        return [f"nextEpochToCommit() = {cur}, expected {target} — the cursor moved past the "
                f"epoch whose commit had to be refused"], cur
    return [], cur


def int_opt(opts, flag, default):
    """The integer after `flag`, or `default`. A flag given as the LAST argument has no value,
    and `opts[i + 1]` would raise IndexError with a traceback instead of printing the usage."""
    if flag not in opts:
        return default
    i = opts.index(flag)
    if i + 1 >= len(opts) or not opts[i + 1].lstrip("-").isdigit():
        raise ValueError(f"{flag} needs an integer")
    return int(opts[i + 1])


def main(argv):
    # `argv[0]` is checked as well as `argv[1]`: the mode guard alone let a non-numeric N
    # through to `int()`, which raised a bare ValueError traceback rather than the usage.
    if len(argv) < 2 or not argv[0].isdigit() or argv[1] not in ("exit", "decay", "byz"):
        print(__doc__)
        return RC_USAGE
    n, mode = int(argv[0]), argv[1]
    opts = argv[2:]
    reuse, keep = "--reuse" in opts, "--keep-up" in opts
    try:
        victim = int_opt(opts, "--victim", n - 1)
        exits = int_opt(opts, "--exits", n - 3 if mode == "decay" else 1)
    except ValueError as e:
        print(f"usage error: {e}\n{__doc__}", flush=True)
        return RC_USAGE
    # `XP_CONTRACTS_DIR` is applied while the compose file is GENERATED (`bring_up`), which
    # `--reuse` skips — so the two together used to run against whatever contracts the standing
    # stand was brought up with, silently, which is the opposite of what the flag is for.
    if reuse and os.environ.get("XP_CONTRACTS_DIR"):
        print("XP_CONTRACTS_DIR has no effect with --reuse: the contracts directory is bound "
              "when the stand is GENERATED. Bring the stand up without --reuse, or unset the "
              "variable.", flush=True)
        return RC_USAGE
    interval = int(os.environ.get("EPOCH_BLOCK_INTERVAL", "32"))
    act = int(os.environ.get("DPOS_ACTIVATION_BLOCK", str(2 * interval)))
    stand = Stand(n, interval, act)
    label = f"floor-halt-{mode}"
    print(f"=== {label}: N={n} interval={interval} activation={act} ===", flush=True)

    rc = RC_FAIL
    try:
        if not reuse:
            bring_up(stand)
        fin0 = wait_dpos_ready(stand)
        print(f"  DPoS ready: finalized={fin0} (epoch {stand.epoch_of(fin0)})", flush=True)

        if mode == "byz":
            honest = [i for i in range(n) if i != victim]
            before = snapshot(stand, honest)
            enable_equivocation(stand, victim)
            jail_epoch, line = wait_sever(stand, honest[0])
            if jail_epoch is None:
                raise AssertionError(f"validator-{honest[0]} never logged '{SEVER_LINE}' — the "
                                     "equivocator was not tombstoned, nothing to refuse")
            print(f"  tombstone landed (epoch {jail_epoch}): {line[:200]}", flush=True)
            steps = [(jail_epoch, n - 1)]
        else:
            honest = list(range(n))
            before = snapshot(stand, honest)
            order = [victim] if mode == "exit" else list(range(n - 1, n - 1 - exits, -1))
            steps = []
            population = n
            for idx in order:
                _, e = undelegate_all(stand, idx)
                population -= 1
                steps.append((e, population))
                if mode == "decay" and population >= MIN_COMMITTEE_LENGTH:
                    # A non-fatal exit: the next epoch commits a fresh, shorter committee and
                    # must NOT be refused. Wait one epoch past its boundary and check the
                    # control — this is what keeps the case from passing on a chain that was
                    # already dead before the population reached the floor.
                    boundary = act + (e + 1) * interval
                    wait_past(stand, boundary + interval, honest, 3 * interval + 120,
                              f"exit→V={population}")
                    stray, seen, _ = refusal_evidence(stand, honest, before, population)
                    if seen:
                        raise AssertionError(
                            f"a committee-floor refusal was logged after the exit that left "
                            f"V={population} >= {MIN_COMMITTEE_LENGTH} — the selection should "
                            f"still have seated a fresh committee (validator-{seen})")
                    print(f"  V={population}: no refusal (a fresh committee seated) — "
                          "control ok", flush=True)

        e_last, population = steps[-1]
        # `fatal` is the EXPECTED boundary block and is used only to bound the wait and to read
        # "did the chain advance past it". The refused EPOCH is never predicted: in `byz` mode
        # `e_last` comes from the severance line, whose `epoch=` is the epoch of the finalized
        # block the tombstone was READ at, which lags the slash by up to one boundary. Pinning
        # the assertion to `e_last + 3` therefore names an epoch the chain may never reach — and
        # since the chain is stopped by then, it never would. The epoch comes out of the refusal
        # the nodes actually logged, matched on its payload; `smoke-byzantine` uses the same
        # split (`carry_wait_target` bounds the wait, `min(shared)` names the epoch).
        fatal = act + (e_last + 1) * interval           # first block of the epoch after the drop
        print(f"  floor boundary: block {fatal} (epoch {e_last + 1}) must REFUSE its commit "
              f"from a selection of V={population} < {MIN_COMMITTEE_LENGTH}", flush=True)

        witness_at, hit = wait_refusal(stand, honest, before, population,
                                       3 * interval + 180, label)
        if witness_at is None:
            print(f"    no refusal seen yet; container states: "
                  f"{[(i, stand.state(i)) for i in honest]}", flush=True)
        else:
            print(f"  validator-{witness_at} refused committee[{hit[0]}] with "
                  f"CommitteeTooSmall({hit[1]}, {hit[2]})", flush=True)

        problems, witnesses, refused_epoch = refusal_evidence(
            stand, honest, before, population)
        problems += unexpected_fatals(stand, honest, before, witnesses)
        problems += did_not_advance(stand, fatal)
        cursor = None
        if refused_epoch is not None:
            cursor_problems, cursor = cursor_stuck(stand, refused_epoch)
            problems += cursor_problems

        states = {i: stand.state(i) for i in honest}
        print(f"  container states: {states}; nextEpochToCommit="
              f"{cursor if cursor is not None else 'unreadable (node down)'}", flush=True)

        if problems:
            print(f"FAIL ({label}):\n  " + "\n  ".join(problems), flush=True)
            rc = RC_FAIL
        else:
            print(f"OK ({label}): population fell to {population} < {MIN_COMMITTEE_LENGTH}; the "
                  f"commit of committee[{refused_epoch}] at block {fatal} was REFUSED with "
                  f"CommitteeTooSmall({population}, {MIN_COMMITTEE_LENGTH}) on "
                  f"{len(witnesses)} of {len(honest)} honest nodes "
                  f"(validator-{witnesses}), neither validator-0 nor the full node finalized "
                  f"past {fatal}, and the commit cursor never moved off {refused_epoch}",
                  flush=True)
            rc = RC_PASS
    except AssertionError as e:
        print(f"FAIL ({label}): {e}", flush=True)
        for i in range(n):
            print(f"    validator-{i}: {stand.state(i)}", flush=True)
        rc = RC_FAIL
    except (RuntimeError, subprocess.TimeoutExpired) as e:
        print(f"ERROR ({label}): {e}", flush=True)
        rc = RC_ERROR
    finally:
        if keep:
            print(f"  --keep-up: leaving stand {stand.project} up", flush=True)
        else:
            subprocess.run(["docker", "compose", "-p", stand.project, *stand.files, "down", "-v",
                            "--remove-orphans"], capture_output=True, text=True)
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
