"""verdicts_follow.py — the PURE decision layer of the FOLLOWER cases.

`case-cert-follow.sh`, `case-cert-cascade.sh`, `case-tx-cascade.sh`, plus `smoke-cert-keyless`,
which has no bash ancestor (FLU-1202). Same split as
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
  * `evaluate_repair_did_not_deliver_the_key` — the below-frontier repair sweep must not have
    reached the epoch BEFORE the node adopted its own key.

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
#: The overlay SERVICES the harness reads. Service names, not host ports: every follower read
#: goes in-container through `SmokeCtx.overlay_*` (`driver.py`), so the harness needs no published
#: port at all. See `docker-compose.cert-follow.yml` for the publishing rule.
CF_SERVICE = "cert-follower"
TAMPER_SERVICE = "cert-follower-tamper"

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

#: The lines the follower writes when it refuses a tampered certificate. EITHER satisfies the
#: gate, because the cold-start `get_latest` pull and the live subscription stream are different
#: call sites and which one sees the first bad certificate depends on start-up ordering:
#:   * `upstream.rs:308` — the subscription path (`decode_finalized`);
#:   * `upstream.rs:324` — the by-height / cold-start pull path (`fetch_finalization`).
#:
#: THE REJECTION HAPPENS AT DECODE, NOT AT VERIFY, and this constant got that wrong once. The
#: earlier value `cert-inlet: BLS verify FAILED` (`cert_inlet.rs:813`) is a real string on a real
#: path — `launch_follower` does spawn `CertInlet` (`dpos.rs:3727,3750,3781`) — but it is
#: unreachable under THIS tamper. `CombinedCertificate` is `vote ‖ seed_flag ‖ seed_slot`, and the
#: trailing 48 bytes are the beacon seed slot, a compressed G1 point, NOT the multisig aggregate
#: (`combined_scheme.rs:146-156`). The MITM flips a nibble 4 bytes from the end
#: (`scripts/cert-mitm-proxy.py:38`), landing inside that point; `read_seed_slot` decodes it
#: eagerly and `BlsSignature::read` does uncompress plus a subgroup check, so the flip fails
#: DECODE with probability ~1. `into_parts()` returns `Err` and the certificate never reaches
#: `CertInlet::ingest`.
#:
#: Before these two, the grep was for "finalization cert FAILED BLS verification" and "dropping
#: mismatched cert" — neither of which appears anywhere in `crates/`. That is three wrong strings
#: on one assertion, which is why the live run is the only thing that closes this.
TAMPER_REJECT_LINES = ("cert-follow: discarding malformed finalized event",
                       "cert-follow: malformed getFinalization response")
#: `:97` — how deep the rejection grep reads.
TAMPER_HINT_TAIL = 400

#: How long the tamper follower has to answer `eth_blockNumber` before the observation window.
TAMPER_UP_S = 60

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


def evaluate_follower_ingested(before, after, service):
    """THE CONTROL PHASE 4a WAS MISSING: the FOLLOWER itself finalized new blocks over the window
    in which its vote-only admissions stayed flat.

    `evaluate_v0_advanced` is the chain-side control and phases 3 and 4b both take it, but on its
    own it is not enough here. The reading being defended is a counter on the FOLLOWER, and the
    counter only moves when the follower ADMITS a certificate — so a follower whose WS upstream
    died, or whose `CertInlet` stopped ingesting, produces exactly the flat counter the phase
    reports as "it stopped taking vote-only admissions", while v0 advances happily throughout.
    That is the same false-negative shape the module header names for phases 3 and 4b: an absence
    is also what a subsystem that never ran produces.

    The follower's own finalized height moving is the tightest available proof that certificates
    were delivered AND accepted during the window, which is what makes the flat counter mean
    "accepted with `PK_epoch` resolved" rather than "accepted nothing"."""
    if int(after) > int(before):
        return True, ""
    return False, (f"{service} finalized nothing over the vote-only window ({before}→{after}) — "
                   f"the flat {CF_VOTE_ONLY_FAMILY} below would be the reading of a follower that "
                   "stopped ingesting certificates altogether, not of one that admits them with "
                   "PK_epoch in its key store")


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


def evaluate_tamper_alive(alive):
    """The tamper follower is UP before the observation window opens.

    Without it the phase concludes "it verified and rejected" from a node that never started. Zero
    finalized progress is the expected reading either way, and `"null"` is a legitimate PASS here
    (reth leaves `finalized` unset until something finalizes), so the negative cannot be made
    self-witnessing — it needs a separate liveness reading, and a live node answers
    `eth_blockNumber` with a real height even while `finalized` is unset."""
    if alive:
        return True, ""
    return False, ("cert-follower-tamper never answered eth_blockNumber — it is not up, so its "
                   "zero finalized progress is not evidence that verification rejected anything")


def evaluate_tamper_rejected(logs: str):
    """The POSITIVE witness: the follower SAID it refused. A gate now, not a hint.

    Zero progress alone cannot separate "the certificates were refused" from "they were never
    delivered" — a MITM that came up but forwarded nothing produces the identical reading.

    EITHER line counts: see `TAMPER_REJECT_LINES` on why there are two. The failure names both, so
    a run that refused on the path this check did not expect reads as an expectation mismatch
    rather than as a follower that accepted a forged certificate.

    RETURNS THE LINE IT MATCHED, third element, the shape `evaluate_bogus_rejected` uses. The two
    lines are two different code PATHS — the live subscription (`upstream.rs:308`) and the
    cold-start / by-height pull (`upstream.rs:324`) — and which one sees the first bad certificate
    depends on start-up ordering. The case tears its stack down when it finishes, so the container
    log is gone before anyone can read it afterwards; if the transcript does not say which path
    fired, nothing does."""
    text = logs or ""
    for line in TAMPER_REJECT_LINES:
        if line in text:
            return True, "", line
    return False, ("cert-follower-tamper logged none of " + repr(TAMPER_REJECT_LINES) +
                   " — zero finalized progress alone does not show the certificates were "
                   "REJECTED rather than never delivered"), ""


# ══ smoke-cert-follow phase 4: the follower obtains PK_epoch (FLU-1167) ═══════════════

#: The phase-4 services (`docker-compose.cert-follow.yml`). The proxy relays VERBATIM until it is
#: armed; the follower behind it therefore follows normally first, which is what lets it obtain
#: the key at all.
SEED_MITM_SERVICE = "cert-mitm-seed"
SEED_TAMPER_SERVICE = "cert-follower-seed"
#: The file whose EXISTENCE arms the seed-slot clearing, on the proxy's tmpfs.
SEED_ARM_FILE = "/armed/arm"

#: `beacon/follower.rs` — the INFO a follower writes ONCE per epoch key it adopted, after
#: verifying the upstream's artifact against `committee[epoch]` read off its OWN chain state.
#:
#: THIS IS THE ADOPTION WITNESS, and it stays the load-bearing one. It is a POSITIVE witness
#: rather than an absence: the follower prints it only after `verify_artifact_for_epoch` accepted
#: the upstream's artifact against the committee this node read from its OWN chain state, so a
#: lying upstream cannot produce it.
#:
#: `CF_ADOPTED_FAMILY` below counts the same event and is read BESIDE it, not instead of it. The
#: counter lives on the COMMONWARE registry, which a `--cert-follow` node serves only under a
#: devnet build plus a `--dpos.metrics-port` the compose overlay has to pass; the log line needs
#: neither. A witness with two devnet preconditions does not get to be the one a verdict rests on.
CF_KEY_LINE = "cert-follow: PK_epoch obtained and verified against committee[epoch]"

#: `beacon/metrics.rs` — the SAME adoption event as `CF_KEY_LINE`, counted. Registered under this
#: name; the scrape line doubles the suffix (`nodes.counter_sample`), which is why no caller
#: hand-writes the sample name.
#:
#: It is read as CORROBORATION, and the split between the two witnesses is the point: the log line
#: says the follower's own beacon announced the adoption, the counter says the family that only a
#: follower registers moved on the endpoint that only a follower now serves. The second is what
#: went from unscrapeable to scrapeable when `spawn_devnet_metrics` moved into `run_node_stack`,
#: ahead of the validator/follower branch, and the compose overlay caught up.
CF_ADOPTED_FAMILY = "dpos_follower_artifact_adopted_total"
#: Its sibling — the upstream had nothing to give. Printed, never asserted: a miss before the
#: first hit is ordinary (the follower can ask while its own executor is still short of the block
#: that committed `committee[epoch]`), so a bound on it would be a bound on timing, not on
#: correctness.
CF_MISS_FAMILY = "dpos_follower_artifact_miss_total"

#: `cert_inlet.rs:815` — incremented for every certificate of a beacon-active epoch this node
#: processes while it holds NO key for that epoch, i.e. with the multisig quorum checked and the
#: seed slot not. The acceptance criterion for FLU-1167 is that this STOPS moving once the key
#: lands; `smoke-cert-keyless` below additionally requires that it MOVED first.
#:
#: RE-KEYED BY FLU-1202 and the new key is the one that makes the precondition below sound. It
#: used to fire on "the scheme carries no `cert_seed_pin`"; a scheme holds no key material any
#: more, so it now fires on `!key_known` — the answer of the acquisition ladder for THIS
#: certificate's epoch. Nothing but a genuinely keyless epoch can move it.
#:
#: IT IS INCREMENTED BEFORE `finalization.verify`, not after, so a tick is "a certificate of a
#: keyless beacon-active epoch reached the inlet", which is a superset of "…and was accepted".
#: That direction is the harmless one for both readings here: as a PRECONDITION it is what is
#: wanted, and as the post-key FLATNESS reading it is paired with the follower's own finalized
#: progress, which no unaccepted certificate can produce.
CF_VOTE_ONLY_FAMILY = "dpos_cert_vote_only_admissions_total"

#: How long the follower gets to obtain `PK_epoch` after it has aligned. Generous: the fetch is
#: off-path, rate-limited to one upstream round-trip per epoch per 5 s, and the follower asks
#: only for the epoch that MINTED the key a live epoch verifies under — a stable epoch mints
#: nothing, so on a chain sitting inside a long stable epoch the ask is for an older epoch and the
#: upstream must still hold it.
CF_KEY_S = 180
#: Slower than the other polls on purpose: each iteration reads a follower's WHOLE log (see
#: `SEED_LOG_TAIL`), which is megabytes by phase 4.
CF_KEY_POLL_S = 5
#: The window over which the vote-only counter must not move. Same shape as `TAMPER_OBSERVE_S`
#: and the same warning applies: THE WINDOW IS THE ASSERTION. At ~1 blk/s a KEYLESS follower
#: takes one vote-only admission per second, so 30 s of flatness is 30 admissions that did not
#: happen.
CF_VOTE_ONLY_WINDOW_S = 30

#: `scripts/cert-mitm-proxy.py` — the proxy's own three witnesses. It counts what it rewrote and
#: prints the first rewrite's before/after, which is what makes "the tamper landed" an assertion
#: rather than an assumption (the `tear_journal_to_torn` readback rule, applied to a proxy).
SEED_ARMED_LINE = "cert-mitm: ARMED"
SEED_CLEARED_LINE = "cert-mitm: seed slot CLEARED"
#: How long the proxy gets to notice the arm file and clear its first certificate.
SEED_ARM_S = 90
SEED_ARM_POLL_S = 3
#: The post-arm observation window. Again: THE WINDOW IS THE ASSERTION.
SEED_OBSERVE_S = 45
#: The fewest refusals the window must contain. At ~1 certificate per second across
#: `SEED_OBSERVE_S` the live runs show forty-odd; this floor is low enough that a slow host cannot
#: trip it and high enough that a stream which simply stopped cannot pass. It replaces a
#: "finalized did not advance" reading that measured the EL transport rather than the cert check —
#: see `evaluate_seed_tamper_refused_every_cert`.
MIN_SEED_REJECTS = 5

#: `cert_inlet.rs` — the refusal a BEACON-ACTIVE scheme writes when a certificate's seed slot
#: does not verify (here: is absent on a beacon-active epoch, the `None => false` arm of
#: `combined_scheme.rs`'s `verify_certificate`). "Beacon-active" and not "keyed": the arm is
#: reached on any scheme carrying an ORACLE, which `Randomness::oracle_for` attaches whether or
#: not `PK_epoch` resolves — see `evaluate_seed_vote_only_flat` for the half that does
#: discriminate.
#:
#: NOTE how this differs from `TAMPER_REJECT_LINES`, and why the two phases cannot share a
#: constant: phase 3's nibble flip breaks the G1 point so the certificate fails DECODE and never
#: reaches `CertInlet::ingest` at all. Phase 4's cleared slot decodes perfectly and fails at
#: VERIFY, which is the only arm that can prove the follower is using `PK_epoch`.
SEED_REJECT_LINE = "cert-inlet: BLS verify FAILED"
#: How deep the phase-4 greps read — `None` is the WHOLE log, and it has to be.
#:
#: `CF_KEY_LINE` is written ONCE, in the follower's first seconds. Both followers now run for the
#: better part of ten minutes before phase 4b reads them, at roughly six log lines per block, so
#: any bounded tail scrolls that line away long before it is looked for. A 400-line tail turned a
#: healthy, keyed, block-deriving follower into "it did not obtain PK_epoch" on a live run — a
#: presence grep over a growing log cannot be depth-limited.
SEED_LOG_TAIL = None
#: …and how deep the REFUSAL COUNT reads. BOUNDED, unlike the presence grep above, and for the
#: opposite reason: it is a DELTA across one window, both of whose ends are recent — the baseline
#: is taken at the arming and the refusals arrive after it. A bound keeps the read off a
#: ten-minute log while still covering far more than the window can contain (~1 refusal per second
#: over `SEED_OBSERVE_S`).
SEED_COUNT_TAIL = 4000


def evaluate_key_obtained(logs: str, service: str):
    """The POSITIVE half of FLU-1167: the follower says it holds `PK_epoch`.

    A POSITIVE witness rather than an absence: the follower prints it only after
    `verify_artifact_for_epoch` accepted the upstream's artifact against the committee this node
    read from its own chain state, so a lying upstream cannot produce it. `evaluate_artifact_
    adopted_counter` below reads the same event off the follower's commonware registry; see
    `CF_KEY_LINE` for why this one, and not that one, is the witness the verdict rests on.

    Returns the matched line third, the shape `evaluate_tamper_rejected` uses, because the case
    tears its stack down and the container log is gone before anyone can read it afterwards."""
    for line in (logs or "").splitlines():
        if CF_KEY_LINE in line:
            return True, "", line.strip()
    return False, (f"{service} never logged {CF_KEY_LINE!r} — it did not obtain PK_epoch over its "
                   "cert upstream, so its certificates are still taking vote-only admission "
                   "(FLU-1167)"), ""


def evaluate_artifact_adopted_counter(text: str, service: str):
    """…and the SECOND witness to the same adoption, off a different endpoint.

    Added because the log line, on its own, is a claim about one `info!` in one function. The
    counter is incremented on the line above that `info!` and rendered by a registry the follower
    only serves at all under `--dpos.metrics-port` — so agreement between them says the adoption
    path ran end to end on a node whose beacon really is a follower beacon, not merely that a
    string was printed. It does NOT replace the log check: the endpoint is devnet-only, and a
    verdict that can be silenced by a missing build feature is not one to hang FLU-1167 on.

    An UNREAD scrape fails, for `evaluate_vote_only_flat`'s reason and with the opposite polarity
    to that one's trap: here `""` from an empty scrape and `0` from a real zero would both read as
    "not adopted", and only one of them is evidence. So the emptiness of the whole text is checked
    first and reported as its own failure.

    Returns the two counts third, as a printable pair — the stack is torn down immediately after
    and the endpoint is gone with it."""
    if not (text or "").strip():
        return False, (f"{service}'s commonware registry (:{topology.CONSENSUS_METRICS_PORT}) did "
                       f"not answer — {CF_ADOPTED_FAMILY} was never read, so it is no witness "
                       "either way (is `--dpos.metrics-port` on the follower, and is the binary "
                       "built with `dpos-devnet-metrics`?)"), ""
    adopted = nodes.gauge_val(text, nodes.counter_sample(CF_ADOPTED_FAMILY))
    miss = nodes.gauge_val(text, nodes.counter_sample(CF_MISS_FAMILY))
    pair = f"{CF_ADOPTED_FAMILY}={adopted or '<absent>'} {CF_MISS_FAMILY}={miss or '<absent>'}"
    if not adopted:
        return False, (f"{service} answered on the commonware registry but carries no "
                       f"{CF_ADOPTED_FAMILY} sample — the follower beacon never registered its "
                       f"families, so nothing on this node adopted an epoch artifact ({pair})"), ""
    if _counter(adopted) < 1:
        return False, (f"{service} logged the adoption but {CF_ADOPTED_FAMILY} is still 0 — the "
                       f"log line and the counter disagree about the same event ({pair})"), ""
    return True, "", pair


def evaluate_vote_only_flat(before, after, scrape_ok: bool, window=CF_VOTE_ONLY_WINDOW_S):
    """…and the consequence: vote-only admissions STOP.

    Three-valued on purpose, and the third value is the trap. `""` from the scrape means EITHER
    "metrics-rs never registered this family because it was never incremented" (a real zero, and a
    pass) OR "the endpoint did not answer" (nothing was measured, and a pass would be a lie). The
    caller reads the whole scrape once to tell them apart and passes the verdict here as
    `scrape_ok`; an unread endpoint fails.

    The bound is `after == before`, not `after <= before + slack`: the counter is monotone and the
    claim is that the epoch left vote-only admission, which is exact. At ~1 blk/s a KEYLESS
    follower takes one admission per second, so a single increment across the window is a follower
    still verifying blind."""
    if not scrape_ok:
        return False, ("the follower's reth metrics endpoint did not answer — "
                       f"{CF_VOTE_ONLY_FAMILY} was never read, so its flatness is not evidence "
                       "that vote-only admission stopped (is `--metrics` set on the follower?)")
    b, a = _counter(before), _counter(after)
    if a == b:
        return True, ""
    return False, (f"{CF_VOTE_ONLY_FAMILY} grew {b} -> {a} over {window}s AFTER the follower "
                   "obtained PK_epoch — certificates are still being admitted with the seed slot "
                   "unchecked, so the key never reached the store the inlet reads "
                   "(cert_inlet.rs `ensure_key` -> `BeaconKeys`, which is where the epoch's "
                   "oracle looks it up)")


def _counter(raw) -> int:
    """A metrics-rs counter sample as an int; an absent family (`""`) is a real ZERO.

    Absent-means-zero is safe only where the caller has ALREADY established that the scrape itself
    answered — `evaluate_vote_only_flat`, `evaluate_seed_vote_only_flat` and
    `evaluate_entered_keyless` each ask that question first, and `vote_only_admissions` says so in
    its own docstring. The same coercion applied to an unread endpoint is what makes a
    flat-counter verdict pass forever."""
    text = str(raw or "").strip()
    if not text:
        return 0
    try:
        return int(float(text))
    except ValueError:
        return 0


def evaluate_seed_follower_caught_up(aligned, floor):
    """THE GATE THAT SEPARATES A REAL NEGATIVE FROM A GREEN-LOOKING ONE, and it is not the key
    gate. Holding `PK_epoch` says the follower CAN reject; being caught up says a rejection is the
    only thing that can stop it.

    A follower started last, several hundred blocks behind, back-fills — and certificates that
    reached it BEFORE the proxy was armed are ones the proxy could not have touched. It keeps
    applying those for as long as the backlog lasts, which looks exactly like a follower that
    ignored the seed slot. Live evidence, same build and same proxy, minutes apart:

      * armed ~370 blocks behind → advanced 229 → 268 across the window, phase read it as
        "the seed slot is not being checked";
      * armed caught up          → froze on the spot at 124, one `cert-inlet: BLS verify FAILED`
        per arriving height, `dpos_cert_vote_only_admissions_total` flat at 1.

    So the backlog, not the product, produced the failure — and without this gate the phase's
    verdict is a function of how long the three preceding phases happened to take."""
    if aligned:
        return True, ""
    return False, (f"cert-follower-seed did not catch up with v0 past {floor} before the arming — "
                   "it would then advance on certificates delivered BEFORE the proxy was armed, "
                   "which the proxy never touched, and the negative below would read a backlog "
                   "as a follower that ignores the seed slot")


def evaluate_seed_tamper_landed(mitm_logs: str):
    """THE TAMPER MUST WITNESS ITS OWN TAMPERING before anything concludes from a rejection.

    Same rule as `tear_journal_to_torn`'s readback: a proxy that cleared nothing produces exactly
    the reading a follower that rejected nothing produces — a still follower and a quiet log — and
    the case would report that the seed check is load-bearing on the strength of an untouched
    certificate stream. The proxy therefore re-slices the string it is about to send and prints
    the first rewrite's before/after; this is that print."""
    text = mitm_logs or ""
    if SEED_ARMED_LINE not in text:
        return False, (f"{SEED_MITM_SERVICE} never logged {SEED_ARMED_LINE!r} — the arm file did "
                       "not reach it, so it relayed every certificate verbatim and the negative "
                       "below would pass over an untampered stream")
    if SEED_CLEARED_LINE not in text:
        return False, (f"{SEED_MITM_SERVICE} armed but never logged {SEED_CLEARED_LINE!r} — it "
                       "cleared NO seed slot (every certificate was already seedless, or the "
                       "trailing flag+slot offsets no longer match the wire format)")
    return True, ""


