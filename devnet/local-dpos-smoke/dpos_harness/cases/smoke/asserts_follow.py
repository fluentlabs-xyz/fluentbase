"""asserts_follow.py — the FOLLOWER case bodies (`case-cert-follow.sh`,
`case-cert-cascade.sh`, `case-tx-cascade.sh`, plus `smoke-cert-keyless`, which has no bash
ancestor — it is FLU-1202's live gate and is documented in its own body).

These are the first cases in the suite that need SERVICES THE BASE COMPOSE FILE DOES NOT DEFINE.
Each one brings its own compose overlay up through `SmokeCtx`'s overlay seam (see `driver.py`),
drives the services it defines, and leaves them to the bare `--remove-orphans` teardown.

Same split as everywhere else here: this module decides what to read and when, `verdicts_follow`
decides what the readings mean.

═══ WHAT IS DIFFERENT ABOUT THIS TRIO, AND WHAT A PORT CAN QUIETLY LOSE ══════════════════

1. **Four of the assertions are NEGATIVE** — the tampered-cert rejection, the bogus-checkpoint
   refusal, the bogus follower's lack of progress, and the tx-route monitor's silence. The last of
   those is not an assertion at all today and is labelled as such in the case output: the monitor
   that would emit its string does not exist (audit B9), so it is a TRIPWIRE for the day it comes
   back, never evidence. Two of the other three conclude from a fixed OBSERVATION WINDOW, i.e.
   they pass on a timeout:

       cert-follow phase 3   45 s of watching the tamper follower not move
       cert-cascade phase 3  240 s of waiting for a refusal that must arrive

   Shortening either budget disarms its assertion silently — the case still passes, faster. The
   budgets live in `verdicts_follow` as named constants quoting their bash line for that reason.
   §2.4 item 3.

2. **`cert-follow` phase 3 has a LOUD SKIP path and it is deliberate.** The MITM sidecar
   `pip install`s `websockets` at boot, so on a host with no network it never starts. Bash prints
   `SKIP` and exits 0 on the strength of the two positive phases. That is the right call — an
   un-runnable negative has disproved nothing — but it is exactly one edit away from becoming a
   case that passes because it did not run, so the skip prints its reason and names what it did
   NOT test, and the final OK line differs from the full-pass one.

3. **`cert-cascade` deploys contracts mid-case.** The MockRollup address reaches the follower
   through the ENVIRONMENT (`export MOCK_ROLLUP_ADDR`, interpolated by the compose file), so the
   export must happen before the `up` that consumes it, and the checkpoint block must be
   FINALIZED before it too — the follower reads the Rollup at the `finalized` tag, so a
   checkpoint that is merely mined is a checkpoint that is not there yet.

4. **`tx-cascade`'s privacy half is a NOTE, not a gate** — see `evaluate_l3_peers`. The peer
   COUNT is not what keeps validators hidden from L3; `--trusted-only` plus a one-entry
   trusted-peers list is. Porting the note as an assertion would make the case flaky; porting the
   `>= 1` gate as a note would delete the only check there.

`scripts/cert-mitm-proxy.py` is not PORTED — it is already Python and the compose overlay
launches it (`docker-compose.cert-follow.yml` mounts `./scripts:/proxy:ro`). It gained a second
MODE for phase 4, `seed-slot`, because the original one cannot test what phase 4 tests: a flipped
nibble breaks the G1 point and the certificate fails DECODE, never reaching the seed arm. See the
proxy's own header, and `verdicts_follow.SEED_REJECT_LINE`.
"""

from __future__ import annotations

import json
import os

from . import verdicts, verdicts_follow as vf
from .driver import SmokeFailure
from ...core import nodes, topology

# <smoke>/dpos_harness/cases/smoke/asserts_follow.py → FOUR dirnames to the smoke dir. Pinned by
# tests/test_path_anchors.py alongside `asserts._SMOKE_DIR`, for the same reason: a wrong
# directory here is SILENT — the MockRollup bytecode would come back missing and the deploy would
# go out as an empty `--create`, which fails as if the chain were broken.
_SMOKE_DIR = os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.dirname(os.path.abspath(__file__)))))
MOCK_ROLLUP_JSON = os.path.join(_SMOKE_DIR, "contracts", "MockRollup.json")

#: The per-case compose overlays — the third `-f` of each bash file array.
CERT_FOLLOW_OVERLAY = "docker-compose.cert-follow.yml"
CERT_CASCADE_OVERLAY = "docker-compose.cert-cascade.yml"
TX_CASCADE_OVERLAY = "docker-compose.tx-cascade.yml"


def _ok(ctx, case: str, message: str) -> None:
    """The `OK (<case>): …` line. Suppressed under `--dry-run`, where nothing was measured."""
    if not ctx.dry:
        print(f"OK ({case}): {message}", flush=True)


def _say(ctx, message: str) -> None:
    """A progress line, also suppressed under dry — the transcript is the artifact there."""
    if not ctx.dry:
        print(message, flush=True)


def _align_diag(ctx, *labelled_services) -> str:
    """`(<label>=<reading>, …, v0=<reading>)` for a FAILED align wait.

    Pass this to `ctx.check` as a CALLABLE message — `check` evaluates it only on the failure
    path (its docstring: the bash diagnostic RPCs live inside the failing `echo`, and an f-string
    would issue them on every PASS too, including in the dry transcript).

    Without it the artifact is the follower's log tail alone, which cannot separate "three blocks
    behind" from "on a different chain" — and those are opposite verdicts. `case-cert-catchup.sh`
    already prints both sides (`victim=… v0=…`, :170); this is that, for these cases."""
    parts = [f"{label}={ctx.overlay_check_node(svc)}" for label, svc in labelled_services]
    parts.append(f"v0={ctx.check_external(topology.HOST_RPC_PORT)}")
    return "(" + ", ".join(parts) + ")"


def _balance(ctx, case: str, addr: str, rpc_url: str) -> int:
    """`cast balance` as an int, FAIL-LOUD on anything else.

    Same guard as `asserts._balance` and it is needed twice over here: this reads the balance off
    an L3 FOLLOWER, whose RPC is the newest and least settled thing in the topology. Letting an
    unreachable read present as 0 would make the delta come out as exactly the transfer amount in
    one direction, i.e. a PASS."""
    raw = ctx.cast_balance(addr, rpc_url=rpc_url)
    tok = verdicts.first_token(raw)
    if not tok.lstrip("-").isdigit():
        raise SmokeFailure(case, f"cast balance {addr} returned {raw!r}, not a number "
                                 "(RPC unreachable?) — refusing to read it as a balance")
    return int(tok)


# ══ smoke-cert-follow ═════════════════════════════════════════════════════════════════

