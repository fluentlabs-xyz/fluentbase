//! Executor: serializes reth EL forwarding + forkchoice state from consensus.

pub mod actor;
pub mod ingress;

pub use actor::{Actor, BlockFetcher, Config};
pub use ingress::{CanonicalizeAndBuild, CanonicalizeError, Command, Mailbox, Message};
