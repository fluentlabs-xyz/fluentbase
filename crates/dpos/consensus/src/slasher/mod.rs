//! Slasher: filters equivocation Activity events, builds SlashCallArgs,
//! submits `Staking.slashEquivocation*` txs via the reth `TransactionPool`
//! (no HTTP RPC).

pub mod actor;
pub mod evidence;
pub mod gossip;
pub mod ingress;
pub mod tombstone;

// Re-export the unified trait from staking-reader; the slasher consumes
// `StakingStateRead`.
pub use actor::{Actor, ChargeStore, Config};
pub use fluentbase_staking_reader::StakingStateRead;
pub use gossip::{EvidenceBridge, EvidenceCommitteeFor};
pub use ingress::{Mailbox, Message};
pub use tombstone::TombstoneSet;
