//! Share-confirmation accounting: which widenings of this node's body-checked
//! dealer-log set are worth a signed [`ShareConfirm`] on the wire, and the memory of
//! what it has already put there.
//!
//! The count of members that confirm they hold a usable set is the number that decides
//! an epoch's fate — a member without one demotes to verify-only and votes false on the
//! change-boundary block. This module owns the whole "have I said this already?" side of
//! that statement; the actor keeps everything that needs a live `DkgCeremony`.
//!
//! # The policy this module owns: the CLAIMED-WIDTH memory
//!
//! `confirmed_len` is `target epoch -> the width of the set this node last put on the
//! wire`, and three rules read it. Together they are why the plane's dominant byte cost
//! is not one signed broadcast per recorded log:
//!
//! - **Never re-announce a width no greater than the last** (`previous >=
//!   confirmed.len()`). A faithful change detector, because the published index is
//!   first-wins per seat and irreversible (`signed_log_hash` is the FIRST-recorded hash
//!   of a dealer; `publish_recorded_logs` never overwrites an entry): the set never
//!   shrinks and never changes an entry in place, so equal length means an identical
//!   statement.
//! - **Never announce below the quorum**, whatever the trigger.
//! - **Under [`ConfirmTrigger::Decisive`], skip intermediate widths** — the height tick
//!   carries them.
//!
//! [`ConfirmTrigger::AnyGrowth`] is the backstop for the third rule, and it exists
//! precisely because a halted chain has no height tick: the recorded set can still widen
//! there (a peer's `Reveal` arriving under a parked agreement), so the edge that flushes
//! the widest set must not ride the mechanism it backs up.
//!
//! # The trap: the index is ALREADY durability-gated
//!
//! [`Confirmations::mint`] reads the shared `recorded_dkg_logs` index, and what goes
//! INTO that index is decided upstream by the actor's `publish_recorded_logs`: a dealer
//! whose ceremony-journal write did not land (`nondurable_logs`) is excluded there, so
//! the signed confirmation inherits the exclusion and never claims a log this node could
//! not back after a restart. Do NOT add a second gate here — the exclusion is per recorded
//! log (`(dealer, hash)`, the hash being the one this node publishes for the seat) and
//! lives with the journal writer, which is the only place that knows a write failed.

use crate::beacon::{
    actor::{CommitteeFor, DkgLogIndex},
    ceremony::{Outgoing, Target},
    dkg_agree::{ConfirmPool, ShareConfirm},
    dkg_msg::{DkgBody, DkgMsg},
};
use alloy_primitives::B256;
use commonware_cryptography::{ed25519::PrivateKey as Ed25519PrivateKey, Signer as _};
use commonware_utils::{Faults as _, N3f1};
use fluentbase_bls::PeerPubkey;
use std::collections::BTreeMap;

/// Which widenings of this node's body-checked dealer-log set are worth a
/// [`ShareConfirm`] on the wire.
///
/// Every widening used to be gossiped, and each one is superseded by the next
/// within the same burst of `Reveal`s — `ConfirmPool::record` keeps only the widest
/// a peer ever sees, so every intermediate one was signed, broadcast to the whole
/// committee, and thrown away. At `n = 51` that is ~47.7 KiB per sender-recipient
/// pair per epoch against a single ~98 KiB proposal, i.e. the plane's dominant
/// byte cost by an order of magnitude.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmTrigger {
    /// Any widening. The height tick, and the BACKSTOP of the whole scheme: a width
    /// that [`ConfirmTrigger::Decisive`] skipped is carried by the next one of
    /// these, so the widest set this node holds always reaches its peers.
    AnyGrowth,
    /// Only the two widenings a leader cannot wait a block for.
    ///
    /// The FIRST at or above the quorum, because below it no proposal can clear the
    /// entry bar at all and nothing else will emit for this epoch yet; and the one
    /// that COMPLETES the committee, because the set cannot widen again — nothing
    /// later supersedes it, and it is what the all-present case (every dealer
    /// sealed, every `Reveal` delivered) converges on within milliseconds. Those
    /// two are what keeps the agreement deciding at view 1.
    Decisive,
}

/// This node's share-confirmation accounting. See the module docs — in particular the
/// claimed-width memory this type owns.
pub(crate) struct Confirmations {
    /// Signs each [`ShareConfirm`], and names this node's committee seat.
    me_key: Ed25519PrivateKey,
    /// The epoch's committed roster: the seat index a confirmation is signed under, the
    /// `n` the quorum floor is taken over, and the membership a peer re-verifies it
    /// against.
    committee_for: CommitteeFor,
    /// The pool the epoch-key agreement's entry bar counts: this node's own
    /// confirmations and every peer's that verifies. It also carries the namespace
    /// confirmations are signed under, so the actor and the agreement instances cannot
    /// disagree about it. `None` ⇒ unwired (in-process/test default) ⇒ nothing is
    /// minted at all. A cheap `Arc` handle, shared with every agreement instance — the
    /// actor borrows it back through [`Confirmations::pool`] for the two ceremony-side
    /// uses that stayed with it (the parked-leader wakeup, and recording a peer's).
    pool: Option<ConfirmPool>,
    /// READ handle for the dealer-log hash index the actor publishes. Already
    /// durability-gated on the write side — see the module docs. `None` ⇒ unwired,
    /// exactly like `pool`.
    ///
    /// The set is read from here rather than from the live ceremonies deliberately: a
    /// ceremony is CONSUMED at finalize, so a node that has just derived its share — the
    /// very node whose confirmation matters most — would otherwise fall silent.
    recorded: Option<DkgLogIndex>,
    /// `target epoch -> the size of the body-checked set this node last confirmed`.
    /// Only ever set at or above the quorum — see [`ConfirmTrigger`] and
    /// [`Confirmations::mint`] for which widenings actually go on the wire.
    confirmed_len: BTreeMap<u64, usize>,
}

