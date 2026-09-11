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
//! a real `Err` (the fail-fast counter still fires) — carrying the CLASS the
//! error-owning layer gives it, so a consumer routing on
//! [`ReadError::is_transient`](fluentbase_staking_reader::ReadError::is_transient)
//! — the committee module, and the slasher behind it — sees a torn static-file
//! read as the transient it is.

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
/// - `Err(_)` — `best_block_number()` errored, OR at a materialized height
///   (`height <= best`) `block_hash(height)` returned `Ok(None)` or `Err`.
///   Never a park: a fault at a materialized height MUST surface to the hook's
///   consecutive-error counter, never be folded into the Intra park (which
///   would strand a corruption behind a never-ticking counter, bypassing the
///   fail-fast production posture).
///
///   WHICH error is not cosmetic. Both reth reads below touch the same storage
///   the staking reader's `eth_call` does, so a torn static-file read or a
///   clean state-miss can surface HERE just as well, and
///   [`is_transient`](ReadError::is_transient) is a predicate consumers ROUTE
///   on — the committee module turns a permanent one into a refused epoch and a
///   dropped slashing charge. So a provider error is classified by the error-owning
///   layer ([`fluentbase_staking_reader::classify_transient_provider_error`]),
///   exactly as the read boundary in `reader.rs` classifies its own, and only
///   what that function does not recognise becomes [`ReadError::Backend`].
///   `Ok(None)` at a materialized height is NOT a storage fault of that kind —
///   it is a header-index inconsistency, and it stays `Backend`, i.e.
///   permanent.
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

/// A reth provider error as the error-owning layer classifies it, or
/// [`ReadError::Backend`] for a genuine fault.
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
        /// A torn STATIC-FILE read: the persistence thread appending while this
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
        // height > best ⇒ Ok(None): state not yet materialized (header may or may
        // not exist — the probe does not even read it), so the caller PARKS. It
        // is NEVER Ok(Some) above best (the over-eager header case the bug rode).
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
        // The F2 fail-safe at the PROBE level: at a materialized height
        // (height <= best) a block_hash miss (Ok(None)) or Err is a real
        // header-index / corruption fault, so the probe returns Err — surfacing
        // to the hook's consecutive-error counter — and NEVER Ok(None) (which
        // would silently park a corruption behind the never-ticking counter,
        // bypassing MAX_CONSECUTIVE_ON_FINALIZED_ERRORS).
        let absent = MockProvider::new(100, HashMode::Absent);
        assert!(
            matches!(executed_state_hash(&absent, 50), Err(ReadError::Backend(_))),
            "block_hash Ok(None) at height <= best must be a real error, not a park"
        );

        // A `block_hash` Err is an Err whatever its cause — which CLASS of Err
        // is the next test's subject, and the two are kept apart on purpose:
        // this one owns "never a silent park".
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
        // The anchor probe is a READ BOUNDARY like any other, so a reth storage
        // hiccup has to come out of it with the class the error-owning layer
        // gives it (`classify_transient_provider_error`) — not flattened into
        // `Backend`, which `ReadError::is_transient` reports as PERMANENT. The
        // committee module routes the slasher on that predicate, so flattening
        // turns one torn static-file read into a slashing charge dropped for
        // good.
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

        // And the other direction, so the classification is not "everything is
        // transient now": a pruned state is permanent and stays `Backend`.
        let pruned = MockProvider::new(100, HashMode::Errored(Fault::Permanent));
        let err = executed_state_hash(&pruned, 50).expect_err("a fault, not a park");
        assert!(
            matches!(err, ReadError::Backend(_)) && !err.is_transient(),
            "a permanent provider fault stays a permanent Backend, got {err:?}"
        );

        // The `best_block_number()` leg is classified by the same function —
        // it reads the same storage, so it can tear the same way.
        let best_torn = MockProvider::with_best_fault(100, Fault::TornDecode);
        let err = executed_state_hash(&best_torn, 50).expect_err("a fault, not a park");
        assert!(
            matches!(err, ReadError::TransientStorage(_)) && err.is_transient(),
            "a torn read of the materialized head is TRANSIENT, got {err:?}"
        );

        // `Ok(None)` at a materialized height is NOT a storage fault at all —
        // it is the header-index inconsistency arm, and it stays permanent.
        let absent = MockProvider::new(100, HashMode::Absent);
        let err = executed_state_hash(&absent, 50).expect_err("a fault, not a park");
        assert!(
            matches!(err, ReadError::Backend(_)) && !err.is_transient(),
            "a header-index miss is not a transient storage read, got {err:?}"
        );
    }
}
