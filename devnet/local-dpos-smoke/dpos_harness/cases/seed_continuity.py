"""seed_continuity.py — the standalone VRF seed-continuity case (dpos_harness, deterministic).

WHY THIS EXISTS
    A seed partial now rides on EVERY vote of a beacon-active epoch, `Subject::Nullify`
    included (task `.claude/tasks/2026_07_29__vrf_seed_continuity/`). Because sigma signs
    the ROUND alone, a nullification and a notarization of the same view recover the
    byte-identical sigma — so the leader elected for view v+1 no longer depends on whether
    view v produced a block. Before the change a nullified v yielded NO sigma and the
    elector fell back to `sha256(LEADER_DOMAIN || sha256(epoch || sorted peer pubkeys)
    || view)`: a public, epoch-fixed table anyone could compute two epochs ahead.

    That fallback table is exactly what makes this case TWO-SIDED rather than a smoke
    test. Under the OLD binary the leader after a nullified view was the offline
    prediction with CERTAINTY (probability 1.0, not "usually"). Under the NEW binary the
    elector's input is sigma of round (E, v), uncorrelated with the fallback hash, so a
    coincidental match has probability ~= the predicted leader's stake share, 1/n. ONE
    mismatch therefore falsifies the old code; m mismatches drive the false-negative rate
    to (1/n)^m — at n=7, m=5 that is ~6e-5.

WHAT THIS CASE DOES (deterministic, SCRIPTED)
    1. Bring up a minimal fast devnet (n=7 static committee, uniform stake, geo OFF).
    2. Wait for DPoS active; stop k=1 committee member (never validator-0, the pinned RPC
       host) so views whose leader is the stopped node nullify while quorum survives.
       k=1 <= f-1 keeps one unit of quorum slack — k=f would sit at exactly n-f.
    3. Observe for a window, then reconstruct the view->leader map from two INFO logs that
       are already emitted today (no log-level change, no production code change):
         - `derive-seed: block derived`  (node/derive.rs)  -> height + that block's round
         - `dpos: proposing order block` (consensus/application.rs) -> height, proposer-only
    4. ASSERT A (positive control): the leader of view 1 of each epoch MUST equal the
       offline fallback prediction — view 1 takes the fallback even after the change. This
       is what stops a broken Python predictor from mismatching everywhere and turning
       assertion B vacuously green. A control failure is INCONCLUSIVE (2), never PASS/FAIL.
    5. ASSERT B (the two-sided one): over produced views whose immediate predecessor was
       nullified, at least `m_min` observed leaders must DIFFER from the fallback
       prediction.
    6. ASSERT C: finalized advanced (the mandatory partial did not make nullification
       harder to reach) and no node reports `engine_demoted_bad_share`.
    7. PASS/FAIL/INCONCLUSIVE + teardown (unless SIM_KEEP_UP=1).

WHAT THIS CASE DOES NOT PROVE
    That the recovered sigma is the CORRECT threshold signature — only that it is not the
    fallback. Byte-identity of sigma across the notarize/nullify branches of one view is a
    unit-test property (`bls/combined_scheme.rs`, `notarize_and_nullify_of_same_round_...`)
    and is not separately observable live. And it says nothing about cross-node leader
    AGREEMENT: a stopped leader never proposes, so its views are nullify-only, and the old
    code would have agreed on those too — observing divergence needs a SLOW leader, not a
    dead one, and the only two-sided route to it is an A/B run against a pre-change build.

REUSE: BringUp.run(), nodes.logs_since / finalized_dec / node_fin_in / beacon_metric,
Chain.consensus_keys, the Runner docker seam (LIVE COMPOSE_FILE overlay), core.events.EventLog.
"""

from __future__ import annotations

import hashlib
import os
import re
import time
from bisect import bisect_right

from ..core import topology


# ── PURE VERDICT LAYER (docker-free, unit-tested in tests/test_case_seed_continuity.py) ──

LEADER_DOMAIN = b"fluent/leader"
BALANCE_COMPACT_PRECISION = 10_000_000_000

