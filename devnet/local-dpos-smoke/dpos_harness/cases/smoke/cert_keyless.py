"""smoke-cert-keyless (standalone) — FLU-1202's live gate. NO bash ancestor.

A node enters a beacon-active epoch WITHOUT that epoch's key, its certificates are admitted on
the multisig half alone through that window, and then the key arrives and certificate
verification starts working — through the scheme the node already had, with the surviving repair
sweep demonstrably following the key rather than delivering it. Before FLU-1202 the same node
needed `apply_pin` to patch its already-built scheme; the scheme now delegates to an oracle that
reads the live key store, so there is nothing left to patch.

WHAT THE SWEEP LEG ASSERTS, AND WHY IT IS AN ORDER. `repair_keyless_schemes` still walks the
below-frontier epochs whose key store no longer misses and re-drives their finalization fetch, so
it fires for exactly the epoch this case creates — live, 32 s after the adoption. An earlier
version of the case demanded its absence and went red on a correct node. The answerable question
is whether the sweep spoke BEFORE the node adopted its own key; if it did not, it cannot have been
the delivery mechanism.

THE PRECONDITION IS WHY THIS IS A CASE AND NOT A LINE IN ANOTHER ONE. Deleting a repair path is
invisible on a chain whose keys always arrive early — the conclusion "verification works" is
satisfied by a node that held the key all along, and such a node exercises nothing. So the body
gates every reading on `dpos_cert_vote_only_admissions_total >= 1`, which the cert inlet
increments only under `!key_known` for the certificate's own epoch, and fails loud on the
precondition when the keyless window never happened.

It runs on the `cert-follow` overlay and starts ONE honest follower. A `--cert-follow` node is
the only class that enters a beacon-active epoch keyless by construction rather than by fault
injection: it mints nothing and can obtain `PK_epoch` only by fetching the epoch's agreed
artifact, and that fetch is triggered by `observe_cert`, which fires on a certificate that has
already been through the inlet. Every other reproduction in this tree manufactures the same state
with a tuned genesis and an eight-minute DKG choreography.

STANDALONE, though it is the one member of this family that poisons nothing. Its subject is the
first seconds of a node's life against a live chain, and a chained run would hand it a stack
whose beacon-active epochs are minutes old and whose followers are already keyed.
"""

from __future__ import annotations

from . import asserts_follow, driver


def run_case(argv=None) -> int:
    return driver.run("smoke-cert-keyless", [asserts_follow.assert_cert_keyless], argv,
                      overlays=(asserts_follow.CERT_FOLLOW_OVERLAY,))
