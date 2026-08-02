"""asserts_prod.py — the READING half of `case-production-path`, `case-vrf-rotation` and
`case-byzantine-vrf`, plus the trigger the four rotation-driving cases share.

`verdicts_rotation.py` decides what the readings MEAN; this module decides WHAT to read and WHEN,
and it is where the ordering that makes each assertion meaningful lives. `asserts_prod_dkg.py`
holds the other two cases and imports the shared half from here.

═══ THE SHARED TRIGGER ════════════════════════════════════════════════════════════════════

Four of the five cases open the same way: register validator-5 through governance, delegate enough
BLEND to outrank an incumbent, then SCAN the ahead-committed committees for the first one that
differs from E0's. `register_joiner` and `scan_for_rotation` are that, once.

The scan is not a formality and the four bash copies all say so: `committee[N]` reads `EffBal(N-1)`
and a delegate is effective in `EffBal(E+2)`, so v5 enters at E+3 — and every one of them
deliberately COMPUTES that by watching the chain rather than hardcoding it, "so a re-tuned EffBal
timeline does not silently skew us" (`case-vrf-rotation.sh:193`). A ported case that hardcoded E0+3
would still pass today and would start testing nothing the moment the warmup constants move.

═══ THE BEACON WINDOW, TWICE ══════════════════════════════════════════════════════════════

`assert_beacon_window` here is `lib.sh:332-365` and reads the FULL seven-node set.
`assert_honest_beacon_window` is `case-byzantine-vrf.sh:112-139`, which reads the six honest ones
and — deliberately — omits the across-height distinctness check. Both route through
`verdicts.evaluate_beacon_window`, the second with `require_distinct=False`, so the three
properties they DO share cannot drift apart. See that function's note for why the difference is
kept rather than tidied away.

═══ WHY THE DIAGNOSTIC PRINTS SURVIVED THE PORT ═══════════════════════════════════════════

`case-production-path.sh:198-200` prints four `[cm-diag]` lines unconditionally — head, epoch,
`nextEpochToCommit`, `committeeSelectionEpoch` and the first four committees — before it asserts
anything. They are kept because they are READS: dropping them changes the command transcript this
port is checked against, and because the committee-selection pair is the first thing anyone looks
at when the committee is not what the case expected.
"""

from __future__ import annotations

from . import verdicts as V
from . import verdicts_prod as VP
from . import verdicts_rotation as VR
from .prod import ProdFailure
from ...core import topology
from ...stack.production_path import PP_VAL_COUNT

#: `case-vrf-rotation.sh:108` — every DERIVING node on the production-path stack: the six
#: validators and the full-node. The full-node's membership is not decoration: it derives
#: prev_randao by verifying the cert seed against the committed PK_E without holding a share, so
#: its agreement is what proves the derive path is share-independent.
PROD_NODES = tuple(topology.validator(i) for i in range(PP_VAL_COUNT)) + (topology.FULL_NODE,)

#: `case-production-path.sh:113` / `lib.sh:1306` — the initial committee is v0..v4; the joiner is
#: the sixth container, which boots as a follower and joins later. That difference IS the rotation.
JOINER_IDX = PP_VAL_COUNT - 1
JOINER_SERVICE = topology.validator(JOINER_IDX)


# ══ shared plumbing ════════════════════════════════════════════════════════════════════════

def _ok(ctx, message: str) -> None:
    if not ctx.dry:
        print(f"OK ({ctx.label}): {message}", flush=True)


def _say(ctx, message: str) -> None:
    if not ctx.dry:
        print(f"  {message}", flush=True)


def owner_addrs(ctx, count=PP_VAL_COUNT):
    """`for i in 0..N-1; do pp_owner_addr $i; done` — the durable per-idx owner addresses.

    Read ONCE per use site and passed down, not re-read per comparison: each one is a
    `docker compose exec … cat` plus a host-side `cast wallet address`, and the committee
    membership loops below would otherwise issue six of them per poll iteration."""
    return [ctx.owner_addr(i, dry_value=f"0x{0xa0 + i:02x}" + "00" * 19) for i in range(count)]


def wait_nodes_have(ctx, block, timeout=VR.NODES_HAVE_S, nodes=PROD_NODES) -> bool:
    """`wait_nodes_have <block> [timeout]` (lib.sh:370-393) — every node can serve `block`.

    A correctness gate, not politeness. Followers (the full-node, and a freshly-promoted joiner)
    lag the validators at a fresh boundary, so a cross-node compare issued the instant the tip
    moves races their catch-up and reports "node behind" for a node that is three seconds late.
    On expiry it prints WHICH node is missing the block, as bash does, so a genuinely stuck
    follower fails loud instead of being waited on."""
    def have_all():
        return all(_has_block(ctx, svc, block) for svc in nodes)

    if ctx.poll(have_all, timeout, poll_s=1):
        return True
    print(f"  [wait_nodes_have] timeout at block {block} — per-node status:", flush=True)
    for svc in nodes:
        print(f"    {svc}: {'has' if _has_block(ctx, svc, block) else 'MISSING'} block {block}",
              flush=True)
    return False


