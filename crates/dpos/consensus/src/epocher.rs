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
}
