use super::inner_evm_context::InnerEvmContext;
use crate::types::{CallInputs, Gas, InterpreterResult};
use crate::{
    db::Database,
    primitives::{Bytes, EVMError, Env, U256},
    FrameOrResult, CALL_STACK_LIMIT,
};
use core::{
    fmt,
    ops::{Deref, DerefMut},
};
use fluentbase_core::Account;
use fluentbase_types::ExitCode;
use std::boxed::Box;

/// EVM context that contains the inner EVM context and precompiles.
pub struct EvmContext<DB: Database> {
    /// Inner EVM context.
    pub inner: InnerEvmContext<DB>,
}

impl<DB: Database + Clone> Clone for EvmContext<DB>
where
    DB::Error: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<DB> fmt::Debug for EvmContext<DB>
where
    DB: Database + fmt::Debug,
    DB::Error: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmContext")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl<DB: Database> Deref for EvmContext<DB> {
    type Target = InnerEvmContext<DB>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<DB: Database> DerefMut for EvmContext<DB> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<DB: Database> EvmContext<DB> {
    /// Create new context with database.
    pub fn new(db: DB) -> Self {
        Self {
            inner: InnerEvmContext::new(db),
        }
    }

    /// Creates a new context with the given environment and database.
    #[inline]
    pub fn new_with_env(db: DB, env: Box<Env>) -> Self {
        Self {
            inner: InnerEvmContext::new_with_env(db, env),
        }
    }

    /// Sets the database.
    ///
    /// Note that this will ignore the previous `error` if set.
    #[inline]
    pub fn with_db<ODB: Database>(self, db: ODB) -> EvmContext<ODB> {
        EvmContext {
            inner: self.inner.with_db(db),
        }
    }

    /// Make call frame
    #[inline]
    pub fn make_call_frame(
        &mut self,
        inputs: &CallInputs,
    ) -> Result<FrameOrResult, EVMError<DB::Error>> {
        let gas = Gas::new(inputs.gas_limit);

        let return_result = |instruction_result: ExitCode| {
            Ok(FrameOrResult::new_call_result(
                InterpreterResult {
                    result: instruction_result,
                    gas,
                    output: Bytes::new(),
                },
                inputs.return_memory_offset.clone(),
            ))
        };

        // Check depth
        if self.depth > CALL_STACK_LIMIT {
            return return_result(ExitCode::CallDepthOverflow);
        }

        let mut caller = Account::new_from_jzkt(&inputs.context.caller);
        let mut account = Account::new_from_jzkt(&inputs.contract);
        let bytecode = account.load_rwasm_bytecode();

        // Create subroutine checkpoint
        let checkpoint = Account::checkpoint();

        // Touch address. For "EIP-158 State Clear", this will erase empty accounts.
        if inputs.transfer.value == U256::ZERO {
            // TODO: "how can we touch account?"
        }

        // Transfer value from caller to called account
        match Account::transfer(&mut caller, &mut account, inputs.transfer.value) {
            Ok(_) => {}
            Err(err) => {
                Account::rollback(checkpoint);
                return return_result(err);
            }
        }

        // if bytecode is empty then just return Ok
        if bytecode.is_empty() {
            return return_result(ExitCode::Ok);
        }

        // let contract = Box::new(Contract::new_with_context(
        //     inputs.input.clone(),
        //     bytecode,
        //     code_hash,
        //     &inputs.context,
        // ));
        // Create interpreter and executes call and push new CallStackFrame.
        Ok(FrameOrResult::new_call_frame(
            inputs.return_memory_offset.clone(),
            checkpoint,
        ))
    }
}

/// Test utilities for the [`EvmContext`].
#[cfg(any(test, feature = "test-utils"))]
pub(crate) mod test_utils {
    use super::*;
    use crate::types::{CallContext, CallInputs, CallScheme, Transfer};
    use crate::{
        db::{CacheDB, EmptyDB},
        primitives::{address, Address, Bytes, Env, B256, U256},
        InnerEvmContext,
    };
    use std::boxed::Box;

    /// Mock caller address.
    pub const MOCK_CALLER: Address = address!("0000000000000000000000000000000000000000");

    /// Creates `CallInputs` that calls a provided contract address from the mock caller.
    pub fn create_mock_call_inputs(to: Address) -> CallInputs {
        CallInputs {
            contract: to,
            transfer: Transfer {
                source: MOCK_CALLER,
                target: to,
                value: U256::ZERO,
            },
            input: Bytes::new(),
            gas_limit: 0,
            context: CallContext {
                address: MOCK_CALLER,
                caller: MOCK_CALLER,
                code_address: MOCK_CALLER,
                apparent_value: U256::ZERO,
                scheme: CallScheme::Call,
            },
            is_static: false,
            return_memory_offset: 0..0,
        }
    }

    /// Creates an evm context with a cache db backend.
    /// Additionally loads the mock caller account into the db,
    /// and sets the balance to the provided U256 value.
    pub fn create_cache_db_evm_context_with_balance(
        env: Box<Env>,
        mut db: CacheDB<EmptyDB>,
        balance: U256,
    ) -> EvmContext<CacheDB<EmptyDB>> {
        db.insert_account_info(
            MOCK_CALLER,
            crate::primitives::AccountInfo {
                nonce: 0,
                balance,
                code_hash: B256::default(),
                code: None,
                ..Default::default()
            },
        );
        create_cache_db_evm_context(env, db)
    }

