use crate::{
    handler::mainnet,
    primitives::{EVMError, Env, Spec},
    Context,
};
use fluentbase_types::{ExitCode, IJournaledTrie};
use std::sync::Arc;

/// Handle that validates env.
pub type ValidateEnvHandle<'a> = Arc<dyn Fn(&Env) -> Result<(), EVMError<ExitCode>> + 'a>;

/// Handle that validates transaction environment against the state.
/// Second parametar is initial gas.
pub type ValidateTxEnvAgainstState<'a, EXT, DB> =
    Arc<dyn Fn(&mut Context<EXT, DB>) -> Result<(), EVMError<ExitCode>> + 'a>;

/// Initial gas calculation handle
pub type ValidateInitialTxGasHandle<'a> = Arc<dyn Fn(&Env) -> Result<u64, EVMError<ExitCode>> + 'a>;

/// Handles related to validation.
pub struct ValidationHandler<'a, EXT: 'a, DB: IJournaledTrie + 'a> {
    /// Validate and calculate initial transaction gas.
    pub initial_tx_gas: ValidateInitialTxGasHandle<'a>,
    /// Validate transactions against state data.
    pub tx_against_state: ValidateTxEnvAgainstState<'a, EXT, DB>,
    /// Validate Env.
    pub env: ValidateEnvHandle<'a>,
}

impl<'a, EXT: 'a, DB: IJournaledTrie + 'a> ValidationHandler<'a, EXT, DB> {
    /// Create new ValidationHandles
    pub fn new<SPEC: Spec + 'a>() -> Self {
        Self {
            initial_tx_gas: Arc::new(mainnet::validate_initial_tx_gas::<SPEC, DB>),
            env: Arc::new(mainnet::validate_env::<SPEC, DB>),
            tx_against_state: Arc::new(mainnet::validate_tx_against_state::<SPEC, EXT, DB>),
        }
    }
}

impl<'a, EXT, DB: IJournaledTrie> ValidationHandler<'a, EXT, DB> {
    /// Validate env.
    pub fn env(&self, env: &Env) -> Result<(), EVMError<ExitCode>> {
        (self.env)(env)
    }

    /// Initial gas
    pub fn initial_tx_gas(&self, env: &Env) -> Result<u64, EVMError<ExitCode>> {
        (self.initial_tx_gas)(env)
    }

    /// Validate ttansaction against the state.
    pub fn tx_against_state(
        &self,
        context: &mut Context<EXT, DB>,
    ) -> Result<(), EVMError<ExitCode>> {
        (self.tx_against_state)(context)
    }
}
