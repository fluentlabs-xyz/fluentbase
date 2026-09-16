//! Materialized-state-gated executed-hash probe — the shared source of the
//! `executed_hash` closure fed to both `EpochTransition` instances (the signer
//! boundary hook and the beacon-plane poller).
//!
//! `provider.block_hash(n)` alone is a header probe: it resolves as soon as a
//! header is written, which during a reth pipeline backfill runs far ahead of
//! executed state. Feeding that to the boundary hook drives committee reads at
//! un-executed hashes and trips the signer's consecutive-error self-shutdown, so
//! the probe gates on reth's materialized head (`best_block_number`, not the
//! static-file header tip) and reports `Ok(None)` for a not-yet-materialized
//! height while a genuine fault at a materialized height stays a real `Err`.

use alloy_primitives::B256;
use fluentbase_staking_reader::ReadError;
use reth_storage_api::{BlockHashReader, BlockNumReader};

/// Three-valued executed-state probe for a committee read at `height`:
///
/// - `Ok(None)` — strictly when `height > best_block_number()`: the header may
///   already exist (pipeline backfill writes headers ahead of state) but the
///   executed state is not materialized, so the caller parks and reads nothing at
///   this height.
/// - `Ok(Some(hash))` — `height <= best_block_number()` and the header resolves:
///   state is materialized past `height` and the committee read is safe.
/// - `Err(_)` — `best_block_number()` errored, or at a materialized height
///   (`height <= best`) `block_hash(height)` returned `Ok(None)` or `Err`. Never a
///   park: a fault at a materialized height must surface to the hook's
///   consecutive-error counter, not be folded into the park behind a never-ticking
///   counter.
///
/// A provider error is classified by the error-owning layer
/// ([`fluentbase_staking_reader::classify_transient_provider_error`]) so a torn
/// static-file read or a clean state-miss keeps the class consumers route on; only
/// what that function does not recognise becomes [`ReadError::Backend`]. `Ok(None)`
/// at a materialized height is a header-index inconsistency and stays `Backend`,
/// i.e. permanent.
pub fn executed_state_hash<P>(provider: &P, height: u64) -> Result<Option<B256>, ReadError>
where
    P: BlockHashReader + BlockNumReader,
{
    let best = provider.best_block_number().map_err(classify)?;
    if height > best {
        return Ok(None);
    }
    match provider.block_hash(height) {
        Ok(Some(hash)) => Ok(Some(hash)),
        Ok(None) => Err(ReadError::Backend(format!(
            "block_hash({height}) is None at a materialized height (best={best}): \
             header-index inconsistency, not a not-yet-materialized park"
        ))),
        Err(e) => Err(classify(e)),
    }
}

