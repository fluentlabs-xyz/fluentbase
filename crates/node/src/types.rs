// Binding reth header-validator cap (consensus.rs::with_max_extra_data_size),
// checked on EVERY header build AND import → MUST be byte-identical on every
// node. The `extra_data` carries only the production record
// (`[version: u8][leader_index: u8][accused: u8]` = 3 bytes), comfortably inside
// the Ethereum/reth standard `alloy_consensus::MAXIMUM_EXTRA_DATA_SIZE`. An
// explicit cap is still stated because it is what makes the vote-time
// EXACT-length rule load-bearing — the OrderBlock codec tolerates 4 KiB
// (`order_block.rs`), so without a named bound consensus could finalize a block
// no devp2p node could execute.
pub const FLUENT_MAXIMUM_EXTRA_DATA_SIZE: usize = 32;
