"""beacon.py — the two shared VRF helpers `lib.sh` hoisted out of the case scripts.

`wait_nodes_have` (lib.sh:348-371) and `assert_beacon_window` (lib.sh:310-343) were pulled up so
the VRF, fault and boundary cases share ONE copy of the cross-node prev_randao compare instead of
three that drift. They live here rather than in `core/` for the same reason: they are case-layer
assertions over the node set, not transport.

The verdict itself is in `verdicts.evaluate_beacon_window`; this module is the READING half — it
decides which heights and which nodes, and hands the readings over.
"""

from __future__ import annotations

from . import verdicts
from .driver import BEACON_NODES
from ...core import nodes as core_nodes, topology

#: `lib.sh:349` — how long the followers get to catch up to the top of a window.
NODES_HAVE_TIMEOUT_S = 120
#: `lib.sh:320,323` — the tail depth of the per-node diagnostic on a window failure.
WINDOW_FAIL_LOG_TAIL = 80


def wait_nodes_have(ctx, block, timeout=NODES_HAVE_TIMEOUT_S, nodes=BEACON_NODES) -> bool:
    """Wait until EVERY node in `nodes` can serve `block` (lib.sh:348-371).

    Not politeness — a correctness gate. The import follower syncs the validator chain over devp2p
    and lags the validators' finalized tip by a few blocks, so a cross-node compare issued the
    instant the tip moves races the follower's catch-up and reports "follower has no mixHash for
    block N" for a follower that is merely three seconds behind. On expiry this prints WHICH node
    is missing the block, so a genuinely stuck follower fails loud instead of being waited on.
    """
    def have_all():
        return all(_has(ctx, svc, block) for svc in nodes)

    if ctx.poll(have_all, timeout):
        return True
    print(f"  [wait_nodes_have] timeout at block {block} — per-node status:", flush=True)
    for svc in nodes:
        state = "has" if _has(ctx, svc, block) else "MISSING"
        print(f"    {svc}: {state} block {block}", flush=True)
    return False


def _has(ctx, service: str, block) -> bool:
    mh = ctx.mixhash_of(service, block)
    return bool(mh) and mh != "null"


def read_beacon_window(ctx, lo, hi, nodes=BEACON_NODES):
    """[(height, [mixhash per node])] over the inclusive height range, in `nodes` order."""
    return [(n, [ctx.mixhash_of(svc, n) for svc in nodes]) for n in range(int(lo), int(hi) + 1)]


def assert_beacon_window(ctx, case: str, lo, hi, label: str, nodes=BEACON_NODES):
    """`assert_beacon_window` (lib.sh:310-343) — read the window and apply the verdict.

    Raises `SmokeFailure` on a bad window, after dumping the offending node's log tail the way
    bash does. Returns the per-height agreed values on success."""
    rows = read_beacon_window(ctx, lo, hi, nodes)
    ok, msg, mixes = verdicts.evaluate_beacon_window(list(nodes), rows, label)
    ctx.check(case, ok, f"(beacon-window) {msg}",
              on_fail=lambda: _dump_offender(ctx, msg, nodes))
    if not ctx.dry:
        print(f"  [{label}] blocks [{lo}..{hi}]: {len(mixes)}/{len(mixes)} distinct non-zero "
              f"prev_randao, byte-identical across all {len(nodes)} nodes", flush=True)
    return mixes


def _dump_offender(ctx, msg: str, nodes) -> None:
    """bash dumps `--tail=80` of the node named in the message for the two per-node failures (a
    missing block, a zero mixHash) and nothing for the two window-wide ones (disagreement, stuck
    randomness), whose diagnostic is the printed table. Same split here, derived from the message
    rather than duplicated as a flag."""
    for svc in nodes:
        if f"{svc} has no mixHash" in msg or f"on {svc}" in msg:
            ctx.dump_logs(WINDOW_FAIL_LOG_TAIL, svc)
            return


# ── the EPOCH-KEY AGREEMENT PLANE (reading half) ─────────────────────────────

#: The committee of the static stack. NOT `BEACON_NODES`: that set includes the import follower,
#: which is not a `--dpos` node and runs no agreement instance, so folding it in would turn every
#: per-node stage verdict into a guaranteed red.
COMMITTEE_NODES = tuple(topology.validator(i) for i in range(4))

