"""smoke-vrf-rotation — `scripts/case-vrf-rotation.sh`.

The LIVE per-epoch DKG beacon stays live, node-agreed and VARYING across a committee change, and
the prev_randao beacon RELIVES from the first block of the new epoch (the boundary deriver reads
PK_E out of the in-block `beacon_outcome`).

The threshold beacon has NO on-chain mirror any more — `getEpochBeaconKey` /
`commitEpochBeaconKey` are removed — so the per-block seed rides the consensus cert and
`prev_randao = H(seed)`. The rotation is therefore proven NOT by reading a rotated group key
off-chain, but by asserting the OBSERVABLE output: prev_randao stays non-zero, varying and
byte-IDENTICAL across every node before, ACROSS and after the committee-change boundary, while the
per-member ACTIVE_LINE count keeps growing — which is what proves a fresh DKG actually re-dealt and
that every member, the joiner included, casts verified seed partials.

FIVE assertions, in `asserts_prod.assert_vrf_rotation`:

  1. BASELINE — threshold-active from EPOCH 2. There are no beacon flags left (no genesis bake, no
     `--dpos.no-beacon`): every node runs the same always-on live DKG, epoch 1 is seedless, and the
     first key is the DETERMINISTIC epoch-2 ceremony's PK_2 because committee[2] DKGs during epoch 1
     even on a stable committee. "Keyless until the first rotation" is not the premise any more.
  2. TRIGGER — register the external joiner and wait for the FIRST epoch whose committee differs.
  3. ROTATION — cross that boundary.
  4. EARLY-JOIN / RELIVE — the joiner participated in committee[E_new]'s DKG during E_new-1 FROM
     ITS FOLLOWER PHASE (the always-on beacon plane deals and receives regardless of consensus
     role), so it is a FULL beacon signer from block 1 of E_new rather than an observer.
  5. CARRY-FORWARD — a STABLE epoch runs no fresh ceremony, yet the beacon stays live: the prior
     key carries forward in memory and still verifies the seed.

Runs the PRODUCTION-PATH stack, because that is the only harness that can actually rotate the
committee. Long (~6-9 min); needs foundry.
"""

from __future__ import annotations

from . import asserts_prod, prod


def run_case(argv=None) -> int:
    return prod.run("smoke-vrf-rotation", [asserts_prod.assert_vrf_rotation], argv)
