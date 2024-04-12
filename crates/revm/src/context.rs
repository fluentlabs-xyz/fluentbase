pub(crate) mod evm_context;
mod inner_evm_context;

pub use evm_context::EvmContext;
pub use inner_evm_context::InnerEvmContext;

use crate::primitives::HandlerCfg;
use fluentbase_types::IJournaledTrie;
use std::boxed::Box;

/// Main Context structure that contains both EvmContext and External context.
pub struct Context<EXT, DB: IJournaledTrie> {
    /// Evm Context.
    pub evm: EvmContext<DB>,
    /// External contexts.
    pub external: EXT,
}

impl<EXT: Clone, DB: IJournaledTrie + Clone> Clone for Context<EXT, DB> {
    fn clone(&self) -> Self {
        Self {
            evm: self.evm.clone(),
            external: self.external.clone(),
        }
    }
}

impl<DB: IJournaledTrie> Context<(), DB> {
    /// Creates new context with database.
    pub fn new_with_db(db: DB) -> Context<(), DB> {
        Context {
            evm: EvmContext::new_with_env(db, Box::default()),
            external: (),
        }
    }
}

impl<EXT, DB: IJournaledTrie> Context<EXT, DB> {
    /// Creates new context with external and database.
    pub fn new(evm: EvmContext<DB>, external: EXT) -> Context<EXT, DB> {
        Context { evm, external }
    }
}

/// Context with handler configuration.
pub struct ContextWithHandlerCfg<EXT, DB: IJournaledTrie> {
    /// Context of execution.
    pub context: Context<EXT, DB>,
    /// Handler configuration.
    pub cfg: HandlerCfg,
}

impl<EXT, DB: IJournaledTrie> ContextWithHandlerCfg<EXT, DB> {
    /// Creates new context with handler configuration.
    pub fn new(context: Context<EXT, DB>, cfg: HandlerCfg) -> Self {
        Self { cfg, context }
    }
}

impl<EXT: Clone, DB: IJournaledTrie + Clone> Clone for ContextWithHandlerCfg<EXT, DB> {
    fn clone(&self) -> Self {
        Self {
            context: self.context.clone(),
            cfg: self.cfg,
        }
    }
}
