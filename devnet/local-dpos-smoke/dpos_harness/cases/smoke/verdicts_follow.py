"""verdicts_follow.py — the PURE decision layer of the three FOLLOWER cases.

`case-cert-follow.sh`, `case-cert-cascade.sh`, `case-tx-cascade.sh`. Same split as
`verdicts.py` / `verdicts_fault.py`: `asserts_follow.py` decides what to read and when, this
module decides what the readings MEAN, and `tests/test_smoke_follow_verdicts.py` drives every
one of them through BOTH outcomes.

═══ WHY THE SPLIT MATTERS MORE HERE THAN ANYWHERE ELSE IN THE SUITE ══════════════════════

FOUR of these decisions are NEGATIVE — they pass when something does NOT happen:

  * `evaluate_tamper_no_progress` — a follower fed byte-flipped certificates must make ZERO
    finalized progress. It passes on `null` / `0x0`.
  * `evaluate_bogus_rejected` — a follower pointed at an unverifiable L1 trust root must refuse.
  * `evaluate_bogus_no_progress` — …and must not have finalized anything while refusing.
  * `evaluate_no_isolated_warning` — the tx-route monitor must NOT have cried ISOLATED while the
    uplink was healthy.

A negative assertion is the one shape that cannot be validated by a green live run: a healthy
chain satisfies it by doing nothing, which is indistinguishable from the check being broken,
absent, or pointed at the wrong node. `evaluate_tamper_no_progress` in particular passes on a
TIMEOUT — the case observes for a fixed window and concludes from the absence of movement — so
the only place its FAIL direction is ever executed is the unit suite. Every one of the four has
a test below that makes the forbidden thing HAPPEN.

The paired `v0 advanced` guard exists for the same reason and is not decoration: without it, a
tamper-follower that made no progress because THE WHOLE CHAIN was stalled would read exactly
like one that made no progress because it rejected every certificate.

Return convention: `(ok: bool, message: str)` — empty message on success, the bash diagnostic on
failure. Three functions return a third element (a NOTE, a hint, a witness) where bash prints one.
"""

from __future__ import annotations

from ...core import nodes, topology

# ══ smoke-cert-follow ═════════════════════════════════════════════════════════════════

#: `case-cert-follow.sh:24-25` — host ports of the two followers. The tamper follower publishes on
#: 38545 and the positive one on 28545; swapping them would make phase 3 read the HONEST node,
#: which of course makes no progress either once it is stopped — a negative assertion that passes
#: for the wrong reason is the failure mode this file is organised around.
CF_PORT = 28545
TAMPER_PORT = 38545

#: `:39`, `:57` — the follower alignment budget for phases 1 and 2.
CF_ALIGN_S = 180
#: `:52` — how long v0 gets to advance a full epoch past the stop height before the restart.
CF_GAP_ADVANCE_S = 120
#: `:47` — the graceful stop timeout for the follower (the same 40 s ceiling reth's
#: `on_graceful_shutdown` gets everywhere else in this tree).
CF_STOP_TIMEOUT_S = 40

#: `:68-75` — the MITM sidecar readiness budget and poll cadence. The sidecar `pip install`s
#: `websockets` at boot, so this window is a NETWORK budget, not a process-start one.
MITM_UP_S = 90
MITM_POLL_S = 3
#: `:70` — the line the proxy prints once it is listening.
MITM_READY_LINE = "cert-mitm: listening"

#: `:85` — the tamper OBSERVATION window. THIS IS THE ASSERTION, not a settle time: the case
#: concludes "the follower made zero progress" from the absence of movement across it, so
#: shortening it weakens the claim by exactly the amount shortened and nothing goes red.
TAMPER_OBSERVE_S = 45

#: `:98` — the driver lines that name the rejection. Best-effort observability only: the
#: finalized-progress gate above is what fails the case, and log capture is not guaranteed.
TAMPER_HINT_LINES = ("finalization cert FAILED BLS verification", "dropping mismatched cert")
#: `:97` — how deep the hint grep reads.
TAMPER_HINT_TAIL = 400

#: `:42`, `:61`, `:107` — fail-path log depth.
CF_LOG_TAIL = 200

#: `case-cert-follow.sh:93` — the two readings that mean "this node has finalized NOTHING". reth
#: leaves `finalized` unset (`null`) until something is finalized, and a node that only ever
#: cold-started reports genesis (`0x0`). Both are zero progress; neither is a hex height.
NO_PROGRESS_HEADS = ("null", "0x0")


def align_floor(height, interval) -> int:
    """`:38`, `:52` — `height + EPOCH_INTERVAL`: a floor at least one epoch boundary away.

    A floor of `height` alone would be satisfied by the follower reproducing the one block it
    already had; a whole epoch forces it through a committee handoff, which is where a follower
    that verifies certificates against a STALE committee stops being able to follow."""
    return int(height) + int(interval)


