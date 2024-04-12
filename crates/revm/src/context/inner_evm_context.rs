use crate::primitives::{
    Address, Bytes, EVMError, Env, Spec,
    SpecId::{self, *},
};
use crate::types::InterpreterResult;
use fluentbase_core::{Account, AccountCheckpoint};
use fluentbase_types::{EmptyJournalTrie, ExitCode, IJournaledTrie};
use revm_primitives::RWASM_MAX_CODE_SIZE;
use std::boxed::Box;

/// EVM contexts contains data that EVM needs for execution.
#[derive(Debug)]
pub struct InnerEvmContext<DB: IJournaledTrie> {
    /// EVM Environment contains all the information about config, block and transaction that
    /// evm needs.
    pub env: Box<Env>,
    /// Database to load data from.
    pub db: DB,
    /// Error that happened during execution.
    pub error: Result<(), EVMError<ExitCode>>,
    /// Current recursion depth
    pub depth: u64,
    /// Spec id
    pub spec_id: SpecId,
}

impl Default for InnerEvmContext<EmptyJournalTrie> {
    fn default() -> Self {
        Self {
            env: Box::new(Default::default()),
            db: EmptyJournalTrie::default(),
            error: Ok(()),
            depth: 0,
            spec_id: Default::default(),
        }
    }
}

impl<DB: IJournaledTrie + Clone> Clone for InnerEvmContext<DB> {
    fn clone(&self) -> Self {
        Self {
            env: self.env.clone(),
            db: self.db.clone(),
            error: self.error.clone(),
            depth: self.depth.clone(),
            spec_id: self.spec_id.clone(),
        }
    }
}

impl<DB: IJournaledTrie> InnerEvmContext<DB> {
    pub fn new(db: DB) -> Self {
        Self {
            env: Box::default(),
            db,
            error: Ok(()),
            depth: 0,
            spec_id: Default::default(),
        }
    }

    /// Creates a new context with the given environment and database.
    #[inline]
    pub fn new_with_env(db: DB, env: Box<Env>) -> Self {
        Self {
            env,
            db,
            error: Ok(()),
            depth: 0,
            spec_id: Default::default(),
        }
    }

    /// Sets the database.
    ///
    /// Note that this will ignore the previous `error` if set.
    #[inline]
    pub fn with_db<ODB: IJournaledTrie>(self, db: ODB) -> InnerEvmContext<ODB> {
        InnerEvmContext {
            env: self.env,
            db,
            error: Ok(()),
            depth: 0,
            spec_id: Default::default(),
        }
    }

    /// Returns the configured EVM spec ID.
    #[inline]
    pub const fn spec_id(&self) -> SpecId {
        self.spec_id
    }

    /// Load access list for berlin hard fork.
    ///
    /// Loading of accounts/storages is needed to make them warm.
    #[inline]
    pub fn load_access_list(&mut self) -> Result<(), EVMError<ExitCode>> {
        for (address, _slots) in self.env.tx.access_list.iter() {
            Account::new_from_jzkt(address);
        }
        Ok(())
    }

    /// Return environment.
    #[inline]
    pub fn env(&mut self) -> &mut Env {
        &mut self.env
    }

    /// Handles call return.
    #[inline]
    pub fn call_return(
        &mut self,
        interpreter_result: &InterpreterResult,
        journal_checkpoint: AccountCheckpoint,
    ) {
        // revert changes or not.
        if matches!(interpreter_result.result, ExitCode::Ok) {
            // Account::commit();
        } else {
            Account::rollback(journal_checkpoint);
        }
    }

    /// Handles create return.
    #[inline]
    pub fn create_return<SPEC: Spec>(
        &mut self,
        interpreter_result: &mut InterpreterResult,
        address: Address,
        journal_checkpoint: AccountCheckpoint,
    ) {
        // if return is not ok revert and return.
        if !matches!(interpreter_result.result, ExitCode::Ok) {
            Account::rollback(journal_checkpoint);
            return;
        }
        // Host error if present on execution
        // if ok, check contract creation limit and calculate gas deduction on output len.
        //
        // EIP-3541: Reject new contract code starting with the 0xEF byte
        if SPEC::enabled(LONDON)
            && !interpreter_result.output.is_empty()
            && interpreter_result.output.first() == Some(&0xEF)
        {
            Account::rollback(journal_checkpoint);
            interpreter_result.result = ExitCode::CreateContractStartingWithEF;
            return;
        }

        // EIP-170: Contract code size limit
        // By default limit is 0x6000 (~25kb)
        if SPEC::enabled(SPURIOUS_DRAGON)
            && interpreter_result.output.len()
                > self
                    .env
                    .cfg
                    .limit_contract_code_size
                    .unwrap_or(RWASM_MAX_CODE_SIZE)
        {
            Account::rollback(journal_checkpoint);
            interpreter_result.result = ExitCode::ContractSizeLimit;
            return;
        }
        let gas_for_code = interpreter_result.output.len() as u64 * 200; // 200 is CODEDEPOSIT cost
        if !interpreter_result.gas.record_cost(gas_for_code) {
            // record code deposit gas cost and check if we are out of gas.
            // EIP-2 point 3: If contract creation does not have enough gas to pay for the
            // final gas fee for adding the contract code to the state, the contract
            //  creation fails (i.e. goes out-of-gas) rather than leaving an empty contract.
            if SPEC::enabled(HOMESTEAD) {
                Account::rollback(journal_checkpoint);
                interpreter_result.result = ExitCode::OutOfFuel;
                return;
            } else {
                interpreter_result.output = Bytes::new();
            }
        }
        // if we have enough gas we can commit changes.
        // Account::commit();

        // set code
        let mut contract = Account::new_from_jzkt(&address);
        contract.update_rwasm_bytecode(&interpreter_result.output);
        contract.write_to_jzkt();

        interpreter_result.result = ExitCode::Ok;
    }
}
