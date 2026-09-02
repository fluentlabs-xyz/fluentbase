//!Handler related to a Fluent chain

use crate::{RwasmFrame, RwasmHaltReason};
use alloy_primitives::U256;
use fluentbase_sdk::calldata_quadratic_surcharge;
use revm::{
    context::{
        journaled_state::account::JournaledAccountTr, result::InvalidTransaction, Block, ContextTr,
        JournalTr,
    },
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
            ctx.cfg().is_amsterdam_eip8037_enabled(),
            ctx.cfg().tx_gas_limit_cap(),
            ctx.cfg().is_legacy_bytecode_enabled(),
        )?;

        // Quadratic calldata surcharge for large inputs (>128 KB).
        //
        // REVM has already verified the pre-surcharge intrinsic gas. Re-check the total so the
        // execution-gas calculation cannot subtract a larger intrinsic cost from the tx limit.
        let surcharge = calldata_quadratic_surcharge(ctx.tx().input().len() as u64);
        gas.initial_total_gas = gas.initial_total_gas.checked_add(surcharge).ok_or(
            InvalidTransaction::CallGasCostMoreThanGasLimit {
                gas_limit: ctx.tx().gas_limit(),
                initial_gas: u64::MAX,
            },
        )?;
        if gas.initial_total_gas > ctx.tx().gas_limit() {
            return Err(InvalidTransaction::CallGasCostMoreThanGasLimit {
                gas_limit: ctx.tx().gas_limit(),
                initial_gas: gas.initial_total_gas,
            }
            .into());
        }

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

impl<EVM, ERROR> InspectorHandler for RwasmHandler<EVM, ERROR>
where
    EVM: InspectorEvmTr<
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Frame = RwasmFrame,
        Inspector: Inspector<<<Self as Handler>::Evm as EvmTr>::Context, EthInterpreter>,
    >,
    ERROR: EvmTrError<EVM>,
{
    type IT = EthInterpreter;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RwasmBuilder, RwasmContext, RwasmSpecId};
    use fluentbase_sdk::CALLDATA_QUADRATIC_THRESHOLD;
    use revm::{
        context::{result::InvalidTransaction, BlockEnv, CfgEnv, ContextTr, TxEnv},
        context_interface::{cfg::gas::calculate_initial_tx_gas, result::EVMError},
        database::InMemoryDB,
        primitives::{Address, Bytes},
        state::AccountInfo,
        Database, ExecuteCommitEvm,
    };

    #[test]
    fn rejects_calldata_surcharge_that_exceeds_gas_limit() {
        let caller = Address::repeat_byte(0x11);
        let target = Address::repeat_byte(0x22);
        let input = Bytes::from(vec![0; CALLDATA_QUADRATIC_THRESHOLD as usize + 32]);
        let intrinsic_gas =
            calculate_initial_tx_gas(RwasmSpecId::CANCUN, &input, false, 0, 0, 0).initial_total_gas;
        let surcharge = calldata_quadratic_surcharge(input.len() as u64);
        assert!(surcharge > 0);

        let tx = TxEnv::builder()
            .caller(caller)
            .to(target)
            .data(input)
            .gas_limit(intrinsic_gas)
            .gas_price(1)
            .build()
            .unwrap();

        let initial_balance = U256::from(intrinsic_gas);
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            caller,
            AccountInfo {
                balance: initial_balance,
                ..Default::default()
            },
        );

        let mut ctx = RwasmContext::new(db, RwasmSpecId::CANCUN);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::CANCUN);
        ctx.cfg.legacy_bytecode_enabled = false;
        ctx.block = BlockEnv::default();
        let mut evm = ctx.build_rwasm();

        assert!(matches!(
            evm.transact_commit(tx),
            Err(EVMError::Transaction(
                InvalidTransaction::CallGasCostMoreThanGasLimit {
                    gas_limit,
                    initial_gas,
                }
            )) if gas_limit == intrinsic_gas && initial_gas == intrinsic_gas + surcharge
        ));
        assert_eq!(
            evm.0.ctx.db_mut().basic(caller).unwrap().unwrap().balance,
            initial_balance
        );
    }
}
