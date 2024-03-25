use crate::{gas::Gas, EVMData};
use fluentbase_types::ExitCode;
use revm_primitives::{
    db::Database,
    EVMError,
    EVMResultGeneric,
    Env,
    Output,
    ResultAndState,
    Spec,
};

/// Handle call return and return final gas value.
type CallReturnHandle = fn(&Env, i32, Gas) -> Gas;

/// Reimburse the caller with ethereum it didn't spent.
type ReimburseCallerHandle = fn(&mut EVMData<'_>, &Gas) -> EVMResultGeneric<(), ExitCode>;

/// Reward beneficiary with transaction rewards.
type RewardBeneficiaryHandle = ReimburseCallerHandle;

/// Calculate gas refund for transaction.
type CalculateGasRefundHandle = fn(&Env, &Gas) -> u64;

/// Main return handle, takes state from journal and transforms internal result to external.
type MainReturnHandle =
    fn(&mut EVMData<'_>, i32, Output, &Gas) -> Result<ResultAndState, EVMError<ExitCode>>;

/// End handle, takes result and state and returns final result.
/// This will be called after all the other handlers.
///
/// It is useful for catching errors and returning them in a different way.
type EndHandle = fn(
    &mut EVMData<'_>,
    evm_output: Result<ResultAndState, EVMError<ExitCode>>,
) -> Result<ResultAndState, EVMError<ExitCode>>;

/// Handler acts as a proxy and allow to define different behavior for different
/// sections of the code. This allows nice integration of different chains or
/// to disable some mainnet behavior.
pub struct Handler {
    // Uses env, call result and returned gas from the call to determine the gas
    // that is returned from transaction execution..
    pub call_return: CallReturnHandle,
    /// Reimburse the caller with ethereum it didn't spent.
    pub reimburse_caller: ReimburseCallerHandle,
    /// Reward the beneficiary with caller fee.
    pub reward_beneficiary: RewardBeneficiaryHandle,
    /// Calculate gas refund for transaction.
    /// Some chains have it disabled.
    pub calculate_gas_refund: CalculateGasRefundHandle,
    /// Main return handle, returns the output of the transact.
    pub main_return: MainReturnHandle,
    /// End handle.
    pub end: EndHandle,
}

impl Handler {
    /// Handler for the mainnet
    pub fn mainnet<SPEC: Spec>() -> Self {
        Self {
            call_return: mainnet::handle_call_return::<SPEC>,
            calculate_gas_refund: mainnet::calculate_gas_refund::<SPEC>,
            reimburse_caller: mainnet::handle_reimburse_caller::<SPEC>,
            reward_beneficiary: mainnet::reward_beneficiary::<SPEC>,
            main_return: mainnet::main_return,
            end: mainnet::end_handle,
        }
    }

    /// Handle call return, depending on instruction result gas will be reimbursed or not.
    pub fn call_return(&self, env: &Env, call_result: i32, returned_gas: Gas) -> Gas {
        (self.call_return)(env, call_result, returned_gas)
    }

    /// Reimburse the caller with gas that were not spend.
    pub fn reimburse_caller(
        &self,
        data: &mut EVMData<'_>,
        gas: &Gas,
    ) -> Result<(), EVMError<ExitCode>> {
        (self.reimburse_caller)(data, gas)
    }

    /// Calculate gas refund for transaction. Some chains have it disabled.
    pub fn calculate_gas_refund(&self, env: &Env, gas: &Gas) -> u64 {
        (self.calculate_gas_refund)(env, gas)
    }

    /// Reward beneficiary
    pub fn reward_beneficiary(
        &self,
        data: &mut EVMData<'_>,
        gas: &Gas,
    ) -> Result<(), EVMError<ExitCode>> {
        (self.reward_beneficiary)(data, gas)
    }

    /// Main return.
    pub fn main_return(
        &self,
        data: &mut EVMData<'_>,
        call_result: i32,
        output: Output,
        gas: &Gas,
    ) -> Result<ResultAndState, EVMError<ExitCode>> {
        (self.main_return)(data, call_result, output, gas)
    }

    /// End handler.
    pub fn end(
        &self,
        data: &mut EVMData<'_>,
        end_output: Result<ResultAndState, EVMError<ExitCode>>,
    ) -> Result<ResultAndState, EVMError<ExitCode>> {
        (self.end)(data, end_output)
    }
}

mod mainnet {
    use crate::{gas::Gas, EVMData};
    use fluentbase_types::ExitCode;
    use revm_primitives::{
        db::Database,
        EVMError,
        Env,
        ExecutionResult,
        HaltReason,
        OutOfGasError,
        Output,
        ResultAndState,
        Spec,
        State,
        SuccessReason,
        LONDON,
    };

    /// Handle output of the transaction
    #[inline]
    pub(crate) fn handle_call_return<SPEC: Spec>(
        env: &Env,
        call_result: i32,
        returned_gas: Gas,
    ) -> Gas {
        let mut gas = Gas::new(env.tx.gas_limit);
        let tx_gas_limit = env.tx.gas_limit;
        // Spend the gas limit. Gas is reimbursed when the tx returns successfully.
        gas.record_cost(tx_gas_limit);

        if call_result == ExitCode::Ok.into_i32() {
            gas.erase_cost(returned_gas.remaining());
            gas.record_refund(returned_gas.refunded());
        } else {
            gas.erase_cost(returned_gas.remaining());
        }

        gas
    }

    #[inline]
    pub(crate) fn handle_reimburse_caller<SPEC: Spec>(
        data: &mut EVMData<'_>,
        _gas: &Gas,
    ) -> Result<(), EVMError<ExitCode>> {
        let _ = data;
        // let caller = data.env.tx.caller;
        // let effective_gas_price = data.env.effective_gas_price();
        //
        // return balance of not spend gas.
        // let (caller_account, _) = data
        //     .journaled_state
        //     .load_account(caller, data.db)
        //     .map_err(EVMError::Database)?;
        //
        // caller_account.info.balance = caller_account.info.balance.saturating_add(
        //     effective_gas_price * U256::from(gas.remaining() + gas.refunded() as u64),
        // );

        Ok(())
    }

    /// Reward beneficiary with gas fee.
    #[inline]
    pub(crate) fn reward_beneficiary<SPEC: Spec>(
        _data: &mut EVMData<'_>,
        _gas: &Gas,
    ) -> Result<(), EVMError<ExitCode>> {
        // let beneficiary = data.env.block.coinbase;
        // let effective_gas_price = data.env.effective_gas_price();
        //
        // // transfer fee to coinbase/beneficiary.
        // // EIP-1559 discard basefee for coinbase transfer. Basefee amount of gas is discarded.
        // let coinbase_gas_price = if SPEC::enabled(LONDON) {
        //     effective_gas_price.saturating_sub(data.env.block.basefee)
        // } else {
        //     effective_gas_price
        // };
        //
        // let (coinbase_account, _) = data
        //     .journaled_state
        //     .load_account(beneficiary, data.db)
        //     .map_err(EVMError::Database)?;
        //
        // coinbase_account.mark_touch();
        // coinbase_account.info.balance = coinbase_account
        //     .info
        //     .balance
        //     .saturating_add(coinbase_gas_price * U256::from(gas.spend() - gas.refunded() as
        // u64));

        Ok(())
    }

    /// Calculate gas refund for transaction.
    ///
    /// If config is set to disable gas refund, it will return 0.
    ///
    /// If spec is set to london, it will decrease the maximum refund amount to 5th part of
    /// gas spend. (Before london it was 2th part of gas spend)
    #[inline]
    pub(crate) fn calculate_gas_refund<SPEC: Spec>(env: &Env, gas: &Gas) -> u64 {
        if env.cfg.is_gas_refund_disabled() {
            0
        } else {
            // EIP-3529: Reduction in refunds
            let max_refund_quotient = if SPEC::enabled(LONDON) { 5 } else { 2 };
            (gas.refunded() as u64).min(gas.spend() / max_refund_quotient)
        }
    }

    /// Main return handle, returns the output of the transaction.
    #[inline]
    pub(crate) fn main_return(
        _data: &mut EVMData<'_>,
        call_result: i32,
        output: Output,
        gas: &Gas,
    ) -> Result<ResultAndState, EVMError<ExitCode>> {
        // used gas with refund calculated.
        let gas_refunded = gas.refunded() as u64;
        let final_gas_used = gas.spend() - gas_refunded;

        const OK_EXIT_CODE: i32 = ExitCode::Ok.into_i32();
        const PANIC_EXIT_CODE: i32 = ExitCode::Panic.into_i32();
        let result = match call_result {
            OK_EXIT_CODE => ExecutionResult::Success {
                reason: SuccessReason::Return,
                gas_used: final_gas_used,
                gas_refunded,
                logs: vec![],
                output,
            },
            PANIC_EXIT_CODE => ExecutionResult::Revert {
                gas_used: final_gas_used,
                output: match output {
                    Output::Call(return_value) => return_value,
                    Output::Create(return_value, _) => return_value,
                },
            },
            _ => ExecutionResult::Halt {
                reason: HaltReason::OutOfGas(OutOfGasError::Basic),
                gas_used: final_gas_used,
            },
        };

        Ok(ResultAndState {
            result,
            state: State::new(),
        })
    }

    /// Mainnet end handle does not change the output.
    #[inline]
    pub(crate) fn end_handle(
        _data: &mut EVMData<'_>,
        evm_output: Result<ResultAndState, EVMError<ExitCode>>,
    ) -> Result<ResultAndState, EVMError<ExitCode>> {
        evm_output
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        type SpecDefault = revm_primitives::CancunSpec;

        #[test]
        fn test_consume_gas() {
            let mut env = Env::default();
            env.tx.gas_limit = 100;

            let gas =
                handle_call_return::<SpecDefault>(&env, ExitCode::Ok.into_i32(), Gas::new(90));
            assert_eq!(gas.remaining(), 90);
            assert_eq!(gas.spend(), 10);
            assert_eq!(gas.refunded(), 0);
        }

        #[test]
        fn test_consume_gas_with_refund() {
            let mut env = Env::default();
            env.tx.gas_limit = 100;

            let mut return_gas = Gas::new(90);
            return_gas.record_refund(30);

            let gas = handle_call_return::<SpecDefault>(&env, ExitCode::Ok.into_i32(), return_gas);
            assert_eq!(gas.remaining(), 90);
            assert_eq!(gas.spend(), 10);
            assert_eq!(gas.refunded(), 30);

            let gas =
                handle_call_return::<SpecDefault>(&env, ExitCode::Panic.into_i32(), return_gas);
            assert_eq!(gas.remaining(), 90);
            assert_eq!(gas.spend(), 10);
            assert_eq!(gas.refunded(), 0);
        }

        #[test]
        fn test_revert_gas() {
            let mut env = Env::default();
            env.tx.gas_limit = 100;

            let gas =
                handle_call_return::<SpecDefault>(&env, ExitCode::Panic.into_i32(), Gas::new(90));
            assert_eq!(gas.remaining(), 90);
            assert_eq!(gas.spend(), 10);
            assert_eq!(gas.refunded(), 0);
        }
    }
}
