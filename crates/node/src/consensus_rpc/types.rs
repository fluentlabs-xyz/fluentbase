//! Wire types for the `consensus` RPC namespace.
//!
//! Two-tier finality (deferred execution): `Finalized` = INCLUSION tier (the
//! ordering artifact is committee-agreed, ~3Δ); `ResultFinalized` = RESULT
//! tier (the derived block's hash became committee-attested via the `result`
//! commitment embedded in the finalized artifact K heights above it).
//! `Notarized`/`Nullified` events and `latest_notarized` remain deferred —
//! additive later (`Event` is an enum; `ConsensusState` gains a field).

use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::certified_block::CertifiedBlock;

/// An event pushed over the `consensus_subscribe` WS stream.
///
/// `Arc<CertifiedBlock>`: the payload is a multi-MB hex string worst-case and
/// every broadcast subscriber receives a clone — `Arc` makes that a refcount
/// bump (serde `rc` feature; JSON wire shape unchanged).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Finalized {
        #[serde(flatten)]
        block: Arc<CertifiedBlock>,
        /// Unix-ms at which the serving node observed this finalization.
        seen: u64,
    },
    /// Result tier: `height`'s derived block hash became committee-attested
    /// (it is the `result` commitment of the inclusion-finalized artifact at
    /// `height + K`).
    #[serde(rename_all = "camelCase")]
    ResultFinalized {
        height: u64,
        executed_hash: B256,
        /// Unix-ms at which the serving node observed the attestation.
        seen: u64,
    },
}

/// `consensus_getFinalization` query: the latest finalized, or a specific height.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Query {
    Latest,
    Height(u64),
}

/// `consensus_getLatest` snapshot (finalized-only v1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusState {
    pub latest_finalized: Option<Arc<CertifiedBlock>>,
    /// Highest height whose execution result is committee-attested.
    pub latest_result_finalized: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;

    fn certified() -> CertifiedBlock {
        CertifiedBlock {
            height: 9,
            epoch: 2,
            view: 4,
            digest: B256::repeat_byte(0x33),
            certificate: "00ab".into(),
            block: "cafe".into(),
        }
    }

    #[test]
    fn event_finalized_is_tagged_and_flattened() {
        let e = Event::Finalized {
            block: Arc::new(certified()),
            seen: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&e).expect("serialize");
        assert!(json.contains("\"type\":\"finalized\""));
        // `block` is flattened — its fields sit alongside `type`/`seen`, not nested.
        assert!(json.contains("\"certificate\""));
        assert!(json.contains("\"seen\""));
        let back: Event = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(e, back);
    }

    #[test]
    fn query_round_trips_both_variants() {
        assert_eq!(serde_json::to_string(&Query::Latest).unwrap(), "\"latest\"");
        assert_eq!(
            serde_json::to_string(&Query::Height(7)).unwrap(),
            "{\"height\":7}"
        );
        let h: Query = serde_json::from_str("{\"height\":7}").unwrap();
        assert_eq!(h, Query::Height(7));
    }

    #[test]
    fn consensus_state_default_is_empty() {
        let s = ConsensusState::default();
        let json = serde_json::to_string(&s).expect("serialize");
        assert_eq!(
            json,
            "{\"latestFinalized\":null,\"latestResultFinalized\":null}"
        );
        let back: ConsensusState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, back);
    }

    #[test]
    fn result_finalized_event_round_trips() {
        let e = Event::ResultFinalized {
            height: 7,
            executed_hash: B256::repeat_byte(0x42),
            seen: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&e).expect("serialize");
        assert!(json.contains("\"type\":\"resultFinalized\""));
        assert!(json.contains("\"executedHash\""));
        let back: Event = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(e, back);
    }
}