# `derive-seed`'s `seed_round` is the block's OWN round, not its parent's: the executor
# resolves the seed at `Round::new(epoch, View::new(block.proposal_view))`
# (`crates/dpos/consensus/src/executor.rs:1429-1440`, re-canonicalised at `:2090-2095`).
#
# This was got wrong once and it is worth the paragraph. Rule PIN's WITNESS (`parent_seed`,
# carried in the block body) is the PARENT's round — but this log line reports the seed the
# block CONSUMES for its own derive, which is its own. The two are different objects that
# both say "seed_round". The offset-by-one predicted a leader for the wrong view and every
# positive control failed; nothing in the unit suite caught it, because the fixture encoded
# the same wrong belief. The LIVE control did — that is what it is for.

# `?seed_round` renders through the derived Debug of `Round { epoch: Epoch(u64), view:
# View(u64) }` (commonware consensus/src/types.rs — both newtypes derive Debug and impl
# Display only). Matching the two integers positionally keeps this tolerant of a Debug
# reshuffle upstream; the exact rendering is pinned by the unit-test fixture.
_RE_DERIVE = re.compile(
    r"derive-seed: block derived|block derived")
_RE_HEIGHT = re.compile(r"\bheight[=:\s]+(\d+)")
_RE_SEED_ROUND = re.compile(r"seed_round[=:\s]+\S*?Round\s*\{[^}]*?(\d+)[^}]*?(\d+)[^}]*?\}")
_RE_PROPOSING = re.compile(r"dpos: proposing order block")


def fallback_leader_index(epoch: int, view: int, sorted_pubkeys: list, cum: list, total: int):
    """The offline mirror of `weighted_vrf.rs`'s fallback arm. Returns the participant INDEX.

      sorted_pubkeys — raw peer-pubkey bytes, sorted bytewise (commonware `ordered::Set`
                       is a Vec built by `items.sort()`, so the order is Ord on the raw
                       ed25519 key).
      cum            — inclusive prefix sums of COMPACTED stake, same order.
      total          — cum[-1].

    Deterministic; no I/O."""
    h = hashlib.sha256()
    h.update(epoch.to_bytes(8, "big"))
    for pk in sorted_pubkeys:
        h.update(pk)
    fallback_seed = h.digest()

    r = hashlib.sha256()
    r.update(LEADER_DOMAIN)
    r.update(fallback_seed)
    r.update(view.to_bytes(8, "big"))
    target = int.from_bytes(r.digest(), "big") % total
    return bisect_right(cum, target)


def build_cum(compacted_stakes: list):
    """Inclusive prefix sums + total, mirroring `WeightedVrf::build`. All-zero weights fall
    back to uniform 1s (the same guard the Rust `build` applies to keep `total > 0`)."""
    w = list(compacted_stakes)
    if sum(w) == 0:
        w = [1] * len(w)
    cum, acc = [], 0
    for x in w:
        acc += x
        cum.append(acc)
    return cum, acc


def parse_view_leader_map(logs: dict):
    """Reconstruct the produced-view map from `{service: log_text}`. Pure.

    Returns (view_of, proposer_of, epoch_of) keyed by BLOCK HEIGHT:
      view_of[h]     — the view block h was proposed at
      epoch_of[h]    — the epoch block h belongs to
      proposer_of[h] — the docker service that logged "proposing order block" for h

    A height is usable only when all three agree; the caller filters."""
    view_of, epoch_of, proposer_of = {}, {}, {}
    for svc, text in logs.items():
        for line in text.splitlines():
            if _RE_PROPOSING.search(line):
                m = _RE_HEIGHT.search(line)
                if m:
                    proposer_of[int(m.group(1))] = svc
                continue
            if not _RE_DERIVE.search(line):
                continue
            mh, mr = _RE_HEIGHT.search(line), _RE_SEED_ROUND.search(line)
            if not (mh and mr):
                continue
            h = int(mh.group(1))
            view_of[h] = int(mr.group(2))
            epoch_of[h] = int(mr.group(1))
    return view_of, epoch_of, proposer_of


