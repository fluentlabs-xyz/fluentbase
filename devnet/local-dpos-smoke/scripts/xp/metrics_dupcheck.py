#!/usr/bin/env python3
"""No duplicate series on any validator's two metrics registries, and the executor
families are exported at all (R-085 and the reth half of R-070, `.dpos-study/REGISTER.md`;
Э-17 / Э0.4 in `.dpos-study/EXPERIMENTS.md` and `E0-LOG.md`).

    metrics_dupcheck.py --stand N            a scripts/xp stand (ports 26000+N*100+i / 27000+…)
    metrics_dupcheck.py --urls URL [URL …]   any exposition endpoints

What breaks if the finding regresses: before Э0.4 every per-epoch simplex engine
registered under one fixed prefix with no epoch attribute, so a live validator exported
the same `outer_engine_epoch_manager_simplex_*` series up to seven times (223 colliding
series, 55.6 % of samples) and Prometheus dropped all but the first on every scrape while
reporting the target healthy. The fix adds an `epoch` attribute. This check counts
identical `name{labels}` keys per exposition — the exact computation Э-17 ran
(`dpos-experiments/scripts/e17_dupcheck.py`, reused here) — and fails on ANY collision.

The second half fails when the reth registry (`--metrics`) is not served or carries none
of the `reth_dpos_executor_*` families: that is the state Э-6 found the stand in, where the
counters the audit named for its experiment were recorded and exported nowhere.

Exit code: 0 clean, 1 a collision or a missing executor family, 3 an endpoint unreachable.
UNREAD DOMINATES, as in `agreement_check.py`: a run that could not scrape every endpoint did
not check what it was asked to check, and 1 would tell the caller coverage was complete. The
findings it DID observe are still printed as [FAIL] lines. `max()` produced this order by
accident; it is now the stated contract, so the two scripts cannot be read as disagreeing.
"""
import collections
import re
import sys
import urllib.request

SAMPLE = re.compile(r'^([a-zA-Z_:][a-zA-Z0-9_:]*(?:\{[^}]*\})?)\s+(.*)$')
EXECUTOR_FAMILY_PREFIX = "reth_dpos_executor_"
#: reth's recorder prefixes EVERY family it owns (`PrefixLayer::new("reth")`), so the exposition
#: says which of the two registries it is. The label cannot: under `--urls` the label IS the
#: URL, and the substring test on it silently skipped the executor half of this check.
RETH_REGISTRY_MARKER = "# TYPE reth_"


def scrape(url: str):
    with urllib.request.urlopen(url, timeout=8) as resp:
        return resp.read().decode()


def collisions(text: str):
    """Э-17's computation: identical name+labels keys appearing more than once."""
    series = collections.Counter()
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        m = SAMPLE.match(line)
        if m:
            series[m.group(1)] += 1
    dups = {k: v for k, v in series.items() if v > 1}
    total = sum(series.values())
    return total, len(series), dups


def main(argv):
    urls = []
    if "--stand" in argv:
        i_flag = argv.index("--stand")
        # A flag given as the LAST argument has no value; `argv[i + 1]` would raise IndexError
        # with a traceback instead of printing the usage.
        if i_flag + 1 >= len(argv) or not argv[i_flag + 1].isdigit():
            print("--stand needs a stand NUMBER\n" + __doc__, flush=True)
            return 2
        n = int(argv[i_flag + 1])
        for i in range(n):
            urls.append((f"validator-{i} reth", f"http://localhost:{26000 + n * 100 + i}/metrics"))
            urls.append((f"validator-{i} commonware",
     f"http://localhost:{27000 + n * 100 + i}/metrics"))
    elif "--urls" in argv:
        urls = [(u, u) for u in argv[argv.index("--urls") + 1:]]
        if not urls:
            print("--urls needs at least one endpoint\n" + __doc__, flush=True)
            return 2
    else:
        print(__doc__)
        return 2
    findings, unread = 0, 0
    for label, url in urls:
        try:
            text = scrape(url)
        except Exception as e:  # noqa: BLE001 — unreachable is UNREAD, never clean
            print(f"  [UNREAD] {label} {url}: {e}", flush=True)
            unread += 1
            continue
        total, distinct, dups = collisions(text)
        fams = collections.Counter(k.split("{")[0] for k in dups)
        line = f"{label}: {total} samples, {distinct} distinct series, {len(dups)} colliding"
        if dups:
            worst = ", ".join(f"{f}x{max(v for k, v in dups.items() if k.split('{')[0] == f)}"
                              for f in sorted(fams)[:5])
            print(f"  [FAIL] {line} — {worst}", flush=True)
            findings += 1
        else:
            print(f"  [ok ] {line}", flush=True)
        if any(ln.startswith(RETH_REGISTRY_MARKER) for ln in text.splitlines()):
            n_exec = sum(1 for ln in text.splitlines()
                         if ln.startswith("# TYPE " + EXECUTOR_FAMILY_PREFIX))
            if n_exec == 0:
                print(f"  [FAIL] {label}: no `{EXECUTOR_FAMILY_PREFIX}*` family on the "
                      "reth registry — the executor counters are recorded and "
                      "exported nowhere (R-070)", flush=True)
                findings += 1
            else:
                print(f"  [ok ] {label}: {n_exec} executor families exported", flush=True)
    if unread:
        return 3
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