def assert_cert_follow(ctx) -> None:
    """A trustless `--cert-follow` node pulls finality certificates from validator-0's `consensus`
    RPC, verifies each against the ON-CHAIN committee, and drives its own reth.

    Four phases, and each of the last two is the reason the ones before it are not enough:

      1. subscribe-align — it catches up and finalized-aligns with v0 across an epoch boundary;
      2. gap back-fill   — stopped past a real gap and restarted, it catches up via
                           `getFinalization` (persistent resume, not a re-sync from genesis);
      3. tampered reject — fed byte-flipped certificates through a WS man-in-the-middle, it makes
                           ZERO finalized progress;
      4. PK_epoch        — it OBTAINS the epoch key over its cert upstream, verifies the artifact
                           against `committee[epoch]` read off its own chain, stops taking
                           vote-only admissions, and then REFUSES a certificate whose seed slot
                           was cleared behind a valid multisig quorum.

    Phases 1 and 2 prove a follower CAN follow. Phase 3 proves it verifies SOMETHING — but only
    at decode: a nibble flipped inside the trailing G1 point makes `into_parts()` fail, so the
    certificate never reaches the cert inlet and the seed check is never exercised. Phase 4 is the
    one that reaches it (FLU-1167). The cleared slot decodes cleanly and the quorum still
    verifies, so the refusal can only come from the seed arm.

    WHICH HALF OF PHASE 4b CARRIES THE KEYED/KEYLESS CLAIM — IT MOVED, AND FLU-1202 MOVED IT.
    Before that ticket the `None => false` arm of `verify_certificate` was reachable only on a
    scheme carrying `cert_seed_pin`, so the REFUSAL itself separated a keyed follower from a
    keyless one. The scheme now carries an ORACLE rather than key material, and
    `Randomness::oracle_for` attaches one for every beacon-active epoch whether or not the key
    resolves (`beacon/follower.rs:355-367` — `mandatory_at(epoch).then(..)`), so a KEYLESS
    follower refuses a cleared slot too. The refusal count today proves the seed arm is reachable
    and enforced on a beacon-active epoch — a real property, and not this one.

    THE DISCRIMINATION SURVIVES, in `evaluate_seed_vote_only_flat`. The vote-only counter is
    incremented under `!key_known` and BEFORE `finalization.verify` runs (`cert_inlet.rs:815`
    then `:818`), so a keyless follower ticks it once per cleared certificate on its way to
    refusing that certificate. Flat across the window is therefore "this follower held the epoch
    key throughout", which is what makes the refusals attributable to `PK_epoch`. Both readings
    are kept; what changed is which of them may be quoted for which claim.

    HOW TO MAKE THE REFUSAL DISCRIMINATE AGAIN, recorded because the idea is cheap and would
    otherwise be lost. A cleared slot decodes to `None`, which the beacon-active arm refuses
    without ever consulting the key. A slot that is present but WRONG does consult it: keyed gives
    `SeedCheck::Invalid` and refuses, keyless gives `NoKey` and ADMITS. It has to be a well-formed
    compressed G1 point or the certificate dies at decode like phase 3's — so the proxy would
    splice in the PREVIOUS certificate's 48 bytes, which is a real σ for a different round and
    fails `verify_seed` on the round binding. That is a third proxy mode and a phase of its own,
    entirely inside `scripts/cert-mitm-proxy.py`; it is NOT built, and this phase is not it.
    """
    case = "smoke-cert-follow"
    anchor = ctx.baseline_height()
    _say(ctx, f"smoke-cert-follow: DPoS converged; anchor finalized={anchor}")

    # ── Phase 1: subscribe-align ──────────────────────────────────────────────────────
    _say(ctx, "smoke-cert-follow: starting cert-follower (ws://172.20.0.10:8546)")
    ctx.overlay_up("cert-follower", note="cf-up-follower")

    align_target = vf.align_floor(anchor, ctx.interval)
    aligned = ctx.overlay_wait_align(vf.CF_SERVICE, align_target, vf.CF_ALIGN_S)
    ctx.check(case, bool(aligned),
              lambda: f"cert-follower did not align with v0 past {align_target} "
                      + _align_diag(ctx, ("cert-follower", vf.CF_SERVICE)),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, "cert-follower"))
    _ok(ctx, "phase 1 subscribe-align", f"cert-follower aligned with v0 at {aligned}")

    # PHASE 4b's PAIR IS STARTED HERE, ~10 minutes before it is used, and the placement is a fix
    # rather than an optimisation. Started at phase 4 it comes up 200+ blocks behind and has to
    # cold-start, sync, verify a certificate, and only THEN ask for the epoch artifact —
    # `observe_cert` fires on a verified certificate and on nothing else. Live, that took ~100 s
    # on one run and ran past the 180 s budget on the next, so the phase's outcome depended on how
    # long the three preceding phases happened to take.
    #
    # Starting it now is free of side effects: `cert-mitm-seed` relays VERBATIM until the harness
    # arms it, so until phase 4b this follower is indistinguishable from the honest one. It also
    # gives the sidecar's `pip install` the same head start, which is what the skip path below is
    # about. Phase 1's own alignment is measured BEFORE this, so the extra containers cannot
    # perturb it.
    seed_pair = _start_seed_pair(ctx)

    # ── Phase 2: gap back-fill ────────────────────────────────────────────────────────
    _, f1, _ = ctx.overlay_reading(vf.CF_SERVICE, "cert-follower finalized", case)
    _say(ctx, f"smoke-cert-follow: stopping cert-follower at finalized={f1}")
    ctx.overlay_stop("cert-follower", timeout=vf.CF_STOP_TIMEOUT_S, note="cf-stop-follower")
    # NON-FATAL, exactly as bash: `shutdown_flushed` resolves the container through the BARE
    # compose project, which does not define `cert-follower` at all, so this reports "not clean"
    # for a follower that exited perfectly. Bash warns and continues; promoting it to a failure
    # here would fail the case on the reader rather than on the node.
    if not ctx.shutdown_flushed("cert-follower"):
        _say(ctx, "  (warning) cert-follower did not exit cleanly (code 0); continuing")

    gap_target = vf.align_floor(f1, ctx.interval)
    _say(ctx, f"  waiting for v0 to advance past {gap_target} before restart")
    ctx.check(case, ctx.wait_finalized_ge(gap_target + 1, vf.CF_GAP_ADVANCE_S),
              f"v0 did not advance past {gap_target}")
    _, f2, _ = ctx.reading(topology.HOST_RPC_PORT, "v0 finalized", case)

    _say(ctx, f"  restarting cert-follower; must back-fill [{f1 + 1} .. {f2}] via getFinalization")
    ctx.overlay_start("cert-follower", note="cf-start-follower")
    caught = ctx.overlay_wait_align(vf.CF_SERVICE, f2, vf.CF_ALIGN_S)
    ctx.check(case, bool(caught),
              lambda: f"cert-follower did not back-fill the gap to >= {f2} "
                      + _align_diag(ctx, ("cert-follower", vf.CF_SERVICE)),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, "cert-follower"))
    _ok(ctx, "phase 2 gap back-fill", f"cert-follower caught up to {caught} (>= f2={f2})")

    # ── Phase 3: tampered-cert rejection (NEGATIVE) ───────────────────────────────────
    _say(ctx, "smoke-cert-follow: starting cert-mitm + cert-follower-tamper (negative)")
    ctx.overlay_up_ok("cert-mitm", note="cf-up-mitm")

    def mitm_up():
        return vf.mitm_ready(ctx.overlay_logs("cert-mitm"))

    if not ctx.poll(mitm_up, vf.MITM_UP_S, poll_s=vf.MITM_POLL_S):
        print("SKIP (phase 3 tampered-reject): cert-mitm proxy did not start (offline pip / no "
              "python) — positive phases passed; run where the sidecar can install 'websockets' "
              "to exercise the negative.", flush=True)
        _ok(ctx, case, "subscribe-align + gap back-fill verified")
        return

    v0_before = ctx.finalized_dec()
    ctx.overlay_up("cert-follower-tamper", note="cf-up-tamper")
    # LIVENESS FIRST. Everything below concludes "it rejected" from an ABSENCE, and a follower that
    # never started produces the same absence. `"null"` stays a legitimate PASS on the finalized
    # read (reth leaves it unset until something finalizes), so the witness has to be a separate
    # reading — `eth_blockNumber`, which a live node answers even with `finalized` unset.
    ctx.check(case, *vf.evaluate_tamper_alive(
        ctx.poll(lambda: ctx.overlay_head_dec(vf.TAMPER_SERVICE) >= 0, vf.TAMPER_UP_S)),
        on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, "cert-follower-tamper", "cert-mitm"))
    _say(ctx, f"  tamper-follower up; observing for {vf.TAMPER_OBSERVE_S}s while v0 advances")
    # THE OBSERVATION WINDOW IS THE ASSERTION — see the module header. Not a settle time.
    ctx.sleep(vf.TAMPER_OBSERVE_S)
    v0_after = ctx.finalized_dec()
    tamper_head = ctx.overlay_check_node(vf.TAMPER_SERVICE,
                                         dry_value="null|null").split("|", 1)[0]

    # The control first: a chain-wide stall makes the follower's stillness meaningless.
    ctx.check(case, *vf.evaluate_v0_advanced(v0_before, v0_after))
    ctx.check(case, *vf.evaluate_tamper_no_progress(tamper_head),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, "cert-follower-tamper",
                                                    "cert-mitm"))
    # AND THE DRIVER SAID SO. Zero progress cannot separate "the certs were refused" from "they
    # were never delivered" — a MITM that came up but forwarded nothing reads identically.
    rejected, msg, matched = vf.evaluate_tamper_rejected(
        ctx.overlay_logs("cert-follower-tamper", tail=vf.TAMPER_HINT_TAIL,
                         dry_value=vf.TAMPER_REJECT_LINES[0]))
    ctx.check(case, rejected, msg,
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, "cert-follower-tamper",
                                                    "cert-mitm"))
    _ok(ctx, "phase 3 tampered-reject",
        f"tamper-follower made ZERO finalized progress (finalized={tamper_head}) and logged "
        f"{matched!r} while v0 advanced {v0_before}→{v0_after}")

    _cert_follow_phase4(ctx, case, seed_pair)

    _ok(ctx, case, "subscribe-align + gap back-fill + tampered-cert rejection + PK_epoch delivery "
                   "(vote-only admissions stopped, cleared-seed certificate REFUSED) all verified")