fn classify(e: reth_storage_api::errors::provider::ProviderError) -> ReadError {
    fluentbase_staking_reader::classify_transient_provider_error(&e)
        .unwrap_or_else(|| ReadError::Backend(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_chainspec::ChainInfo;
    use reth_storage_api::errors::{
        db::DatabaseError,
        provider::{ProviderError, ProviderResult},
    };

    /// The three reth storage faults the probe has to tell apart, built fresh
    /// per call so the mock needs no `Clone` on `ProviderError`.
    #[derive(Clone, Copy)]
    enum Fault {
        /// A clean state-miss — reth's own "the header is here, the state is
        /// not" answer during a pipeline backfill / unwind.
        StateMiss,
        /// A torn static-file read: the persistence thread appending while this
        /// read ran. Typed as `DatabaseError::Decode`.
        TornDecode,
        /// A genuine, permanent fault: the state at that block is pruned and no
        /// retry will bring it back.
        Permanent,
    }

    impl Fault {
        fn error(self) -> ProviderError {
            match self {
                Self::StateMiss => ProviderError::StateForHashNotFound(B256::ZERO),
                Self::TornDecode => ProviderError::Database(DatabaseError::Decode),
                Self::Permanent => ProviderError::StateAtBlockPruned(7),
            }
        }
    }

    /// `block_hash` resolution the mock provider should return, decoupled from
    /// the materialized head so the "materialized but missing" fail-safe is
    /// reachable (best high, header still absent/erroring).
    enum HashMode {
        Present(B256),
        Absent,
        Errored(Fault),
    }

    /// Reth provider exercising exactly the two reads the probe makes: the
    /// materialized head (a height, or a fault of its own) and a `block_hash`
    /// outcome. Unused trait methods return benign values (never
    /// `unimplemented!` — the probe touches none of them, but a total impl keeps
    /// the mock honest).
    struct MockProvider {
        best: u64,
        best_fault: Option<Fault>,
        hash_mode: HashMode,
    }

    impl MockProvider {
        fn new(best: u64, hash_mode: HashMode) -> Self {
            Self {
                best,
                best_fault: None,
                hash_mode,
            }
        }

        fn with_best_fault(best: u64, fault: Fault) -> Self {
            Self {
                best,
                best_fault: Some(fault),
                hash_mode: HashMode::Present(B256::ZERO),
            }
        }
    }

    impl BlockHashReader for MockProvider {
        fn block_hash(&self, _number: u64) -> ProviderResult<Option<B256>> {
            match self.hash_mode {
                HashMode::Present(h) => Ok(Some(h)),
                HashMode::Absent => Ok(None),
                HashMode::Errored(fault) => Err(fault.error()),
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
            match self.best_fault {
                Some(fault) => Err(fault.error()),
                None => Ok(self.best),
            }
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
        // height > best ⇒ Ok(None): state is not materialized yet, so the caller
        // parks; the probe never returns Ok(Some) above best even when a header
        // exists.
        let h = B256::repeat_byte(0xAB);
        let provider = MockProvider::new(100, HashMode::Present(h));
        assert_eq!(executed_state_hash(&provider, 101).unwrap(), None);
    }

    #[test]
    fn at_or_below_materialized_head_resolves_the_header() {
        let h = B256::repeat_byte(0xCD);
        let provider = MockProvider::new(100, HashMode::Present(h));
        assert_eq!(executed_state_hash(&provider, 100).unwrap(), Some(h));
    }

    #[test]
    fn block_hash_fault_at_materialized_height_is_a_real_error_not_a_park() {
        // At a materialized height a block_hash miss or error is a real fault, so
        // the probe returns Err and never a park: an Ok(None) here would hide the
        // corruption behind the never-ticking consecutive-error counter.
        let absent = MockProvider::new(100, HashMode::Absent);
        assert!(
            matches!(executed_state_hash(&absent, 50), Err(ReadError::Backend(_))),
            "block_hash Ok(None) at height <= best must be a real error, not a park"
        );

        // Any `block_hash` error is an error whatever its cause; which class is
        // the next test's subject.
        for fault in [Fault::StateMiss, Fault::TornDecode, Fault::Permanent] {
            let errored = MockProvider::new(100, HashMode::Errored(fault));
            assert!(
                executed_state_hash(&errored, 50).is_err(),
                "block_hash Err at height <= best must propagate as a real error"
            );
        }
    }

    #[test]
    fn a_transient_storage_fault_keeps_its_transient_class_through_the_probe() {
        // A reth storage hiccup must keep the class the error-owning layer gives
        // it, not flatten into `Backend`, which `is_transient` reports as
        // permanent; the committee module routes the slasher on that predicate.
        let state_miss = MockProvider::new(100, HashMode::Errored(Fault::StateMiss));
        let err = executed_state_hash(&state_miss, 50).expect_err("a fault, not a park");
        assert!(
            matches!(err, ReadError::StateNotMaterialized { .. }) && err.is_transient(),
            "a clean state-miss at a materialized height is TRANSIENT, got {err:?}"
        );

        let torn = MockProvider::new(100, HashMode::Errored(Fault::TornDecode));
        let err = executed_state_hash(&torn, 50).expect_err("a fault, not a park");
        assert!(
            matches!(err, ReadError::TransientStorage(_)) && err.is_transient(),
            "a torn static-file read is TRANSIENT, got {err:?}"
        );

        // The other direction: a pruned state is permanent and stays `Backend`.
        let pruned = MockProvider::new(100, HashMode::Errored(Fault::Permanent));
        let err = executed_state_hash(&pruned, 50).expect_err("a fault, not a park");
        assert!(
            matches!(err, ReadError::Backend(_)) && !err.is_transient(),
            "a permanent provider fault stays a permanent Backend, got {err:?}"
        );

        // The materialized-head read is classified the same way; it reads the
        // same storage.
        let best_torn = MockProvider::with_best_fault(100, Fault::TornDecode);
        let err = executed_state_hash(&best_torn, 50).expect_err("a fault, not a park");
        assert!(
            matches!(err, ReadError::TransientStorage(_)) && err.is_transient(),
            "a torn read of the materialized head is TRANSIENT, got {err:?}"
        );

        // `Ok(None)` at a materialized height is the header-index inconsistency
        // arm, and it stays permanent.
        let absent = MockProvider::new(100, HashMode::Absent);
        let err = executed_state_hash(&absent, 50).expect_err("a fault, not a park");
        assert!(
            matches!(err, ReadError::Backend(_)) && !err.is_transient(),
            "a header-index miss is not a transient storage read, got {err:?}"
        );
    }
}