def seed_reject_count(logs: str) -> int:
    """How many certificates this follower refused at the cert inlet's BLS verify."""
    return sum(1 for ln in (logs or "").splitlines() if SEED_REJECT_LINE in ln)


def evaluate_seed_tamper_refused_every_cert(before, after, want=MIN_SEED_REJECTS,
                                            window=SEED_OBSERVE_S):
    """The negative, measured on the REFUSALS themselves and not on the follower's height.

    WHY NOT THE HEIGHT, WHICH IS WHAT THIS PHASE ORIGINALLY MEASURED. On this stand a follower's
    `finalized` has a SECOND source, and a poisoned certificate stream is exactly what turns it
    on. Live, with every certificate correctly refused:

        cert-inlet: BLS verify FAILED; skipping …            (×N, one per delivered cert)
        cert-inlet: 3 consecutive upstream data faults; rotating to the next configured upstream
        cold-start jump: EL-sync fast-forwarded the anchor from=241 to=271 floor=268
        steady-state re-jump landed; re-seeding executor + marshal floor landing_h=271 floor=268

    The follower rejects every tampered certificate, counts the rejections as upstream data
    faults, rotates (to its single configured upstream), and meanwhile its EL keeps syncing blocks
    over devp2p from the validator named in `--trusted-peers`. The steady-state re-jump then
    fast-forwards the anchor onto that EL-synced tip, so `finalized` advances 238 → 271 without a
    single certificate having been accepted. Reading that as "the seed slot is not being checked"
    is reading the EL transport and calling it the cert check.

    Phase 3 gets away with the height reading only because ITS follower never had a valid chain to
    jump onto — it sits at the anchor with `finalized` unset, so there is nothing to fast-forward
    to. That is a property of phase 3's setup, not a property of followers.

    So the refusals are the measurement. A GROWING count over the window proves two things at
    once that the height proved neither of: certificates were still being delivered, and every one
    of them was refused. Paired with `evaluate_seed_vote_only_flat` below — which proves the
    follower HELD `PK_epoch` while it refused them, the half the refusal count itself no longer
    carries — it is the whole property."""
    b, a = int(before), int(after)
    if a - b >= int(want):
        return True, ""
    return False, (f"cert-follower-seed refused only {a - b} certificates over {window}s "
                   f"(want >= {want}) — with every certificate carrying a CLEARED seed slot it "
                   "should refuse each one it is handed, so either the stream stopped (nothing "
                   "was delivered, and the negative proves nothing) or the seed slot is not being "
                   "checked and a tampered seed rides a valid quorum silently")


