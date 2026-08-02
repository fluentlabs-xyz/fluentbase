//! Origin-offset epoch boundaries for the sequencer→DPoS migration.
//!
//! Marshal/reth block heights are absolute, but DPoS epochs are numbered
//! relative to the on-chain `dposActivationBlock` (see the staking `ChainConfig`)
//! so the migration anchor becomes relative-epoch 0 — `Inline::genesis(0)` then
//! returns `app.genesis` (the anchor) with no marshal lookup. `OriginEpocher`
//! bridges the two: it is `commonware_consensus::types::FixedEpocher` arithmetic
//! shifted by `origin`. `origin = 0` is byte-equivalent to `FixedEpocher`
//! (non-migration / pristine-genesis path).

use commonware_consensus::types::{Epoch, EpochInfo, Epocher, Height};
use std::num::NonZeroU64;

/// [`Epocher`] with a configurable epoch-0 origin height.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginEpocher {
    origin: u64,
    length: u64,
}

impl OriginEpocher {
    /// `origin` = the height at which relative epoch 0 begins
    /// (`dposActivationBlock`); `length` = `epochBlockInterval`.
    pub const fn new(origin: u64, length: NonZeroU64) -> Self {
        Self {
            origin,
            length: length.get(),
        }
    }

    /// The largest epoch-TERMINAL height at or below `floor`, or `None` when no
    /// terminal exists there (a `floor` below the origin, or inside epoch 0 before
    /// its own terminal — nothing has ended yet).
    ///
    /// This is the height `Inline::genesis(E)` needs before a per-epoch engine can
    /// spawn for the epoch that follows it, and after a jump raises the floor it is
    /// the one height no repair path will ever fetch: repair starts at `floor + 1`
    /// and `store_finalization` drops anything at or below the floor. The boundary
    /// seeding at both `SetFloor` producers keys off exactly this value.
    ///
    /// `last(E_of(floor))` is NOT the same thing and is wrong in general position:
    /// it returns the terminal of the epoch the floor sits INSIDE, which is above
    /// the floor whenever the floor is not itself a terminal.
    pub fn terminal_at_or_below(&self, floor: Height) -> Option<Height> {
        let info = self.containing(floor)?;
        if info.last() == floor {
            return Some(floor);
        }
        self.last(Epoch::new(info.epoch().get().checked_sub(1)?))
    }

    /// First/last absolute height of `epoch`, or `None` on overflow.
    fn bounds(&self, epoch: Epoch) -> Option<(Height, Height)> {
        let first = epoch
            .get()
            .checked_mul(self.length)?
            .checked_add(self.origin)?;
        let last = first.checked_add(self.length - 1)?;
        Some((Height::new(first), Height::new(last)))
    }
}

impl Epocher for OriginEpocher {
    fn containing(&self, height: Height) -> Option<EpochInfo> {
        // Heights below the origin predate DPoS and have no relative epoch.
        let rel = height.get().checked_sub(self.origin)?;
        let epoch = Epoch::new(rel / self.length);
        let (first, last) = self.bounds(epoch)?;
        Some(EpochInfo::new(epoch, height, first, last))
    }

    fn first(&self, epoch: Epoch) -> Option<Height> {
        self.bounds(epoch).map(|(first, _)| first)
    }

    fn last(&self, epoch: Epoch) -> Option<Height> {
        self.bounds(epoch).map(|(_, last)| last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commonware_consensus::types::FixedEpocher;
    use commonware_utils::NZU64;

    #[test]
    fn origin_zero_matches_fixed_epocher() {
        let origin = OriginEpocher::new(0, NZU64!(32));
        let fixed = FixedEpocher::new(NZU64!(32));
        for h in [0u64, 1, 31, 32, 63, 64, 1000] {
            assert_eq!(
                origin.containing(Height::new(h)),
                fixed.containing(Height::new(h)),
                "containing mismatch at height {h}"
            );
        }
        for e in [0u64, 1, 2, 100] {
            assert_eq!(origin.first(Epoch::new(e)), fixed.first(Epoch::new(e)));
            assert_eq!(origin.last(Epoch::new(e)), fixed.last(Epoch::new(e)));
        }
    }

    #[test]
    fn anchor_is_relative_epoch_zero() {
        // origin = 64 (aligned to interval 32), so the anchor at height 64 is
        // the first block of relative epoch 0; genesis(0) never consults last().
        let ep = OriginEpocher::new(64, NZU64!(32));
        let info = ep.containing(Height::new(64)).expect("anchor in range");
        assert_eq!(info.epoch(), Epoch::new(0));
        assert_eq!(info.first(), Height::new(64));
        assert_eq!(ep.first(Epoch::new(0)), Some(Height::new(64)));
        // last(0) is the boundary block the marshal must hold before genesis(1).
        assert_eq!(ep.last(Epoch::new(0)), Some(Height::new(95)));
        assert_eq!(ep.first(Epoch::new(1)), Some(Height::new(96)));
    }

    #[test]
    fn pre_origin_height_unsupported() {
        let ep = OriginEpocher::new(64, NZU64!(32));
        assert_eq!(ep.containing(Height::new(63)), None);
    }

    #[test]
    fn terminal_at_or_below_floor() {
        // origin 64, length 32 ⇒ epoch 0 = [64,95], epoch 1 = [96,127], epoch 2 = [128,159].
        let ep = OriginEpocher::new(64, NZU64!(32));

        // General position: the floor sits inside epoch 2, so the answer is epoch 1's
        // terminal — NOT epoch 2's, which is above the floor.
        assert_eq!(
            ep.terminal_at_or_below(Height::new(140)),
            Some(Height::new(127))
        );
        // The floor IS a terminal.
        assert_eq!(
            ep.terminal_at_or_below(Height::new(127)),
            Some(Height::new(127))
        );
        // One above a terminal: still that terminal.
        assert_eq!(
            ep.terminal_at_or_below(Height::new(128)),
            Some(Height::new(127))
        );
        // Inside epoch 0, before anything has ended.
        assert_eq!(ep.terminal_at_or_below(Height::new(80)), None);
        // Epoch 0's own terminal is the first answerable height.
        assert_eq!(
            ep.terminal_at_or_below(Height::new(95)),
            Some(Height::new(95))
        );
        // Below the origin there are no relative epochs at all.
        assert_eq!(ep.terminal_at_or_below(Height::new(63)), None);
    }

    #[test]
    fn terminal_at_or_below_is_never_above_its_floor() {
        let ep = OriginEpocher::new(64, NZU64!(32));
        for h in 64u64..=300 {
            if let Some(t) = ep.terminal_at_or_below(Height::new(h)) {
                assert!(t <= Height::new(h), "terminal {t:?} above floor {h}");
                assert_eq!(
                    ep.containing(t).map(|i| i.last()),
                    Some(t),
                    "{t:?} is not an epoch terminal"
                );
            }
        }
    }
}
