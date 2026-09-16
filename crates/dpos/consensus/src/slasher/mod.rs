//! Slasher: filters equivocation Activity events, builds SlashCallArgs,
//! submits `Staking.slashEquivocation*` txs via the reth `TransactionPool`
//! (no HTTP RPC).

pub mod actor;
pub mod evidence;
pub mod gossip;
pub mod ingress;
pub mod tombstone;

// The unified trait is re-exported from staking-reader, which the slasher consumes.
pub use actor::{Actor, ChargeStore, Config};
pub use fluentbase_staking_reader::StakingStateRead;
pub use gossip::{EvidenceBridge, EvidenceCommitteeFor};
pub use ingress::{Mailbox, Message};
pub use tombstone::TombstoneSet;
