// Includes.
use crate::{
    handler::mainnet,
    primitives::{EVMError, EVMResultGeneric, Spec},
    Context,
};
use fluentbase_types::{ExitCode, IJournaledTrie};
use std::sync::Arc;

/// Load access list accounts and beneficiary.
/// There is no need to load Caller as it is assumed that
/// it will be loaded in DeductCallerHandle.
pub type LoadAccountsHandle<'a, EXT, DB> =
    Arc<dyn Fn(&mut Context<EXT, DB>) -> Result<(), EVMError<ExitCode>> + 'a>;

/// Deduct the caller to its limit.
pub type DeductCallerHandle<'a, EXT, DB> =
    Arc<dyn Fn(&mut Context<EXT, DB>) -> EVMResultGeneric<(), ExitCode> + 'a>;

/// Handles related to pre execution before the stack loop is started.
pub struct PreExecutionHandler<'a, EXT, DB: IJournaledTrie> {
    /// Main load handle
    pub load_accounts: LoadAccountsHandle<'a, EXT, DB>,
    /// Deduct max value from the caller.
    pub deduct_caller: DeductCallerHandle<'a, EXT, DB>,
}

impl<'a, EXT: 'a, DB: IJournaledTrie + 'a> PreExecutionHandler<'a, EXT, DB> {
    /// Creates mainnet MainHandles.
    pub fn new<SPEC: Spec + 'a>() -> Self {
        Self {
            load_accounts: Arc::new(mainnet::load_accounts::<SPEC, EXT, DB>),
            deduct_caller: Arc::new(mainnet::deduct_caller::<SPEC, EXT, DB>),
        }
    }
}

impl<'a, EXT, DB: IJournaledTrie> PreExecutionHandler<'a, EXT, DB> {
    /// Deduct caller to its limit.
    pub fn deduct_caller(&self, context: &mut Context<EXT, DB>) -> Result<(), EVMError<ExitCode>> {
        (self.deduct_caller)(context)
    }

    /// Main load
    pub fn load_accounts(&self, context: &mut Context<EXT, DB>) -> Result<(), EVMError<ExitCode>> {
        (self.load_accounts)(context)
    }
}
