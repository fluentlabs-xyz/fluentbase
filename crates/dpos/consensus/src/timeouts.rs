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

use crate::application::{BLOCK_INTERVAL, VERIFY_EXEC_BUDGET};
use commonware_consensus::types::ViewDelta;
use std::time::Duration;

/// Build/propagation/skew margin on top of the pace component (tempo geo-prod
/// calibration: their leader = pace + 750ms).
const LEADER_MARGIN: Duration = Duration::from_millis(750);

/// Vote-collection margin on top of leader + the verify exec-gate budget.
const VOTE_MARGIN: Duration = Duration::from_millis(450);

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
    /// Fluent 1 block/sec set. Deadlines are measured from view entry
    /// (commonware `voter/state.rs`: `enter_view` arms both from the same
    /// instant — NOT additive). Derived from the cadence source of truth
    /// (`application::BLOCK_INTERVAL` / `VERIFY_EXEC_BUDGET`) so a retune
    /// there cannot silently invalidate the timeouts:
    ///   leader        = pace component (≤ BLOCK_INTERVAL by construction:
    ///                   the pace sleep is capped at one interval from now)
    ///                   + 750ms build/propagation/skew margin (tempo
    ///                   geo-prod calibration: their leader = pace + 750ms);
    ///   certification = leader + verify exec-gate budget
    ///                   (`VERIFY_EXEC_BUDGET`: worst-case derive+execute of
    ///                   one block, ~500ms today with growth headroom to 1s)
    ///                   + 450ms vote collection;
    ///   timeout_retry = 1000ms nullify re-broadcast cadence;
    ///   fetch         = 1000ms resolver fetch (worst-case 4 MB block).
    /// 1s cadence ⇒ leader 1750ms, certification 3200ms.
    pub fn fluent_1s() -> Self {
        let leader = BLOCK_INTERVAL + LEADER_MARGIN;
        Self {
            leader,
            certification: leader + VERIFY_EXEC_BUDGET + VOTE_MARGIN,
            timeout_retry: Duration::from_millis(1000),
            fetch: Duration::from_millis(1000),
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
        // Fluent-specific tripwires on top of commonware's set: the verify
        // exec-gate polls up to VERIFY_EXEC_BUDGET inside the certification
        // window, and paced proposals consume up to BLOCK_INTERVAL of the
        // leader window — timeouts that don't leave room for either cause
        // systematic nullify storms that nothing else would attribute.
        if self.certification < self.leader + VERIFY_EXEC_BUDGET {
            return Err(
                "certification leaves less than VERIFY_EXEC_BUDGET after the leader deadline",
            );
        }
        if self.leader <= BLOCK_INTERVAL {
            return Err("leader_timeout must exceed BLOCK_INTERVAL (paced proposals would miss)");
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
        t.leader = Duration::from_millis(4000); // > certification 3200
        assert!(t.validated().is_err());
    }

    #[test]
    fn skip_above_activity_rejected() {
        let mut t = ConsensusTimeouts::fluent_1s();
        t.skip = ViewDelta::new(999); // > activity 64
        assert!(t.validated().is_err());
    }

    #[test]
    fn certification_without_verify_budget_rejected() {
        let mut t = ConsensusTimeouts::fluent_1s();
        // leader ≤ certification still holds, but the exec-gate budget no
        // longer fits inside the certification window.
        t.certification = t.leader + VERIFY_EXEC_BUDGET - Duration::from_millis(1);
        assert!(t.validated().is_err());
    }

    #[test]
    fn leader_below_block_interval_rejected() {
        let mut t = ConsensusTimeouts::fluent_1s();
        t.leader = BLOCK_INTERVAL; // paced proposal consumes the whole window
        assert!(t.validated().is_err());
    }
}