def successors_of_nullified(view_of: dict, epoch_of: dict):
    """Heights whose view's immediate predecessor (same epoch) produced NO block — i.e. the
    experimental set S. A gap between consecutive produced views inside one epoch IS a run
    of nullified views."""
    by_epoch = {}
    for h, v in view_of.items():
        by_epoch.setdefault(epoch_of[h], []).append((v, h))
    out = []
    for _, pairs in by_epoch.items():
        pairs.sort()
        for i in range(1, len(pairs)):
            if pairs[i][0] > pairs[i - 1][0] + 1:
                out.append(pairs[i][1])
    return sorted(out)


def evaluate_seed_continuity_case(controls: list, samples: list, fin_delta: int,
                                  bad_share_nodes: list, m_min: int = 5,
                                  s_min: int = 20, controls_min: int = 3):
    """Pure verdict. Returns (exit_code, reason); 0 pass, 1 fail, 2 inconclusive.

      controls — [(epoch, predicted_idx, observed_idx)] for view 1 of each epoch.
      samples  — [(epoch, view, predicted_idx, observed_idx)] for the successors of
                 nullified views.
      fin_delta— finalized advance over the observation window.
      bad_share_nodes — services reporting engine_demoted_bad_share > 0.

    Deterministic; no I/O."""
    if bad_share_nodes:
        return (1, "SHARE-GATE FIRED: engine_demoted_bad_share > 0 on "
                   f"{' '.join(bad_share_nodes)} — a committee member was demoted for a share "
                   "that does not verify against its own sharing; the mandatory-partial change "
                   "would have made this member a nullify-quorum blocker")
    if fin_delta <= 0:
        return (1, f"NO PROGRESS: finalized did not advance ({fin_delta}) with k=1 down — "
                   "quorum should be intact; nullification may no longer be reachable")

    bad_controls = [c for c in controls if c[1] != c[2]]
    if len(controls) < controls_min:
        return (2, f"INCONCLUSIVE: only {len(controls)} positive control(s) observed "
                   f"(need {controls_min}) — too few epoch boundaries in the window to "
                   "validate the offline predictor")
    if bad_controls:
        e, p, o = bad_controls[0]
        return (2, f"INCONCLUSIVE: positive control FAILED at epoch {e} — view 1 must take the "
                   f"fallback arm and the predictor said index {p}, observed {o} "
                   f"({len(bad_controls)}/{len(controls)} controls wrong). The predictor or its "
                   "committee/epoch inputs are wrong, so the experimental arm carries no "
                   "information. First suspects: the epoch value fed to the fallback hash, and "
                   "the parent-vs-own seed_round offset")

    if len(samples) < s_min:
        return (2, f"INCONCLUSIVE: only {len(samples)} successor-of-nullified sample(s) "
                   f"(need {s_min}) — the stopped member was rarely elected, or the stop never "
                   "took effect; nothing was measured")
    mismatches = [s for s in samples if s[2] != s[3]]
    rate = (len(samples) - len(mismatches)) / len(samples)
    if len(mismatches) < m_min:
        return (1, f"FALLBACK STILL IN USE: only {len(mismatches)} of {len(samples)} leaders "
                   f"after a nullified view differed from the offline fallback prediction "
                   f"(need {m_min}; match rate {rate:.2f}). Under the PRE-change binary this "
                   "count is 0 with certainty, so this reads as a nullification certificate "
                   "carrying no seed — the change is not in force")
    return (0, f"BRANCH-INDEPENDENT: {len(mismatches)} of {len(samples)} leaders after a "
               f"nullified view differed from the fallback prediction (match rate {rate:.2f}, "
               f"expected ~1/n by chance); {len(controls)} positive control(s) matched, so the "
               "predictor is byte-correct. A nullification certificate carried a seed and it "
               "reached the elector")


# ── ENV PROFILE (minimal, fast, deterministic) ─────────────────────────────────

