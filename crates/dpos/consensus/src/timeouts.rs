//! Simplex timeout family, Fluent-tuned for 1 block/sec and
//! pre-validated against commonware's construction-time invariants.
//!
//! commonware sets `leader_deadline` and `certification_deadline` both from
//! the same view-entry instant (NOT additive) and **panics** at
//! `voter/actor.rs:136` if `leader_timeout > certification_timeout`; the
//! config doc also requires `skip_timeout ≤ activity_timeout`. We surface
//! both as a typed `Err` *before* they reach the engine (same philosophy as
//! the staking-reader committee-size guard).

// **** выглядит так как можно пренести в другой файл, отдельный зажирно

use commonware_consensus::types::ViewDelta;
use std::time::Duration;

/// The six Simplex timeouts (`simplex::Config` fields).
#[derive(Clone, Copy, Debug)]
pub struct ConsensusTimeouts {
    pub leader: Duration,
    pub certification: Duration,
    pub timeout_retry: Duration,
    pub fetch: Duration,
    pub activity: ViewDelta,
    pub skip: ViewDelta,
}

impl ConsensusTimeouts {
    /// Fluent 1 block/sec starting set, derived from the real
    /// `state.rs`/`config.rs` semantics.
    /// `leader ≤ certification` and `skip ≤ activity` hold by construction.
    /// `leader`/`certification` are testnet-calibrated (couple to the 50M
    /// Reth execution budget) — these are the documented starting values.
    pub fn fluent_1s() -> Self {
        Self {
            leader: Duration::from_millis(400),
            certification: Duration::from_millis(750),
            timeout_retry: Duration::from_millis(200),
            fetch: Duration::from_millis(750),
            activity: ViewDelta::new(64),
            skip: ViewDelta::new(4),
        }
    }

    /// Enforce commonware's construction-time invariants up-front so a
    /// misconfiguration is an actionable error, not a deep panic
    /// (`leader ≤ certification` — `voter/actor.rs:136`; `skip ≤ activity` —
    /// `config.rs` doc).
    pub fn validated(self) -> Result<Self, &'static str> {
        // Mirror commonware `simplex::Config::assert()` (config.rs:160-201) for
        // the fields this struct owns, so a misconfiguration is an actionable
        // error here rather than a deep panic inside `Engine::new`. Commonware
        // asserts EVERY timeout > 0 (leader, certification, timeout_retry, and
        // fetch_timeout), leader ≤ certification, activity ≠ 0, skip ≠ 0, and
        // skip ≤ activity — these checks reproduce that exact set (not stricter).
        if self.leader.is_zero()
            || self.certification.is_zero()
            || self.timeout_retry.is_zero()
            || self.fetch.is_zero()
        {
            return Err("all simplex timeouts must be greater than zero");
        }
        if self.activity.is_zero() || self.skip.is_zero() {
            return Err("activity_timeout / skip_timeout must be greater than zero");
        }
        if self.leader > self.certification {
            return Err(
                "leader_timeout > certification_timeout (commonware panics on construction)",
            );
        }
        if self.skip.get() > self.activity.get() {
            return Err("skip_timeout > activity_timeout");
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluent_1s_satisfies_commonware_invariants() {
        let t = ConsensusTimeouts::fluent_1s().validated().expect("valid");
        assert!(t.leader <= t.certification);
        assert!(t.skip.get() <= t.activity.get());
    }

    #[test]
    fn inverted_leader_certification_rejected() {
        let mut t = ConsensusTimeouts::fluent_1s();
        t.leader = Duration::from_millis(900); // > certification 750
        assert!(t.validated().is_err());
    }

    #[test]
    fn skip_above_activity_rejected() {
        let mut t = ConsensusTimeouts::fluent_1s();
        t.skip = ViewDelta::new(999); // > activity 64
        assert!(t.validated().is_err());
    }
}
