//! Contains trait [`DefaultRwasm`] used to create a default context.
use crate::RwasmSpecId;
use fluentbase_sdk::TX_GAS_LIMIT_CAP;
use revm::{
    context::{BlockEnv, CfgEnv, TxEnv},
    database_interface::EmptyDB,
    Context, Journal, MainContext,
};

/// Type alias for the default context type of the RwasmEvm.
pub type RwasmContext<DB> = Context<BlockEnv, TxEnv, CfgEnv<RwasmSpecId>, DB, Journal<DB>, ()>;

/// Trait that allows for a default context to be created.
pub trait DefaultRwasm {
    /// Create a default context.
    fn rwasm() -> RwasmContext<EmptyDB>;
}

impl DefaultRwasm for RwasmContext<EmptyDB> {
    fn rwasm() -> Self {
        Context::mainnet()
            .with_tx(TxEnv::builder().build_fill())
            .with_cfg(fluent_cfg(RwasmSpecId::OSAKA))
            .with_chain(())
    }
}

/// A Fluent EVM configuration for `spec`: the per-transaction gas limit cap of the chain
/// (`TX_GAS_LIMIT_CAP`) on top of revm's defaults.
pub fn fluent_cfg(spec: RwasmSpecId) -> CfgEnv<RwasmSpecId> {
    let mut cfg = CfgEnv::new_with_spec(spec);
    cfg.tx_gas_limit_cap = Some(TX_GAS_LIMIT_CAP);
    cfg
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::api::builder::RwasmBuilder;
    use revm::{
        inspector::{InspectEvm, NoOpInspector},
        ExecuteEvm,
    };

    #[test]
    fn default_run_rwasm() {
        let ctx = Context::rwasm();
        // convert to rwasm context
        let mut evm = ctx.build_rwasm_with_inspector(NoOpInspector {});
        // execute
        let _ = evm.transact(TxEnv::builder().build_fill());
        // inspect
        let _ = evm.inspect_one_tx(TxEnv::builder().build_fill());
    }
}
