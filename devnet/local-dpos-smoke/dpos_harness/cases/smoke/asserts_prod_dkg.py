"""asserts_prod_dkg.py — the READING half of `case-vrf-dkg-halt` and `case-vrf-dkg-durability`.

The two DKG-durability cases on the rotation stack. They are a matched PAIR and neither means much
alone: durability phase 1 kills two of five committee members POST-seal and the cluster recovers;
halt sits two of five OUT of the ceremony before it seals and the chain wedges FOREVER. Same node
count, opposite participation, opposite outcome — which is why the halt case is the durability
case's negative control and why neither may be "optimised" into agreement with the other.

═══ SITTING A MEMBER OUT MEANS TEARING ITS JOURNAL ════════════════════════════════════════

A stop-and-start does NOT sit a member out any more. `maybe_start`'s `JournalLoad::Present` arm
re-derives the seeded dealer whenever `last_height < epoch_start(target) − DKG_MARGIN_BLOCKS`
(`crates/dpos/consensus/src/beacon/actor.rs:1046-1054`), so a victim restarted before the seal
deadline seals normally and the ceremony finalizes without it. Only the `JournalLoad::Torn` arm
(`:1056-1069`) sits out unconditionally, and only `torn_warned` (`:998`) makes that permanent for
the epoch. `tear_journal_to_torn` below is the single implementation of that, shared by BOTH cases
— durability tears ONE journal (quorum survives), halt tears TWO (dealer-quorum does not).

═══ THE ON-DISK PROBE FAMILY ══════════════════════════════════════════════════════════════

Both cases decide WHEN to kill from files inside a validator's datadir, not from a height:

    beacon-dkgjournal-e<E>.bin   written at DEAL-START, evicted only by the past-boundary sweep
    beacon-share-e<E>.bin        written at FINALIZE

so `journal present AND share absent` is the open ceremony window, and it stays open for the whole
deal→boundary span rather than being a race. The probes are `verdicts_onchain.journal_probe` /
`share_probe`, ported in chunk 4 and spelled identically by these two bash files — imported, not
re-derived.

TWO TRANSPORTS, and mixing them up is a silent no-op rather than an error:

  * `exec_test` → `docker compose exec`, which only works on a RUNNING container. Fine for the
    gates, which run before the kill.
  * `vol_test` / `vol_mutate` → a THROWAWAY `genesis-init` container on the shared /runtime volume.
    MANDATORY for any file question about a victim that is currently STOPPED, because
    exec-on-stopped succeeds and does nothing (`case-vrf-dkg-durability.sh:90-92`).

═══ THREE ASSERTIONS THAT PASS BY TIMEOUT ═════════════════════════════════════════════════

`STALL_WINDOW_S` (12 s, durability's consensus-quorum control), `FREEZE_WINDOW_S` (30 s) and
`REFREEZE_WINDOW_S` (15 s, the halt's terminal and permanent windows) are the assertions, not
waits before them. A chain that has merely not produced a block YET is indistinguishable from one
that never will over a short enough window, so shortening any of the three disarms the proof while
leaving it green (§2.4 item 3). They are copied.
"""

from __future__ import annotations

from . import verdicts_onchain as VO
from . import verdicts_rotation as VR
from .asserts_prod import (JOINER_IDX, JOINER_SERVICE, _ok, _say, assert_beacon_window,
                           assert_still_finalizing, initial_committee_gate,
                           register_joiner, scan_for_rotation, wait_nodes_have)
from ...core import topology
from ...stack.production_path import PP_VAL_COUNT


# ══ the on-disk / log probe family ═════════════════════════════════════════════════════════

def journal_present(ctx, idx, epoch) -> bool:
    """`dkg_journal_present <idx> <epoch>` — a NON-EMPTY ceremony journal exists.

    `find` under the datadir TREE rather than a hardcoded path, so a reth datadir-layout change
    cannot silently turn the gate into a permanent "absent" — which would make the case time out at
    the gate instead of failing at an assertion."""
    return ctx.exec_test(topology.validator(idx), VO.journal_probe(idx, epoch), dry_value=True)


def share_absent(ctx, idx, epoch) -> bool:
    """`dkg_share_absent <idx> <epoch>` — NO share file yet, i.e. the ceremony has not finalized.

    The INVERTED half of both gates. Combined with a present journal it is what makes the kill land
    PRE-finalize; without it the kill could land after the share was written, which is a different
    (and in the halt case, opposite) experiment."""
    return not ctx.exec_test(topology.validator(idx), VO.share_probe(idx, epoch), dry_value=False)


def share_present_vol(ctx, idx, epoch) -> bool:
    """`share_present_vol <idx> <epoch>` (`case-vrf-dkg-durability.sh:93-97`) — a non-empty share
    file, read through the VOLUME so it answers whether the validator is running or stopped."""
    return ctx.vol_test(
        f"find /runtime/reth-data/v{idx} -type f -name 'beacon-share-e{epoch}.bin' "
        "-size +0c 2>/dev/null | grep -q .")