def evaluate_seed_vote_only_flat(before, after, scrape_ok: bool, window=SEED_OBSERVE_S):
    """…and the follower held `PK_epoch` while it refused them.

    THIS IS THE HALF THAT MAKES THE REFUSAL COUNT MEAN THE RIGHT THING, AND AFTER FLU-1202 IT IS
    THE *ONLY* HALF THAT DOES. Read the two halves separately:

      * the REFUSALS say the seed arm is reachable and enforced. They no longer say the follower
        holds `PK_epoch`. `verify_certificate` reaches `match certificate.seed { .. None => false }`
        on any scheme carrying an ORACLE, and `Randomness::oracle_for` attaches one for every
        beacon-active epoch whether or not the key resolves (`beacon/follower.rs:355-367` —
        `mandatory_at(epoch).then(..)`). A KEYLESS follower refuses a cleared slot too.
      * this COUNTER is what separates the two states. `cert_inlet.rs:815` increments it under
        `!key_known` and does so BEFORE `finalization.verify` runs (`:818`), so a keyless follower
        ticks it once per cleared certificate ON ITS WAY to refusing that certificate. Flat across
        the window therefore means "this follower resolved the epoch key for every certificate it
        was handed" — which is exactly what makes the refusals attributable to `PK_epoch` rather
        than to the mere presence of an oracle.

    So the pair still discriminates keyed from keyless; the member that carries the discrimination
    moved from the refusals to this counter. Before FLU-1202 the refusals carried it, because the
    `None => false` arm was reachable only on a scheme holding `cert_seed_pin`.

    Three-valued for the same reason `evaluate_vote_only_flat` is: an empty scrape is an unread
    endpoint, not a zero, and must never satisfy this."""
    if not scrape_ok:
        return False, ("cert-follower-seed's reth metrics endpoint did not answer — "
                       f"{CF_VOTE_ONLY_FAMILY} was never read, so nothing rules out that the "
                       "cleared certificates were admitted vote-only rather than refused")
    b, a = _counter(before), _counter(after)
    if a == b:
        return True, ""
    return False, (f"{CF_VOTE_ONLY_FAMILY} grew {b} -> {a} over {window}s — cert-follower-seed "
                   "processed cleared-seed certificates while holding NO key for their epoch, so "
                   "its refusals are the beacon-active-epoch arm firing and not a PK_epoch check; "
                   "the keyed/keyless distinction this phase claims is unwitnessed")


