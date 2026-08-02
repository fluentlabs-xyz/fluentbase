"""asserts_follow.py — the three FOLLOWER case bodies (`case-cert-follow.sh`,
`case-cert-cascade.sh`, `case-tx-cascade.sh`).

These are the first cases in the suite that need SERVICES THE BASE COMPOSE FILE DOES NOT DEFINE.
Each one brings its own compose overlay up through `SmokeCtx`'s overlay seam (see `driver.py`),
drives the services it defines, and leaves them to the bare `--remove-orphans` teardown.

Same split as everywhere else here: this module decides what to read and when, `verdicts_follow`
decides what the readings mean.

═══ WHAT IS DIFFERENT ABOUT THIS TRIO, AND WHAT A PORT CAN QUIETLY LOSE ══════════════════

1. **Four of the assertions are NEGATIVE** — the tampered-cert rejection, the bogus-checkpoint
   refusal, the bogus follower's lack of progress, and the tx-route monitor's silence. Two of
   them conclude from a fixed OBSERVATION WINDOW, i.e. they pass on a timeout:

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

`scripts/cert-mitm-proxy.py` is NOT touched and NOT ported: it is already Python, the compose
overlay launches it (`docker-compose.cert-follow.yml:58` mounts `./scripts:/proxy:ro`), and
compose files are outside this change. It stays where it is and is referenced as-is.
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


def _align_diag(ctx, *labelled_ports) -> str:
    """`(<label>=<reading>, …, v0=<reading>)` for a FAILED `wait_follower_align`.

    Pass this to `ctx.check` as a CALLABLE message — `check` evaluates it only on the failure
    path (its docstring: the bash diagnostic RPCs live inside the failing `echo`, and an f-string
    would issue them on every PASS too, including in the dry transcript).

    Without it the artifact is the follower's log tail alone, which cannot separate "three blocks
    behind" from "on a different chain" — and those are opposite verdicts. `case-cert-catchup.sh`
    already prints both sides (`victim=… v0=…`, :170); this is that, for these cases."""
    parts = [f"{label}={ctx.check_external(port)}" for label, port in labelled_ports]
    parts.append(f"v0={ctx.check_external(topology.HOST_RPC_PORT)}")
    return "(" + ", ".join(parts) + ")"


def _reading(ctx, case: str, port, what: str):
    """`check_external <port>` -> `(head_hex, head_dec, hash)`, FAIL-LOUD on the sentinel.

    §2.4 item 5, at the two places in this chunk where a reading is used as DATA rather than as a
    verdict. `nodes.hex_to_dec("null")` is 0, and 0 is a perfectly usable height: `cert-follow`
    would compute its back-fill gap from an unreachable follower and require v0 to pass block 33,
    which a live chain did minutes ago — the phase would still pass, over a gap that was never
    created. `cert-cascade` would push `"null"` into a `bytes32` and fail three commands later.

    Deliberately NOT used for the tamper or bogus readings: there `"null"` is the expected PASS
    value, and its verdict function is the thing that says so."""
    reading = ctx.check_external(port, dry_value="0x80|0x" + "ab" * 32)
    head, _, digest = (reading or "").partition("|")
    if not head.startswith("0x"):
        raise SmokeFailure(case, f"{what} on host port {port} read {reading!r} — refusing to read "
                                 "an unreachable node as height 0")
    return head, nodes.hex_to_dec(head), digest


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

    Three phases, and the third is the reason the first two are not enough:

      1. subscribe-align — it catches up and finalized-aligns with v0 across an epoch boundary;
      2. gap back-fill   — stopped past a real gap and restarted, it catches up via
                           `getFinalization` (persistent resume, not a re-sync from genesis);
      3. tampered reject — fed byte-flipped certificates through a WS man-in-the-middle, it makes
                           ZERO finalized progress.

    Phases 1 and 2 prove a follower CAN follow. Only phase 3 proves it verifies: a node that
    accepted every certificate it was handed would pass both positive phases perfectly.
    """
    case = "smoke-cert-follow"
    anchor = ctx.finalized_dec()
    _say(ctx, f"smoke-cert-follow: DPoS converged; anchor finalized={anchor}")

    # ── Phase 1: subscribe-align ──────────────────────────────────────────────────────
    _say(ctx, "smoke-cert-follow: starting cert-follower (ws://172.20.0.10:8546)")
    ctx.overlay_up("cert-follower", note="cf-up-follower")

    align_target = vf.align_floor(anchor, ctx.interval)
    aligned = ctx.wait_follower_align(vf.CF_PORT, align_target, vf.CF_ALIGN_S)
    ctx.check(case, bool(aligned),
              lambda: f"cert-follower did not align with v0 past {align_target} "
                      + _align_diag(ctx, ("cert-follower", vf.CF_PORT)),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, "cert-follower"))
    _ok(ctx, "phase 1 subscribe-align", f"cert-follower aligned with v0 at {aligned}")

    # ── Phase 2: gap back-fill ────────────────────────────────────────────────────────
    _, f1, _ = _reading(ctx, case, vf.CF_PORT, "cert-follower finalized")
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
    f2 = ctx.finalized_dec()

    _say(ctx, f"  restarting cert-follower; must back-fill [{f1 + 1} .. {f2}] via getFinalization")
    ctx.overlay_start("cert-follower", note="cf-start-follower")
    caught = ctx.wait_follower_align(vf.CF_PORT, f2, vf.CF_ALIGN_S)
    ctx.check(case, bool(caught),
              lambda: f"cert-follower did not back-fill the gap to >= {f2} "
                      + _align_diag(ctx, ("cert-follower", vf.CF_PORT)),
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
    _say(ctx, f"  tamper-follower up; observing for {vf.TAMPER_OBSERVE_S}s while v0 advances")
    # THE OBSERVATION WINDOW IS THE ASSERTION — see the module header. Not a settle time.
    ctx.sleep(vf.TAMPER_OBSERVE_S)
    v0_after = ctx.finalized_dec()
    tamper_head = ctx.check_external(vf.TAMPER_PORT, dry_value="null|null").split("|", 1)[0]

    # The control first: a chain-wide stall makes the follower's stillness meaningless.
    ctx.check(case, *vf.evaluate_v0_advanced(v0_before, v0_after))
    ctx.check(case, *vf.evaluate_tamper_no_progress(tamper_head),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CF_LOG_TAIL, "cert-follower-tamper",
                                                    "cert-mitm"))
    if vf.tamper_rejection_hint(ctx.overlay_logs("cert-follower-tamper",
                                                 tail=vf.TAMPER_HINT_TAIL)):
        _say(ctx, "  (hint) rejection fired at the driver live-tail verify")
    _ok(ctx, "phase 3 tampered-reject",
        f"tamper-follower made ZERO finalized progress (finalized={tamper_head}) while v0 "
        f"advanced {v0_before}→{v0_after}")

    _ok(ctx, case, "subscribe-align + gap back-fill + tampered-cert rejection all verified")


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
    anchor = ctx.finalized_dec()
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
    fin_hex, _, fin_hash = _reading(ctx, case, topology.HOST_RPC_PORT, "producer finalized")
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
    aligned = ctx.wait_follower_align(vf.T1_PORT, align_target, vf.CC_ALIGN_S)
    ctx.check(case, bool(aligned),
              lambda: f"tier-1 did not align with v0 past {align_target} "
                      + _align_diag(ctx, ("tier-1", vf.T1_PORT)),
              on_fail=lambda: ctx.overlay_dump_logs(vf.CC_LOG_TAIL, "cert-follower-l1"))
    ctx.check(case, *vf.evaluate_l1_checkpoint_verified(ctx.overlay_logs("cert-follower-l1")))
    _ok(ctx, "phase 1 L1-checkpoint align",
        f"tier-1 verified the Rollup checkpoint and aligned at {aligned}")

    # ── Phase 2: tier-2 follower fed ONLY by tier 1 ───────────────────────────────────
    ctx.overlay_up("cert-follower-tier2", note="cc-up-tier2")
    t2_target = nodes.hex_to_dec(str(aligned).split("|", 1)[0]) + ctx.interval
    t2_aligned = ctx.wait_follower_align(vf.T2_PORT, t2_target, vf.CC_ALIGN_S)
    ctx.check(case, bool(t2_aligned),
              lambda: f"tier-2 did not align via the tier-1 window past {t2_target} "
                      + _align_diag(ctx, ("tier-2", vf.T2_PORT), ("tier-1", vf.T1_PORT)),
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
        """The refusal, by either witness. The `ps` read is LAZY, as bash's is: the follower
        usually logs and stays up, and paying for a second command per poll would double the read
        load on a docker daemon that is also running eight nodes."""
        logs = ctx.overlay_logs("cert-follower-l1-bogus")
        ok_line, _, witness = vf.evaluate_bogus_rejected(logs, "")
        if ok_line:
            return witness
        ok_state, _, witness = vf.evaluate_bogus_rejected(
            logs, ctx.overlay_ps_state("cert-follower-l1-bogus"))
        return witness if ok_state else False

    hit = ctx.poll(refused, vf.CC_REJECT_S, poll_s=vf.CC_REJECT_POLL_S, dry_value="dry-witness")
    ctx.check(case, bool(hit), vf.BOGUS_NOT_REFUSED,
              on_fail=lambda: ctx.overlay_dump_logs(vf.CC_BOGUS_LOG_TAIL,
                                                    "cert-follower-l1-bogus"))
    bogus_reading = ctx.check_external(vf.BOGUS_PORT, dry_value="null|null")
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
    sentry_rpc = topology.host_url(vf.SENTRY_PORT)
    l3_rpc = topology.host_url(vf.DOWNSTREAM_PORT)

    # ── L2 sentry: cert-follow v0 + devp2p pinned to v0 ───────────────────────────────
    _say(ctx, "smoke-tx-cascade: starting L2 sentry (cert-follow v0; devp2p → v0)")
    ctx.overlay_up("sentry", note="txc-up-sentry")
    ctx.check(case, bool(ctx.wait_follower_align(vf.SENTRY_PORT, anchor_dec, vf.TXC_ALIGN_S)),
              f"sentry (L2) did not align past {anchor_dec}",
              on_fail=lambda: ctx.overlay_dump_logs(vf.TXC_LOG_TAIL, "sentry"))
    _say(ctx, f"  sentry (L2) aligned with v0 past {anchor_dec}")

    # ── L3 downstream: cert-follow the SENTRY + devp2p pinned to ONLY the sentry ──────
    pk = ctx.enode_pubkey(sentry_rpc)
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
    ctx.cast_rpc_write("admin_addTrustedPeer", vf.downstream_enode(pk3), rpc_url=sentry_rpc,
                       note="txc-trust-l3-on-sentry")
    _say(ctx, "  sentry now trusts L3 (mutual) → awaiting L3 align via the sentry")
    ctx.check(case, bool(ctx.wait_follower_align(vf.DOWNSTREAM_PORT, anchor_dec, vf.TXC_ALIGN_S)),
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

    # The fail-loud monitor must not have cried wolf while the uplink was demonstrably healthy.
    ctx.check(case, *vf.evaluate_no_isolated_warning(ctx.overlay_logs("sentry", "downstream")))

    _ok(ctx, case, "tx submitted to L3 (reaches ONLY the sentry) relayed via devp2p tx-gossip to "
                   "a hidden validator, mined by the proposer, finalized, and synced back to L3 "
                   "with state applied.")


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