def _logs(ctx, idx, dry_lines=()):
    """One whole-log read per node per gate iteration, ANSI-STRIPPED.

    bash issues a separate `docker compose logs` for EACH predicate — two per node in the halt
    gate, two in the durability seal gate — because a shell function cannot return text cheaply.
    One read answers all of them: they are several questions about the same text, and the only
    thing lost is a redundant re-read of a log that grows to megabytes (the trade
    `asserts_onchain.resumed` already makes)."""
    return ctx.logs_all(topology.validator(idx), dry_value="\n".join(dry_lines))


def _has_line(logs: str, message: str, epoch) -> bool:
    """`grep "<message>" | grep -qE "epoch=<N>( |,|$)"` — MESSAGE first, then the epoch FIELD.

    Two greps and not one, in bash's order: tracing renders fields in an order the case does not
    control, so a single pattern spanning both would be asserting a render order. The field pattern
    is right-anchored so `epoch=2` does not also match `epoch=20`."""
    return bool(VO.epoch_field_lines(logs, message, epoch))


def seal_line_present(ctx, idx, epoch) -> bool:
    return _has_line(_logs(ctx, idx), VR.SEAL_LINE, epoch)


def share_computed(ctx, idx, epoch, dry_value=False) -> bool:
    """`share_computed <idx> <epoch>` — the node FINALIZED a share for the epoch.

    Used in BOTH directions and by both cases: as the positive recovery signal (durability phase 3
    asserts every survivor has one) and as an ABSENCE (the halt's DKG-None discriminator, and the
    torn victim's sit-out). The dry answer is therefore a parameter — a single canned value would
    make one of the two directions untraversable in the transcript."""
    return _has_line(_logs(ctx, idx, (f"{VR.SHARE_LINE} epoch={epoch}",) if dry_value else ()),
                     VR.SHARE_LINE, epoch)


def torn_sitout_logged(ctx, idx, epoch) -> bool:
    """`torn_sitout <idx> <epoch>` — the node hit the `JournalLoad::Torn` arm for the epoch.

    The product warn at `crates/dpos/consensus/src/beacon/actor.rs:1060-1067`, emitted once per
    epoch under `torn_warned` (`:998`). BOTH DKG cases key their central claim on it: durability
    phase 3 that ONE sit-out is survivable, halt that TWO make the ceremony unreachable."""
    return _has_line(_logs(ctx, idx, (f"{VR.TORN_LINE} epoch={epoch}",)), VR.TORN_LINE, epoch)


def ceremony_started_count(ctx, idx, epoch) -> int:
    """`ceremony_started_count <idx> <epoch>` — how many times the node logged `ceremony started`
    (`actor.rs:1133`) for the epoch. A SECOND one after a torn restart is a re-deal."""
    return len(VO.epoch_field_lines(
        _logs(ctx, idx, (f"{VR.CEREMONY_STARTED_LINE} epoch={epoch}",)),
        VR.CEREMONY_STARTED_LINE, epoch))


def tear_journal_to_torn(ctx, idx, epoch, tag: str) -> None:
    """Make v<idx> sit out epoch <epoch>'s ceremony PERMANENTLY, by tearing its on-disk ceremony
    journal while the container is STOPPED. `case-vrf-dkg-durability.sh:399-416`, and the halt case
    runs the same three writes against two victims.

    WHY A TEAR AND NOT A KILL. Stopping a member and starting it back is NOT enough any more:
    `maybe_start`'s `JournalLoad::Present` arm re-derives the seeded dealer whenever
    `last_height < epoch_start(target) − DKG_MARGIN_BLOCKS` (`beacon/actor.rs:1046-1054`), so a
    victim restarted before the seal deadline seals normally and the ceremony finalizes without it
    ever having missed anything. The `Torn` arm (`:1056-1069`) is the one that sits out
    UNCONDITIONALLY, and `torn_warned` makes that permanent for the epoch.

    THE RECIPE (`share_state.rs:470-516`): `load_journal` returns `Torn` iff the file is NON-EMPTY
    and nothing decoded. Overwriting the first record's 4-byte big-endian length prefix with
    0xffffffff makes `after_len.len() < len` at `:495` — "truncated record body" — so the loop
    BREAKS before `decode_record` ever runs (which is why the recipe is state-agnostic: plaintext
    and encrypted framing tear identically) and `out` is empty while the file is not. Truncating to
    zero bytes is the WRONG recipe: `:483` maps an empty file to `NoFile` and the node RE-DEALS.

    TWO THINGS THAT MAKE IT A SILENT NO-OP IF DROPPED:

      * the SHARE FILE is deleted first. `maybe_start` returns at `actor.rs:982-989` on
        `store.contains_key(&target)`, BEFORE `load_journal` — a finalized share short-circuits the
        whole thing and the torn journal is never even read.
      * every write goes through a THROWAWAY container on the shared volume. `docker compose exec`
        on a STOPPED container SUCCEEDS and does nothing, which would leave the journal intact and
        the case measuring a node that was never torn.

    The readback is therefore an assertion, not a log line."""
    ctx.vol_mutate(idx, f"beacon-share-e{epoch}.bin", '[ -n "$f" ] && rm -f "$f"; true',
                   note=f"{tag}-rm-share")
    ctx.vol_mutate(idx, f"beacon-dkgjournal-e{epoch}.bin",
                   '[ -n "$f" ] && printf "\\377\\377\\377\\377" | dd of="$f" bs=1 count=4 '
                   'conv=notrunc 2>/dev/null; true',
                   note=f"{tag}-tear-journal")
    firstbytes = ctx.vol_mutate(idx, f"beacon-dkgjournal-e{epoch}.bin",
                                'od -An -tx1 -N4 "$f" 2>/dev/null | tr -d " \\n"; true',
                                note=f"{tag}-verify-tear", dry_value=VR.TORN_MAGIC)
    ctx.check(*VR.evaluate_corruption_landed(firstbytes, idx))
    _say(ctx, f"corruption verified: v{idx}'s E{epoch} journal first 4 bytes = 0x{VR.TORN_MAGIC} "
              "(will load as Torn)")