def mitm_ready(logs: str) -> bool:
    """`:70` — the proxy announced itself. Absence is a SKIP, never a failure: the sidecar needs
    a pip install from the network and a machine without one has not disproved anything."""
    return MITM_READY_LINE in (logs or "")


def evaluate_v0_advanced(before, after):
    """`:88` — the producer moved during the observation window.

    Without this, a chain-wide stall would present as a passing negative assertion: the tamper
    follower makes no progress either way, and the case would report that certificate
    verification is load-bearing on the strength of a dead devnet."""
    if int(after) > int(before):
        return True, ""
    return False, (f"v0 stalled during tamper phase ({before}→{after}); cannot attribute follower "
                   "stall to rejection")


def evaluate_tamper_no_progress(tamper_head):
    """`:91-108` — the negative. A follower fed only byte-flipped certificates must finalize
    NOTHING.

    Cold start is expected to succeed and is not a hole: the anchor is trust-blind and
    transitively authenticated, so the driver's LIVE-tail verify is what has to reject, and it
    does so by never advancing `finalized` past that anchor. Hence the two accepted readings —
    unset, or genesis — and hence the failure message says verification is not load-bearing
    rather than "the follower is behind"."""
    head = (tamper_head or "").strip()
    if head in NO_PROGRESS_HEADS:
        return True, ""
    return False, (f"tamper-follower advanced finalized to {head} despite byte-flipped certs — "
                   "verification is NOT load-bearing!")


def tamper_rejection_hint(logs: str) -> bool:
    """`:96-99` — did the driver's live-tail verify say so out loud? Observability, not a gate."""
    text = logs or ""
    return any(line in text for line in TAMPER_HINT_LINES)


# ══ smoke-cert-cascade ════════════════════════════════════════════════════════════════

#: `case-cert-cascade.sh:20-22` — the three followers' host ports.
T1_PORT = 28545
T2_PORT = 48545
BOGUS_PORT = 58545

#: `:59`, `:76` — the tier-1 and tier-2 alignment budgets. Longer than cert-follow's 180 s: tier 2
#: syncs through tier 1's window rather than off the producer.
CC_ALIGN_S = 240
#: `:94` — how long the bogus follower gets to refuse, and `:103` the poll cadence.
CC_REJECT_S = 240
CC_REJECT_POLL_S = 3
#: `:47`, `:92` — `wait_finalized_ge "$set_block"` with NO second argument, i.e. lib.sh:182's
#: `${2:-60}` default. Named here rather than left implicit because the bash reads as if it had no
#: budget at all, and a port that gave it a generous one would hide a chain that stopped
#: finalizing between the send and the follower start.
CC_FINALIZE_S = 60
#: `:62`, `:79`, `:107` — fail-path log depths (the bogus dump is deliberately shallower).
CC_LOG_TAIL = 200
CC_BOGUS_LOG_TAIL = 120

#: `:66` — the line tier 1 prints once it has verified the Rollup checkpoint against its own
#: synced chain. Alignment alone does NOT imply it ran: a follower with a misconfigured L1 URL
#: aligns perfectly off the cert feed, so this grep is the whole of the trust-root assertion.
L1_VERIFIED_LINE = "L1 Rollup checkpoint verified"
#: `:96` — the refusal.
BOGUS_REJECT_LINE = "NOT in the local chain"
#: `:88` — a checkpoint hash that exists nowhere in the chain.
BOGUS_CHECKPOINT_HASH = "0x" + "deadbeef" * 8
#: `:106` — the message when the refusal budget ran out. Named so the assertion's poll-expiry
#: branch and the verdict below cannot drift into saying two different things.
BOGUS_NOT_REFUSED = "bogus-checkpoint follower did not refuse"
#: `:40`, `:87` — MockRollup's setter, and the batch index the case writes.
SET_CHECKPOINT_SIG = "setCheckpoint(uint256,bytes32)"
CHECKPOINT_BATCH = 1
#: The compose variables the two MockRollup addresses are interpolated into
#: (`docker-compose.cert-cascade.yml:35,90`).
MOCK_ROLLUP_ENV = "MOCK_ROLLUP_ADDR"
BOGUS_ROLLUP_ENV = "BOGUS_ROLLUP_ADDR"


def evaluate_l1_checkpoint_verified(logs: str):
    """`:65-67` — tier 1 aligned AND the L1 checkpoint assert actually ran.

    The two are independent. `--cert-follow.l1-rpc-url` pointed at nothing still yields a follower
    that aligns off the certificate feed; only this line says the L1 trust root was consulted, so
    dropping it turns phase 1 into a second copy of cert-follow's phase 1."""
    if L1_VERIFIED_LINE in (logs or ""):
        return True, ""
    return False, "tier-1 aligned but the L1 checkpoint assert never ran"


