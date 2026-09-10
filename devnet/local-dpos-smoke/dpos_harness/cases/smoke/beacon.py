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
COMMITTEE_NODES = tuple(topology.validator(i) for i in range(5))

#: The canned log the COMMITTEE-CHANGED branch scans under `--dry-run`. It walks the WHOLE happy
#: path (all four stages plus the rehydrate line, with the fields the parsers read) so that
#: branch's body is traversable at all. A dry run of THIS stand does not reach it — the canned
#: committees below are equal, so the transcript takes the carry-forward branch the live stand
#: takes — and the changed branch's own directions are driven by `tests/test_smoke_cases.py`.
_DRY_PLANE_LOG = "\n".join([
    f"INFO {verdicts.AGREE_REHYDRATE_LINE} entries=0",
    f"INFO {verdicts.AGREE_STARTED_LINE} epoch=3",
    f"INFO {verdicts.AGREE_DECIDED_LINE} epoch=3 view=1 pinned=4",
    f"INFO {verdicts.AGREE_ADOPTED_LINE} epoch=3 pinned=4",
    f"INFO {verdicts.AGREE_SHARE_LINE} epoch=3",
])

#: The canned log a `--dry-run` answers the CARRY-FORWARD scan with: the store's rehydrate line
#: and NOT ONE stage line. A dry run walks that branch (the canned committees below are equal),
#: because it is the branch this stand's live run takes — its committee never rotates.
_DRY_CARRY_LOG = f"INFO {verdicts.AGREE_REHYDRATE_LINE} entries=0"

#: The canned registry a `--dry-run` answers the plane scrape with — every family the case parses,
#: at zero, so the transcript walks the same branch a healthy live run does.
#:
#: IT RENDERS THE SAMPLE SPELLING, `# HELP`/`# TYPE` included, and not the registered one. A canned
#: registry written in the names the reader passes in is a fixture that agrees with the reader by
#: construction: it walked the happy branch of `_assert_artifact_rejections` for every dry run
#: while the live scrape — which doubles the `_total` (`nodes.counter_sample`) — made all eight
#: families unreadable. The dry registry must be shaped like the thing it stands in for.
_DRY_PLANE_METRICS = "\n".join(
    f"# HELP {m} canned.\n# TYPE {m} counter\n{core_nodes.counter_sample(m)} 0"
    for m in (verdicts.AGREE_REJECT_METRIC,) + verdicts.AGREE_REPORTED_METRICS)

#: `getEpochCommittee(uint64)(address[])` — the ONE read the plane branch is chosen from.
COMMITTEE_SIG = "getEpochCommittee(uint64)(address[])"

#: The canned committee a `--dry-run` answers BOTH committee reads with. Deliberately the SAME
#: value for `epoch-1` and `epoch`: the static stand never rotates, so an unchanged committee is
#: what a live run reads, and the transcript must walk the branch the live run walks.
_DRY_COMMITTEE = "[" + ", ".join("0x" + c * 40 for c in "1234") + "]"


def assert_epoch_key_plane(ctx, case: str, epoch, window, nodes=COMMITTEE_NODES) -> str:
    """Observe how epoch `epoch` got its key, on the branch the CHAIN says it took.

    A ceremony runs only on a committee change: `commitEpochCommittee` sets
    `dkgQual[e] = (committee[e] != committee[e-1])` and that bit is the only thing that announces
    an agreement target (`staking-reader/reader.rs:139,:643`). So there are two healthy stories for
    an epoch and exactly one of them is true of any given epoch:

      * COMMITTEE CHANGED — every committee member runs the whole four-stage story, at one view,
        over one pinned set (`assert_agreement_plane`);
      * COMMITTEE UNCHANGED — nobody runs anything, `chain_key_epoch` walks back to the last set
        bit and carry-forward serves the key (`assert_carry_forward_plane`).

    Demanding the first unconditionally is what made this a false red on this stand, whose
    committee is frozen for the whole run. The branch is decided by READING BOTH COMMITTEES, never
    by the presence or absence of a plane log line — that inference is circular, because the log
    lines are the very thing the branch then judges.

    `window` is `[(height, agreed prev_randao)]` from the boundary window this case already
    verified; the carry branch uses it as its key evidence rather than probing the chain again.

    Returns the one-sentence summary of what was proved, for the case's own report line.
    """
    prev_raw = ctx.staking_call(COMMITTEE_SIG, int(epoch) - 1, dry_value=_DRY_COMMITTEE)
    cur_raw = ctx.staking_call(COMMITTEE_SIG, int(epoch), dry_value=_DRY_COMMITTEE)
    ok, msg, changed = verdicts.evaluate_committee_change(prev_raw, cur_raw, epoch)
    ctx.check(case, ok, msg)
    if not ctx.dry:
        print(f"  [agreement plane] committee[{epoch}] "
              f"{'!=' if changed else '=='} committee[{int(epoch) - 1}] — "
              f"{'a ceremony was due' if changed else 'no ceremony was due (carry-forward)'}",
              flush=True)
    if changed:
        return assert_agreement_plane(ctx, case, epoch, nodes)
    return assert_carry_forward_plane(ctx, case, epoch, window, nodes)