def _has_block(ctx, service: str, block) -> bool:
    mh = ctx.mixhash_of(service, block)
    return bool(mh) and mh != "null"


def assert_beacon_window(ctx, lo, hi, label: str, nodes=PROD_NODES, require_distinct=True):
    """`assert_beacon_window <lo> <hi> <label>` (lib.sh:332-365) — read the window, apply the
    verdict, dump the offending node's tail on failure exactly as bash does."""
    rows = [(n, [ctx.mixhash_of(svc, n) for svc in nodes]) for n in range(int(lo), int(hi) + 1)]
    ok, msg, mixes = V.evaluate_beacon_window(list(nodes), rows, label,
                                              require_distinct=require_distinct)
    ctx.check(ok, f"(beacon-window) {msg}", on_fail=lambda: _dump_offender(ctx, msg, nodes))
    _say(ctx, f"[{label}] blocks [{lo}..{hi}]: {len(mixes)}/{len(mixes)} distinct non-zero "
              f"prev_randao, byte-identical across all {len(nodes)} nodes")
    return mixes


def assert_honest_beacon_window(ctx, lo, hi, label: str, nodes):
    """`assert_honest_beacon_window` (`case-byzantine-vrf.sh:112-139`) — the same window over the
    HONEST set only, and WITHOUT the across-height distinctness check.

    Both differences are the case's: the byzantine node's own reth may diverge while it churns
    forged boundary views, so including it would fail the compare for the behaviour being
    provoked; and bash's copy simply does not run the variance check, so neither does this."""
    return assert_beacon_window(ctx, lo, hi, label, nodes=nodes, require_distinct=False)


def _dump_offender(ctx, msg: str, nodes) -> None:
    """bash dumps `--tail=80` of the node NAMED in the message for the two per-node failures (a
    missing block, a zero mixHash) and nothing for the two window-wide ones, whose diagnostic is
    the printed table. Same split, derived from the message rather than duplicated as a flag."""
    for svc in nodes:
        if f"{svc} has no mixHash" in msg or f"on {svc}" in msg:
            ctx.dump_logs(VR.LOG_TAIL_NODE, svc)
            return


def initial_committee_gate(ctx, addrs=None, name_epoch=True):
    """`E0=$(pp_current_epoch); GOT0=$(pp_committee E0); [[ GOT0 == the initial five ]]`.

    The bring-up's own sanity gate, spelled by four cases immediately after it returns. It is what
    makes every later "the committee CHANGED" statement meaningful — without a pinned starting set,
    a rotation is just two committees that happen to differ.

    Returns `(e0, got0, addrs)` so the caller can carry all three forward without re-reading."""
    addrs = addrs if addrs is not None else owner_addrs(ctx)
    e0 = ctx.current_epoch()
    expect0 = VR.expected_initial_set(addrs[:ctx.profile.committee_size])
    # The canned committee is the EXPECTED one, so everything downstream of this gate — the
    # rotated set, the probe-member mapping, the displaced index — has a realistic five-address
    # baseline to derive from under `--dry-run`. A canned "" would make the whole case's committee
    # arithmetic degenerate in the transcript and hide the branch a live run takes.
    got0 = ctx.committee(e0, dry_value=expect0)
    ctx.check(*VR.evaluate_initial_committee(got0, expect0, e0 if name_epoch else None))
    _say(ctx, f"committee(epoch {e0}) == initial 5")
    return e0, got0, addrs


