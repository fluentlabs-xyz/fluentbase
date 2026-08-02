//! Materialized-state-gated executed-hash probe — the shared source of the
//! `executed_hash` closure fed to both
//! [`EpochTransition`](fluentbase_staking_reader::EpochTransition) instances
//! (the signer boundary hook, `dpos.rs`; the beacon-plane poller,
//! `node/src/dpos.rs`).
//!
//! `provider.block_hash(n)` alone is a HEADER probe: it resolves `Some` the
//! instant a header is written, which during a reth PIPELINE backfill runs FAR
//! ahead of executed state. Feeding that to the epoch-boundary hook drove
//! committee state reads at un-executed hashes → reth `StateForHashNotFound` →
//! `ReadError::Backend("no state found …")` → the signer's 3-consecutive-error
//! self-shutdown (`MAX_CONSECUTIVE_ON_FINALIZED_ERRORS`), killing a node that
//! was merely catching up. This helper gates the probe on reth's MATERIALIZED
//! head (`best_block_number()` — the executed/canonical head, NOT the
//! static-file header tip `last_block_number`, which races ahead during
//! backfill; project memory `reth-sync-progress-best-number-vs-last-block`), so
//! a not-yet-materialized height reports `Ok(None)` (the EpochTransition Intra
//! park fires and re-pokes) while a genuine fault at a materialized height stays
//! a real `Err` (the fail-fast counter still fires).

use alloy_primitives::B256;
use fluentbase_staking_reader::ReadError;
use reth_storage_api::{BlockHashReader, BlockNumReader};

/// Three-valued executed-state probe for a committee read at `height`:
///
/// - `Ok(None)` — STRICTLY when `height > best_block_number()`: the header may
///   already exist (pipeline backfill writes headers ahead of state) but the
///   executed STATE is not materialized yet. The caller PARKS — it derives /
///   reads NOTHING at this un-executed height, only defers the committee read
///   until materialization.
/// - `Ok(Some(hash))` — `height <= best_block_number()` and the header
///   resolves: state is materialized past `height`, the committee read is safe.
/// - `Err(ReadError::Backend)` — `best_block_number()` errored, OR at a
///   materialized height (`height <= best`) `block_hash(height)` returned
///   `Ok(None)` or `Err`. A miss at a materialized height is a real
///   header-index / corruption / reorg-at-H fault, NOT a transient — it MUST
///   surface to the hook's consecutive-error counter, never be folded into the
///   Intra park (which would strand a corruption behind a never-ticking
///   counter, bypassing the fail-fast production posture).
///
/// `best_block_number()` reads an in-memory atomic and returns `Ok`
/// unconditionally, so its `?` never fires in practice; it is mapped for
/// totality only.
pub fn executed_state_hash<P>(provider: &P, height: u64) -> Result<Option<B256>, ReadError>
where
    P: BlockHashReader + BlockNumReader,
{
    let best = provider
        .best_block_number()
        .map_err(|e| ReadError::Backend(e.to_string()))?;
    if height > best {
        return Ok(None);
    }
    match provider.block_hash(height) {
        Ok(Some(hash)) => Ok(Some(hash)),
        Ok(None) => Err(ReadError::Backend(format!(
            "block_hash({height}) is None at a materialized height (best={best}): \
             header-index inconsistency, not a not-yet-materialized park"
        ))),
        Err(e) => Err(ReadError::Backend(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_chainspec::ChainInfo;
    use reth_storage_api::errors::provider::{ProviderError, ProviderResult};

    /// `block_hash` resolution the mock provider should return, decoupled from
    /// the materialized head so the "materialized but missing" fail-safe is
    /// reachable (best high, header still absent/erroring).
    enum HashMode {
        Present(B256),
        Absent,
        Errored,
    }

    /// Reth provider exercising exactly the two reads the probe makes: a fixed
    /// materialized head and a `block_hash` outcome. Unused trait methods return
    /// benign values (never `unimplemented!` — the probe touches none of them,
    /// but a total impl keeps the mock honest).
    struct MockProvider {
        best: u64,
        hash_mode: HashMode,
    }

    impl BlockHashReader for MockProvider {
        fn block_hash(&self, _number: u64) -> ProviderResult<Option<B256>> {
            match self.hash_mode {
                HashMode::Present(h) => Ok(Some(h)),
                HashMode::Absent => Ok(None),
                HashMode::Errored => Err(ProviderError::StateForHashNotFound(B256::ZERO)),
            }
        }
        fn canonical_hashes_range(&self, _start: u64, _end: u64) -> ProviderResult<Vec<B256>> {
            Ok(vec![])
        }
    }

    impl BlockNumReader for MockProvider {
        fn chain_info(&self) -> ProviderResult<ChainInfo> {
            Ok(ChainInfo::default())
        }
        fn best_block_number(&self) -> ProviderResult<u64> {
            Ok(self.best)
        }
        fn last_block_number(&self) -> ProviderResult<u64> {
            Ok(self.best)
        }
        fn block_number(&self, _hash: B256) -> ProviderResult<Option<u64>> {
            Ok(None)
        }
    }

    #[test]
    fn above_materialized_head_is_park_edge_none() {
        // height > best ⇒ Ok(None): state not yet materialized (header may or may
        // not exist — the probe does not even read it), so the caller PARKS. It
        // is NEVER Ok(Some) above best (the over-eager header case the bug rode).
        let h = B256::repeat_byte(0xAB);
        let provider = MockProvider {
            best: 100,
            hash_mode: HashMode::Present(h),
        };
        assert_eq!(executed_state_hash(&provider, 101).unwrap(), None);
    }

    #[test]
    fn at_or_below_materialized_head_resolves_the_header() {
        let h = B256::repeat_byte(0xCD);
        let provider = MockProvider {
            best: 100,
            hash_mode: HashMode::Present(h),
        };
        assert_eq!(executed_state_hash(&provider, 100).unwrap(), Some(h));
    }

    #[test]
    fn block_hash_fault_at_materialized_height_is_a_real_error_not_a_park() {
        // The F2 fail-safe at the PROBE level: at a materialized height
        // (height <= best) a block_hash miss (Ok(None)) or Err is a real
        // header-index / corruption fault, so the probe returns Err — surfacing
        // to the hook's consecutive-error counter — and NEVER Ok(None) (which
        // would silently park a corruption behind the never-ticking counter,
        // bypassing MAX_CONSECUTIVE_ON_FINALIZED_ERRORS).
        let absent = MockProvider {
            best: 100,
            hash_mode: HashMode::Absent,
        };
        assert!(
            matches!(executed_state_hash(&absent, 50), Err(ReadError::Backend(_))),
            "block_hash Ok(None) at height <= best must be a real error, not a park"
        );

        let errored = MockProvider {
            best: 100,
            hash_mode: HashMode::Errored,
        };
        assert!(
            matches!(
                executed_state_hash(&errored, 50),
                Err(ReadError::Backend(_))
            ),
            "block_hash Err at height <= best must propagate as a real error"
        );
    }
}