def evaluate_bogus_rejected(logs: str, ps_state: str):
    """`:95-110` — the negative. Two witnesses of ONE refusal, and either is sufficient.

    The follower may print `NOT in the local chain` and stay up, or it may exit on the refusal
    before the log read catches it. Requiring the line alone would poll a dead container for the
    full budget and then report that it never refused — a false FAIL. Requiring `exited` alone
    would accept a container that died for any other reason, which is a false PASS."""
    if BOGUS_REJECT_LINE in (logs or ""):
        return True, "", "refusal logged"
    if (ps_state or "").strip() == "exited":
        return True, "", "exited on the refusal"
    return False, BOGUS_NOT_REFUSED, ""


def evaluate_bogus_no_progress(reading: str, anchor):
    """`:111-116` — …and it finalized nothing while refusing.

    Refusing to follow and then following anyway is the failure this second half exists for. The
    comparison is against the pre-case ANCHOR rather than against zero: the follower shares the
    devnet's genesis, so a fresh node legitimately reads `null`, and `> anchor` is what
    distinguishes "did not follow" from "followed"."""
    head = (reading or "null|null").split("|", 1)[0].strip()
    if head == "null":
        return True, ""
    if nodes.hex_to_dec(head) > int(anchor):
        return False, f"bogus-checkpoint follower made finalized progress ({reading})"
    return True, ""


def contract_address(receipt: dict, what: str):
    """The `contractAddress` of a `cast send --create … --json` receipt, FAIL-LOUD when absent.

    `case-cert-cascade.sh:37` pipes the JSON through python and would abort on a KeyError under
    `set -e`. Here a missing address would flow into `MOCK_ROLLUP_ADDR=None`, the compose file's
    `:-0x0…0` default would take over, and the case would then measure a follower pointed at the
    zero address — which fails in a way that reads like a product bug."""
    addr = (receipt or {}).get("contractAddress")
    if isinstance(addr, str) and addr.startswith("0x") and len(addr) == 42:
        return True, "", addr
    return False, f"{what} deploy returned no contractAddress (receipt={receipt})", ""


def receipt_block(receipt: dict, what: str):
    """The `blockNumber` of a send receipt as a decimal height, FAIL-LOUD when absent.

    It is the height the case then waits to see FINALIZED — the follower reads the Rollup at the
    `finalized` tag, so starting it before the checkpoint block is final measures a rollup that,
    from the follower's point of view, has no checkpoint yet."""
    blk = (receipt or {}).get("blockNumber")
    if blk is None:
        return False, f"{what} returned no blockNumber (receipt={receipt})", 0
    return True, "", nodes.hex_to_dec(blk)


# ══ smoke-tx-cascade ══════════════════════════════════════════════════════════════════

#: `case-tx-cascade.sh:20-23` — the two cascade tiers. NOTE the ports are CROSSED relative to the
#: tiers' depth: the L2 sentry publishes on 38545 and the L3 downstream on 28545.
SENTRY_PORT = 38545
DOWNSTREAM_PORT = 28545
SENTRY_IP = "172.20.0.30"
DOWNSTREAM_IP = "172.20.0.31"

#: `:26` — the allowance the L3-submitted contract call writes.
TXC_ALLOW = 4242
#: `:126` — 0.05 ETH, the value the L3-submitted transfer moves.
TXC_TRANSFER_WEI = 50_000_000_000_000_000

#: `:37`, `:69` — the alignment budget for each tier.
TXC_ALIGN_S = 200
#: `:60-64` — how many times the L3 enode read is retried while its RPC comes up, and the gap.
TXC_ENODE_TRIES = 30
TXC_ENODE_SLEEP_S = 2
#: `:110-114` — the per-tx receipt budget on the PRODUCER, one read per second.
TXC_RECEIPT_TRIES = 90
#: `:129-133` — the round-trip budget on L3. Longer than the producer's: it waits for the block to
#: be mined AND finalized AND synced back down two tiers.
TXC_L3_RECEIPT_TRIES = 120
TXC_RECEIPT_SLEEP_S = 1
#: `:122` — how long the tx block gets to finalize.
TXC_FINALIZE_S = 120
#: `:39`, `:71`, `:117` — fail-path log depths.
TXC_LOG_TAIL = 120
TXC_MINE_LOG_TAIL = 80

#: `:141` — the fail-loud monitor line that must NOT appear while the uplink is healthy.
ISOLATED_LINE = "tx-route ISOLATED"


def sentry_enode(pubkey: str) -> str:
    """`:50` — the sentry's enode rebuilt against its FIXED compose IP. `admin_nodeInfo`'s
    embedded address is unreliable inside docker, so only the pubkey comes from the node."""
    return topology.enode(pubkey, SENTRY_IP)