def register_joiner(ctx, addrs, idx=JOINER_IDX, delegate=True):
    """The shared TRIGGER: register + key + activate (+ delegate) validator-`idx`, verbatim from
    `case-production-path.sh:212-231` and copied unchanged into the other three.

    Six sends in a fixed order and each one has a reason to be where it is:

      1. `approve(STAKING_RT, 1e18)` — `registerValidator` pulls the self-stake, so the allowance
         must exist first. No `||` in bash: under `set -e` a failed approve aborts.
      2. `registerValidator(addr, 0, 1e18)` — commission 0, the minimum self-stake.
      3. `setConsensusKeys(...)` — AFTER registration (the validator record must exist) and after
         the bring-up's `setBlsVerifier` (the PoP is verified on the way in).
      4. governance `activateValidator(addr)` — Pending → Active. The joiner keeps running the
         unified follower substrate throughout; there is no operator choreography.
      5. `approve(STAKING_RT, 2e18)` + 6. `delegate(addr, 2e18)` — the weight that makes the
         joiner outrank an incumbent. `case-byzantine-vrf.sh:361` STOPS after step 4 and drives
         the weight through a dedicated toggle delegator instead, so that an `undelegate` later
         never trips `OwnerSelfStakeBelowMinimum` on the joiner's own self-stake. That is what
         `delegate=False` is.

    Returns the REG_FLOOR — the finalized head captured BEFORE the first send, which the caller
    then requires the cluster to converge strictly past. Captured first because the point is that
    the joiner's follower substrate tracked the chain THROUGH the registration load; a floor read
    afterwards would already include it."""
    reg_floor = ctx.check_external(topology.HOST_RPC_PORT).split("|", 1)[0]
    key = ctx.owner_key(idx)
    addr = addrs[idx]
    staking = ctx.staking_rt

    ctx.cast_send_checked([ctx.token, "approve(address,uint256)(bool)", staking, VR.STAKE_1E18],
                          note=f"approve-register-v{idx}", key=key)
    ctx.check(ctx.cast_send([staking, "registerValidator(address,uint16,uint256)",
                             addr, 0, VR.STAKE_1E18], note=f"registerValidator-v{idx}", key=key),
              f"registerValidator v{idx}")
    ck = ctx.consensus_keys(idx)
    ctx.check(ctx.cast_send([staking, "setConsensusKeys(address,bytes,bytes,bytes32)",
                             ck.get("validatorAddress", ""), ck.get("blsPubkeyUncompressed", ""),
                             ck.get("blsPoPUncompressed", ""), ck.get("peerPubkey", "")],
                            note=f"setConsensusKeys-v{idx}", key=key),
              f"setConsensusKeys v{idx}")
    ctx.gov_action(staking, ctx.calldata("activateValidator(address)", addr),
                   f"activateValidator-v{idx}")
    if delegate:
        ctx.cast_send_checked([ctx.token, "approve(address,uint256)(bool)", staking,
                               VR.DELEGATE_2E18], note=f"approve-delegate-v{idx}", key=key)
        ctx.check(ctx.cast_send([staking, "delegate(address,uint256)", addr, VR.DELEGATE_2E18],
                                note=f"delegate-v{idx}", key=key),
                  f"delegate v{idx}")
        _say(ctx, f"v{idx} registered + activated + delegated")
    else:
        _say(ctx, f"v{idx} registered + activated (not yet delegated into the committee)")
    return reg_floor


def rotated_dry(got0: str, joiner_addr: str) -> str:
    """The canned ROTATED committee for a `--dry-run`: the joiner in, the highest-sorted incumbent
    out.

    Same SIZE and genuinely DIFFERENT, which is what makes a dry run walk the change branch of
    `rotation_reached` rather than the no-change one — and, downstream, gives `probe_members` a
    five-container answer with validator-0 at the front instead of a single-element list. A dry
    transcript that exercised only the degenerate branch would show none of the per-member work
    that dominates a live run."""
    members = [a for a in (got0 or "").split() if a != joiner_addr]
    return " ".join(sorted(members[:-1] + [joiner_addr])) if members else joiner_addr


def scan_for_rotation(ctx, got0: str, joiner_addr: str, timeout=VR.ROTATION_SCAN_S):
    """`case-vrf-rotation.sh:196-209` — the FIRST ahead-committed committee that differs from E0's
    and contains the joiner.

    `committee[E+1]` is committed one boundary EARLY, so watching it is what lets the case observe
    the change before the chain reaches it. Returns `(e_new, got_new)`; raises through `ctx.check`
    on expiry or on a committee that turns out not to have changed after all.

    The second check is not redundant: the scan reads the ahead-committed set while this re-reads
    `committee[E_new]` once the boundary has been committed, and a DEFERRED change (the incoming
    DKG missed its qualification, so the incumbent was re-committed) makes the second read equal
    to E0's after the first said otherwise."""
    box = {"e": None}
    rotated = rotated_dry(got0, joiner_addr)

    def changed():
        e = ctx.current_epoch()
        ahead = ctx.committee(e + 1, dry_value=rotated)
        if VR.rotation_reached(ahead, got0, joiner_addr):
            box["e"] = e + 1
            return True
        return False

    ctx.poll(changed, timeout, poll_s=2)
    ctx.check(*VR.evaluate_rotation_found(box["e"], timeout),
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, JOINER_SERVICE))
    e_new = box["e"] if box["e"] is not None else 1
    got_new = ctx.committee(e_new, dry_value=rotated)
    ctx.check(*VR.evaluate_real_rotation(got_new, got0, e_new))
    _say(ctx, f"committee changed at E_new={e_new}: [{got_new}] (was [{got0}])")
    return e_new, got_new