def _start_seed_pair(ctx):
    """Bring up phase 4b's proxy + follower early, and report whether the proxy came up.

    Returns `{"ready": bool}`. A proxy that could not `pip install websockets` (offline host) is
    NOT an error — phase 4b skips loudly, exactly as phase 3 does — but its follower must then not
    be started at all, because a follower pointed at a dead upstream for ten minutes is a
    container burning CPU to prove nothing."""
    ctx.overlay_up_ok("cert-mitm-seed", note="cf-up-mitm-seed")

    def ready():
        return vf.mitm_ready(ctx.overlay_logs(vf.SEED_MITM_SERVICE))

    if not ctx.poll(ready, vf.MITM_UP_S, poll_s=vf.MITM_POLL_S):
        return {"ready": False}
    ctx.overlay_up("cert-follower-seed", note="cf-up-follower-seed")
    _say(ctx, "smoke-cert-follow: cert-mitm-seed + cert-follower-seed started early (relaying "
              "VERBATIM until armed) so phase 4b's follower is caught up when it is needed")
    return {"ready": True}


def _cert_follow_phase4(ctx, case: str, seed_pair) -> None:
    """Phase 4 (FLU-1167) — the follower obtains `PK_epoch`, and the seed check bites.

    ON THE SAME BRING-UP, deliberately: the ticket asks for exactly this scenario and a separate
    case would pay a second ~4 min migration to reproduce a stack phase 1 already built. It runs
    LAST because its negative half poisons a second certificate stream, and nothing may follow a
    poisoned stream.

    THE ORDER INSIDE IT IS THE ASSERTION, and it is the one thing a re-write can quietly break.
    `observe_cert` — the only trigger a follower has for fetching the epoch artifact — runs after
    a certificate VERIFIES (`cert_inlet.rs`: "skipped/tampered certs above never reach here"). So
    the seed-slot proxy must relay verbatim until the follower has the key, and the harness arms
    it only after reading the adoption line. Arm first and the follower stays vote-only forever:
    it would accept every cleared certificate, the negative would never fire, and the case would
    report that as "the follower is not verifying seeds" — which would be true, but for the
    harness's reason rather than the product's.
    """
    _say(ctx, "smoke-cert-follow: phase 4 — the follower must obtain PK_epoch and stop admitting "
              "certificates vote-only")

    # ── 4a. the POSITIVE half, on the HONEST follower ─────────────────────────────────
    #
    # Read on `cert-follower` and not on the phase-3 tamper node: that one refuses every
    # certificate, so it never calls `observe_cert` and could never obtain a key — its silence
    # would be a property of phase 3, not evidence about phase 4.
    box = {"logs": ""}

    def has_key():
        box["logs"] = ctx.overlay_logs(vf.CF_SERVICE, tail=vf.SEED_LOG_TAIL,
                                       dry_value=vf.CF_KEY_LINE + " epoch=2")
        return vf.CF_KEY_LINE in box["logs"]

    ctx.poll(has_key, vf.CF_KEY_S, poll_s=vf.CF_KEY_POLL_S)
    got, msg, key_line = vf.evaluate_key_obtained(box["logs"], vf.CF_SERVICE)
    ctx.check(case, got, msg,
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.CF_SERVICE))
    _say(ctx, f"  cert-follower holds PK_epoch: {key_line}")

    # The SAME adoption, off the follower's commonware registry. An ADDITIONAL witness, read
    # after the log line and never in place of it: the counter is incremented immediately before
    # the `info!` that produced the line above, so once the line is present the counter is already
    # up and there is no window to race. What the second reading adds is the endpoint — a registry
    # only a follower beacon registers these families on, and one a `--cert-follow` container did
    # not serve at all until `spawn_devnet_metrics` moved ahead of the validator/follower branch
    # and the compose overlay started passing `--dpos.metrics-port`.
    adopted_ok, adopted_msg, counters = vf.evaluate_artifact_adopted_counter(
        ctx.overlay_node_metrics_text(
            vf.CF_SERVICE,
            dry_value=f"{nodes.counter_sample(vf.CF_ADOPTED_FAMILY)} 1\n"
                      f"{nodes.counter_sample(vf.CF_MISS_FAMILY)} 0\n"),
        vf.CF_SERVICE)
    ctx.check(case, adopted_ok, adopted_msg,
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.CF_SERVICE))
    _say(ctx, f"  …and its beacon registry agrees: {counters}")

    # …and the consequence. TWO reads of the whole scrape, not two `metric` calls: an ABSENT
    # family and an UNREACHABLE endpoint both answer "" through a single-family read, and one of
    # those is a pass while the other is a measurement that never happened.
    text0 = ctx.overlay_el_metrics_text(vf.CF_SERVICE,
                                        dry_value=f"{vf.CF_VOTE_ONLY_FAMILY} 4\n")
    before = nodes.metric_val(text0, vf.CF_VOTE_ONLY_FAMILY, "")
    v0_before = ctx.finalized_dec()
    cf_before = nodes.hex_to_dec(
        ctx.overlay_check_node(vf.CF_SERVICE, dry_value="0x80|0xaa").split("|", 1)[0])
    # THE WINDOW IS THE ASSERTION — see the module header. Not a settle time.
    ctx.sleep(vf.CF_VOTE_ONLY_WINDOW_S)
    text1 = ctx.overlay_el_metrics_text(vf.CF_SERVICE,
                                        dry_value=f"{vf.CF_VOTE_ONLY_FAMILY} 4\n")
    after = nodes.metric_val(text1, vf.CF_VOTE_ONLY_FAMILY, "")
    v0_after = ctx.finalized_dec()
    cf_after = nodes.hex_to_dec(
        ctx.overlay_check_node(vf.CF_SERVICE, dry_value="0x9e|0xbb").split("|", 1)[0])

    # THE CONTROLS FIRST, exactly as phases 3 and 4b take them — this phase had NEITHER, and the
    # reading it defends is the one most exposed to their absence. A flat admission counter is
    # produced just as well by a stalled producer or by a follower that stopped ingesting
    # certificates altogether as by one that admits them with PK_epoch pinned.
    ctx.check(case, *vf.evaluate_v0_advanced(v0_before, v0_after))
    ctx.check(case, *vf.evaluate_follower_ingested(cf_before, cf_after, vf.CF_SERVICE),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.CF_SERVICE))
    ctx.check(case, *vf.evaluate_vote_only_flat(before, after,
                                                bool(text0.strip()) and bool(text1.strip())),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.CF_SERVICE))
    _ok(ctx, "phase 4a PK_epoch obtained",
        f"cert-follower verified the epoch artifact against committee[epoch] ({counters}) and "
        f"took NO further vote-only admission over {vf.CF_VOTE_ONLY_WINDOW_S}s "
        f"({vf.CF_VOTE_ONLY_FAMILY}={before or '0'} → {after or '0'}) while v0 advanced "
        f"{v0_before}→{v0_after} and the follower itself finalized {cf_before}→{cf_after} "
        "(so the certificates were arriving and being ADMITTED, not merely absent)")

    # ── 4b. the NEGATIVE half, on its own follower behind the seed-slot proxy ────────
    if not seed_pair["ready"]:
        print("SKIP (phase 4b cleared-seed reject): cert-mitm-seed did not start (offline pip / "
              "no python) — phase 4a passed, so the follower DOES obtain PK_epoch; what is NOT "
              "tested is that the key is then used to refuse a cleared seed slot.", flush=True)
        return

    # It must FOLLOW first — that is how it obtains its own key, and a node that never came up
    # would satisfy every reading below by doing nothing.
    seed_box = {"logs": ""}

    def seed_has_key():
        seed_box["logs"] = ctx.overlay_logs(vf.SEED_TAMPER_SERVICE, tail=vf.SEED_LOG_TAIL,
                                            dry_value=vf.CF_KEY_LINE + " epoch=2")
        return vf.CF_KEY_LINE in seed_box["logs"]

    ctx.poll(seed_has_key, vf.CF_KEY_S, poll_s=vf.CF_KEY_POLL_S)
    got, msg, seed_key_line = vf.evaluate_key_obtained(seed_box["logs"], vf.SEED_TAMPER_SERVICE)
    ctx.check(case, got, msg,
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.SEED_TAMPER_SERVICE,
                                                    vf.SEED_MITM_SERVICE))
    _say(ctx, f"  cert-follower-seed holds PK_epoch too: {seed_key_line}")

    # …AND IT MUST BE CAUGHT UP BEFORE THE ARMING. Holding the key is not enough, and this gate
    # is the difference between a real negative and a green-looking one.
    #
    # This follower is started LAST, after three earlier phases have run, so it comes up several
    # hundred blocks behind and back-fills. Arming while it is still behind leaves it advancing on
    # certificates that reached it BEFORE the proxy was armed — which the proxy, by construction,
    # cannot have touched. Live evidence: armed while ~370 blocks behind, it advanced 229 → 268
    # over the window and the phase read that as "the seed slot is not being checked"; armed while
    # caught up, it froze on the spot at 124 and logged one `BLS verify FAILED` per arriving
    # height. Same build, same proxy, opposite verdict — the only difference was the backlog.
    #
    # The floor is v0's finalized read a moment ago, the same idiom phase 2 uses: v0 keeps
    # moving, so passing its earlier value bounds the remaining gap by the time this took rather
    # than demanding an equality that a live chain can never hold.
    _, v0_now, _ = ctx.reading(topology.HOST_RPC_PORT, "v0 finalized (pre-arm)", case)
    caught = ctx.overlay_wait_align(vf.SEED_TAMPER_SERVICE, v0_now, vf.CF_ALIGN_S)
    ctx.check(case, *vf.evaluate_seed_follower_caught_up(caught, v0_now),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.SEED_TAMPER_SERVICE,
                                                    vf.SEED_MITM_SERVICE))
    _say(ctx, f"  cert-follower-seed caught up with v0 at {caught} (>= {v0_now}) — nothing it "
              "already holds can carry it past the arming")

    # ARM. Everything before this instant was an honest relay; everything after carries a cleared
    # seed slot on an untouched multisig quorum.
    #
    # THE BASELINES ARE READ FIRST, and they are the measurement — not the follower's height.
    # A poisoned certificate stream turns on a SECOND source for `finalized`: the follower refuses
    # every tampered certificate, counts the refusals as upstream data faults, rotates, and its EL
    # meanwhile keeps syncing over devp2p from the validator in `--trusted-peers`; the
    # steady-state re-jump then fast-forwards the anchor onto that EL tip. Live, with every
    # certificate correctly refused, `finalized` still went 238 → 271. So the refusals and the
    # vote-only counter are read, and the height is not.
    seed_rejects_before = vf.seed_reject_count(
        ctx.overlay_logs(vf.SEED_TAMPER_SERVICE, tail=vf.SEED_COUNT_TAIL, dry_value=""))
    vo_text0 = ctx.overlay_el_metrics_text(vf.SEED_TAMPER_SERVICE,
                                           dry_value=f"{vf.CF_VOTE_ONLY_FAMILY} 1\n")
    vo_before = nodes.metric_val(vo_text0, vf.CF_VOTE_ONLY_FAMILY, "")
    ctx.overlay_exec_write(vf.SEED_MITM_SERVICE, vf.SEED_ARM_FILE, "1", note="cf-arm-seed-mitm")

    # THE TAMPER MUST WITNESS ITS OWN TAMPERING (the `tear_journal_to_torn` rule): the proxy
    # re-slices the frame it is about to send and prints the first rewrite's before/after, and
    # this waits for that print before a single conclusion is drawn.
    def cleared():
        return vf.SEED_CLEARED_LINE in ctx.overlay_logs(vf.SEED_MITM_SERVICE,
                                                        dry_value=vf.SEED_ARMED_LINE + "\n" +
                                                        vf.SEED_CLEARED_LINE)

    ctx.poll(cleared, vf.SEED_ARM_S, poll_s=vf.SEED_ARM_POLL_S)
    ctx.check(case, *vf.evaluate_seed_tamper_landed(
        ctx.overlay_logs(vf.SEED_MITM_SERVICE,
                         dry_value=vf.SEED_ARMED_LINE + "\n" + vf.SEED_CLEARED_LINE)),
        on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.SEED_MITM_SERVICE))
    _say(ctx, f"  cert-mitm-seed armed and CLEARING seed slots; observing for {vf.SEED_OBSERVE_S}s")

    v0_before = ctx.finalized_dec()
    # THE OBSERVATION WINDOW IS THE ASSERTION — see the module header.
    ctx.sleep(vf.SEED_OBSERVE_S)
    v0_after = ctx.finalized_dec()
    seed_logs = ctx.overlay_logs(vf.SEED_TAMPER_SERVICE, tail=vf.SEED_COUNT_TAIL,
                                 dry_value="\n".join([vf.SEED_REJECT_LINE] * 40))
    seed_rejects_after = vf.seed_reject_count(seed_logs)
    vo_text1 = ctx.overlay_el_metrics_text(vf.SEED_TAMPER_SERVICE,
                                           dry_value=f"{vf.CF_VOTE_ONLY_FAMILY} 1\n")
    vo_after = nodes.metric_val(vo_text1, vf.CF_VOTE_ONLY_FAMILY, "")

    # The control first, exactly as in phase 3: a chain-wide stall makes every reading below
    # meaningless, because nothing would have been delivered to refuse.
    ctx.check(case, *vf.evaluate_v0_advanced(v0_before, v0_after))
    dump_seed = lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.SEED_TAMPER_SERVICE,
                                              vf.SEED_MITM_SERVICE)
    # It refused every certificate it was handed…
    ctx.check(case, *vf.evaluate_seed_tamper_refused_every_cert(
        seed_rejects_before, seed_rejects_after), on_fail=dump_seed)
    # …and the follower held `PK_epoch` throughout. The refusals alone no longer say that — any
    # scheme carrying an ORACLE refuses a cleared slot, keyed or not — so this counter is the only
    # member of the pair that separates a keyed follower from a keyless one (see the verdict).
    ctx.check(case, *vf.evaluate_seed_vote_only_flat(
        vo_before, vo_after, bool(vo_text0.strip()) and bool(vo_text1.strip())),
        on_fail=dump_seed)
    _ok(ctx, "phase 4b cleared-seed reject",
        f"cert-follower-seed refused {seed_rejects_after - seed_rejects_before} cleared-seed "
        f"certificates over {vf.SEED_OBSERVE_S}s ({vf.SEED_REJECT_LINE!r}) while v0 advanced "
        f"{v0_before}→{v0_after}, and admitted NONE of them vote-only "
        f"({vf.CF_VOTE_ONLY_FAMILY}={vo_before or '0'} → {vo_after or '0'}) — the seed slot is "
        "checked, and the flat vote-only counter says it was checked against a PK_epoch this "
        "follower actually holds")

    # PACING. The same instrument `smoke-base` uses and the same band (45..66 per 60 s) — the one
    # that produced the historical 26-27 blk/60s regression reading. Measured on v0 and AFTER the
    # negative half, so what it reports is the producer's rate with two followers, two proxies and
    # a poisoned stream all attached.
    r0 = ctx.finalized_dec()
    ctx.sleep(verdicts.PACING_WINDOW_S)
    r1 = ctx.finalized_dec()
    ctx.check(case, *verdicts.evaluate_pacing(r1 - r0))
    _say(ctx, f"  pacing {r1 - r0} blk/{verdicts.PACING_WINDOW_S}s")


