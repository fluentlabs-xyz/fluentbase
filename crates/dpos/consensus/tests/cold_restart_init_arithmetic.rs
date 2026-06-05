//! Regression: cold-start init in `dpos.rs::launch` derives the correct
//! `(finalized, head)` from reth's `canonical_state` + `genesis_hash`. Pins the
//! PRODUCTION `derive_cold_start_heights` directly (not a copy), so a future
//! reth pin / refactor that breaks the read pattern fails loud at compile or
//! test time instead of silently regressing at runtime.
//!
//! Note: `last_execution_finalized_height` (= `provider.last_block_number()`)
//! is a separate provider call, not part of this pure derivation, so it is not
//! covered here.

use alloy_consensus::Header;
use alloy_primitives::B256;
use fluentbase_consensus::dpos::derive_cold_start_heights;
use reth_chain_state::CanonicalInMemoryState;
use reth_ethereum_primitives::EthPrimitives;
use reth_primitives_traits::SealedHeader;

fn sealed(height: u64, hash: B256) -> SealedHeader {
    SealedHeader::new(
        Header {
            number: height,
            ..Default::default()
        },
        hash,
    )
}

/// Canonical state with head == finalized (graceful-shutdown restart).
fn cs_finalized(height: u64, hash: B256) -> CanonicalInMemoryState<EthPrimitives> {
    let s = sealed(height, hash);
    CanonicalInMemoryState::with_head(s.clone(), Some(s), None)
}

/// Canonical state with head AHEAD of finalized (warm restart: blocks executed
/// past the last finalization).
fn cs_head_ahead(
    head_h: u64,
    head_hash: B256,
    fin_h: u64,
    fin_hash: B256,
) -> CanonicalInMemoryState<EthPrimitives> {
    CanonicalInMemoryState::with_head(
        sealed(head_h, head_hash),
        Some(sealed(fin_h, fin_hash)),
        None,
    )
}

#[test]
fn pristine_network_falls_back_to_genesis() {
    let cs = CanonicalInMemoryState::<EthPrimitives>::empty();
    let genesis = B256::repeat_byte(0xAA);
    let (fin_num, fin_hash, head_num, head_hash) = derive_cold_start_heights(&cs, genesis);
    assert_eq!(fin_num, 0, "pristine: finalized number = genesis");
    assert_eq!(
        fin_hash, genesis,
        "pristine: finalized hash = genesis fallback"
    );
    assert_eq!(head_num, 0, "pristine: head number = 0 (empty chain_info)");
    // head_hash comes from chain_info (an empty-state default-header hash), NOT
    // the genesis fallback used for `finalized` — proving the two are sourced
    // independently rather than both collapsing to the genesis fallback.
    assert_ne!(
        head_hash, fin_hash,
        "pristine: head (chain_info) and finalized (genesis fallback) are independent"
    );
}

#[test]
fn graceful_restart_reads_canonical_finalized() {
    let fin_hash = B256::repeat_byte(0x06);
    let cs = cs_finalized(6, fin_hash);
    let genesis = B256::repeat_byte(0xAA);
    let (fin_num, fh, head_num, head_hash) = derive_cold_start_heights(&cs, genesis);
    assert_eq!(fin_num, 6, "graceful: finalized from canonical_state");
    assert_eq!(
        fh, fin_hash,
        "graceful: finalized hash from canonical_state"
    );
    assert_eq!(head_num, 6, "graceful: head from chain_info");
    assert_eq!(head_hash, fin_hash, "graceful: head == finalized");
}

#[test]
fn warm_restart_head_ahead_of_finalized() {
    // The realistic warm-restart case: execution head is several blocks past
    // the last finalization. The derivation must keep them distinct (head from
    // chain_info, finalized from get_finalized_num_hash) — not collapse them.
    let fin_hash = B256::repeat_byte(0x06);
    let head_hash = B256::repeat_byte(0x08);
    let cs = cs_head_ahead(8, head_hash, 6, fin_hash);
    let genesis = B256::repeat_byte(0xAA);
    let (fin_num, fh, head_num, hh) = derive_cold_start_heights(&cs, genesis);
    assert_eq!(fin_num, 6, "warm: finalized from get_finalized_num_hash");
    assert_eq!(fh, fin_hash, "warm: finalized hash");
    assert_eq!(head_num, 8, "warm: head ahead of finalized");
    assert_eq!(hh, head_hash, "warm: head hash");
}