def apply_case_env_defaults():
    """Static 7-member committee (f=2), uniform stake, geo OFF, no churn — a clean docker
    set with no SLOT_IDENTITY rebinding. Uniform stake is deliberate: it makes the
    coincidental-match probability exactly 1/n. setdefault so an operator override wins."""
    prof = {
        "SIM_QUICK": "1",
        "SIM_VALIDATORS": "7",
        "SIM_INITIAL_COMMITTEE": "7",
        "SIM_SPARES": "0",
        "SIM_ROTATION_SLOTS": "0",
        "SIM_EPOCH_INTERVAL": "32",
        "SIM_NO_CASCADE": "1",
        "SIM_BYZANTINE": "0",
        "SIM_GEO_LATENCY": "0",
    }
    for k, v in prof.items():
        os.environ.setdefault(k, v)
    return {k: os.environ[k] for k in prof}


# ── LIVE POLL HELPERS ──────────────────────────────────────────────────────────

def _await_dpos_active(nodes, chain, deadline_s: int):
    """Wait until DPoS is active (epoch>=1) and finalized is advancing (two rising samples)."""
    deadline = time.time() + deadline_s
    prev, rising, ep, fin = -1, 0, 0, 0
    while time.time() < deadline:
        ep = chain.current_epoch()
        fin = nodes.finalized_dec()
        if ep >= 1 and fin > 0:
            if fin > prev:
                rising += 1
                prev = fin
            if rising >= 2:
                return fin, ep
        time.sleep(5)
    from ..chain.writes import ChainError
    raise ChainError("await-active",
                     f"DPoS not active/finalizing within {deadline_s}s (epoch={ep}, fin={fin})")


def _service_pubkeys(chain, n: int):
    """peer pubkey -> docker service, from the deterministic genesis derivation."""
    out = {}
    for i in range(n):
        pk = chain.consensus_keys(i).get("peerPubkey", "")
        if not pk:
            from ..chain.writes import ChainError
            raise ChainError("consensus-keys", f"no peerPubkey for {topology.validator(i)}")
        out[bytes.fromhex(pk[2:] if pk.startswith("0x") else pk)] = topology.validator(i)
    return out


