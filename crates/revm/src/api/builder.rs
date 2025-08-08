//! Optimism builder trait [`RwasmBuilder`] used to build [`OpEvm`].
use crate::{evm::RwasmEvm, precompiles::RwasmPrecompiles, RwasmSpecId};
use revm::context::Transaction;
use revm::{
    context::Cfg,
    context_interface::{Block, JournalTr},
    handler::instructions::EthInstructions,
    interpreter::interpreter::EthInterpreter,
    state::EvmState,
    Context, Database,
};

/// Type alias for default OpEvm
pub type DefaultRwasmEvm<CTX, INSP = ()> =
    RwasmEvm<CTX, INSP, EthInstructions<EthInterpreter, CTX>, RwasmPrecompiles>;

/// Trait that allows for optimism OpEvm to be built.
pub trait RwasmBuilder: Sized {
    /// Type of the context.
    type Context;

    /// Build the op.
    fn build_rwasm(self) -> DefaultRwasmEvm<Self::Context>;

    /// Build the op with an inspector.
    fn build_rwasm_with_inspector<INSP>(
        self,
        inspector: INSP,
    ) -> DefaultRwasmEvm<Self::Context, INSP>;
}

impl<BLOCK, TX, CFG, DB, JOURNAL> RwasmBuilder for Context<BLOCK, TX, CFG, DB, JOURNAL, ()>
where
    BLOCK: Block,
    TX: Transaction,
    CFG: Cfg<Spec = RwasmSpecId>,
    DB: Database,
    JOURNAL: JournalTr<Database = DB, State = EvmState>,
{
    type Context = Self;

    fn build_rwasm(self) -> DefaultRwasmEvm<Self::Context> {
        RwasmEvm::new(self, ())
    }

    fn build_rwasm_with_inspector<INSP>(
        self,
        inspector: INSP,
    ) -> DefaultRwasmEvm<Self::Context, INSP> {
        RwasmEvm::new(self, inspector)
    }
}
