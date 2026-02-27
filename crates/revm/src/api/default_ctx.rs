//! Contains trait [`DefaultRwasm`] used to create a default context.
use crate::RwasmSpecId;
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
            .with_cfg(CfgEnv::new_with_spec(RwasmSpecId::PRAGUE))
            .with_chain(())
    }
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
        // convert to optimism context
        let mut evm = ctx.build_rwasm_with_inspector(NoOpInspector {});
        // execute
        let _ = evm.transact(TxEnv::builder().build_fill());
        // inspect
        let _ = evm.inspect_one_tx(TxEnv::builder().build_fill());
    }
}
