//! [`CommitteeReadsFacade`] — the existing [`beacon::CommitteeReads`] surface,
//! answered from the committee module.
//!
//! This type exists so the module can land WITHOUT rewriting the beacon's five
//! projections, the follower's, and the stand's in the same change. It is a
//! pure view: it holds no map, no cursor and no policy of its own.
//!
//! Two things about it are deliberate and temporary.
//!
//! * The `at: B256` parameter is IGNORED. The trait resolves a cursor once per
//!   compound read and passes it down so the two halves of
//!   [`beacon::CommitteeReads::committee_pair`] cannot straddle a block. The
//!   module makes that unspellable a stronger way — the record is write-once
//!   and its value does not depend on which in-window anchor read it — so the
//!   parameter has nothing left to decide. It disappears with the trait's shape
//!   in the beacon-boundary step, not here.
//! * [`beacon::CommitteeReads::qual_read_at`] is the IDENTITY of
//!   [`beacon::CommitteeReads::read_at`]. The separate qual cursor existed for
//!   one window — no EL-finalized marker and a live cert cursor — where
//!   `read_at` fell back to the genesis hash and the write-once memo in
//!   `beacon::carry` would have frozen `false` for that epoch for the life of
//!   the process. The module has no such window: below `commit_height(E)` it
//!   answers `NotReadable` without reading anything at all, and at or above it
//!   the bit is final, because the contract writes it in the same
//!   `commit_epoch_committee` call that writes the committee
//!   (`contracts/staking/src/consensus.rs:632-635`). The method leaves the
//!   trait in the same step the `at` parameter does.
//!
//! Every method answers `None` on ANY [`CommitteeError`] — the trait has no
//! room for a reason, and all three of its consumers already treat `None` as
//! "undecided, ask again". The distinction between "not yet" and "the contract
//! forked" is not lost, only not carried HERE: the module logged it.

use super::{Committee, CommitteeError};
use crate::beacon;
use alloy_primitives::B256;
use fluentbase_bls::{scheme::EpochCommittee, PeerPubkey};
use std::sync::Arc;

/// The beacon's committee surface, backed by the committee module.
///
/// ONE field, and that is the point: `read_at()` must answer the hash the
/// record was (or will be) read at, so the anchor is asked OF THE MODULE
/// ([`Committee::anchor_hash`]) instead of being handed in beside it. A second
/// constructor argument would have let a caller pair this view with a cursor
/// that is not the one behind the records — the two-authoritative-cursors
/// defect the module exists to close, one level up.
pub struct CommitteeReadsFacade {
    inner: Arc<dyn Committee>,
}

impl CommitteeReadsFacade {
    pub fn new(inner: Arc<dyn Committee>) -> Self {
        Self { inner }
    }

    /// The record, or `None` for any reason at all. One place, so the four
    /// methods below cannot drift in how they fold an error.
    fn record(&self, epoch: u64) -> Option<Arc<super::CommitteeRecord>> {
        match self.inner.committee(epoch) {
            Ok(record) => Some(record),
            Err(CommitteeError::NotReadable { .. })
            | Err(CommitteeError::OutOfWindow { .. })
            | Err(CommitteeError::Read(_)) => None,
        }
    }
}

impl beacon::CommitteeReads for CommitteeReadsFacade {
    /// The module's anchor: `executed_state_hash(ordering_finalized)`. `None`
    /// while that height is not executed yet — the same "this node cannot read
    /// state yet" the trait asks for, arrived at by a state probe rather than
    /// by a header probe.
    fn read_at(&self) -> Option<B256> {
        self.inner.anchor_hash()
    }

    /// Identity of [`read_at`](beacon::CommitteeReads::read_at). See the module
    /// docs for why the second cursor has nothing left to protect.
    fn qual_read_at(&self) -> Option<B256> {
        self.read_at()
    }

    /// `at` is ignored — the record is write-once and anchor-independent inside
    /// the window (§5.1 "Инвариант окна").
    fn committee(
        &self,
        epoch: u64,
        _at: B256,
    ) -> Option<commonware_utils::ordered::Set<PeerPubkey>> {
        Some(self.record(epoch)?.participants.clone())
    }

    /// `at` is ignored, same reason as
    /// [`committee`](beacon::CommitteeReads::committee).
    fn committee_bls(&self, epoch: u64, _at: B256) -> Option<EpochCommittee> {
        Some(self.record(epoch)?.bls.clone())
    }

    /// `(changed, committed)`. The second leg is unconditionally `true` here:
    /// the module only ever produces a record for an epoch whose committee it
    /// actually read, so "is it committed" is answered by having a record at
    /// all, and an uncommitted epoch takes the `None` path above rather than
    /// reporting `(false, false)`. The `!(bit || committed) ⇒ None` guard in
    /// `beacon::carry` stays where it is; the facade does not duplicate it.
    fn dkg_qual(&self, epoch: u64, _at: B256) -> Option<(bool, bool)> {
        Some((self.record(epoch)?.changed, true))
    }
}