# ══ smoke-cert-keyless (FLU-1202) ═════════════════════════════════════════════════════

#: `epoch_manager.rs:1221` — the ONE line the below-frontier repair sweep writes when it is what
#: made a late key take effect. It is the whole observable surface of `repair_keyless_schemes`:
#: the sweep's only remaining effect is this `hint_finalized` re-drive, and this `info!` is
#: emitted immediately before it, once per epoch it re-drives (`hinted` is the sweep's own memo).
#:
#: THE CASE ASSERTS ITS ORDER, NOT ITS ABSENCE, and the first live run is why. An earlier version
#: of `smoke-cert-keyless` required this line never to appear; it went red on a run whose product
#: behaviour was entirely correct, and the failure was the assertion's:
#:
#:   11:52:56  cert-follow: PK_epoch obtained and verified against committee[epoch] epoch=2
#:   11:53:28  below-frontier epoch obtained a late beacon key … epoch=Epoch(2) boundary=159
#:
#: The sweep walks the registered epochs BELOW the frontier whose key store no longer misses, so
#: an epoch whose key landed and which then crossed below the frontier is exactly what it is built
#: to find. It fires here as a CONSEQUENCE of the event under test, thirty-two seconds after the
#: node had already adopted the key by its own artifact fetch. Requiring its absence was requiring
#: that a routine downstream effect of the tested event not happen.
#:
#: What still has to be ruled out is the sweep DELIVERING the key — that is the reading which
#: would mean the case measured the repair path instead of the oracle — and the ordering is what
#: rules it out. See `evaluate_repair_did_not_deliver_the_key`.
#:
#: THE FOLLOWER DOES RUN THE SWEEP. A `--cert-follow` node has an `EpochManager` (`dpos.rs`'s
#: `launch_follower` registers `EpochEngineMetrics` and derives its own boundary delivery so
#: `soft_enter` registers the current epoch), so this reading is a live discriminator on this node
#: and not a structural tautology. `crates/node/src/cert_follow/mod.rs` says "NO DkgActor, NO
#: beacon oracle, NO signer" and says nothing about the epoch manager; reading that list as
#: exhaustive is what produced the wrong assertion above.
#:
#: Read as a whole-log grep with a `\`-continuation in the Rust source, so the constant stops at
#: the em-dash: the rendered message continues "— re-driving its finalization fetch", but matching
#: only the stable prefix keeps a re-wrap of that string from silencing the reading.
KEYLESS_REPAIR_LINE = "below-frontier epoch obtained a late beacon key"