def assert_sat_out_torn(ctx, idx, epoch) -> None:
    """The victim hit the Torn arm and did NOT re-deal — polled, because the warn fires on the
    first post-restart `maybe_start` tick rather than at boot.

    Both halves are needed and they fail differently: a missing Torn line means the tear landed as
    something other than `Torn` (most likely `NoFile`), and a SECOND `ceremony started` means the
    node re-dealt, which is self-equivocation and would also put the dealer back in the count."""
    ctx.poll(lambda: torn_sitout_logged(ctx, idx, epoch), VR.TORN_POLL_S,
             poll_s=VR.TORN_POLL_GAP_S)
    ctx.check(*VR.evaluate_torn_sitout(torn_sitout_logged(ctx, idx, epoch), idx, epoch),
              on_fail=lambda: ctx.dump_logs(160, topology.validator(idx)))
    started = ceremony_started_count(ctx, idx, epoch)
    ctx.check(*VR.evaluate_no_re_deal(started, idx, epoch),
              on_fail=lambda: ctx.dump_logs(160, topology.validator(idx)))
    _say(ctx, f"v{idx} sat out on the Torn arm for E{epoch} — no re-deal ({started} 'ceremony "
              "started')")


# ══ case-vrf-dkg-halt ══════════════════════════════════════════════════════════════════════