def assert_still_finalizing(ctx, verdict=VR.evaluate_still_finalizing) -> None:
    """The closing check every one of the five ends with: `BEFORE=$(baseline_height); sleep 6;
    AFTER=$(finalized_dec); (( AFTER > BEFORE ))`.

    `baseline_height` for the PRE and `finalized_dec` for the POST, which is not an inconsistency:
    the PRE must never be a coerced 0 (that would make the comparison pass trivially on a dead
    chain), while the POST is compared against a real number and a transient 0 there fails
    honestly."""
    before = ctx.baseline_height()
    ctx.sleep(VR.FINALIZING_SLEEP_S)
    after = ctx.finalized_dec(dry_value=before + 1)
    ctx.check(*verdict(before, after))
    _say(ctx, f"chain still finalizing under tx load ({before} → {after})")


# ══ case-production-path ═══════════════════════════════════════════════════════════════════

def assert_production_path(ctx) -> None:
    """`scripts/case-production-path.sh:184-385` — everything after the bring-up.

    The full DPoS production lifecycle on a chain whose staking cluster was deployed at RUNTIME:
    an external sixth validator joins through governance while its supervisor keeps following
    in-process, AUTO-promotes at its first committee boundary, displaces an incumbent that must
    keep following, and finally one committee member is ejected by the participation-floor jail —
    all under the background value-transfer load the bring-up started.

    SEVEN assertions, in the order the evidence becomes available. The order is the case: the
    promotion cannot be observed before the join, and the ejection needs a committee that has
    already rotated."""
    addrs = owner_addrs(ctx)

    # ── 1: Gap A — the committee validators are PLANE-NATIVE ─────────────────────────
    #
    # Read off validator-0's LIVE argv rather than the compose file: a regression that re-added a
    # WS upstream to a committee validator would leave the compose file looking correct. The
    # joiner legitimately keeps the WS escape until it is activated, which is why this reads v0.
    ctx.check(*VR.evaluate_plane_native(ctx.proc_cmdline(topology.PINNED_RPC_HOST)))
    _say(ctx, "committee validators plane-native (no WS hub) — Gap A")

    # ── 2: the committee is the initial five ─────────────────────────────────────────
    _cm_diag(ctx)
    e, got, addrs = initial_committee_gate(ctx, addrs, name_epoch=False)

    # ── 3: register the EXTERNAL sixth validator ─────────────────────────────────────
    joiner = addrs[JOINER_IDX]
    reg_floor = register_joiner(ctx, addrs)
    ctx.check(bool(ctx.wait_converge(VR.REG_CONVERGE_PP_S, reg_floor)),
              "nodes (v5 on follower substrate) lost alignment during registration",
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, JOINER_SERVICE))
    _say(ctx, "v5 follower substrate tracked the chain through registration")

    # ── 4: AUTO-promotion — no operator action, just observe ─────────────────────────
    #
    # `committee[N]` is committed one boundary EARLY, so the joiner showing up in the ahead-
    # committed set is the signal its own supervisor acts on: it stops the follower lap at
    # boundary−1 and promotes IN PROCESS. Nothing here touches the node.
    box = {"join": None}

    def joined_ahead():
        e_now = ctx.current_epoch()
        if VR.committee_has(ctx.committee(e_now + 1, dry_value=joiner), joiner):
            box["join"] = e_now + 1
            return True
        return False

    ctx.poll(joined_ahead, VR.JOIN_SCAN_S, poll_s=2)
    ctx.check(box["join"] is not None, "v5 never appeared in an ahead-committed committee")
    join_e = box["join"] if box["join"] is not None else 1
    _say(ctx, f"v5 in committee(epoch {join_e}), current epoch {join_e - 1}; expecting "
              "in-process promotion")

    # ── 5: the joiner ENTERS CONSENSUS at its first committee boundary ───────────────
    #
    # Convergence strictly PAST the boundary block, with the joiner in the reading set, is what
    # proves the supervisor promoted and the joiner follows as a signer. The budget covers the
    # full external-join flow — catch-up, the multi-block epoch-E ceremony that only seals near
    # the boundary, the promotion, and seven nodes re-aligning; 240 s was calibrated for the
    # PRE-fix path where the joiner never promoted at all.
    join_boundary = ctx.epoch_first_block(join_e)
    ctx.check(bool(ctx.wait_converge(VR.JOIN_CONVERGE_S, hex(join_boundary))),
              f"v5 did not enter consensus past its committee boundary (block {join_boundary})",
              on_fail=lambda: _dump_join_failure(ctx))
    _say(ctx, f"v5 entered consensus: all 6 validators + full-node aligned past block "
              f"{join_boundary}")

    # The promotion must be the IN-PROCESS Verifier→Signer transition, not a restart.
    ctx.check(*VR.evaluate_promoted_in_process(
        ctx.log_count(JOINER_SERVICE, VR.PROMOTED_LINE, dry_value=1)))
    _say(ctx, "v5 promoted in-process (Verifier→Signer)")

    e = ctx.current_epoch()
    # The dry committee is the ROTATED shape — the joiner in, the LAST incumbent out — so a dry
    # transcript walks the displacement branch that a live run takes. A canned set that dropped
    # validator-0 instead would send the demotion probe at the host-RPC node, which is the one
    # outcome the case refuses (`evaluate_displaced`).
    got = ctx.committee(e, dry_value=" ".join(sorted(addrs[:JOINER_IDX - 1] + [joiner])))
    ctx.check(*VR.evaluate_join_committee(got, joiner, e, ctx.profile.committee_size))
    _say(ctx, "committee rotated: v5 entered top-5, lowest dropped")

    # ── 6: DEMOTION — the displaced validator must keep following ────────────────────
    #
    # It demotes to verify-only IN PROCESS (rotation-out → `reconcile_roles` drops its signer
    # engine) and keeps following: it is still Active, therefore still in the tracked set,
    # therefore its marshal follows finalizations over the consensus plane instead of wedging as
    # a silent verifier. Two in-container readings 8 s apart are the whole proof.
    idx = VR.displaced_idx(addrs[:ctx.profile.committee_size], got)
    ctx.check(*VR.evaluate_displaced(idx))
    idx = 1 if idx is None else idx
    victim_svc = topology.validator(idx)
    pre = ctx.check_node(victim_svc, dry_value="0x100|0xdry").split("|", 1)[0]
    ctx.sleep(VR.DEMOTION_SETTLE_S)
    post = ctx.check_node(victim_svc, dry_value="0x101|0xdry").split("|", 1)[0]
    ctx.check(*VR.evaluate_still_following(pre, post, idx),
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, victim_svc))
    _say(ctx, f"displaced validator-{idx} demoted in-process and keeps following ({pre} → {post})")

    # ── 7: liveness ejection — RETIRED with the participation-floor jail ─────────────
    #
    # This step stopped a committee member for a full epoch and asserted it was JAILED and then
    # absent from `committee[J+3]`. Both halves are gone: the jail machinery (`Staking.slash`,
    # `_slashValidator`, `jailedBefore`, `readmitExpiredJails`) is deleted, and its replacement —
    # `ProductionLiveness`'s temporary exclusion — cannot fire on a smoke stack at all. Either
    # reason alone is sufficient:
    #   * the tier ships FLAG-OFF (`ChainConfig._productionLivenessDisabled` is seeded true), so
    #     no verdict is evaluated until governance enables it after the pre-enable evidence;
    #   * a verdict needs `due >= minVerdictDueBlocks` (100), i.e. roughly
    #     `EPOCH_INTERVAL >= 100 * n`; at the smoke interval of 32 the tier is inert by
    #     construction.
    # Exercising it is soak work, and the mechanism is covered contract-side in
    # `ProductionLiveness.t.sol`. Do NOT restore this step by re-pointing the J-derivation at
    # `readmitAtEpoch`: on a stack where the tier never fires it would assert nothing.

    # In unified mode the joiner NEVER runs the legacy silent-verifier wait, so the committee
    # watchdog WARN must be absent from its ENTIRE log. An ABSENCE assertion over an ANSI-carrying
    # log — the reader strips, or this passes for the wrong reason.
    hits = [ln for ln in ctx.logs_all(JOINER_SERVICE).splitlines() if VR.WATCHDOG_LINE in ln]
    ctx.check(*VR.evaluate_watchdog_silent(hits))
    _say(ctx, "v5 watchdog silent for the whole run (unified mode)")

    assert_still_finalizing(ctx)
    _ok(ctx, "runtime forge deploy → 5-member committee → cold restart into unified --dpos → an "
             "external sixth validator registered through governance, AUTO-promoted in-process at "
             "its first committee boundary and displaced an incumbent that kept following → one "
             "all under background tx load")


