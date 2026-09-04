#!/usr/bin/env python3
"""E5 — the non-byzantine path at V > 4: how many HONEST exits the network takes.

usage: exits.py <N> [--same-epoch]

Validators leave one at a time via `undelegate(validator, self_stake)`, signed
with their own owner key. No byzantine node, no governance transaction, no
evidence. After each exit we wait for an epoch boundary and check whether the
chain is still alive. With `--same-epoch` every exit is sent back to back inside
one epoch — the check that the population can jump straight past the safe size
of four down to three.
"""
import json, subprocess, sys, time

N = int(sys.argv[1])
SAME = "--same-epoch" in sys.argv
P = f"xp{N}"
SUB = f"172.{20+N}.0"
RPC = f"http://localhost:{28000+N}"
STAKING = "0x0000000000000000000000000000000000520011"
INTERVAL, ACT = 32, 64

def sh(*a):
    return subprocess.run(list(a), capture_output=True, text=True).stdout.strip()

def alive():
    return [i for i in range(N)
            if sh("docker", "inspect", "-f", "{{.State.Status}}",
                  f"{P}-validator-{i}-1") == "running"]

def head():
    o = sh("cast", "block-number", "--rpc-url", RPC)
    try:
        return int(o)
    except Exception:
        return None

def epoch_of(b):
    return (b - ACT) // INTERVAL if b is not None and b >= ACT else None

def exit_validator(i):
    v = json.loads(sh("docker", "exec", f"{P}-validator-0-1", "cat",
                      "/runtime/addresses.json"))["validators"][i]
    key = "0x" + sh("docker", "exec", f"{P}-validator-0-1", "cat",
                    f"/runtime/keys/owner-{i}.hex")
    owner = sh("cast", "wallet", "address", "--private-key", key)
    amt = sh("cast", "call", "--rpc-url", RPC, STAKING,
             "getValidatorDelegation(address,address)(uint256)", v, owner).split()[0]
    b = head()
    out = sh("cast", "send", "--rpc-url", RPC, "--private-key", key, "--legacy",
             STAKING, "undelegate(address,uint256)", v, amt)
    st = [l for l in out.splitlines() if l.startswith("status")]
    bn = [l for l in out.splitlines() if l.startswith("blockNumber")]
    print(f"  exit validator-{i}: self-stake={amt} at block={b} (epoch {epoch_of(b)}) "
          f"-> {st[0] if st else 'NO STATUS'} {bn[0] if bn else ''}", flush=True)
    return bool(st and st[0].split()[1] == "1")

def wait_dpos_ready():
    """READINESS GATE. The first E5 run was void: the script started while
    bringupN.sh still held phase 1, the exits landed on the sequencer chain, and
    the "network went down" it reported was the flush gate's graceful stop, not
    the mechanism. Wait here until all N nodes run under --dpos (all alive,
    finalized above activation and still climbing)."""
    import re
    last = None
    for _ in range(120):
        a = alive()
        b = head()
        run = all("--dpos" in sh("docker", "inspect", "-f", "{{json .Config.Cmd}}",
                                 f"{P}-validator-{i}-1") for i in range(N))
        if len(a) == N and run and b is not None and b > ACT and (last is None or b > last):
            if last is not None:
                print(f"  DPoS ready: {N}/{N} alive, head={b} (epoch {epoch_of(b)})",
                      flush=True)
                return True
            last = b
        time.sleep(5)
    print("  TIMEOUT waiting for the DPoS phase", flush=True)
    return False

print(f"=== E5: N={N}, honest exits{' (all in one epoch)' if SAME else ''} ===", flush=True)
print(f"  visible population V={N}; prediction from the code: exit number {N-3} is the fatal one",
      flush=True)

if not wait_dpos_ready():
    sys.exit(3)

done = 0
ORDER = [int(x) for x in (sys.argv[2].split(",")
         if len(sys.argv) > 2 and sys.argv[2].isdigit() is False and "," in sys.argv[2]
         else [])] or list(reversed(range(N)))
for i in ORDER:                       # from the tail, so validator-0 (sequencer/hub) leaves last
    if len(alive()) < N:
        print("  network already down — stopping", flush=True)
        break
    ok = exit_validator(i)
    if not ok:
        print("  undelegate FAILED — stopping", flush=True)
        break
    done += 1
    print(f"  --- exits so far: {done}; V should now be {N-done} ---", flush=True)
    if SAME and done < N - 3:
        continue
    # Wait up to two epoch boundaries and look.
    for k in range(24):
        time.sleep(5)
        a = alive()
        b = head()
        print(f"    t+{5*(k+1):3d}s alive={len(a)}/{N} head={b} epoch={epoch_of(b)}",
              flush=True)
        if len(a) < N:
            break
    if len(alive()) < N:
        print(f"\n*** NETWORK DOWN after {done} honest exit(s) (V {N} -> {N-done}) ***",
              flush=True)
        break

print("\n=== fatal lines ===", flush=True)
for i in range(N):
    L = sh("docker", "logs", f"{P}-validator-{i}-1")
    fat = [l for l in L.splitlines() if "did not succeed" in l]
    print(f"  validator-{i}: {len(fat)} 'did not succeed'", flush=True)
    if fat:
        print("     " + fat[-1][:240], flush=True)
print("\n=== jail/tombstone/equivocation lines across all nodes ===", flush=True)
tot = 0
for i in range(N):
    L = sh("docker", "logs", f"{P}-validator-{i}-1")
    tot += sum(1 for l in L.splitlines()
               if any(k in l for k in ("tombston", "equivocat", "ValidatorJailed")))
print(f"  total: {tot}", flush=True)