def assert_vrf_dkg_halt(ctx) -> None:
    """`scripts/case-vrf-dkg-halt.sh` — the >f PRE-SEAL TERMINAL halt (G2).

    On the first driven rotation ceremony, TWO committee[E_new] STAYERS are stopped BEFORE they
    seal, have their E_new ceremony journals TORN on disk, and are restarted. Each comes back into
    `JournalLoad::Torn` and sits out E_new permanently, so the new committee's DKG can never reach
    dealer-quorum 4 → `beacon_for_epoch(E_new)` stays None → on E_new's first block every proposer
    hits the `skipping propose` arm and SKIPS the boundary view FOREVER. The chain can never
    produce E_new's first block, so it can never reach a later change-epoch to self-heal. A
    TERMINAL halt, not a heal-at-next-epoch.

    WHY THE JOURNAL IS TORN AND NOT MERELY THE NODE KILLED. The case used to just stop the two
    victims and start them again, on the premise that a pre-seal restart resumes player-only. That
    premise is GONE: `maybe_start`'s `Present` arm re-derives the seeded dealer whenever
    `last_height < epoch_start(target) − DKG_MARGIN_BLOCKS` (`beacon/actor.rs:1046-1054`), so a
    victim restarted before the seal deadline seals normally, the ceremony finalizes, and the
    boundary crosses. The case failed for exactly that reason and it was a harness defect, not a
    product bug. The `Torn` arm (`:1056-1069`) sits out unconditionally and `torn_warned` (`:998`)
    makes it permanent — that is the only mechanism that still removes a dealer from the count.

    AND THE TEAR IS NOT SUFFICIENT ON ITS OWN. A victim that had already SEALED has broadcast its
    dealer log, so the survivors finalize regardless of what its journal says afterwards. The tear
    must therefore land before the SEAL DEADLINE (`epoch_start(E_new) − DKG_MARGIN_BLOCKS`), which
    is what the pre-seal gate and its rail enforce.

    WHY THE VICTIMS COME BACK rather than staying down: on n=5 the DKG dealer-quorum and the
    consensus notarization quorum are BOTH 4. Keeping two down stalls CONSENSUS below the boundary
    (3 < 4), so the proposer never even reaches the boundary view and `skipping propose` never
    fires — an indistinct consensus stall, not the DKG-None boundary skip this case isolates.
    Restarting restores consensus quorum (the chain CLIMBS to the boundary) while the torn sit-out
    leaves the DKG permanently at 3 dealers, so the boundary is reached and skipped.

    ANTI-VACUITY. A chain frozen for an unrelated reason must not read as a pass, so the halt is
    only claimed after the mechanism is proven: the corruption readback
    (`evaluate_corruption_landed`), the Torn-arm line on BOTH victims with no re-deal
    (`assert_sat_out_torn`), and no member holding an E_new share at all.
    `VR.evaluate_terminal_halt` is then the line that fails if a shareless committee is ever
    allowed across the boundary.
    """
    e0, got0, addrs = initial_committee_gate(ctx)
    _say(ctx, f"bring-up done: committee(epoch {e0}) == initial 5")

    joiner = addrs[JOINER_IDX]
    reg_floor = register_joiner(ctx, addrs)
    ctx.check(bool(ctx.wait_converge(VR.REG_CONVERGE_S, reg_floor)),
              "nodes lost alignment during v5 registration",
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, JOINER_SERVICE))
    e_new, got_new = scan_for_rotation(ctx, got0, joiner)

    # The two victims: committee[E_new] STAYERS from v1..v4. Not v0 (killing it removes the host
    # RPC every reading in the case goes through) and not the joiner (it is not a stayer).
    kills = VR.kill_candidates(addrs, got_new)
    ctx.check(*VR.evaluate_kill_set(kills, got_new))
    k0, k1 = (kills + [1, 2])[:2]
    _say(ctx, f"pre-seal victims = validator-{k0} + validator-{k1} (committee[E_new] stayers, not "
              "leader/joiner)")

    # ── the PRE-seal gate ────────────────────────────────────────────────────────────
    #
    # The rail rides INSIDE the loop, and the order within one iteration is bash's: the gate is
    # evaluated first and the rail only when the gate is not yet satisfied. Hoisting the rail out
    # would report "the window never opened" for a run whose window merely closed — 600 s late and
    # with the wrong diagnosis.
    #
    # The rail is the SEAL DEADLINE (`boundary − DKG_MARGIN_BLOCKS`, actor.rs:83), not the
    # boundary: the ceremony seals a margin early, so the last 20 blocks before the boundary are
    # already post-seal and a victim torn there has ALREADY broadcast its dealer log — the
    # survivors finalize without it and the boundary crosses. That is the outcome this case is the
    # negative control for. Railing on the boundary accepted it silently.
    boundary = ctx.epoch_first_block(e_new)
    box = {"gated": False}

    def both_preseal():
        if all(VR.preseal_ok(journal_present(ctx, i, e_new),
                             seal_line_present(ctx, i, e_new),
                             share_absent(ctx, i, e_new)) for i in (k0, k1)):
            box["gated"] = True
            return True
        ctx.check(*VR.evaluate_seal_window_not_missed(ctx.finalized_dec(), boundary,
                                                      f"E_new={e_new}"))
        return False

    ctx.poll(both_preseal, VR.PRESEAL_GATE_S, poll_s=1)
    ctx.check(*VR.evaluate_preseal_gate(box["gated"], k0, k1))
    pre_halt = ctx.baseline_height()
    _say(ctx, f"v{k0} and v{k1} are both PRE-seal for E_new={e_new} (terminal kill window); "
              f"pre-halt finalized={pre_halt}")

    # ── stop PRE-seal, TEAR both journals, then RESTART (see the docstring) ─────────
    #
    # The stop is only the window in which the files can be touched — the sit-out comes from the
    # tear, not from the downtime. Both victims are torn while BOTH are down, so neither can seal
    # in the gap: if v_k0 were restarted before v_k1's journal was torn it could still reach the
    # deadline and seal, putting a fourth dealer log back in the count.
    ctx.compose_stop(topology.validator(k0), topology.validator(k1), note="halt-preseal-stop")
    for i in (k0, k1):
        tear_journal_to_torn(ctx, i, e_new, tag=f"halt-v{i}")
    ctx.sleep(VR.RESTART_GAP_S)
    ctx.compose_start(topology.validator(k0), topology.validator(k1), note="halt-preseal-start")

    # ── the SIT-OUT: the mechanism, asserted before anything is measured ────────────
    #
    # Fail here rather than 600 s later at the climb or, worse, green at a freeze that had another
    # cause. Without this the case would claim a shareless-committee halt from a chain that might
    # simply have lost a peer.
    for i in (k0, k1):
        assert_sat_out_torn(ctx, i, e_new)
    _say(ctx, f"both victims sat out E_new={e_new} on the Torn arm — the C_r1 ceremony can reach "
              "at most 3 dealer logs < dealer-quorum 4")

    # ── the CLIMB: the discriminator, not a warm-up ─────────────────────────────────
    edge = boundary - 1
    _say(ctx, f"waiting for the chain to climb to the E_new={e_new} boundary edge ({edge}) — "
              "proves the restarted nodes rejoined and the boundary view is reached")
    ctx.poll(lambda: ctx.tip_dec(dry_value=edge) >= edge, VR.CLIMB_S, poll_s=3)
    head = ctx.tip_dec(dry_value=edge)
    ctx.check(*VR.evaluate_climbed(head, edge),
              on_fail=lambda: [ctx.dump_logs(40, topology.validator(i)) for i in (0, k0, k1)])
    _say(ctx, f"chain climbed to head={head} (boundary edge {edge}) — quorum restored, boundary "
              "reached")

    # ── the POSITIVE log, best-effort ───────────────────────────────────────────────
    #
    # It may lag the head reaching the edge (the proposer's first boundary attempt plus a log
    # flush), so it is NOT the hard gate — the freeze plus the no-share proof below is
    # authoritative and timing-robust. Printed either way, because when it IS there it names the
    # mechanism outright.
    if _has_line(ctx.logs_all_project(), VR.SKIP_PROPOSE_LINE, e_new):
        _say(ctx, f"POSITIVE log — a proposer reached + SKIPPED the E_new={e_new} boundary view "
                  "(beacon=None)")
    else:
        _say(ctx, "(skipping-propose log not yet flushed; the head-frozen-at-edge + no-share proof "
                  "below is authoritative)")

    # ── the TERMINAL halt: a SUSTAINED no-progress window ───────────────────────────
    halt_head = ctx.tip_dec(dry_value=edge)
    _say(ctx, f"asserting the SUSTAINED no-progress halt at the boundary edge (head frozen >= "
              f"{VR.FREEZE_WINDOW_S} s from {halt_head})")
    frozen = _frozen_for(ctx, VR.FREEZE_WINDOW_S)

    def terminal_detail():
        h0 = ctx.tip_dec()
        ctx.sleep(VR.FREEZE_SAMPLE_S)
        return VR.evaluate_terminal_halt(False, h0, ctx.tip_dec())[1]

    ctx.check(frozen, terminal_detail,
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, topology.PINNED_RPC_HOST))
    _say(ctx, "head frozen over the window (the boundary view is skipped forever)")

    # ── the DKG-None discriminator ─────────────────────────────────────────────────
    #
    # NO committee member computed an E_new share. This is what separates the DKG-None terminal
    # halt (the ceremony stayed below dealer-quorum, so nobody holds a share) from a hypothetical
    # successful-DKG stall, where the survivors WOULD hold one. An ABSENCE per node.
    for i in range(PP_VAL_COUNT):
        ctx.check(*VR.evaluate_no_share_computed(i, share_computed(ctx, i, e_new), e_new))
    _say(ctx, "no committee member finalized an E_new share (the DKG stayed below dealer-quorum 4)")

    # A clean option-A stall, not a crash.
    ctx.check(*VR.evaluate_no_panic(VR.panic_lines(ctx.logs_all_project())))

    # ── the halt is PERMANENT ──────────────────────────────────────────────────────
    ctx.check(*VR.evaluate_permanent_halt(_frozen_for(ctx, VR.REFREEZE_WINDOW_S)))
    fin = ctx.finalized_dec()
    ctx.check(*VR.evaluate_below_boundary(fin, boundary))
    _say(ctx, f"halt is permanent — head still frozen, finalized {fin} never crossed the E_new "
              f"boundary {boundary}")

    _ok(ctx, f">f PRE-SEAL TERMINAL halt — on C_r1 (E{e_new}), TEARING the E{e_new} ceremony "
             f"journals of 2 committee stayers (v{k0}+v{k1}) PRE-seal and restarting them restored "
             "CONSENSUS quorum (the chain climbed to the boundary) while both sat out on the "
             "JournalLoad::Torn arm with no re-deal, leaving the DKG permanently below "
             f"DEALER-quorum 4 → beacon_for_epoch(E{e_new})=None → the proposer SKIPPED the "
             f"boundary view → the chain FROZE at the boundary (finalized never crossed {boundary})"
             " and stayed frozen: a clean permanent option-A halt, no panic, no E_new share. "
             "Tearing down.")