def assert_agreement_plane(ctx, case: str, epoch, nodes=COMMITTEE_NODES) -> str:
    """The COMMITTEE-CHANGED branch: observe the epoch-key agreement plane for `epoch`
    across the committee. Reached only when `committee[epoch] != committee[epoch-1]`, which
    is what set the chain's `dkgQual` bit and made a ceremony due at all.

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
    for stage, line in verdicts.AGREE_STAGES:
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

    _assert_artifact_rejections(ctx, case, nodes)
    return (f"the epoch-{epoch} key was AGREED on the plane — every committee member started an "
            "instance, decided one pinned set at the same view, adopted it and stored its share")


def assert_carry_forward_plane(ctx, case: str, epoch, window, nodes=COMMITTEE_NODES) -> str:
    """The other branch: `committee[epoch] == committee[epoch-1]`, so the chain's `dkgQual` bit is
    CLEAR and no ceremony was due. Two things are asserted, and each can go red on its own.

      * QUIESCENCE — not one of the four stage lines appears for this epoch on any committee node.
        A node that ran an instance here paid for a key `chain_key_epoch` will never name (a
        bit-clear epoch is not a mint), which means the plane's trigger no longer follows the
        on-chain committee diff.
      * THE KEY WAS THERE ANYWAY — the boundary window this case already verified must contain at
        least one height INSIDE this epoch, carrying a non-zero node-agreed `prev_randao`. That is
        carry-forward doing its job: a broken carry resolves no key and the deriver falls back to
        `order.digest()`, i.e. a zero mixHash.

    The durable artifact store and the artifact-rejection counter are asserted here too. Both are
    properties of the node's WIRING rather than of this epoch's ceremony, so an epoch that ran no
    ceremony is no reason to stop reading them."""
    logs = {svc: ctx.logs_required(svc, case, f"carry-forward plane scan ({svc})",
                                   dry_value=_DRY_CARRY_LOG)
            for svc in nodes}
    ctx.check(case, *verdicts.evaluate_artifact_store_durable(
        {s: bool(verdicts.AGREE_REHYDRATE_LINE in t) for s, t in logs.items()}))

    seen = {svc: [stage for stage, line in verdicts.AGREE_STAGES
                  if verdicts.epoch_lines(text, line, epoch)]
            for svc, text in logs.items()}
    ctx.check(case, *verdicts.evaluate_agree_quiescent(seen, epoch))

    lo = verdicts.epoch_start(ctx.activation_block, ctx.interval, int(epoch))
    hi = verdicts.epoch_start(ctx.activation_block, ctx.interval, int(epoch) + 1) - 1
    ok, msg = verdicts.evaluate_carry_forward_key(epoch, window, lo, hi)
    ctx.check(case, ok, msg)
    if not ctx.dry:
        print(f"  [agreement plane] no instance started for epoch {epoch} on any of the "
              f"{len(nodes)} committee node(s); the key was carried forward — {msg}", flush=True)

    _assert_artifact_rejections(ctx, case, nodes)
    return (f"NO ceremony was due for epoch {epoch} (committee[{epoch}] == "
            f"committee[{int(epoch) - 1}], so the chain's dkgQual bit is clear): no committee "
            "member started an agreement instance, and the boundary window shows the epoch was "
            "served a usable key anyway — the carry-forward path")


def _assert_artifact_rejections(ctx, case: str, nodes) -> None:
    """`dpos_dkg_artifact_rejected_total == 0`, plus the seven reported-never-asserted families.

    ONE registry scrape per node, then eight families parsed out of it. Eight separate
    `node_metric` calls would be eight `docker compose exec … curl`s per node at eight different
    instants — a set of counters read as a snapshot that is not one.

    ALL EIGHT ARE READ THROUGH `counter_sample`. Every one of them is a `Counter` whose REGISTERED
    name already ends in `_total` (`beacon/metrics.rs`), and the registry appends its own, so the
    scrape says `…_total_total`. `gauge_val` matches the name anchored at its end, so passing the
    registered spelling returned "" from all four nodes on the first live run that reached here —
    the gating counter reported as unreadable and the seven printed ones as `na`. The anchoring is
    not the bug and is deliberately left alone (`node_metric` and the deferred-height gauge depend
    on it to keep `…_refetch` from answering for `…_refetch_failed_total`); the bug was reading a
    registry with the name the registry does not use.

    Shared by both branches: a rejection is a served artifact refused as proven misbehaviour, and
    an epoch that agreed nothing can still be served (and refuse) an artifact for another one."""
    texts = {svc: ctx.node_metrics_text(svc, dry_value=_DRY_PLANE_METRICS) for svc in nodes}
    rejects = {svc: core_nodes.gauge_val(txt, core_nodes.counter_sample(
                   verdicts.AGREE_REJECT_METRIC))
               for svc, txt in texts.items()}
    ok, msg = verdicts.evaluate_agree_rejections(rejects)
    ctx.check(case, ok, msg)
    if not ctx.dry:
        print(f"  [agreement plane] {msg}", flush=True)
        for m in verdicts.AGREE_REPORTED_METRICS:
            sample = core_nodes.counter_sample(m)
            vals = ", ".join(f"{s}={core_nodes.gauge_val(texts[s], sample) or 'na'}"
                             for s in sorted(texts))
            print(f"  [agreement plane] {m}: {vals}", flush=True)