# ══ smoke-cert-keyless ════════════════════════════════════════════════════════════════

def assert_cert_keyless(ctx) -> None:
    """A node enters a beacon-active epoch WITHOUT its key, its certificates are admitted on the
    multisig half alone through that window, and then the key arrives and certificate verification
    starts working — with NO repair path in the trace (FLU-1202).

    WHAT CHANGED, AND WHY IT NEEDED A CASE OF ITS OWN. `CombinedScheme` used to COPY the epoch's
    key material in at construction, so a scheme built before its epoch's key resolved was wrong
    for the life of that epoch and had to be PATCHED afterwards — `apply_pin`, `with_cert_seed_pin`
    and the `unpinned_epochs` sweep that drove them. The scheme now holds no material at all: it
    delegates to a beacon-provided oracle that reads the live key store on every call, so a key
    that lands after the scheme was built is picked up by the next certificate and there is
    nothing to repair. That whole subsystem is deleted, and with it the only reason a late key
    used to be a state-machine problem rather than a lookup.

    Deleting a repair path is not a change a green run can witness — a chain whose keys always
    arrive early looks identical either way. So the case is built around the ONE window in which
    the two designs differ, and it refuses to conclude anything until it has proved it was in it.

    THE PRECONDITION IS THE CASE. `evaluate_entered_keyless` gates every reading below on
    `dpos_cert_vote_only_admissions_total >= 1`, which `cert_inlet.rs:815` increments only under
    `!key_known` for the certificate's own epoch. Without that gate the case passes on a node that
    held the key from its first certificate — which is the majority of nodes on a healthy chain,
    and which proves nothing at all. The unit suite drives exactly that world
    (`test_smoke_follow_cases.py`) and requires the case to fail on the PRECONDITION rather than
    to sail through to a green flatness reading.

    WHY A `--cert-follow` FOLLOWER IS THE NODE. It is the one node class that enters a
    beacon-active epoch keyless BY CONSTRUCTION rather than by fault injection: it holds no share,
    mints nothing, and can only obtain `PK_epoch` by fetching the epoch's agreed artifact over its
    cert upstream — and the fetch is triggered by `observe_cert`, which fires on a certificate
    that has ALREADY been through the inlet. So the first certificate of a beacon-active epoch is
    necessarily admitted keyless, the window necessarily exists, and no node has to be stopped,
    starved or slowed to create it. Every other reproduction in this tree costs a tuned genesis
    and an eight-minute DKG choreography to manufacture the same state.

    ORDER, AND WHAT EACH STEP RULES OUT:

      1. the follower comes up and takes at least one KEYLESS admission — the precondition;
      2. it obtains `PK_epoch` over its cert upstream, witnessed by the log line AND by the
         adoption counter on the other registry;
      3. over a fixed window its vote-only admissions are FLAT while v0 advances and the follower
         itself finalizes — so certificates were arriving, being admitted, and every one of them
         had its seed slot checked;
      4. …and the below-frontier repair sweep, if it ran at all, ran AFTER the adoption — so the
         key came from this node's own artifact fetch and the sweep followed it rather than
         delivering it.

    THE TWO CONTROLS IN STEP 3 ARE NOT DECORATION, and they are the same pair `_cert_follow_phase4`
    takes. A flat admission counter is produced just as well by a stalled producer, or by a
    follower whose upstream died, as by one that is verifying seeds — and the second of those
    reads as a PASS on the counter alone.

    NO PART OF THIS CASE POISONS THE STACK. It starts one honest follower on the `cert-follow`
    overlay and leaves the MITM services the overlay also defines untouched, so it is the one
    member of this family that could in principle be chained. It is kept standalone anyway: it is
    the only case whose subject is the FIRST seconds of a node's life on a chain, and a chained
    run would hand it a stack whose beacon-active epochs are minutes old.
    """
    case = "smoke-cert-keyless"
    anchor = ctx.baseline_height()
    _say(ctx, f"smoke-cert-keyless: DPoS converged; anchor finalized={anchor}")

    def dump():
        ctx.overlay_dump_logs(vf.CF_LOG_TAIL, vf.CF_SERVICE)

    # ── 1. THE PRECONDITION: it entered a beacon-active epoch with NO key ─────────────
    _say(ctx, "smoke-cert-keyless: starting cert-follower (ws://172.20.0.10:8546) — it holds no "
              "epoch key and must admit its first certificates on the multisig half alone")
    ctx.overlay_up("cert-follower", note="ck-up-follower")

    pre = {"text": ""}

    def took_a_keyless_admission():
        # The WHOLE scrape, not one family: an absent family and an unreachable endpoint both
        # answer "" through a single-family read, and one of those is a real zero while the other
        # is a measurement that never happened (`evaluate_entered_keyless` separates them).
        pre["text"] = ctx.overlay_el_metrics_text(
            vf.CF_SERVICE, dry_value=f"{vf.CF_VOTE_ONLY_FAMILY} 3\n")
        return vf.vote_only_admissions(pre["text"]) >= 1

    ctx.poll(took_a_keyless_admission, vf.KEYLESS_ADMISSION_S,
             poll_s=vf.KEYLESS_ADMISSION_POLL_S)
    entered, msg, keyless_n = vf.evaluate_entered_keyless(pre["text"], vf.CF_SERVICE)
    ctx.check(case, entered, msg, on_fail=dump)
    _ok(ctx, "phase 1 keyless entry",
        f"cert-follower took {keyless_n} admission(s) on the multisig quorum ALONE "
        f"({vf.CF_VOTE_ONLY_FAMILY}={keyless_n}) — it is inside a beacon-active epoch it has no "
        "key for, which is the window this case exists to observe")

    # ── 2. …and then the key ARRIVES ──────────────────────────────────────────────────
    #
    # The WHOLE log (`SEED_LOG_TAIL is None`): the line is written once, in the follower's first
    # seconds, and a bounded tail scrolls it away — that turned a healthy keyed follower into "it
    # never obtained PK_epoch" on a live run of the sibling case.
    box = {"logs": ""}

    def has_key():
        box["logs"] = ctx.overlay_logs(vf.CF_SERVICE, tail=vf.SEED_LOG_TAIL,
                                       dry_value=vf.CF_KEY_LINE + " epoch=2")
        return vf.CF_KEY_LINE in box["logs"]

    ctx.poll(has_key, vf.CF_KEY_S, poll_s=vf.CF_KEY_POLL_S)
    got, msg, key_line = vf.evaluate_key_obtained(box["logs"], vf.CF_SERVICE)
    ctx.check(case, got, msg, on_fail=dump)
    # The SAME adoption off the other registry — corroboration, never a replacement: the counter
    # lives on an endpoint a `--cert-follow` node serves only under a devnet build plus a
    # `--dpos.metrics-port` the compose overlay has to pass, and a witness with two devnet
    # preconditions does not get to be the one a verdict rests on.
    adopted_ok, adopted_msg, counters = vf.evaluate_artifact_adopted_counter(
        ctx.overlay_node_metrics_text(
            vf.CF_SERVICE,
            dry_value=f"{nodes.counter_sample(vf.CF_ADOPTED_FAMILY)} 1\n"
                      f"{nodes.counter_sample(vf.CF_MISS_FAMILY)} 0\n"),
        vf.CF_SERVICE)
    ctx.check(case, adopted_ok, adopted_msg, on_fail=dump)
    _ok(ctx, "phase 2 late key",
        f"cert-follower obtained PK_epoch over its cert upstream ({counters}) — {key_line}")

    # ── 3. VERIFICATION STARTS WORKING, with the scheme it already had ────────────────
    #
    # Nothing rebuilt that scheme and nothing patched it. The reading that says so is the counter
    # going flat while certificates keep arriving: every one of them now resolves a key through
    # the oracle's live store read and has its seed slot checked.
    text0 = ctx.overlay_el_metrics_text(vf.CF_SERVICE,
                                        dry_value=f"{vf.CF_VOTE_ONLY_FAMILY} {keyless_n}\n")
    before = nodes.metric_val(text0, vf.CF_VOTE_ONLY_FAMILY, "")
    v0_before = ctx.finalized_dec()
    cf_before = nodes.hex_to_dec(
        ctx.overlay_check_node(vf.CF_SERVICE, dry_value="0x80|0xaa").split("|", 1)[0])
    # THE WINDOW IS THE ASSERTION — see the module header. Not a settle time.
    ctx.sleep(vf.CF_VOTE_ONLY_WINDOW_S)
    text1 = ctx.overlay_el_metrics_text(vf.CF_SERVICE,
                                        dry_value=f"{vf.CF_VOTE_ONLY_FAMILY} {keyless_n}\n")
    after = nodes.metric_val(text1, vf.CF_VOTE_ONLY_FAMILY, "")
    v0_after = ctx.finalized_dec()
    cf_after = nodes.hex_to_dec(
        ctx.overlay_check_node(vf.CF_SERVICE, dry_value="0x9e|0xbb").split("|", 1)[0])

    # THE CONTROLS FIRST. A flat counter is also what a dead upstream produces.
    ctx.check(case, *vf.evaluate_v0_advanced(v0_before, v0_after))
    ctx.check(case, *vf.evaluate_follower_ingested(cf_before, cf_after, vf.CF_SERVICE),
              on_fail=dump)
    ctx.check(case, *vf.evaluate_vote_only_flat(before, after,
                                                bool(text0.strip()) and bool(text1.strip())),
              on_fail=dump)

    # ── 4. …and the repair sweep FOLLOWED the key rather than delivering it ───────────
    #
    # Read AFTER the window and not before it: the sweep reaches an epoch when that epoch drops
    # below the frontier, which on this geometry happens during the window, and a grep taken at
    # key-arrival time would be silent about the whole interval the flatness verdict covers. The
    # first live run of this case is the evidence — the sweep spoke 32 s after the adoption.
    repaired_ok, repaired_msg, repair_note = vf.evaluate_repair_did_not_deliver_the_key(
        ctx.overlay_logs(vf.CF_SERVICE, tail=vf.SEED_LOG_TAIL,
                         dry_value=vf.CF_KEY_LINE + " epoch=2"), vf.CF_SERVICE)
    ctx.check(case, repaired_ok, repaired_msg, on_fail=dump)

    _ok(ctx, case,
        f"cert-follower entered a beacon-active epoch keyless ({keyless_n} admission(s) on the "
        f"multisig quorum alone), obtained PK_epoch, and then took NO further vote-only admission "
        f"over {vf.CF_VOTE_ONLY_WINDOW_S}s ({vf.CF_VOTE_ONLY_FAMILY}={before or '0'} -> "
        f"{after or '0'}) while v0 advanced {v0_before}->{v0_after} and the follower itself "
        f"finalized {cf_before}->{cf_after} — the late key took effect through the scheme it "
        f"already had ({repair_note})")