def _frozen_for(ctx, seconds) -> bool:
    """`head_frozen_for <s>` — `a=$(head_dec); sleep <s>; b=$(head_dec); (( b <= a ))`.

    The TIP, never finalized: with the DKG outcome None the proposer declines to BUILD the boundary
    block at all, so the tip is what freezes, and finalized could still be climbing toward it from
    below. `<=` rather than `==` is bash's and it accepts a head that went BACKWARDS as frozen,
    which is correct — a re-org is not progress either."""
    a = ctx.tip_dec()
    ctx.sleep(seconds)
    return VR.head_frozen(a, ctx.tip_dec())


# ══ case-vrf-dkg-durability ════════════════════════════════════════════════════════════════

def assert_vrf_dkg_durability(ctx) -> None:
    """`scripts/case-vrf-dkg-durability.sh:161-483` — the consolidated live-DKG durability suite,
    TWO ceremonies on ONE bring-up.

    PHASE 1 — POST-SEAL FLEET RECOVERY on the epoch-2 bootstrap ceremony. Two committee[2] members
    are stopped POST-seal, which drops the cluster below the CONSENSUS quorum so the chain STALLS
    (the timing-robust "recovery required" control), then restarted. Each reloads its OWN epoch-2
    share from disk, rejoins, the boundary crosses, and the beacon is byte-identical everywhere
    with no slash.

    The recovery mechanism is the DURABLE SHARE FILE and not the journal resume, and the case is
    explicit about it: the kill lands POST-finalize (the all-in seal+finalize is about one block),
    so `beacon-share-e2.bin` is already written and `build_beacon_plane::load_all` reloads it into
    the CeremonyStore with the EXACT share. The actor does not re-run `maybe_start` for an
    already-past epoch, so there is NO post-restart resume line for epoch 2 — which is precisely
    why this case does not subsume `case-vrf-dkg-restart-midwindow`, whose PRE-finalize restart is
    the only path that exercises `Player::resume`.

    PHASE 3 — TORN-JOURNAL SIT-OUT on the first driven rotation. A committee[E_new] member has its
    on-disk ceremony journal CORRUPTED while stopped; on restart it hits the `JournalLoad::Torn`
    arm, SITS OUT gracefully (never re-deals, so it cannot self-equivocate), and the ceremony
    finalizes among the other four = quorum, so the chain stays LIVE. It then re-derives
    prev_randao from the cert seed like any verify-only node, byte-identical to the survivors.
    """
    e0, got0, addrs = initial_committee_gate(ctx)
    _say(ctx, f"bring-up done: committee(epoch {e0}) == initial 5; beacon active from the epoch-2 "
              "bootstrap")

    _phase1_post_seal_recovery(ctx, addrs)
    torn = _phase3_torn_sitout(ctx, got0, addrs)

    assert_still_finalizing(ctx)
    _ok(ctx, "consolidated live-DKG durability suite on ONE rotation-stack bring-up — PHASE 1 "
             "(epoch-2): a 2-of-5 POST-seal kill STALLED the chain below the consensus quorum, "
             "both victims recovered their epoch-2 share from disk, the beacon stayed "
             "byte-identical across all nodes and neither was slashed; PHASE 3 (C_r1): registering "
             f"v5 rotated the committee, a torn journal on validator-{torn} hit the Torn arm and "
             "SAT OUT with no re-deal, the ceremony finalized among the four survivors, the chain "
             f"stayed LIVE, and validator-{torn} re-derived byte-identical prev_randao from the "
             "cert seed")