#: How long the follower gets to take its FIRST keyless admission, and the poll cadence.
#:
#: Generous rather than tight, and it is NOT the assertion: this poll is waiting for the
#: precondition to become OBSERVABLE, not measuring how fast it does. The event itself is
#: structural — a follower obtains `PK_epoch` only through `observe_cert`, which fires on a
#: certificate that already went through the inlet, so the first certificate of a beacon-active
#: epoch is admitted keyless by construction and the counter is at >= 1 before the key can land.
#: The budget covers the bring-up tail: the follower has to boot, connect its upstream, and reach
#: a beacon-active epoch's certificate.
KEYLESS_ADMISSION_S = 120
KEYLESS_ADMISSION_POLL_S = 3


def vote_only_admissions(text: str) -> int:
    """`CF_VOTE_ONLY_FAMILY` off a WHOLE reth scrape, as an int. Absent family ⇒ 0.

    SUBSTRING (`nodes.metric_val`) and not `gauge_val`'s anchored bare-name match, for the reason
    `SmokeCtx.overlay_el_metrics_text` records: metrics-rs renders a labelled counter as
    `name{...}` and the anchored matcher looks at the whole first field.

    Absent-means-zero is safe only because every caller establishes separately that the SCRAPE
    answered — `evaluate_entered_keyless` and `evaluate_vote_only_flat` both take the emptiness of
    the whole text as their first question. Used on its own, this coerces an unread endpoint to a
    reading, which is the exact shape §B of the smoke audit is about."""
    return _counter(nodes.metric_val(text or "", CF_VOTE_ONLY_FAMILY, ""))