def downstream_enode(pubkey: str) -> str:
    """`:71` — the same, for L3's inbound trust on the sentry."""
    return topology.enode(pubkey, DOWNSTREAM_IP)


def evaluate_enode_pubkey(pubkey: str, what: str):
    """`:48`, `:66` — a 128-hex pubkey, or the case cannot build a trusted-peer URL at all.

    `core/rpc._enode_pubkey` already enforces the length, so this is the CALL SITE's fail-loud:
    an empty answer means the node's RPC is not up (or `admin` is not in its `--http.api`), and
    proceeding would write an enode of the literal form `enode://@172.20.0.30:30303` into the
    downstream's trusted-peers list, where it produces a peerless node and no error."""
    pk = (pubkey or "").strip()
    if len(pk) == 128 and all(c in "0123456789abcdefABCDEF" for c in pk):
        return True, ""
    return False, f"bad {what} enode pubkey '{pk[:20]}…'"


def evaluate_l3_peers(count):
    """`:80-83` — the L3 devp2p peer set. Returns `(ok, message, note)`.

    THE TWO HALVES ARE NOT THE SAME STRENGTH, and flattening them in either direction changes
    what the case asserts:

      * `>= 1` is the HARD gate. Zero peers means the tx uplink is absent, so the write-path
        assertions below would be measuring nothing — a tx submitted to an isolated L3 sits in
        its local pool forever and the case would fail later with an unrelated message.
      * `== 1` is a NOTE ONLY, exactly as bash prints it. The privacy invariant — L3 reaches
        nothing but the sentry — is enforced STRUCTURALLY by the compose file (`--trusted-only`,
        `--disable-discovery`, and a trusted-peers list holding one enode), not by this count.
        Promoting it to a failure would make the case flaky on a transient second connection
        without adding an assertion the topology does not already guarantee; demoting the `>= 1`
        half to a note would delete the only gate here.
    """
    n = int(count)
    if n < 1:
        return False, "L3 has NO devp2p peer — the tx uplink is absent", ""
    note = "" if n == 1 else f"L3 reports {n} devp2p peers (expected 1 = sentry only)"
    return True, "", note


def peer_count_from_rpc(raw) -> int:
    """`:80-81` — `cast rpc net_peerCount` -> int. The reply is a QUOTED hex string (`"0x1"`), so
    bash strips the quotes before `printf '%d'`; an unreadable answer is 0, which the gate above
    then rejects rather than treating as data."""
    tok = (str(raw) or "").strip().strip('"').strip()
    if not tok.startswith("0x"):
        return 0
    return nodes.hex_to_dec(tok)


def evaluate_tx_mined(txhash: str, status):
    """`:115-117` — the receipt appeared ON THE PRODUCER.

    That is the proof the case is named for and the node it is read from is the assertion: a
    transaction can only enter the canonical chain through a committee proposer's pool, and L3
    can reach nothing but the sentry, so a receipt on validator-0 means the devp2p tx-gossip
    relay L3→L2→validator worked. Reading it back off L3 instead would prove only that L3 has a
    mempool."""
    if str(status) in ("0x1", "1"):
        return True, ""
    return False, (f"tx {txhash} not mined by a validator (status='{status}') — devp2p tx-gossip "
                   "relay L3→L2→validator failed")


def evaluate_l3_synced_receipt(txhash: str, status):
    """`:133` — the OTHER direction: L3 synced the mined+finalized block back and serves it."""
    if str(status) in ("0x1", "1"):
        return True, ""
    return False, f"L3 never synced the receipt for {txhash}"


def evaluate_l3_state(delta, allowance):
    """`:136-140` — L3 applied the STATE, not just the block.

    Both halves, as in `smoke-tx`: the balance delta proves the value transfer applied on L3's own
    copy of the chain, the allowance proves it executed the CALL and the SSTORE. A follower that
    stored the block without executing it passes the receipt check and fails these."""
    if int(delta) != int(TXC_TRANSFER_WEI):
        return False, f"L3 balance delta {delta} != 0.05 ETH"
    if str(allowance) != str(TXC_ALLOW):
        return False, f"L3 allowance={allowance} != {TXC_ALLOW} (EVM SSTORE not synced)"
    return True, ""


def evaluate_no_isolated_warning(logs: str):
    """`:141-143` — the negative. The node's own tx-route monitor must not have warned ISOLATED
    while both tiers were demonstrably connected.

    This is a check on the MONITOR, not on the cascade: everything above already proved the route
    works, so a warning here is a false positive in the fail-loud path — which is worse than
    useless, because an operator who learns to ignore it will ignore the true one too."""
    if ISOLATED_LINE in (logs or ""):
        return False, ("tx-route monitor warned ISOLATED while peers were connected "
                       "(false positive)")
    return True, ""
