//! Slasher: filters equivocation Activity events, builds SlashCallArgs,
//! submits `Staking.slashEquivocation*` txs via the reth `TransactionPool`
//! (no HTTP RPC).

pub mod actor;
pub mod evidence;
pub mod ingress;

// Re-export the unified trait from staking-reader; the slasher consumes
// `StakingStateRead`.
pub use actor::{Actor, Config};
pub use fluentbase_staking_reader::StakingStateRead;
pub use ingress::{Mailbox, Message};