def _phase1_post_seal_recovery(ctx, addrs) -> None:
    """PHASE 1 (`:193-292`) — the post-seal fleet recovery on the epoch-2 bootstrap ceremony."""
    _say(ctx, "== PHASE 1: post-seal fleet recovery on the epoch-2 ceremony (kill v3+v4 "
              "post-seal) ==")
    v_a, v_b = VR.PHASE1_VICTIMS
    epoch2_start = ctx.epoch_first_block(VR.EPOCH2)
    probe = epoch2_start + VR.BOUNDARY_PROBE_OFFSET

    # ── the POST-seal gate ───────────────────────────────────────────────────────────
    #
    # WHY post-seal and not the wide journal window: a PRE-seal 2-kill is the TERMINAL halt (the
    # killed dealers never re-deal, the DKG cannot reach dealer-quorum, the boundary view is
    # skipped forever — that is `case-vrf-dkg-halt`). POST-seal the victims already broadcast their
    # Reveals AND finalized their share, so the survivors hold a quorum of logs and each victim has
    # persisted its share: recoverable.
    #
    # The seal line is MONOTONE — it never un-fires — so the gate is wide open from seal to the
    # boundary and there is no narrow seal→finalize race. The first version of this case used a
    # simultaneous `seal && share-absent` gate and MISSED, because the all-in finalize writes the
    # share about one block after the seal.
    _say(ctx, "waiting for v3 AND v4 to SEAL epoch 2 (seal line present + journal on disk) — the "
              "recoverable post-seal window")
    box = {"gated": False}

    def both_sealed():
        if all(seal_line_present(ctx, i, VR.EPOCH2) and journal_present(ctx, i, VR.EPOCH2)
               for i in (v_a, v_b)):
            box["gated"] = True
            return True
        ctx.check(*VR.evaluate_window_not_missed(ctx.finalized_dec(), epoch2_start, "epoch-2"))
        return False

    ctx.poll(both_sealed, VR.GATE_S, poll_s=1)
    ctx.check(*VR.evaluate_seal_gate(box["gated"]),
              on_fail=lambda: [ctx.dump_logs(60, topology.validator(i)) for i in (v_a, v_b)])
    _say(ctx, "v3 and v4 have both SEALED epoch 2 (recoverable post-seal kill window)")

    # ── the CONTROL: 2 of 5 down is below the consensus quorum, so the head must FREEZE ──
    pre_stall = ctx.baseline_height()
    _say(ctx, f"stopping v{v_a} + v{v_b} (3 of 5 online < consensus quorum 4 → chain must STALL); "
              f"pre-stall finalized={pre_stall}")
    ctx.compose_stop(topology.validator(v_a), topology.validator(v_b), note="phase1-stop")
    ctx.check(*VR.evaluate_control_stall(_frozen_for(ctx, VR.STALL_WINDOW_S)),
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, topology.PINNED_RPC_HOST))
    _say(ctx, "CONTROL ok — head frozen while 2 of 5 down (consensus quorum 4 not met)")

    # ── restart: the chain must RESUME and the victims must still hold their share ───
    _say(ctx, f"starting v{v_a} + v{v_b} — they must reload their epoch-2 share from disk and the "
              "chain must resume")
    ctx.compose_start(topology.validator(v_a), topology.validator(v_b), note="phase1-start")
    ctx.check(*VR.evaluate_rejoin(ctx.wait_finalized_ge(probe, VR.RESUME_S), probe),
              on_fail=lambda: [ctx.dump_logs(60, topology.validator(i)) for i in (0, v_a, v_b)])
    fin = ctx.finalized_dec(dry_value=pre_stall + 1)
    ctx.check(*VR.evaluate_resumed_past_stall(fin, pre_stall))
    _say(ctx, f"2-DOWN REJOIN ok — chain RESUMED and crossed the epoch-2 boundary (finalized now "
              f"{fin} > pre-stall {pre_stall}); no 2-down wedge")

    for i in (v_a, v_b):
        ctx.sample_until(int(VR.SHARE_POLL_S / VR.SHARE_POLL_GAP_S),
                         lambda _n, idx=i: share_present_vol(ctx, idx, VR.EPOCH2),
                         sleep_s=VR.SHARE_POLL_GAP_S, label=f"v{i}-share-poll")
        ctx.check(*VR.evaluate_share_recovered(share_present_vol(ctx, i, VR.EPOCH2), i),
                  on_fail=lambda idx=i: ctx.dump_logs(120, topology.validator(idx)))
        _say(ctx, f"v{i} RECOVERED — holds its epoch-2 share after the 2-down restart (durable "
                  "reload)")

    # The load-bearing correctness check: the recovered shares are CONSISTENT. A victim that
    # reloaded a WRONG or missing share would derive a divergent seed and fail here.
    ctx.check(wait_nodes_have(ctx, probe),
              f"not all nodes reached the epoch-2 window block {probe} after recovery")
    assert_beacon_window(ctx, epoch2_start, probe, "phase1 recovered epoch-2")

    # No liveness slash on either victim. Best-effort on the runtime-deployed default config (the
    # hard controls are the stall-then-resume and the per-victim share above): under the windowed
    # participation-floor model a recovered victim signs ~every cert, so it stays above the floor.
    slash_log = ctx.logs_all_project()
    for i in (v_a, v_b):
        addr = addrs[i]
        ctx.check(*VR.evaluate_not_slashed(VO.slash_hits(slash_log, addr), i))
        ctx.check(*VR.evaluate_status_readable(ctx.validator_status(addr), i))
    _say(ctx, "PHASE 1 OK — 2-of-5 post-seal kill: chain stalled, both victims recovered their "
              "epoch-2 share, beacon byte-identical, no slash")