    /// Creates a cached db evm context.
    pub fn create_cache_db_evm_context(
        env: Box<Env>,
        db: CacheDB<EmptyDB>,
    ) -> EvmContext<CacheDB<EmptyDB>> {
        EvmContext {
            inner: InnerEvmContext {
                env,
                db,
                error: Ok(()),
                depth: 0,
                spec_id: Default::default(),
            },
        }
    }

    /// Returns a new `EvmContext` with an empty journaled state.
    pub fn create_empty_evm_context(env: Box<Env>, db: EmptyDB) -> EvmContext<EmptyDB> {
        EvmContext {
            inner: InnerEvmContext {
                env,
                db,
                error: Ok(()),
                depth: 0,
                spec_id: Default::default(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::*;

    use crate::{
        db::{CacheDB, EmptyDB},
        primitives::{address, Bytecode, Bytes, Env, U256},
        FrameOrResult, FrameResult,
    };
    use fluentbase_sdk::LowLevelSDK;
    use fluentbase_types::ExitCode;
    use std::boxed::Box;

    // Tests that the `EVMContext::make_call_frame` function returns an error if the
    // call stack is too deep.
    #[test]
    fn test_make_call_frame_stack_too_deep() {
        LowLevelSDK::with_default_jzkt();
        let env = Env::default();
        let db = EmptyDB::default();
        let mut context = create_empty_evm_context(Box::new(env), db);
        context.depth = CALL_STACK_LIMIT + 1;
        let contract = address!("dead10000000000000000000000000000001dead");
        let call_inputs = create_mock_call_inputs(contract);
        let res = context.make_call_frame(&call_inputs);
        let Ok(FrameOrResult::Result(err)) = res else {
            panic!("Expected FrameOrResult::Result");
        };
        assert_eq!(err.interpreter_result().result, ExitCode::CallDepthOverflow);
    }

    // Tests that the `EVMContext::make_call_frame` function returns an error if the
    // transfer fails on the journaled state. It also verifies that the revert was
    // checkpointed on the journaled state correctly.
    #[test]
    fn test_make_call_frame_transfer_revert() {
        LowLevelSDK::with_default_jzkt();
        let env = Env::default();
        let db = EmptyDB::default();
        let mut evm_context = test_utils::create_empty_evm_context(Box::new(env), db);
        let contract = address!("dead10000000000000000000000000000001dead");
        let mut call_inputs = test_utils::create_mock_call_inputs(contract);
        call_inputs.transfer.value = U256::from(1);
        let res = evm_context.make_call_frame(&call_inputs);
        let Ok(FrameOrResult::Result(result)) = res else {
            panic!("Expected FrameOrResult::Result");
        };
        assert_eq!(
            result.interpreter_result().result,
            ExitCode::InsufficientBalance
        );
        // let checkpointed = vec![vec![JournalEntry::AccountLoaded { address: contract }]];
        // assert_eq!(evm_context.journaled_state.journal, checkpointed);
        assert_eq!(evm_context.depth, 0);
    }

    #[test]
    fn test_make_call_frame_missing_code_context() {
        LowLevelSDK::with_default_jzkt();
        let env = Env::default();
        let cdb = CacheDB::new(EmptyDB::default());
        let bal = U256::from(3_000_000_000_u128);
        let mut context = create_cache_db_evm_context_with_balance(Box::new(env), cdb, bal);
        let contract = address!("dead10000000000000000000000000000001dead");
        let call_inputs = test_utils::create_mock_call_inputs(contract);
        let res = context.make_call_frame(&call_inputs);
        let Ok(FrameOrResult::Result(result)) = res else {
            panic!("Expected FrameOrResult::Result");
        };
        assert_eq!(result.interpreter_result().result, ExitCode::Ok);
    }

    #[test]
    fn test_make_call_frame_succeeds() {
        LowLevelSDK::with_default_jzkt();
        let env = Env::default();
        let mut cdb = CacheDB::new(EmptyDB::default());
        let bal = U256::from(3_000_000_000_u128);
        let by = Bytecode::new_raw(Bytes::from(vec![0x60, 0x00, 0x60, 0x00]));
        let contract = address!("dead10000000000000000000000000000001dead");
        cdb.insert_account_info(
            contract,
            crate::primitives::AccountInfo {
                nonce: 0,
                balance: bal,
                code_hash: by.clone().hash_slow(),
                code: Some(by),
                ..Default::default()
            },
        );
        let mut evm_context = create_cache_db_evm_context_with_balance(Box::new(env), cdb, bal);
        let call_inputs = create_mock_call_inputs(contract);
        let res = evm_context.make_call_frame(&call_inputs);
        let Ok(FrameOrResult::Result(FrameResult::Call(call_outcome))) = res else {
            panic!("Expected FrameOrResult::Frame(Frame::Call(..))");
        };
        assert_eq!(call_outcome.memory_offset, 0..0,);
    }
}