def evaluate_entered_keyless(text: str, service: str):
    """THE PRECONDITION OF `smoke-cert-keyless`, and the reason the case is not vacuous.

    Everything the case concludes afterwards — the key landed, admissions went flat, no repair
    path ran — is satisfied just as well by a node that held the epoch key from its first
    certificate and never spent a moment keyless. Such a node proves nothing about FLU-1202: the
    behaviour under test is what happens to an ALREADY-BUILT scheme when its epoch's key shows up
    later, and a node that was never keyless never had one.

    `dpos_cert_vote_only_admissions_total >= 1` is the witness, and it is exact rather than
    circumstantial. `cert_inlet.rs:815` increments it under `!key_known && mandatory_at(epoch)`,
    where `key_known` is the answer of `ensure_key(epoch, Local)` for THIS certificate's epoch —
    so a non-zero count is the node's own statement that it processed a certificate of a
    beacon-active epoch for which it could not resolve a key. The counter is monotone, so WHEN it
    is read does not matter; what it says about the past is fixed at the moment of the increment.

    An UNREAD scrape FAILS rather than reading as zero. Here the two are opposite verdicts of the
    same shape as `evaluate_artifact_adopted_counter`'s: a real zero means "this node was never
    keyless — the case tested nothing", an empty scrape means "nothing was measured", and only the
    first is a statement about the product.

    Returns the count third, printed by the case: it is the SIZE of the keyless window, which is
    the one number a reader wants when the flatness assertion below goes red."""
    if not (text or "").strip():
        return False, (f"{service}'s reth metrics endpoint did not answer — "
                       f"{CF_VOTE_ONLY_FAMILY} was never read, so nothing establishes that this "
                       "node ever entered a beacon-active epoch without its key (is `--metrics` "
                       "set on the follower?)"), 0
    n = vote_only_admissions(text)
    if n < 1:
        return False, (f"{service} answered on the reth recorder but {CF_VOTE_ONLY_FAMILY} is 0 — "
                       "it never took a single admission without the epoch key, so the KEYLESS "
                       "WINDOW this case exists to observe never happened and everything below "
                       "would pass on a node that held the key all along"), 0
    return True, "", n


