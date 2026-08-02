"""smoke-vrf-dkg-durability — `scripts/case-vrf-dkg-durability.sh`.

The consolidated live-DKG durability suite: TWO DKG ceremonies on ONE rotation-stack bring-up, each
its own phase, recoverable end to end.

  PHASE 1 — POST-SEAL FLEET RECOVERY (G1) on the epoch-2 bootstrap ceremony. Two committee[2]
    members are stopped POST-seal, which drops the cluster below the CONSENSUS quorum so the chain
    STALLS while they are down — the timing-robust "recovery required" control — then restarted.
    Each RESUMES from disk, recovers its OWN epoch-2 share, rejoins, the boundary crosses, and
    prev_randao is byte-identical across every node with no slash.

  PHASE 3 — TORN-JOURNAL SIT-OUT (G5) on the first driven rotation. A committee member's on-disk
    ceremony journal is CORRUPTED while it is stopped; on restart it hits the `JournalLoad::Torn`
    arm, SITS OUT gracefully (never re-deals, so it cannot self-equivocate), the ceremony finalizes
    among the other four = quorum, and the chain stays LIVE. It then re-derives prev_randao from
    the cert seed like a verify-only node, byte-identical to the survivors.

THIS CASE DELIBERATELY WRITES CORRUPT BYTES TO DISK. The recipe (share_state.rs:454-482) overwrites
the first journal record's 4-byte big-endian length prefix with `0xffffffff`, far beyond the file
size, so `load_journal` hits "truncated record body", the first record never decodes, and the arm
is Torn rather than NoFile — the file stays non-empty. The distinction is the case: a NoFile node
RE-DEALS, which is the self-equivocation the sit-out exists to avoid, and the no-re-deal assertion
is what would catch a wrong recipe. Every file operation goes through a THROWAWAY container on the
shared volume, because `docker compose exec` on a STOPPED container succeeds and does nothing.

Runs the PRODUCTION-PATH / rotation stack: the genesis stack DKGs exactly once, at epoch 2, then
carries the key forward forever, so it structurally cannot host a second ceremony.

NOT SUBSUMED BY, AND DOES NOT SUBSUME, `smoke-vrf-dkg-restart-midwindow`. Phase 1's kill lands
POST-finalize, so recovery goes through the DURABLE SHARE FILE and the journal-RESUME path never
runs — the midwindow case is the only one that reaches `Player::resume`, and it logs a line this
case never emits. Confirmed live during chunk 4.

The >f PRE-SEAL TERMINAL halt (G2) — phase 1's negative control — runs as its own case
(`smoke-vrf-dkg-halt`): driving it here would need a second committee change, which is not cleanly
drivable on this six-key equal-stake stack.

Long (~12-18 min), foundry-gated; NOT in run-all.
"""

from __future__ import annotations

from . import asserts_prod_dkg, prod


def run_case(argv=None) -> int:
    return prod.run("smoke-vrf-dkg-durability", [asserts_prod_dkg.assert_vrf_dkg_durability],
                    argv)