# ══ smoke-cert-cascade ════════════════════════════════════════════════════════════════

def assert_cert_cascade(ctx) -> None:
    """The lean cert-follower's SERVING side plus its L1 trust root.

      1. L1-checkpoint align — a MockRollup carrying a real finalized hash is deployed on the
         devnet L2 itself, and a tier-1 follower with `--cert-follow.l1-rpc-url` verifies the
         checkpoint against its OWN synced chain before it will follow;
      2. cascade            — a tier-2 follower whose only upstream is tier 1's window-backed
         `consensus` WS aligns too (followers serve followers);
      3. bogus-checkpoint reject — a follower pointed at a Rollup whose checkpoint hash is in no
         block of the chain must refuse, and must finalize nothing while refusing.

    The "L1" here is the devnet's own RPC: only the DATA is mocked, the read path is the real one.
    """
    case = "smoke-cert-cascade"
    anchor = ctx.baseline_height()
    _say(ctx, f"smoke-cert-cascade: DPoS converged; anchor finalized={anchor}")

    key = ctx.funded_key()
    bytecode = _mock_rollup_bytecode(ctx, case)
    send = ["--private-key", f"0x{key}", "--rpc-url", ctx.rpc]

    # ── MockRollup deploy + checkpoint push ───────────────────────────────────────────
    mock = ctx.cast_send(send + ["--create", bytecode], note="cc-deploy-mock-rollup",
                         dry_value='{"contractAddress":"0x' + "11" * 20 + '"}')
    ok, msg, mock_addr = vf.contract_address(mock, "MockRollup")
    ctx.check(case, ok, msg)
    _say(ctx, f"smoke-cert-cascade: MockRollup deployed at {mock_addr}")

    # ONE read, both fields. Bash calls `check_external 8545` TWICE here (`:33-34`), once for the
    # height and once for the hash — two round trips that can straddle a new block, so the pair it
    # pushes may name a height and a hash from DIFFERENT blocks. One atomic read cannot.
    fin_hex, _, fin_hash = ctx.reading(topology.HOST_RPC_PORT, "producer finalized", case)
    set_receipt = ctx.cast_send(
        send + [mock_addr, vf.SET_CHECKPOINT_SIG, str(vf.CHECKPOINT_BATCH), fin_hash],
        note="cc-set-checkpoint", dry_value='{"blockNumber":"0x80"}')
    ok, msg, set_block = vf.receipt_block(set_receipt, "setCheckpoint")
    ctx.check(case, ok, msg)
    _say(ctx, f"smoke-cert-cascade: checkpoint pushed in block {set_block} "
              f"(batch {vf.CHECKPOINT_BATCH} → finalized {fin_hex} {fin_hash})")
    # The follower reads the Rollup at the FINALIZED tag (the two-tier lag is K blocks), so the
    # setCheckpoint block has to be final before it starts — otherwise it looks up a batch that,
    # from its point of view, has not been written yet.
    ctx.check(case, ctx.wait_finalized_ge(set_block, vf.CC_FINALIZE_S),
              "setCheckpoint block never finalized")

    # ── Phase 1: tier-1 follower with the L1 trust root ───────────────────────────────
    ctx.export_compose_env(vf.MOCK_ROLLUP_ENV, mock_addr)
    ctx.overlay_up("cert-follower-l1", note="cc-up-tier1")

    align_target = vf.align_floor(anchor, ctx.interval)
    aligned = ctx.overlay_wait_align(vf.T1_SERVICE, align_target, vf.CC_ALIGN_S)
    ctx.check(case, bool(aligned),
              lambda: f"tier-1 did not align with v0 past {align_target} "
                      + _align_diag(ctx, ("tier-1", vf.T1_SERVICE)),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CC_LOG_TAIL, "cert-follower-l1"))
    ctx.check(case, *vf.evaluate_l1_checkpoint_verified(ctx.overlay_logs("cert-follower-l1")))
    _ok(ctx, "phase 1 L1-checkpoint align",
        f"tier-1 verified the Rollup checkpoint and aligned at {aligned}")

    # ── Phase 2: tier-2 follower fed ONLY by tier 1 ───────────────────────────────────
    ctx.overlay_up("cert-follower-tier2", note="cc-up-tier2")
    t2_target = nodes.hex_to_dec(str(aligned).split("|", 1)[0]) + ctx.interval
    t2_aligned = ctx.overlay_wait_align(vf.T2_SERVICE, t2_target, vf.CC_ALIGN_S)
    ctx.check(case, bool(t2_aligned),
              lambda: f"tier-2 did not align via the tier-1 window past {t2_target} "
                      + _align_diag(ctx, ("tier-2", vf.T2_SERVICE), ("tier-1", vf.T1_SERVICE)),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CC_LOG_TAIL, "cert-follower-tier2",
                                                    "cert-follower-l1"))
    _ok(ctx, "phase 2 cascade", f"tier-2 aligned with v0 at {t2_aligned} through tier 1")

    # ── Phase 3: bogus checkpoint must fail closed (NEGATIVE) ─────────────────────────
    bogus = ctx.cast_send(send + ["--create", bytecode], note="cc-deploy-bogus-rollup",
                          dry_value='{"contractAddress":"0x' + "22" * 20 + '"}')
    ok, msg, bogus_addr = vf.contract_address(bogus, "bogus MockRollup")
    ctx.check(case, ok, msg)
    bogus_receipt = ctx.cast_send(
        send + [bogus_addr, vf.SET_CHECKPOINT_SIG, str(vf.CHECKPOINT_BATCH),
                vf.BOGUS_CHECKPOINT_HASH],
        note="cc-set-bogus-checkpoint", dry_value='{"blockNumber":"0x90"}')
    ok, msg, bogus_block = vf.receipt_block(bogus_receipt, "bogus setCheckpoint")
    ctx.check(case, ok, msg)
    ctx.check(case, ctx.wait_finalized_ge(bogus_block, vf.CC_FINALIZE_S),
              "bogus setCheckpoint block never finalized")

    ctx.export_compose_env(vf.BOGUS_ROLLUP_ENV, bogus_addr)
    ctx.overlay_up("cert-follower-l1-bogus", note="cc-up-bogus")

    def refused():
        """The refusal, by the product line — the only witness. See `evaluate_bogus_rejected` on
        why the container-state one was removed rather than kept as a fallback."""
        ok, _, witness = vf.evaluate_bogus_rejected(
            ctx.overlay_logs("cert-follower-l1-bogus", dry_value=vf.BOGUS_REJECT_LINE))
        return witness if ok else False

    hit = ctx.poll(refused, vf.CC_REJECT_S, poll_s=vf.CC_REJECT_POLL_S, dry_value="dry-witness")
    ctx.check(case, bool(hit), vf.BOGUS_NOT_REFUSED,
              on_fail=lambda: ctx.overlay_dump_logs(vf.CC_BOGUS_LOG_TAIL,
                                                    "cert-follower-l1-bogus"))
    bogus_reading = ctx.overlay_check_node(vf.BOGUS_SERVICE, dry_value="null|null")
    ctx.check(case, *vf.evaluate_bogus_no_progress(bogus_reading, anchor))
    _ok(ctx, "phase 3 bogus-checkpoint reject",
        f"follower refused the unverifiable trust root ({hit}; "
        f"finalized={bogus_reading.split('|', 1)[0]})")

    _ok(ctx, case, "L1 trust root + follower cascade + fail-closed reject all verified")