def _cm_diag(ctx) -> None:
    """`:198-200` — the committee-selection diagnostic, printed unconditionally before the gate.

    `nextEpochToCommit` and `committeeSelectionEpoch` are the first two numbers anyone reads when
    the committee is not what the case expected, and the first four committees show whether the
    selection ever ran. Kept because they are READS: dropping them changes the transcript."""
    head = ctx.check_external(topology.HOST_RPC_PORT).split("|", 1)[0]
    _say(ctx, f"[cm-diag] head={head} epoch={ctx.current_epoch()}")
    nxt = VP.first_token(ctx.staking_call("nextEpochToCommit()(uint64)"))
    sel = VP.first_token(ctx.staking_call("committeeSelectionEpoch()(uint64)"))
    _say(ctx, f"[cm-diag] nextEpochToCommit={nxt} committeeSelectionEpoch={sel}")
    for e in range(4):
        _say(ctx, f"[cm-diag] committee({e})=[{ctx.committee(e)}]")


def _dump_join_failure(ctx) -> None:
    """`:269-280` — the join-failure diagnostic: both nodes' commonware peer gauges, then the
    joiner's log with the block-production noise filtered out, then validator-0's log filtered to
    the discovery/handshake markers.

    Kept in full because the failure it diagnoses is a CONNECTIVITY failure, and the two gauge
    reads are the only thing in the case that can tell "the joiner never dialled anyone" from
    "the joiner dialled and was refused"."""
    print("--- v5 commonware peers (buffered_peer_total series) ---", flush=True)
    print(_grep_any(ctx.peer_gauges_exec(JOINER_SERVICE), VR.PEER_GAUGE_MARKERS, 20), flush=True)
    print("--- v0 commonware peers ---", flush=True)
    print(_grep_any(ctx.peer_gauges_host(), VR.PEER_GAUGE_MARKERS, 20), flush=True)
    print("--- v5 log (filtered) ---", flush=True)
    lines = [ln for ln in ctx.logs_all(JOINER_SERVICE).splitlines()
             if not any(n in ln for n in VR.V5_LOG_NOISE)]
    print("\n".join(lines[-60:]), flush=True)
    print("--- v0 log (v5-related) ---", flush=True)
    hits = _grep_any(ctx.logs_all(topology.PINNED_RPC_HOST), VR.V0_JOIN_MARKERS, 0).splitlines()
    print("\n".join(hits[-40:]), flush=True)