def _committee_material(nodes, chain, staking_rt: str, n: int):
    """The elector's ACTUAL inputs for `epoch`, read from the chain — never assumed.

    Returns (sorted_pubkeys, index_to_service, cum, total) where the index is the position
    in the sorted-pubkey order, i.e. the participant index the elector uses.

    Why this is read and not derived: `rand % total` is sensitive to the MAGNITUDE of the
    weights, not only their ratio, so a plausible-looking guess at the stake elects a
    different leader and every prediction is silently wrong. A first run of this case
    guessed 50e18 (the harness's ROTATION stake) while genesis actually stakes
    `smoke_min_stake` = 1e18 per validator (`genesis-bootstrap/src/bootstrap.rs:394,447`,
    skewed for validator-0 by `HEAVY_STAKE_MULT`), and neither value reconciled with the
    observed leaders — two positive controls are far too thin to identify the parameters by
    search. `getEpochCommitteeWithStakes` is the same source the staking-reader feeds the
    elector from, so reading it removes the guess entirely.

    Read PER VALIDATOR via `getValidatorStatus`, whose return is a FLAT 8-tuple with
    `totalDelegated` third — one value per line out of `cast`. The committee-wide
    `getEpochCommitteeWithStakes` would be the more direct source but returns
    `(address[], ConsensusKeys[], uint256[])`, and hand-decoding a nested ABI blob is the
    same class of positional guess this function exists to eliminate.

    Two call conventions this module already established and a first run got wrong:
    the sim DEPLOYS staking at runtime (`nodes.py:411` "Runtime-deploy variants"), so the
    address must be `BringUp.staking_rt` and never the `0x…5201` predeploy default; and the
    signature must carry its RETURN types or `cast` emits one raw hex blob instead of one
    decoded field per line (`nodes.validator_status`, `rpc.cast_field` is 1-based).

    `totalDelegated` is the CURRENT stake, not stake-at-epoch. This case runs with rotation
    and growth off and issues no staking ops, so the two coincide — and the caller turns
    that from an assumption into a checked precondition by re-reading at the end of the
    window and refusing to score if anything moved."""
    from ..chain.writes import ChainError

    pk_to_svc = _service_pubkeys(chain, n)
    stake_by_pk = {}
    for i in range(n):
        ck = chain.consensus_keys(i)
        addr, pk = ck.get("validatorAddress", ""), ck.get("peerPubkey", "")
        if not (addr and pk):
            raise ChainError("consensus-keys", f"{topology.validator(i)}: missing address/peerPubkey")
        raw = nodes.staking_call(
            "getValidatorStatus(address)(address,uint8,uint256,uint64,uint64,uint16)",
            addr, addr=staking_rt)
        field = nodes.rpc.cast_field(3, raw)  # 1-based: 1 owner, 2 status, 3 totalDelegated
        if not field:
            raise ChainError("committee-read",
                             f"getValidatorStatus({addr}) @ {staking_rt} gave no third field: "
                             f"{raw[:200]!r}")
        # `cast` appends a human-readable annotation — "5000000000000000000 [5e18]" — and
        # `cast_field` strips every space, so the two run together. Take the leading digits.
        digits = re.match(r"\d+", field)
        if not digits:
            raise ChainError("committee-read",
                             f"getValidatorStatus({addr}): third field is not numeric: {field!r}")
        stake = int(digits.group(0))
        if stake == 0:
            raise ChainError("committee-read", f"{topology.validator(i)} has zero stake")
        stake_by_pk[bytes.fromhex(pk[2:] if pk.startswith("0x") else pk)] = stake

    ordered = sorted(stake_by_pk)
    cum, total = build_cum([stake_by_pk[pk] // BALANCE_COMPACT_PRECISION for pk in ordered])
    return (ordered, {i: pk_to_svc[pk] for i, pk in enumerate(ordered)}, cum, total,
            {pk_to_svc[pk]: s for pk, s in stake_by_pk.items()})


# ── THE CASE ───────────────────────────────────────────────────────────────────

def run_case(argv=None) -> int:
    prof = apply_case_env_defaults()

    from ..sim.orchestrator import SimConfig
    from ..stack.bringup import BringUp
    from ..core.proc import Runner
    from ..chain.writes import Chain, ChainError
    from ..core.events import EventLog
    from ..core import nodes

    cfg = SimConfig()
    n = cfg.initial_committee
    f = (n - 1) // 3
    k = int(os.environ.get("SEED_CASE_STOP", "1"))
    if k > max(1, f - 1):
        print(f"CASE-SEED SETUP ERROR: k={k} leaves no quorum slack at n={n} f={f} "
              f"(use k <= {max(1, f - 1)})", flush=True)
        return 2

    window_s = int(os.environ.get("SEED_CASE_WINDOW_S", "600"))
    rpc = os.environ.get("RPC", topology.DEFAULT_RPC_URL)
    keep_up = os.environ.get("SIM_KEEP_UP", "0") == "1"
    runner = Runner(env={"RPC": rpc, "CHAIN_ID": os.environ.get("CHAIN_ID", str(topology.CHAIN_ID))})
    bu = BringUp(cfg.stack_spec(), runner)

    def compose_overlay():
        return {"COMPOSE_FILE": os.environ.get("COMPOSE_FILE", "")}

    victims = [topology.validator(i) for i in range(1, k + 1)]
    live = [topology.validator(i) for i in range(n) if topology.validator(i) not in victims]

    print(f"CASE-SEED: profile {prof} — n={n} f={f} → stopping k={k}: {' '.join(victims)}, "
          f"observing {window_s}s", flush=True)

    def teardown():
        try:
            bu.spammers.stop()
        except Exception:  # noqa: BLE001
            pass
        if not keep_up:
            runner.run_ok(["docker", "compose", "down", "-v", "--remove-orphans"],
                          timeout=300, note="teardown-down")
        else:
            print("CASE-SEED: SIM_KEEP_UP=1 — leaving the stack up", flush=True)

    def fail(code: int, reason: str, extra: dict = None) -> int:
        tag = "FAIL" if code == 1 else "INCONCLUSIVE"
        print(f"CASE-SEED {tag}: {reason}", flush=True)
        try:
            ev = EventLog()
            # The DERIVED artefacts, written next to the bundle: a red assertion B is
            # uninterpretable from a truncated log tail alone, and `bundle_dump` creates an
            # `rpc/` dir but writes nothing into it, so the metrics snapshot would be lost
            # the moment the stack comes down.
            for name, payload in (extra or {}).items():
                with open(os.path.join(ev.out, f"case-seed-{name}"), "w") as fh:
                    fh.write(payload)
            ev.bundle_dump(reason, "case-seed-continuity")
            print(f"CASE-SEED: artefacts in {ev.out}/case-seed-*", flush=True)
        except Exception:  # noqa: BLE001
            pass
        teardown()
        return code

    try:
        bu.run()
        chain = Chain(runner=runner, RPC=rpc, STAKING_RT=bu.staking_rt,
                      CHAIN_CONFIG_RT=bu.chain_config_rt, GOV_ADDR=bu.gov_addr,
                      LIVENESS_RT=bu.liveness_rt, TOKEN=bu.token,
                      CHAIN_ID=os.environ.get("CHAIN_ID", str(topology.CHAIN_ID)))

        fin0, epoch0 = _await_dpos_active(nodes, chain, deadline_s=300)
        print(f"CASE-SEED: DPoS active — baseline fin0={fin0} epoch0={epoch0}", flush=True)

        sorted_pks, idx_to_svc, cum, total, stakes0 = _committee_material(
            nodes, chain, bu.staking_rt, n)
        svc_to_idx = {svc: i for i, svc in idx_to_svc.items()}
        print(f"CASE-SEED: committee weights (compacted) "
              f"{ {s: v // BALANCE_COMPACT_PRECISION for s, v in stakes0.items()} }", flush=True)

        for v in victims:
            runner.run(["docker", "compose", "stop", "--timeout", "40", v],
                       env_overlay=compose_overlay(), timeout=120, note=f"seed-stop-{v}")
        print(f"CASE-SEED: stopped {' '.join(victims)} — their views will nullify", flush=True)

        since = f"{window_s + 30}s"
        time.sleep(window_s)
        fin1 = nodes.finalized_dec()

        # Turn "stake did not move during the window" from an assumption into a checked
        # precondition — the predictor uses CURRENT stake for every epoch in the window.
        _, _, _, _, stakes1 = _committee_material(nodes, chain, bu.staking_rt, n)
        if stakes1 != stakes0:
            return fail(2, f"INCONCLUSIVE: committee stake changed during the window "
                           f"({stakes0} -> {stakes1}) — the predictor's weights are only valid "
                           "for a static committee, so no view can be scored")

        logs = {svc: nodes.logs_since(svc, since) for svc in live}
        view_of, epoch_of, proposer_of = parse_view_leader_map(logs)

        controls, samples = [], []
        for h in sorted(view_of):
            if h not in proposer_of or h not in epoch_of:
                continue
            e, v = epoch_of[h], view_of[h]
            observed = svc_to_idx.get(proposer_of[h])
            if observed is None:
                continue
            predicted = fallback_leader_index(e, v, sorted_pks, cum, total)
            if v == 1:
                controls.append((e, predicted, observed))
        for h in successors_of_nullified(view_of, epoch_of):
            if h not in proposer_of:
                continue
            e, v = epoch_of[h], view_of[h]
            observed = svc_to_idx.get(proposer_of[h])
            if observed is None or v == 1:
                continue
            samples.append((e, v, fallback_leader_index(e, v, sorted_pks, cum, total), observed))

        bad_share = [s for s in live
                     if nodes.beacon_metric(s, "engine_demoted_bad_share") > 0]

        code, reason = evaluate_seed_continuity_case(
            controls, samples, fin1 - fin0, bad_share)
        if code != 0:
            table = "\n".join(f"{e} {v} pred={p} obs={o}" for e, v, p, o in samples)
            return fail(code, reason, {"table.txt": table,
                                       "metrics.txt": "\n\n".join(
                                           f"### {s}\n{nodes.node_metrics(s)}" for s in live)})
        print(f"CASE-SEED PASS: {reason}", flush=True)
        teardown()
        return 0

    except ChainError as e:
        return fail(1, f"chain error [{e.reason_id}]: {e.message}")
    except KeyboardInterrupt:
        print("CASE-SEED: interrupted", flush=True)
        teardown()
        return 130
