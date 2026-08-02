"""cases/smoke — the default-stack smoke cases, mirroring the two bash assertion libraries.

    asserts.py         the four READ-ONLY bodies of `scripts/asserts.sh`
    asserts_fault.py   the six DESTRUCTIVE bodies of `scripts/asserts-fault.sh`
    asserts_follow.py  the three FOLLOWER bodies (`case-cert-follow`, `-cert-cascade`,
                       `-tx-cascade`), the first cases needing services beyond the base compose
    asserts_onchain.py the four ON-CHAIN bodies (`case-liveness`, `-byzantine`, `-cert-catchup`,
                       `-vrf-dkg-restart-midwindow`), the first cases reading the staking surface
    verdicts.py        the pure decision layer of the first four
    verdicts_fault.py  the pure decision layer of the middle six
    verdicts_follow.py the pure decision layer of the three followers
    verdicts_onchain.py the pure decision layer of the four on-chain cases
    beacon.py          the two cross-node prev_randao helpers `lib.sh` hoisted for both
    driver.py          the case wrapper, the ONE read/write seam every body reaches the chain
                       through, the per-case compose OVERLAY seam and the genesis-tuning EXPORT
                       seam

plus nineteen case entrypoints mirroring the nineteen `case-*.sh` files one-for-one: `tx`,
`epoch`, `vrf`, `vrf_boundary` and their aggregate `base`; `deferred`, `peers`, `vrf_fault`,
`crash_survivor`, `full_restart` and their aggregate `fault`; the standalone `vrf_dkg_liveness`;
the three standalone followers `cert_follow`, `cert_cascade`, `tx_cascade`; and the four
standalone on-chain cases `liveness`, `byzantine`, `cert_catchup`,
`vrf_dkg_restart_midwindow`.

═══ TWO FAMILIES, TWO DIFFERENT SOUNDNESS ARGUMENTS ══════════════════════════════════════

`base` runs its four on one bring-up because they are READ-ONLY W.R.T. CONSENSUS — they query
RPC, grep logs, read on-chain state, and at most send a transaction or deploy a throwaway probe;
none stops, restarts or jails a node. The moment one of them disturbs the topology, `base` stops
being sound.

`fault` runs its five on one bring-up for the opposite reason: every one of them DOES disturb the
topology, and each one puts it back before it returns. The restore is part of the assertion, not
cleanup — a body that returned with a node still down would leave the next body measuring a
degraded chain and reporting green. Ordering there is least → most invasive so that a failure
stops the chain before anything worse has been done to the stack.

The three FOLLOWERS have neither argument and each runs alone, as its bash original does. They
ADD nodes rather than disturbing the committee, but each leaves the stack in a state the next
case must not inherit: `cert-follow` leaves a man-in-the-middle poisoning validator-0's
certificate plane, `cert-cascade` leaves two per-run contract addresses in the compose
environment, and `tx-cascade` widens the sentry's trusted-peer set. Chaining them would be a new
soundness claim, not a saved bring-up.

The four ON-CHAIN cases each run alone for the STRONGEST version of that reason: none of them
restores what it broke. `byzantine` can leave a TOMBSTONED validator and a permanently shrunk
committee; `cert-catchup` and `vrf-dkg-restart-midwindow` bring up a TUNED GENESIS (64-block
epochs), which is a different chain, not a different phase of the same one. `liveness`'s exclusion from the `fault` aggregate is recorded in `fault.py` too,
and the two notes must not drift apart.
"""
