//! [`CommitteeReadsFacade`] — the [`beacon::CommitteeReads`] surface, answered from
//! the committee module. Every [`CommitteeError`] folds to `None` here; the halt on
//! an impossible committee happens at the epoch-entry reconcile, which reads
//! [`Committee`] directly.
//!
//! The `at: B256` parameter is ignored: the record is write-once and
//! anchor-independent inside the window, so it has nothing left to decide.

use super::{Committee, CommitteeError};
use crate::beacon;
use alloy_primitives::B256;
use fluentbase_bls::{scheme::EpochCommittee, PeerPubkey};
use std::sync::Arc;

/// One field, deliberately: the anchor is the module's own
/// ([`Committee::anchor_hash`]), never a cursor handed in alongside.
pub struct CommitteeReadsFacade {
    inner: Arc<dyn Committee>,
}

impl CommitteeReadsFacade {
    pub fn new(inner: Arc<dyn Committee>) -> Self {
        Self { inner }
    }

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
    /// The module's anchor, `executed_state_hash(ordering_finalized)`; `None` while
    /// that height is not executed.
    fn read_at(&self) -> Option<B256> {
        self.inner.anchor_hash()
    }

    fn committee(
        &self,
        epoch: u64,
        _at: B256,
    ) -> Option<commonware_utils::ordered::Set<PeerPubkey>> {
        Some(self.record(epoch)?.participants.clone())
    }

    fn committee_bls(&self, epoch: u64, _at: B256) -> Option<EpochCommittee> {
        Some(self.record(epoch)?.bls.clone())
    }

    /// `committed` is always `true`: a record exists only for an epoch the module read.
    fn dkg_qual(&self, epoch: u64, _at: B256) -> Option<(bool, bool)> {
        Some((self.record(epoch)?.changed, true))
    }
}