def _mock_rollup_bytecode(ctx, case: str) -> str:
    """`case-cert-cascade.sh:36` — the creation bytecode out of the vendored forge artifact.

    Fail-loud on an unreadable/empty artifact: an empty `--create` deploys a contract with no
    code, `setCheckpoint` on it then succeeds silently (a call to a codeless address returns
    empty, not an error), and phase 1 would fail as if the follower could not read the Rollup."""
    try:
        with open(MOCK_ROLLUP_JSON) as fh:
            code = json.load(fh)["bytecode"]["object"]
    except (OSError, ValueError, KeyError, TypeError) as e:
        raise SmokeFailure(case, f"could not read the MockRollup bytecode from "
                                 f"{MOCK_ROLLUP_JSON}: {e}")
    if not (isinstance(code, str) and code.startswith("0x") and len(code) > 2):
        raise SmokeFailure(case, f"MockRollup artifact carries no bytecode ({MOCK_ROLLUP_JSON})")
    return code


# ══ smoke-tx-cascade ══════════════════════════════════════════════════════════════════

def assert_tx_cascade(ctx) -> None:
    """The DPoS TRANSACTION WRITE PATH across the operator's sentry cascade.

    Topology: validators (L1, hidden) → sentry (L2, public, knows v0) → downstream (L3, reaches
    ONLY L2). The `--cert-upstream` feed carries CERTIFICATES and no transactions, so the write
    path rides a SEPARATE link — reth devp2p tx-gossip over the trusted-peer mesh, relayed
    L3→L2→validator.

    The assertion is that a transaction submitted to L3 reaches a hidden validator's proposer
    pool, is mined, finalizes, and syncs back to L3 with its STATE applied. The node each read is
    issued against is half of the claim: the receipt is read off validator-0 (only a proposer's
    pool can put a tx on chain), and the balance/allowance are read off L3 (only execution can
    move them there).
    """
    case = "smoke-tx-cascade"
    anchor_dec = ctx.anchor_dec(case)
    # L3 keeps a HOST URL because the tx leg below signs and sends with host-side `cast`, and the
    # container carries no foundry. Its port is 28545, below the ephemeral range, so it is safe to
    # publish; the sentry's was 38545 and is gone — every sentry read/write here goes over exec.
    l3_rpc = topology.host_url(vf.DOWNSTREAM_PORT)

    # ── L2 sentry: cert-follow v0 + devp2p pinned to v0 ───────────────────────────────
    _say(ctx, "smoke-tx-cascade: starting L2 sentry (cert-follow v0; devp2p → v0)")
    ctx.overlay_up("sentry", note="txc-up-sentry")
    ctx.check(case, bool(ctx.overlay_wait_align(vf.SENTRY_SERVICE, anchor_dec, vf.TXC_ALIGN_S)),
              f"sentry (L2) did not align past {anchor_dec}",
              on_fail=lambda: ctx.overlay_dump_logs(vf.TXC_LOG_TAIL, "sentry"))
    _say(ctx, f"  sentry (L2) aligned with v0 past {anchor_dec}")

    # ── L3 downstream: cert-follow the SENTRY + devp2p pinned to ONLY the sentry ──────
    pk = ctx.overlay_enode_pubkey(vf.SENTRY_SERVICE)
    ctx.check(case, *vf.evaluate_enode_pubkey(pk, "sentry"))
    ctx.overlay_exec_write("sentry", "/runtime/sentry-enode.txt", vf.sentry_enode(pk),
                           note="txc-write-sentry-enode")
    _say(ctx, "smoke-tx-cascade: sentry enode captured → starting L3 downstream "
              "(devp2p → sentry ONLY)")
    ctx.overlay_up("downstream", note="txc-up-downstream")

    # Mutual trust: the sentry must ACCEPT L3's inbound (BOTH keep --trusted-only, and the case
    # widens neither). The retry is on L3's RPC coming up, not on the peering.
    pk3 = ctx.sample_until(vf.TXC_ENODE_TRIES,
                           lambda _i: _valid_pubkey(ctx.enode_pubkey(l3_rpc)),
                           sleep_s=vf.TXC_ENODE_SLEEP_S, label="l3-enode",
                           dry_value="cd" * 64)
    ctx.check(case, *vf.evaluate_enode_pubkey(pk3 or "", "downstream"))
    ctx.overlay_rpc_write(vf.SENTRY_SERVICE, "admin_addTrustedPeer", vf.downstream_enode(pk3),
                          note="txc-trust-l3-on-sentry")
    _say(ctx, "  sentry now trusts L3 (mutual) → awaiting L3 align via the sentry")
    ctx.check(case, bool(ctx.overlay_wait_align(vf.DOWNSTREAM_SERVICE, anchor_dec,
                                                vf.TXC_ALIGN_S)),
              f"L3 did not align past {anchor_dec} via the sentry",
              on_fail=lambda: ctx.overlay_dump_logs(vf.TXC_LOG_TAIL, "downstream"))
    _say(ctx, f"  L3 aligned with v0 past {anchor_dec} through the sentry (validators→L2→L3)")

    # ── PRIVACY: L3 reaches the sentry and (structurally) nothing else ────────────────
    peers = vf.peer_count_from_rpc(ctx.cast_rpc("net_peerCount", rpc_url=l3_rpc,
                                                dry_value='"0x1"'))
    ok, msg, note = vf.evaluate_l3_peers(peers)
    ctx.check(case, ok, msg)
    if note:
        _say(ctx, f"  NOTE: {note}")
    _say(ctx, f"  L3 devp2p peers={peers} (sole peer = sentry; no validator contact)")

    # ── WRITE PATH: submit to L3, assert it reaches a hidden validator ────────────────
    key = ctx.funded_key()
    sender = ctx.wallet_address(key)
    dead, blend = verdicts.DEAD_ADDR, verdicts.BLEND_ADDR
    bal_before = _balance(ctx, case, dead, l3_rpc)

    _say(ctx, f"smoke-tx-cascade: submitting value transfer + MockBlendToken.approve to L3 "
              f"({l3_rpc})")
    # EXPLICIT SEQUENTIAL NONCES. Both sends go out `--async` and neither is mined by L3 (a
    # follower mines nothing), so cast's per-invocation "latest" nonce would hand the same number
    # to both and the second would come back "replacement transaction underpriced".
    nonce = _nonce(ctx, case, sender, l3_rpc)
    common = ["--private-key", f"0x{key}", "--rpc-url", l3_rpc, "--chain", ctx.chain_id]
    txh = ctx.cast_send_async(["--nonce", str(nonce)] + common + [dead, "--value", "0.05ether"],
                              note="txc-l3-value-transfer", dry_value="0xvalue")
    ctxh = ctx.cast_send_async(["--nonce", str(nonce + 1)] + common +
                               [blend, "approve(address,uint256)", dead, str(vf.TXC_ALLOW)],
                               note="txc-l3-approve", dry_value="0xapprove")
    _say(ctx, f"  submitted to L3: transfer={txh} (nonce {nonce}) approve={ctxh} "
              f"(nonce {nonce + 1})")

    # Proof it reached a PROPOSER and not merely L3's local pool: the receipt appears on
    # validator-0, a HIDDEN L1 validator that L3 cannot reach.
    maxblk = 0
    for h in (txh, ctxh):
        receipt = ctx.sample_until(
            vf.TXC_RECEIPT_TRIES,
            lambda _i, _h=h: _receipt_or_false(ctx, _h, ctx.rpc),
            sleep_s=vf.TXC_RECEIPT_SLEEP_S, label=f"producer-receipt {h}",
            dry_value={"status": "0x1", "blockNumber": "0x80"})
        status = (receipt or {}).get("status", "")
        ctx.check(case, *vf.evaluate_tx_mined(h, status),
                  on_fail=lambda: ctx.overlay_dump_logs(vf.TXC_MINE_LOG_TAIL, "sentry",
                                                        "downstream"))
        # FAIL-LOUD, not `.get(…, "0x0")`: a receipt with a status but no blockNumber would make
        # the finality wait below target height 0, which every live chain satisfies instantly —
        # the case would then claim the tx block finalized without ever naming it.
        ok, msg, blk = vf.receipt_block(receipt, f"receipt for {h}")
        ctx.check(case, ok, msg)
        maxblk = max(maxblk, blk)
        _say(ctx, f"  validator-0 mined {h} in block {blk} (reached a hidden validator's "
                  "proposer pool)")

    ctx.check(case, ctx.wait_finalized_ge(maxblk, vf.TXC_FINALIZE_S),
              f"tx block {maxblk} not finalized in time")
    _say(ctx, f"  tx block {maxblk} finalized by the validators")

    # ── ROUND TRIP: L3 syncs the mined+finalized block back and applies the state ─────
    l3_receipt = ctx.sample_until(
        vf.TXC_L3_RECEIPT_TRIES, lambda _i: _receipt_or_false(ctx, txh, l3_rpc),
        sleep_s=vf.TXC_RECEIPT_SLEEP_S, label="l3-receipt",
        dry_value={"status": "0x1", "blockNumber": "0x80"})
    ctx.check(case, *vf.evaluate_l3_synced_receipt(txh, (l3_receipt or {}).get("status", "")))

    bal_after = _balance(ctx, case, dead, l3_rpc)
    allowance = verdicts.first_token(
        ctx.cast_call(blend, "allowance(address,address)(uint256)", sender, dead,
                      rpc_url=l3_rpc, dry_value=str(vf.TXC_ALLOW)))
    ctx.check(case, *vf.evaluate_l3_state(bal_after - bal_before, allowance))

    # A TRIPWIRE, not a witness. `vf.evaluate_no_isolated_warning` cannot fail today: the monitor
    # that would emit `tx-route ISOLATED` is absent from the tree and from every git ref (audit
    # B9). It stays because it is the right false-positive guard to already have in place the day
    # the monitor comes back — and the unit suite pins the absence so that day is not silent.
    ctx.check(case, *vf.evaluate_no_isolated_warning(ctx.overlay_logs("sentry", "downstream")))
    _say(ctx, "smoke-tx-cascade: NOT COVERED — tx-route ISOLATION detection. "
              "`crates/node/src/tx_route.rs` does not exist (DPOS_AUDIT B9, doc §8.5.1), so a "
              "node with zero devp2p tx peers still accepts transactions into a pool nothing "
              "drains, silently. This case proves the route WORKS when the peers are there; "
              "nothing here proves anything about what happens when they are not")

    _ok(ctx, case, "tx submitted to L3 (reaches ONLY the sentry) relayed via devp2p tx-gossip to "
                   "a hidden validator, mined by the proposer, finalized, and synced back to L3 "
                   "with state applied. (Isolation DETECTION is out of scope — see the NOT "
                   "COVERED note above.)")


