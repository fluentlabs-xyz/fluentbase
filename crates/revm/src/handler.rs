//!Handler related to a Fluent chain

use crate::{types::SystemInterruptionOutcome, RwasmFrame, RwasmHaltReason};
use alloy_primitives::U256;
use fluentbase_sdk::calldata_quadratic_surcharge;
use revm::{
    context::{journaled_state::account::JournaledAccountTr, Block, ContextTr, JournalTr},
    context_interface::{Cfg, Transaction},
    handler::{validation, EvmTr, EvmTrError, FrameTr, Handler},
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{interpreter::EthInterpreter, InitialAndFloorGas},
    state::EvmState,
    Inspector,
};

/// Rwasm handler that implements the default [`Handler`] trait for the Evm.
#[derive(Debug, Clone)]
pub struct RwasmHandler<CTX, ERROR> {
    /// Phantom data to hold the generic type parameters.
    pub _phantom: core::marker::PhantomData<(CTX, ERROR)>,
}

impl<EVM, ERROR> Handler for RwasmHandler<EVM, ERROR>
where
    EVM: EvmTr<Context: ContextTr<Journal: JournalTr<State = EvmState>>, Frame = RwasmFrame>,
    ERROR: EvmTrError<EVM>,
{
    type Evm = EVM;
    type Error = ERROR;
    type HaltReason = RwasmHaltReason;

    #[inline]
    fn validate_initial_tx_gas(
        &self,
        evm: &mut Self::Evm,
    ) -> Result<InitialAndFloorGas, Self::Error> {
        let ctx = evm.ctx_ref();
        let mut gas = validation::validate_initial_tx_gas(
            ctx.tx(),
            ctx.cfg().spec().into(),
            ctx.cfg().is_eip7623_disabled(),
            ctx.cfg().is_legacy_bytecode_enabled(),
        )
        .map_err(Self::Error::from)?;

        // Quadratic calldata surcharge for large inputs (>128 KB)
        let input_len = ctx.tx().input().len() as u64;
        gas.initial_gas += calldata_quadratic_surcharge(input_len);

        Ok(gas)
    }

    #[inline]
    fn reward_beneficiary(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        let (block, tx, _cfg, journal, _, _) = evm.ctx().all_mut();
        let basefee = block.basefee() as u128;
        let coinbase_gas_price = tx.effective_gas_price(basefee);

        // Transfer fee to coinbase/beneficiary.
        // EIP-1559 discard basefee for coinbase transfer. Basefee amount of gas is discarded.
        #[cfg(feature = "eip1559-full-compatibility")]
        let coinbase_gas_price = if _cfg
            .spec()
            .into()
            .is_enabled_in(revm::primitives::hardfork::SpecId::LONDON)
        {
            coinbase_gas_price.saturating_sub(basefee)
        } else {
            coinbase_gas_price
        };

        journal
            .load_account_mut(block.beneficiary())?
            .incr_balance(U256::from(
                coinbase_gas_price * exec_result.gas().used() as u128,
            ));
        Ok(())
    }
}

impl<CTX, ERROR> Default for RwasmHandler<CTX, ERROR> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<EVM, ERROR> InspectorHandler<SystemInterruptionOutcome> for RwasmHandler<EVM, ERROR>
where
    EVM: InspectorEvmTr<
        SystemInterruptionOutcome,
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Frame = RwasmFrame,
        Inspector: Inspector<<<Self as Handler>::Evm as EvmTr>::Context, EthInterpreter>,
    >,
    ERROR: EvmTrError<EVM>,
{
    type IT = EthInterpreter;
}