#: The canned log a `--dry-run` answers the plane scan with. It has to walk the WHOLE happy path
#: (all four stages plus the rehydrate line, with the fields the parsers read), because a dry run
#: is the only fidelity evidence the transcript carries and a canned log missing a stage would
#: make the branch a live run takes untraversable.
_DRY_PLANE_LOG = "\n".join([
    f"INFO {verdicts.AGREE_REHYDRATE_LINE} entries=0",
    f"INFO {verdicts.AGREE_STARTED_LINE} epoch=3",
    f"INFO {verdicts.AGREE_DECIDED_LINE} epoch=3 view=1 pinned=4",
    f"INFO {verdicts.AGREE_ADOPTED_LINE} epoch=3 pinned=4",
    f"INFO {verdicts.AGREE_SHARE_LINE} epoch=3",
])

#: The canned registry a `--dry-run` answers the plane scrape with — every family the case parses,
#: at zero, so the transcript walks the same branch a healthy live run does.
_DRY_PLANE_METRICS = "\n".join(
    f"{m} 0" for m in (verdicts.AGREE_REJECT_METRIC,) + verdicts.AGREE_REPORTED_METRICS)


def assert_agreement_plane(ctx, case: str, epoch, nodes=COMMITTEE_NODES) -> None:
    """Observe the epoch-key agreement plane for `epoch` across the committee.

    The ORDER of the checks is the order of the plane's own story — started → agreed → adopted →
    share stored — so a failure names the earliest stage that is missing rather than the last
    symptom. Then the two cross-node identities (view, pinned size) and the rejection counter.

    THE VIEW IS PRINTED ON EVERY RUN, pass or fail. It is the number that says whether the key was
    agreed instantly or after a leader timeout, and a green run that does not report it leaves the
    single most diagnostic fact about the plane unrecorded.

    Reads each node's log ONCE (`logs_required`, which fails loud on an empty read — every stage
    check below is an existence assertion over that text, and an unreadable log satisfies none of
    them honestly)."""
    logs = {svc: ctx.logs_required(svc, case, f"agreement-plane scan ({svc})",
                                   dry_value=_DRY_PLANE_LOG)
            for svc in nodes}

    # The durable store, first: it is a property of the node's WIRING rather than of this epoch,
    # so a failure here explains every later one.
    ctx.check(case, *verdicts.evaluate_artifact_store_durable(
        {s: bool(verdicts.AGREE_REHYDRATE_LINE in t) for s, t in logs.items()}))

    diag = verdicts.agree_silence_diag(logs, epoch)
    for stage, line in (("instance started", verdicts.AGREE_STARTED_LINE),
                        ("set agreed", verdicts.AGREE_DECIDED_LINE),
                        ("agreed set adopted", verdicts.AGREE_ADOPTED_LINE),
                        ("PK_epoch + share stored", verdicts.AGREE_SHARE_LINE)):
        present = {s: bool(verdicts.epoch_lines(t, line, epoch)) for s, t in logs.items()}
        ctx.check(case, *verdicts.evaluate_agree_stage(stage, present, epoch, diag))

    # The LAST decided line per node: an instance that had to be re-run for the same target logs a
    # second one, and the one that counts is the one whose artifact the ceremony then adopted.
    decided = {}
    for svc, text in logs.items():
        hits = verdicts.epoch_lines(text, verdicts.AGREE_DECIDED_LINE, epoch)
        decided[svc] = hits[-1] if hits else ""
    views = {s: verdicts.agreed_view(ln) for s, ln in decided.items()}
    pinned = {s: verdicts.agreed_pinned(ln) for s, ln in decided.items()}
    if not ctx.dry:
        for svc in sorted(decided):
            print(f"  [agreement plane] {svc}: epoch {epoch} agreed at view={views[svc]} "
                  f"pinned={pinned[svc]}", flush=True)
    ctx.check(case, *verdicts.evaluate_agree_view(views, epoch))
    ctx.check(case, *verdicts.evaluate_agree_pinned(pinned, epoch))

    # ONE registry scrape per node, then eight families parsed out of it. Eight separate
    # `node_metric` calls would be eight `docker compose exec … curl`s per node at eight different
    # instants — a set of counters read as a snapshot that is not one.
    texts = {svc: ctx.node_metrics_text(svc, dry_value=_DRY_PLANE_METRICS) for svc in nodes}
    rejects = {svc: core_nodes.gauge_val(txt, verdicts.AGREE_REJECT_METRIC)
               for svc, txt in texts.items()}
    ok, msg = verdicts.evaluate_agree_rejections(rejects)
    ctx.check(case, ok, msg)
    if not ctx.dry:
        print(f"  [agreement plane] {msg}", flush=True)
        for m in verdicts.AGREE_REPORTED_METRICS:
            vals = ", ".join(f"{s}={core_nodes.gauge_val(texts[s], m) or 'na'}"
                             for s in sorted(texts))
            print(f"  [agreement plane] {m}: {vals}", flush=True)