def _nonce(ctx, case: str, addr: str, rpc_url: str) -> int:
    """`cast nonce` as an int, FAIL-LOUD on anything else (`case-tx-cascade.sh:100-101`).

    bash gets this for free — the `|| { echo FAIL; exit 1; }` fires on a failed read. Coercing an
    unreadable answer to 0 here would submit both transactions at nonce 0 and 1, which the node
    rejects as "nonce too low": a failure that reads like a broken write path, three assertions
    away from the read that actually failed."""
    raw = ctx.cast_nonce(addr, rpc_url=rpc_url, dry_value="7")
    tok = verdicts.first_token(raw)
    if not tok.isdigit():
        raise SmokeFailure(case, f"could not read sender nonce from L3 (cast nonce {addr} "
                                 f"returned {raw!r})")
    return int(tok)


def _valid_pubkey(pk: str):
    """The retry probe: the pubkey itself once it is well-formed, else False (keep waiting)."""
    return pk if vf.evaluate_enode_pubkey(pk, "")[0] else False


def _receipt_or_false(ctx, txhash: str, rpc_url: str):
    """One `cast receipt … --json` off `rpc_url`; the parsed receipt once it HAS a status, else
    False. Bash spins on `.status // empty` being non-empty and reads the same receipt a second
    time for `.blockNumber`; one parse answers both."""
    r = ctx.cast_receipt(txhash, rpc_url=rpc_url,
                         dry_value='{"status":"0x1","blockNumber":"0x80"}')
    return r if r.get("status") else False