def _phase3_torn_sitout(ctx, got0: str, addrs):
    """PHASE 3 (`:301-474`) — the torn-journal sit-out on the first driven rotation."""
    _say(ctx, "== PHASE 3: torn-journal sit-out on the first driven rotation (corrupt v3's "
              "journal) ==")
    # The cluster must be healthy after phase 1's recovery before phase 3 breaks it again — the
    # `case-fault.sh` hand-off pattern. Without it a phase-3 failure could be phase 1's damage.
    ctx.check(bool(ctx.wait_converge(VR.HANDOFF_CONVERGE_S)),
              "cluster not healthy/converged after Phase 1 — cannot hand off to Phase 3")

    joiner = addrs[JOINER_IDX]
    reg_floor = register_joiner(ctx, addrs)
    ctx.check(bool(ctx.wait_converge(VR.REG_CONVERGE_S, reg_floor)),
              "nodes lost alignment during v5 registration",
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, JOINER_SERVICE))
    e_new, got_new = scan_for_rotation(ctx, got0, joiner)

    torn = VR.torn_victim(addrs, got_new)
    ctx.check(*VR.evaluate_torn_victim(torn, got_new))
    torn = VR.TORN_PREFERRED_IDX if torn is None else torn
    torn_svc = topology.validator(torn)
    _say(ctx, f"torn victim = {torn_svc} (committee[E_new] member, not the leader)")

    # ── the open-window gate: journal present, share absent ─────────────────────────
    boundary = ctx.epoch_first_block(e_new)
    box = {"gated": False}

    def window_open():
        if journal_present(ctx, torn, e_new) and share_absent(ctx, torn, e_new):
            box["gated"] = True
            return True
        ctx.check(*VR.evaluate_window_not_missed(ctx.finalized_dec(), boundary, f"E_new={e_new}"))
        return False

    ctx.poll(window_open, VR.GATE_S, poll_s=1)
    ctx.check(box["gated"],
              f"v{torn}'s E_new={e_new} DKG journal never appeared pre-finalize (the C_r1 ceremony "
              "never started, or finalized before the poll)",
              on_fail=lambda: ctx.dump_logs(120, torn_svc))
    _say(ctx, f"v{torn}'s E_new={e_new} ceremony journal is on disk, pre-finalize")

    # ── stop, CORRUPT to Torn, restart ──────────────────────────────────────────────
    #
    # `tear_journal_to_torn` owns the recipe and the readback — the halt case runs the identical
    # three writes against two victims, and a second copy would drift exactly where it matters.
    _say(ctx, f"stopping v{torn}, corrupting its E_new={e_new} journal to Torn (via the volume), "
              "restarting")
    ctx.compose_stop(torn_svc, note="phase3-stop")
    tear_journal_to_torn(ctx, torn, e_new, tag="phase3")
    ctx.compose_start(torn_svc, note="phase3-start")

    # ── the chain must stay LIVE: quorum 4 survives exactly one sit-out ──────────────
    probe = boundary + VR.BOUNDARY_PROBE_OFFSET
    ctx.check(*VR.evaluate_chain_live_across_torn(ctx.wait_finalized_ge(probe, VR.RESUME_S)),
              on_fail=lambda: ctx.dump_logs(120, topology.PINNED_RPC_HOST))
    ctx.check(wait_nodes_have(ctx, probe),
              f"not all nodes reached the E_new window block {probe}")
    _say(ctx, "chain stayed LIVE across the E_new boundary on the 4 survivors")

    # ── the sit-out: the Torn arm fired, it did NOT re-deal, it is shareless ────────
    assert_sat_out_torn(ctx, torn, e_new)
    ctx.check(*VR.evaluate_shareless(share_computed(ctx, torn, e_new), torn, e_new))
    _say(ctx, f"v{torn} sat out gracefully — Torn arm fired, NO re-deal, shareless for E_new")

    # ── the ceremony finalized among the OTHER committee members ───────────────────
    finalized_members = 0
    for i in range(PP_VAL_COUNT):
        if i == torn or not VR.committee_has(got_new, addrs[i]):
            continue
        ctx.check(*VR.evaluate_member_finalized(share_computed(ctx, i, e_new, dry_value=True),
                                                i, e_new),
                  on_fail=lambda idx=i: ctx.dump_logs(120, topology.validator(idx)))
        finalized_members += 1
    ctx.check(*VR.evaluate_finalized_quorum(finalized_members))
    _say(ctx, f"C_r1 finalized among {finalized_members} survivors (>= quorum 4)")

    # The torn victim re-derives prev_randao from the cert seed like a verify-only node —
    # byte-identical to the survivors. The window compares ALL nodes, the victim included.
    assert_beacon_window(ctx, boundary, probe, f"phase3 torn-sitout E{e_new}")

    # A GRACEFUL sit-out: no panic, and no equivocation evidence. The second is the ON-CHAIN
    # witness for the log-side no-re-deal check above — a torn resume that re-dealt would produce
    # exactly this, so the two together tell a wrong corruption recipe from a product failure.
    ctx.check(*VR.evaluate_no_panic(VR.panic_lines(ctx.logs_all(torn_svc)), f"v{torn}"))
    ctx.check(*VR.evaluate_no_equivocation(VR.equiv_hits(ctx.logs_all_project(), addrs[torn]),
                                           torn))
    _say(ctx, f"PHASE 3 OK — torn-journal sit-out: v{torn} sat out gracefully (no panic, no "
              "re-deal, no equivocation), C_r1 finalized on the 4, chain stayed live, prev_randao "
              "byte-identical")
    return torn