def _grep_any(text: str, markers, limit: int) -> str:
    """`grep -m<limit> -E "a|b"` / `grep -iE "…"` — case-insensitive, because both bash call sites
    that feed this are `-i` and the log renders these markers in mixed case."""
    low = [m.lower() for m in markers]
    hits = [ln for ln in (text or "").splitlines() if any(m in ln.lower() for m in low)]
    return "\n".join(hits[:limit] if limit else hits)


# ══ case-vrf-rotation ══════════════════════════════════════════════════════════════════════

def assert_vrf_rotation(ctx) -> None:
    """`scripts/case-vrf-rotation.sh:126-386` — the live per-epoch DKG beacon stays LIVE and
    NODE-AGREED across a committee change, with the joiner participating as an EARLY-JOIN member.

    The threshold beacon has no on-chain mirror: the per-block seed rides the consensus cert and
    `prev_randao = H(seed)`. So the rotation is proven NOT by reading a rotated group key off-chain
    but by asserting the OBSERVABLE output — prev_randao stays non-zero, varying and byte-identical
    on every honest node BEFORE, ACROSS and AFTER the boundary, while the per-member ACTIVE_LINE
    count keeps GROWING (a fresh DKG re-dealt and every member, the joiner included, casts verified
    seed partials).

    FIVE numbered phases, matching the bash exactly."""
    # ── 1) BASELINE: the beacon is threshold-active from epoch 2 ─────────────────────
    #
    # There are no beacon flags any more — no genesis bake, no `--dpos.no-beacon`. Every node goes
    # through the same always-on live DKG: epoch 1 is seedless and the FIRST key is the
    # DETERMINISTIC epoch-2 ceremony's PK_2, because committee[2] DKGs during epoch 1 even on a
    # stable committee. So the beacon is threshold-active from epoch 2, not keyless-until-rotation.
    e0, got0, addrs = initial_committee_gate(ctx)
    epoch2_boundary = ctx.epoch_first_block(2)
    ctx.check(ctx.wait_finalized_ge(epoch2_boundary + 4, VR.EPOCH2_WAIT_S),
              f"chain did not reach epoch 2 ({epoch2_boundary}) — cannot observe the deterministic "
              "epoch-2 activation",
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, topology.PINNED_RPC_HOST))
    fin = ctx.finalized_dec(dry_value=epoch2_boundary + 8)
    ctx.check(int(fin) > 0, "no finalized block")
    lo = V.window_lo(fin, VR.BASELINE_WINDOW, epoch2_boundary)
    _say(ctx, f"checking baseline prev_randao window [{lo}..{fin}] (inside epoch >= 2)")
    assert_beacon_window(ctx, lo, fin, "baseline epoch-2-active")
    _say(ctx, "baseline beacon THRESHOLD-ACTIVE from epoch 2 on the stable committee")

    # ── 2) TRIGGER: register the joiner and wait for the committee to CHANGE ─────────
    joiner = addrs[JOINER_IDX]
    reg_floor = register_joiner(ctx, addrs)
    ctx.check(bool(ctx.wait_converge(VR.REG_CONVERGE_S, reg_floor)),
              "nodes lost alignment during registration",
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, JOINER_SERVICE))
    _say(ctx, "v5 follower substrate tracked the chain through registration")

    e_new, got_new = scan_for_rotation(ctx, got0, joiner)

    # EARLY-JOIN: under the always-on beacon plane ALL of committee[E_new] holds a share at E_new —
    # the stayers ran the ceremony alongside their signer engine, and the joiner ran it from its
    # FOLLOWER phase. So the growth probe set is the FULL committee, joiner included.
    members = VR.probe_members(addrs, got_new, topology.validator)
    ctx.check(*VR.evaluate_probe_members(members, JOINER_SERVICE))
    members = members or [topology.PINNED_RPC_HOST, JOINER_SERVICE]
    _say(ctx, "committee[E_new] share-holders (active-growth probe set, incl. joiner v5): "
              + " ".join(members))

    # The POSITIVE early-join proof: the joiner logged the live-DKG ceremony lifecycle during
    # E_new-1, while it was still a cert-follower. That is the direct evidence it dealt/received in
    # E_new's ceremony — i.e. it holds a share, rather than merely deriving from a committed key.
    dkg = VR.dkg_lifecycle_lines(ctx.logs_all(
        JOINER_SERVICE, dry_value=VR.DKG_LIFECYCLE_LINES[0] + " epoch=3"))
    ctx.check(*VR.evaluate_early_join(dkg),
              on_fail=lambda: [print(f"    {ln}", flush=True) for ln in dkg])
    _say(ctx, "EARLY-JOIN proof — validator-5 ran committee[E_new]'s DKG from its follower phase:")
    for line in dkg:
        _say(ctx, f"  {line}")

    # ── 3) ROTATION: cross the E_new boundary ────────────────────────────────────────
    boundary = ctx.epoch_first_block(e_new)
    if not ctx.wait_finalized_ge(boundary, VR.E_NEW_CLIMB_S):
        _fail_e_new_climb(ctx, boundary)
    _say(ctx, f"crossed the E_new={e_new} boundary (block {boundary}) — asserting the live + "
              "agreed beacon next")

    # ── 4) EARLY-JOIN / RELIVE: the beacon is live from E_new's first block ──────────
    before = {v: ctx.log_count(v, VR.ACTIVE_LINE, dry_value=1) for v in members}
    relive_hi = boundary + VR.RELIVE_OFFSET
    ctx.check(ctx.wait_finalized_ge(relive_hi, VR.RELIVE_WAIT_S),
              f"chain did not advance past the E_new boundary ({relive_hi}) — cannot observe a "
              "sustained post-rotation beacon")
    ctx.check(wait_nodes_have(ctx, relive_hi),
              f"not all nodes reached the relive window block {relive_hi} (a follower never "
              "caught up)")
    assert_beacon_window(ctx, boundary, relive_hi, f"relive E{e_new}")

    # The growth probe tracks the HEAD, not finalized: the ACTIVE_LINE fires at the SPECULATIVE tip
    # (notarize-time derive under deferred execution), and finalized can catch up to a burst-ahead
    # tip WITHOUT the tip producing new blocks — a false "frozen".
    head0 = ctx.tip_dec()
    ctx.check(bool(ctx.poll(lambda: ctx.tip_dec() >= head0 + VR.HEAD_GROWTH_BLOCKS,
                            VR.HEAD_GROWTH_S)),
              f"head did not advance >= {VR.HEAD_GROWTH_BLOCKS} past {head0} within "
              f"{VR.HEAD_GROWTH_S}s — cannot observe a sustained post-rotation beacon")
    for v in members:
        after = ctx.log_count(v, VR.ACTIVE_LINE, dry_value=before[v] + 1)
        ctx.check(*VR.evaluate_member_active_growth(before[v], after, v, JOINER_SERVICE),
                  on_fail=lambda s=v: ctx.dump_logs(VR.LOG_TAIL_NODE, s))
        role = ("EARLY-JOIN: v5 votes as a new member" if v == JOINER_SERVICE
                else "beacon relive")
        _say(ctx, f"{v} — active-count grew {before[v]} → {after} across the rotation ({role})")

    # ── 5) CARRY-FORWARD: a STABLE epoch runs no fresh ceremony ──────────────────────
    #
    # The DKG only re-deals on a committee CHANGE, so on a stable epoch the prior key carries
    # forward in memory and still verifies the seed. The stable epoch is SCANNED FOR rather than
    # assumed to be E_new+1: the committee may keep oscillating afterwards (a joiner whose
    # delegated weight does not durably hold its slot is bumped back out a couple of epochs later
    # — a stake-dynamics artifact of the minimal devnet set, not a beacon property).
    wait_boundary = ctx.epoch_first_block(e_new + 2)
    ctx.check(ctx.wait_finalized_ge(VR.carry_window_hi(wait_boundary, ctx.epoch_len),
                                    VR.CARRY_WAIT_S),
              f"chain did not elapse into epoch {e_new + 2}",
              on_fail=lambda: ctx.dump_logs(VR.LOG_TAIL_NODE, topology.PINNED_RPC_HOST))
    cur_now = ctx.current_epoch(dry_value=e_new + 2)
    e_s = None
    for e in range(e_new + 1, max(cur_now, e_new + 2)):
        if VR.is_stable_epoch(ctx.committee(e, dry_value=got_new),
                              ctx.committee(e - 1, dry_value=got_new)):
            e_s = e
            break
    ctx.check(*VR.evaluate_stable_epoch(e_s, e_new + 1, cur_now - 1))
    e_s = e_new + 1 if e_s is None else e_s
    _say(ctx, f"stable epoch E_s={e_s} (committee == committee({e_s - 1})=E_new), AFTER "
              f"E_new={e_new}")

    s_lo = ctx.epoch_first_block(e_s)
    s_hi = VR.carry_window_hi(s_lo, ctx.epoch_len)
    _say(ctx, f"checking live-across-stable prev_randao window [{s_lo}..{s_hi}]")
    ctx.check(ctx.wait_finalized_ge(s_hi, VR.CARRY_WINDOW_S),
              f"chain did not reach the carry-forward window block {s_hi}")
    ctx.check(wait_nodes_have(ctx, s_hi),
              f"not all nodes reached the carry-forward window block {s_hi}")
    assert_beacon_window(ctx, s_lo, s_hi, f"carry-forward E{e_s}")

    assert_still_finalizing(ctx)
    _ok(ctx, "live per-epoch DKG beacon STAYED LIVE + NODE-AGREED across a committee change with "
             f"EARLY-JOIN — baseline threshold-active from epoch 2 on the stable committee E{e0}; "
             f"registering v5 changed the committee at E{e_new} and the fresh DKG re-dealt; v5 ran "
             "committee[E_new]'s DKG from its FOLLOWER phase and is a full beacon signer from "
             f"block 1 of E{e_new}; stable epoch E{e_s} carried the key forward in memory while "
             "the threshold beacon stayed live + node-agreed")


