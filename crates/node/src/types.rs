// Binding reth header-validator cap (consensus.rs::with_max_extra_data_size),
// checked on EVERY header build AND import → MUST be byte-identical on every
// node. The `extra_data` carries only the liveness attestation now (PK_E layer
// removed): 17 (hdr) + ceil(51/8)=7 (bitmap) = 24, which fits the Ethereum/reth
// standard `alloy_consensus::MAXIMUM_EXTRA_DATA_SIZE`.
pub const FLUENT_MAXIMUM_EXTRA_DATA_SIZE: usize = 32;