impl Confirmations {
    /// Inert until BOTH seams are wired: without a pool there is nowhere to record a
    /// confirmation, and without the index there is no set to state.
    pub(crate) fn new(me_key: Ed25519PrivateKey, committee_for: CommitteeFor) -> Self {
        Self {
            me_key,
            committee_for,
            pool: None,
            recorded: None,
            confirmed_len: BTreeMap::new(),
        }
    }

    /// Wire the confirmation pool. Hand in the SAME pool every agreement instance holds.
    pub(crate) fn set_pool(&mut self, pool: ConfirmPool) {
        self.pool = Some(pool);
    }

    /// Wire the shared dealer-log hash index this node states over — the same index the
    /// agreement plane proposes from, so a confirmation can never name a set the
    /// proposal path would not.
    pub(crate) fn set_recorded(&mut self, recorded: DkgLogIndex) {
        self.recorded = Some(recorded);
    }

    /// The pool, for the two uses that stayed with the actor: waking a parked agreement
    /// leader when the dealer-log index (not a confirmation) grew, and recording a
    /// peer's inbound confirmation. Both are one call on a shared `Arc` handle, so
    /// handing it back is not a wider seam than the pool already has.
    pub(crate) fn pool(&self) -> Option<&ConfirmPool> {
        self.pool.as_ref()
    }

    /// Mint this node's share-confirmation for every target epoch whose body-checked
    /// dealer-log set has grown enough for `trigger` to want it on the wire.
    ///
    /// Minting is event-driven off the recording paths (every caller runs immediately
    /// after `publish_recorded_logs`) and never on a timer: the statement only changes
    /// when a log is recorded.
    ///
    /// NEVER below the quorum, whatever the trigger. `rejects_structurally` makes a
    /// proposal pin at least a quorum and `ShareConfirm::covers` is a superset test,
    /// so a narrower confirmation cannot cover any proposal that will ever exist —
    /// it is bytes for a statement nothing can count.
    ///
    /// Inert without both the pool and the shared recorded index.
    pub(crate) fn mint(&mut self, trigger: ConfirmTrigger) -> Vec<Outgoing> {
        let (Some(pool), Some(recorded)) = (self.pool.clone(), self.recorded.as_ref()) else {
            return Vec::new();
        };
        let Ok(index) = recorded.read() else {
            return Vec::new();
        };
        let held: Vec<(u64, BTreeMap<u8, B256>)> =
            index.iter().map(|(e, set)| (*e, set.clone())).collect();
        drop(index);

        let me = self.me_key.public_key();
        let mut out = Vec::new();
        for (epoch, set) in held {
            let Some(roster) = (self.committee_for)(epoch) else {
                continue;
            };
            let Some(idx) = roster
                .iter()
                .position(|pk| *pk == me)
                .and_then(|i| u8::try_from(i).ok())
            else {
                continue; // not a member of this target's committee
            };
            let n = roster.len();
            let confirmed: Vec<(u8, B256)> =
                set.into_iter().filter(|(i, _)| (*i as usize) < n).collect();
            if confirmed.len() < N3f1::quorum(n) as usize {
                continue; // nothing could ever count it — see the doc comment
            }
            let previous = self.confirmed_len.get(&epoch).copied();
            if previous.is_some_and(|last| last >= confirmed.len()) {
                continue; // says no more than what is already on the wire
            }
            if trigger == ConfirmTrigger::Decisive && previous.is_some() && confirmed.len() != n {
                continue; // an intermediate width; the next height tick carries it
            }
            self.confirmed_len.insert(epoch, confirmed.len());
            let confirm = ShareConfirm::sign(pool.namespace(), &self.me_key, idx, epoch, confirmed);
            let members: Vec<PeerPubkey> = roster.iter().cloned().collect();
            pool.record(&members, confirm.clone());
            out.push(Outgoing {
                target: Target::Broadcast,
                msg: DkgMsg {
                    ceremony_epoch: epoch,
                    body: DkgBody::Confirm(confirm),
                },
            });
        }
        out
    }

    /// Age both halves of the accounting out at `floor` (an epoch is kept while
    /// `epoch >= floor`).
    ///
    /// Share-confirmations and the claimed-width memory are per-target scratch on one
    /// lifetime: useful only while that target's agreement can still run. They age on
    /// the caller's ONE window, the same predicate every other per-epoch map there uses.
    pub(crate) fn retain(&mut self, floor: u64) {
        if let Some(pool) = self.pool.as_ref() {
            pool.retain(|e| e >= floor);
        }
        self.confirmed_len.retain(|e, _| *e >= floor);
    }

    /// The width this node last put on the wire for `epoch`. Test-only: production
    /// reads it only through [`Self::mint`]'s own rules.
    #[cfg(test)]
    pub(crate) fn claimed_width(&self, epoch: u64) -> Option<usize> {
        self.confirmed_len.get(&epoch).copied()
    }
}