def _fail_e_new_climb(ctx, boundary) -> None:
    """`:264-272` — on an E_new-climb timeout, distinguish a STALL from merely-slow by sampling
    finalized twice, then dump the STAYERS' live-DKG lifecycle.

    The two samples are the diagnosis: "advancing, just slow" and "FROZEN" call for opposite
    responses (raise the budget vs find the stall), and a bare timeout message names neither."""
    h1 = ctx.finalized_dec()
    ctx.sleep(VR.STALL_SAMPLE_S)
    h2 = ctx.finalized_dec()
    print("  --- live-DKG ceremony lifecycle on the stayers (started/computed?) ---", flush=True)
    for svc in (topology.validator(0), topology.validator(2), topology.validator(3),
                topology.validator(4)):
        hits = [ln for ln in ctx.logs_all(svc).splitlines() if "live DKG" in ln]
        print(f"  [{svc}]:\n" + "\n".join(f"    {ln}" for ln in hits[-6:]), flush=True)
    ctx.dump_logs(VR.LOG_TAIL_NODE, topology.PINNED_RPC_HOST)
    raise ProdFailure(ctx.label,
                      f"chain did not reach E_new boundary block {boundary} in "
                      f"{VR.E_NEW_CLIMB_S}s (finalized {h1} → {h2} over {VR.STALL_SAMPLE_S}s — "
                      f"{VR.stall_verdict(h1, h2)})")