def evaluate_repair_did_not_deliver_the_key(logs: str, service: str,
                                            marker=KEYLESS_REPAIR_LINE, control=CF_KEY_LINE):
    """…and the late key came from this node's OWN artifact fetch, not from the repair sweep.

    `repair_keyless_schemes` is what survives of the machinery FLU-1202 deleted. It patches
    nothing any more — a scheme reads its epoch key live through its oracle — but it still walks
    the registered below-frontier epochs whose key store misses, calls `ensure_key` on them, and
    re-drives their finalization fetch. Two of those steps could in principle be what made this
    case's certificates start verifying, so the sweep has to be ruled out.

    IT IS RULED OUT BY ORDER, NOT BY ABSENCE. See `KEYLESS_REPAIR_LINE` for the live run that
    settled this: the sweep fires for an epoch whose key has ALREADY landed and which has since
    dropped below the frontier, which is precisely the epoch this case creates. Demanding silence
    demanded that a downstream consequence of the tested event not occur, and reds a correct node.

    So the question this asks is the answerable one: did this node say it had adopted `PK_epoch`
    BEFORE the sweep said a word? If it did, the sweep cannot have been the delivery mechanism —
    the key was in the store, from the follower's own `fetch_and_verify`, before the sweep looked.
    If the sweep spoke first, the two roads are indistinguishable from the log and the case must
    not claim the oracle's.

    NO SWEEP LINE AT ALL IS A PASS and is reported as such. It is the cleanest outcome, it is what
    happens whenever the epoch stays at or above the frontier for the whole case, and requiring
    the line to be present would make the verdict depend on where the chain's boundary fell.

    CONSERVATIVE ON EPOCH. The first sweep line of ANY epoch is compared against the adoption, not
    just one for the adopted epoch — a sweep for an unrelated epoch that preceded the adoption
    still reds. That direction is deliberate: matching epochs across the two lines means matching
    the sweep's registered epoch against the adoption's MINTING epoch, which coincide here and
    need not in general, and a red that says "look at this" is the safe half of that trade.

    A NEGATIVE, so it carries its positive control INSIDE the same text. An absence assertion over
    an unreadable log is a grep that matches its own absence (`SmokeCtx.logs_required`), and here
    the log is read through `overlay_logs`, which answers "" on a timeout or a daemon error. The
    control is `CF_KEY_LINE`, already established present by `evaluate_key_obtained` — so the same
    read that reports the ordering has to show the key line in it.

    Returns a printable note third: the case tears its stack down and the container log goes with
    it, so whether the sweep ran at all has to survive in the transcript."""
    text = logs or ""
    lines = text.splitlines()
    key_at = next((i for i, line in enumerate(lines) if control in line), None)
    if key_at is None:
        return False, (f"{service}'s log does not carry {control!r} — the text this ordering is "
                       "being read over is not the log that was just proved to contain the key "
                       "adoption, so what it says about the repair sweep is not evidence (empty "
                       "read? wrong service? truncated tail?)"), ""
    sweep_at = next((i for i, line in enumerate(lines) if marker in line), None)
    if sweep_at is None:
        return True, "", "the repair sweep never touched this epoch"
    if sweep_at < key_at:
        return False, (f"{service} logged {marker!r} BEFORE it logged the key adoption — the "
                       "below-frontier repair sweep reached this epoch first, so its `ensure_key` "
                       "and its finalization re-drive cannot be told apart from the follower's "
                       "own artifact fetch, and this run does not show a late key taking effect "
                       f"through the oracle alone: {lines[sweep_at].strip()!r}"), ""
    return True, "", (f"the repair sweep ran {sweep_at - key_at} log line(s) AFTER the adoption, "
                      "so it followed the key rather than delivering it")


