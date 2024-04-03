use crate::types::{CreateInputs, Gas, InterpreterResult, SStoreResult, SelfDestructResult};
use crate::{
    db::Database,
    primitives::{
        keccak256, Address, AnalysisKind, Bytecode, Bytes, CreateScheme, EVMError, Env, HashSet,
        Spec,
        SpecId::{self, *},
        B256, U256,
    },
    FrameOrResult, CALL_STACK_LIMIT,
};
use fluentbase_core::{Account, AccountCheckpoint};
use fluentbase_types::ExitCode;
use revm_primitives::{MAX_CODE_SIZE, RWASM_MAX_CODE_SIZE};
use std::boxed::Box;

/// EVM contexts contains data that EVM needs for execution.
#[derive(Debug)]
pub struct InnerEvmContext<DB: Database> {
    /// EVM Environment contains all the information about config, block and transaction that
    /// evm needs.
    pub env: Box<Env>,
    /// Database to load data from.
    pub db: DB,
    /// Error that happened during execution.
    pub error: Result<(), EVMError<DB::Error>>,
    /// Current recursion depth
    pub depth: u64,
    /// Spec id
    pub spec_id: SpecId,
}

impl<DB: Database + Clone> Clone for InnerEvmContext<DB>
where
    DB::Error: Clone,
{
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

impl<DB: Database> InnerEvmContext<DB> {
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
    pub fn with_db<ODB: Database>(self, db: ODB) -> InnerEvmContext<ODB> {
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
    pub fn load_access_list(&mut self) -> Result<(), EVMError<DB::Error>> {
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

    /// Fetch block hash from database.
    #[inline]
    pub fn block_hash(&mut self, number: U256) -> Result<B256, EVMError<DB::Error>> {
        self.db.block_hash(number).map_err(EVMError::Database)
    }

    /// Mark account as touched as only touched accounts will be added to state.
    #[inline]
    pub fn touch(&mut self, address: &Address) {
        todo!("do we need this func?")
    }

    /// Loads an account into memory. Returns `true` if it is cold accessed.
    #[inline]
    pub fn load_account(
        &mut self,
        address: Address,
    ) -> Result<(&mut Account, bool), EVMError<DB::Error>> {
        todo!("hmm")
    }

    /// Load account from database to JournaledState.
    ///
    /// Return boolean pair where first is `is_cold` second bool `exists`.
    #[inline]
    pub fn load_account_exist(
        &mut self,
        address: Address,
    ) -> Result<(bool, bool), EVMError<DB::Error>> {
        todo!("do we need this func?")
    }

    /// Return account balance and is_cold flag.
    #[inline]
    pub fn balance(&mut self, address: Address) -> Result<(U256, bool), EVMError<DB::Error>> {
        todo!("do we need this func?")
    }

    /// Return account code and if address is cold loaded.
    #[inline]
    pub fn code(&mut self, address: Address) -> Result<(Bytecode, bool), EVMError<DB::Error>> {
        todo!("do we need this func?")
    }

    /// Get code hash of address.
    #[inline]
    pub fn code_hash(&mut self, address: Address) -> Result<(B256, bool), EVMError<DB::Error>> {
        todo!("do we need this func?")
    }

    /// Load storage slot, if storage is not present inside the account then it will be loaded from database.
    #[inline]
    pub fn sload(
        &mut self,
        address: Address,
        index: U256,
    ) -> Result<(U256, bool), EVMError<DB::Error>> {
        todo!("do we need this func?")
    }

    /// Storage change of storage slot, before storing `sload` will be called for that slot.
    #[inline]
    pub fn sstore(
        &mut self,
        address: Address,
        index: U256,
        value: U256,
    ) -> Result<SStoreResult, EVMError<DB::Error>> {
        todo!("do we need this func?")
    }

    /// Returns transient storage value.
    #[inline]
    pub fn tload(&mut self, address: Address, index: U256) -> U256 {
        todo!("do we need this func?")
    }

    /// Stores transient storage value.
    #[inline]
    pub fn tstore(&mut self, address: Address, index: U256, value: U256) {
        todo!("do we need this func?")
    }

    /// Selfdestructs the account.
    #[inline]
    pub fn selfdestruct(
        &mut self,
        address: Address,
        target: Address,
    ) -> Result<SelfDestructResult, EVMError<DB::Error>> {
        todo!("do we need this func?")
    }

    /// Make create frame.
    #[inline]
    pub fn make_create_frame(
        &mut self,
        spec_id: SpecId,
        inputs: &CreateInputs,
    ) -> Result<FrameOrResult, EVMError<DB::Error>> {
        // Prepare crate.
        let gas = Gas::new(inputs.gas_limit);

        let return_error = |e| {
            Ok(FrameOrResult::new_create_result(
                InterpreterResult {
                    result: e,
                    gas,
                    output: Bytes::new(),
                },
                None,
            ))
        };

        // Check depth
        if self.depth > CALL_STACK_LIMIT {
            return return_error(ExitCode::CallDepthOverflow);
        }

        // Fetch balance of caller.
        let mut caller = Account::new_from_jzkt(&inputs.caller);

        // Check if caller has enough balance to send to the created contract.
        if caller.balance < inputs.value {
            return return_error(ExitCode::InsufficientBalance);
        }

        // Increase nonce of caller and check if it overflows
        let old_nonce = caller.inc_nonce()?;

        // Create address
        let mut init_code_hash = B256::ZERO;
        let created_address = match inputs.scheme {
            CreateScheme::Create => inputs.caller.create(old_nonce),
            CreateScheme::Create2 { salt } => {
                init_code_hash = keccak256(&inputs.init_code);
                inputs.caller.create2(salt.to_be_bytes(), init_code_hash)
            }
        };
        let mut callee = Account::new_from_jzkt(&created_address);

        // Load account so it needs to be marked as warm for access list.
        // TODO: "how can we load created account?"

        let checkpoint = Account::checkpoint();

        // create account, transfer funds and make the journal checkpoint.
        match Account::create_account(&mut caller, &mut callee, inputs.value) {
            Ok(_) => {}
            Err(err) => return return_error(err),
        }

        let bytecode = Bytecode::new_raw(inputs.init_code.clone());

        let contract = Box::new(Contract::new(
            Bytes::new(),
            bytecode,
            init_code_hash,
            created_address,
            inputs.caller,
            inputs.value,
        ));

        Ok(FrameOrResult::new_create_frame(created_address, checkpoint))
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
            Account::commit();
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
        Account::commit();

        // set code
        let mut contract = Account::new_from_jzkt(&address);
        contract.update_rwasm_bytecode(&interpreter_result.output);
        contract.write_to_jzkt();

        interpreter_result.result = ExitCode::Ok;
    }
}
