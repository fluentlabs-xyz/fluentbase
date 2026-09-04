#!/usr/bin/env python3
"""Watch stand xp<N> through an equivocation incident (EXPERIMENTS E1/E3/E4).

usage: inc.py <N> <byz-indices|-> <label>

Turns on equivocation for the listed validators (or for none, with '-'), then
polls the stand while the chain is alive and records:
  - the block/epoch at which nodes die, and the error text;
  - which nodes died and which survived;
  - jail/tombstone lines with their timestamps.
It then tries to bring everything back (`docker compose start`) and watches
whether the network heals on its own.
"""
import json, re, subprocess, sys, time

N = int(sys.argv[1]); BYZ = sys.argv[2]; LABEL = sys.argv[3]
XP = str(__import__("pathlib").Path(__file__).resolve().parent)
P = f"xp{N}"
SUB = f"172.{20+N}.0"
C1 = ["-f", f"{XP}/out/xp{N}.yml"]
C2 = C1 + ["-f", f"{XP}/out/xp{N}.dpos.yml"]
ANSI = re.compile(r"\x1b\[[0-9;]*m")
INTERVAL = 32
ACT = 64

def sh(*a, **kw):
    return subprocess.run(list(a), capture_output=True, text=True, **kw).stdout

def logs(i, tail=None):
    a = ["docker", "logs"] + (["--tail", str(tail)] if tail else []) + [f"{P}-validator-{i}-1"]
    r = subprocess.run(a, capture_output=True, text=True)
    return ANSI.sub("", r.stdout + r.stderr)

def rpc(i, method, params="[]"):
    body = f'{{"jsonrpc":"2.0","method":"{method}","params":{params},"id":1}}'
    out = sh("docker", "exec", f"{P}-validator-0-1", "sh", "-c",
             f"curl -s -m 5 -X POST -H 'Content-Type: application/json' --data '{body}' "
             f"http://{SUB}.{10+i}:8545")
    try:
        return json.loads(out).get("result")
    except Exception:
        return None

def state(i):
    return sh("docker", "inspect", "-f", "{{.State.Status}}/{{.State.ExitCode}}",
              f"{P}-validator-{i}-1").strip()

def epoch_of(b):
    return (b - ACT) // INTERVAL if b >= ACT else 0

def snap(tag):
    row = []
    for i in range(N):
        f = rpc(i, "eth_getBlockByNumber", '["finalized",false]') or {}
        h = rpc(i, "eth_blockNumber")
        n = int(f["number"], 16) if f.get("number") else None
        row.append(f"v{i}:{state(i)} fin={n}"
                   f"{'/e' + str(epoch_of(n)) if n is not None else ''}")
    print(f"  [{tag}] " + "  ".join(row), flush=True)
    return row

print(f"=== {LABEL}: N={N}, byzantine={BYZ} ===", flush=True)
snap("baseline")

if BYZ != "-":
    ov = sh(f"{XP}/byz_overlay.sh", str(N), BYZ).strip()
    if not ov:
        sys.exit("byz_overlay.sh refused (committee < 5?) — no overlay written")
    args = ["docker", "compose", "-p", P] + C2 + ["-f", ov, "up", "-d", "--force-recreate"] + \
           [f"validator-{i}" for i in BYZ.split(",")]
    print("  enabling equivocation:", " ".join(args[-3:]), flush=True)
    subprocess.run(args, capture_output=True, text=True)
    t_byz = time.time()
else:
    t_byz = time.time()

dead = {}
t0 = time.time()
while time.time() - t0 < 600:
    alive = [i for i in range(N) if state(i).startswith("running")]
    for i in range(N):
        if not state(i).startswith("running") and i not in dead:
            dead[i] = time.time() - t_byz
    snap(f"t+{time.time()-t_byz:5.0f}s")
    if len(dead) >= 1 and len(alive) == 0:
        break
    if len(dead) >= 1 and time.time() - t0 > 120 and len(alive) > 0:
        # A partial death is a result too; give it a little longer.
        pass
    time.sleep(4)

print(f"\n=== dead nodes (seconds after enabling byzantine): {dead} ===", flush=True)

print("\n=== fatal lines ===", flush=True)
for i in range(N):
    L = logs(i)
    fat = [l for l in L.splitlines() if "executor fatal error" in l
           or "did not succeed" in l or "OuterEngine exited" in l]
    print(f"  validator-{i}: {len(fat)} lines", flush=True)
    for l in fat[:3]:
        print("     " + l[:300], flush=True)

print("\n=== slasher / jail / tombstone timeline (validator-0) ===", flush=True)
for l in logs(0).splitlines():
    if any(k in l for k in ("equivocation charge", "tombstoned", "ValidatorJailed",
                            "holding verified", "severing its transport")):
        print("  " + l[:260], flush=True)

print("\n=== recovery: docker compose start, 120 s ===", flush=True)
subprocess.run(["docker", "compose", "-p", P] + C2 + ["start"] +
               [f"validator-{i}" for i in range(N)], capture_output=True, text=True)
for k in range(12):
    time.sleep(10)
    snap(f"restart+{10*(k+1)}s")
print("\n=== post-restart fatal lines (tail) ===", flush=True)
for i in range(N):
    L = logs(i, tail=400)
    fat = [l for l in L.splitlines() if "did not succeed" in l]
    print(f"  validator-{i}: {len(fat)} 'did not succeed' in last 400 lines", flush=True)
    if fat:
        print("     " + fat[-1][:300], flush=True)