# ══ smoke-cert-cascade ════════════════════════════════════════════════════════════════

#: `case-cert-cascade.sh:20-22` — the three followers' host ports.
T1_SERVICE = "cert-follower-l1"
T2_SERVICE = "cert-follower-tier2"
BOGUS_SERVICE = "cert-follower-l1-bogus"

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


def evaluate_bogus_rejected(logs: str):
    """`:95-110` — the negative, on the ONE witness that names the refusal: the product line.

    THE `exited` WITNESS IS GONE, and its removal is the point. It read a container STATE as proof
    of a refusal, so a follower that died of a bad address, an OOM or an unresolvable L1 URL was
    counted as a trust root that worked — the false PASS its own docstring described. An exit code
    cannot tell those apart from a refusal either: every one of them is eyre-1.

    Nothing is lost by dropping it, because the refusal is LOGGED BEFORE the exit — the eyre error
    reaches `error!(?e, "consensus thread exited with error")` (`bins/fluent/src/main.rs:407`) and
    only then `exit(1)` (`:429`) — and the string is contract-pinned as "MUST survive verbatim"
    (`crates/dpos/consensus/src/cold_start_jump.rs:894-897`)."""
    if BOGUS_REJECT_LINE in (logs or ""):
        return True, "", "refusal logged"
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
SENTRY_SERVICE = "sentry"
DOWNSTREAM_SERVICE = "downstream"
#: L3 keeps a host URL: the tx leg signs and sends with host-side `cast` and the container has no
#: foundry. 28545 is below the ephemeral range, so publishing it is safe.
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
#:
#: ⚠ THE MONITOR THIS NAMES DOES NOT EXIST. `crates/node/src/tx_route.rs` —
#: `spawn_tx_route_monitor`, `ISOLATION_GRACE`, the `fluent_tx_route_isolated` /
#: `fluent_tx_relay_peers` gauges — was written, unit-tested and docker-validated on 2026-06-30
#: and is absent from the working tree and from every git ref (audit `DPOS_AUDIT.md` **B9**;
#: doc §8.5.1 carries the same warning and keeps the design as the spec to re-implement against).
#: So this string has ZERO emitters and the check below CANNOT FAIL. It is kept, not deleted, and
#: NOT as a witness: it is a TRIPWIRE on the day the monitor comes back, paired with the unit-suite
#: pin in `test_smoke_follow_verdicts.py` that fails the moment any of the symbols reappear. The
#: hole it was written for is open — a node with zero devp2p tx peers accepts transactions into a
#: local pool nothing drains, silently — so what must not happen is that the case's OK line goes
#: on implying somebody is watching for it.
ISOLATED_LINE = "tx-route ISOLATED"
#: The symbols whose reappearance means the monitor landed and this gate must be upgraded from a
#: tripwire to a real presence-AND-silence assertion. Pinned in the unit suite against `crates/`.
TX_ROUTE_SYMBOLS = ("tx_route", "spawn_tx_route_monitor", "fluent_tx_route_isolated",
                    "fluent_tx_relay_peers")


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
    """`:141-143` — A TRIPWIRE, NOT A WITNESS, and the difference is the whole of this fix.

    It reads: "if the tx-route monitor ever warns ISOLATED on a demonstrably healthy uplink, that
    is a false positive in a fail-loud path, and an operator who learns to ignore it will ignore
    the true one too."

    That sentence is still worth having. What it is NOT is evidence that the monitor is healthy,
    because THERE IS NO MONITOR: `crates/node/src/tx_route.rs` and every symbol doc §8.5.1
    describes are absent from the tree and from every git ref, though the changelog records them
    as landed and docker-validated on 2026-06-30 (audit **B9**). With no emitter the string can
    never appear, so this returns `True` unconditionally, on every run, for ever.

    The false GREEN was never this function — it was the case's OK line, which read as though the
    tx-route path was being watched. So the fix is not to delete the check (the day the monitor
    lands, this is exactly the right false-positive guard to already have in place) and not to
    dress it up as a witness. It is to keep it, say plainly in the case that it settles nothing
    today, and pin the ABSENCE in the unit suite so that the moment `tx_route` reappears under
    `crates/`, the harness goes red and forces this gate to be rewritten as a presence-AND-silence
    assertion — which is the form it should have had all along, and which cannot be written
    against a subsystem that does not exist."""
    if ISOLATED_LINE in (logs or ""):
        return False, ("tx-route monitor warned ISOLATED while peers were connected "
                       "(false positive) — note that the monitor was ABSENT from the tree as of "
                       "audit B9, so this firing means it has landed since and this gate needs "
                       "upgrading from a tripwire to a real presence-and-silence assertion")
    return True, ""
