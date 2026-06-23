//! The per-epoch validator role as a pure function of current state.
//!
//! Replaces the boundary-coupled transition zoo (the supervisor `Entry` branch
//! plus the implicit decision in `EpochEngine::new`) with one value derived from
//! current state. A committee member becomes a [`Role::Signer`] the instant it is
//! in `committee[E]` at the live frontier and caught up to the upstream tip — no
//! epoch-boundary wait (the cycle-2 fix; see the `dpos_role_state_binding` plan).
//!
//! `role()` is BEACON-INDEPENDENT and SYNC-INDEPENDENT: whether a node holds a
//! usable DKG share, and whether the E-1 boundary block has reached the local
//! marshal yet, are SPAWN-time concerns the reconciler gates SEPARATELY (the
//! share-gate and the `Inline::genesis` precondition). A `Signer` that holds no
//! share for a beacon-active epoch, or whose boundary block has not yet landed,
//! stays on the verify-only scheme (no participating engine) until both hold —
//! because a shareless Simplex member rejects honest peers' seeded votes and
//! wedges the chain, and the engine `unreachable!`s without its boundary block.
//! Neither is modelled here.
//!
//! "Caught up" is NOT a separate signal: a node reaches the live frontier when
//! `is_live` (its f+1-corroborated `highest_observed_epoch` reaches `E`) and its
//! always-on executor has derived the chain up to E-1's boundary (the
//! `Inline::genesis` spawn gate). The validator's executor is the sole reth
//! writer and follows the chain by LOCAL derivation, so no cert-follow plane and
//! no `caught_up` flag are needed on a validator.

/// The role a validator plays for a given epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Run a participating Simplex engine for the epoch (propose / vote / sign).
    Signer,
    /// Verify-only: follow finalized certs, never propose or sign.
    Verifier,
}

/// The role for an epoch `E`, as a pure function of current state.
///
/// `Signer` iff the local node is in `committee[E]` (`is_member`); `Verifier`
/// otherwise. The caller only invokes this at the live frontier (`is_live`), so
/// liveness is not a separate input. The spawn gates (share present, boundary
/// block present) are applied by the reconciler on top of a `Signer` verdict.
pub fn role(is_member: bool) -> Role {
    if is_member {
        Role::Signer
    } else {
        Role::Verifier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signer_requires_membership() {
        assert_eq!(role(true), Role::Signer);
        assert_eq!(role(false), Role::Verifier);
    }
}
