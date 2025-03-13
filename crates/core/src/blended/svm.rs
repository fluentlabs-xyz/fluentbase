use crate::blended::BlendedRuntime;
use alloc::{boxed::Box, sync::Arc, vec};
use fluentbase_sdk::{
    Account,
    Address,
    Bytes,
    ContractContext,
    ExitCode,
    SovereignAPI,
    TxContextReader,
};
use revm_interpreter::{CreateInputs, Gas, InstructionResult, InterpreterResult};
use solana_ee_core::{
    account::AccountSharedData,
    builtins::register_builtins,
    common::compile_accounts_for_tx_ctx,
    compute_budget::ComputeBudget,
    context::{InvokeContext, TransactionContext},
    error::TransactionError,
    feature_set::FeatureSet,
    loaded_programs::{LoadedProgram, LoadedProgramsForTxBatch, ProgramRuntimeEnvironments},
    message_processor::MessageProcessor,
    native_loader,
    system_processor,
    sysvar_cache::SysvarCache,
};
use solana_program::{
    bpf_loader,
    bpf_loader_upgradeable,
    clock::Clock,
    epoch_schedule::EpochSchedule,
    hash::Hash,
    message::{legacy, LegacyMessage, SanitizedMessage},
    pubkey::Pubkey,
    rent::Rent,
    system_program,
};
use solana_rbpf::{
    program::{BuiltinFunction, BuiltinProgram, FunctionRegistry},
    vm::Config,
};

pub enum SvmError {
    TransactionError(TransactionError),
    BincodeError(bincode::Error),
}

impl<SDK: SovereignAPI> BlendedRuntime<SDK> {
    fn init_config(&self) -> Config {
        Config {
            enable_instruction_tracing: false,
            reject_broken_elfs: true,
            sanitize_user_provided_values: true,
            ..Default::default()
        }
    }

    fn exec_svm_message(&mut self, message: &[u8]) -> Result<(), SvmError> {
        let message: legacy::Message =
            bincode::deserialize(message).map_err(|err| SvmError::BincodeError(err))?;
        let message: SanitizedMessage = SanitizedMessage::Legacy(LegacyMessage::new(message));

        let config = self.init_config();

        let tx_access_list = self.sdk.context().tx_access_list_addresses();

        let blockhash = Hash::default();

        let rent = Rent::free();

        let compute_budget = ComputeBudget::default();
        let mut sysvar_cache = SysvarCache::default();
        sysvar_cache.set_rent(rent.clone());
        sysvar_cache.set_clock(Clock::default());
        sysvar_cache.set_epoch_schedule(EpochSchedule::default());

        let system_program_id = system_program::id();
        let native_loader_id = native_loader::id();
        let bpf_loader_upgradeable_id = bpf_loader_upgradeable::id();
        let bpf_loader_id = bpf_loader::id();

        let pk_exec = Pubkey::from([8; 32]);
        let pk_9 = Pubkey::from([9; 32]);
        let (pk_program_data, _) =
            Pubkey::find_program_address(&[pk_exec.as_ref()], &bpf_loader_upgradeable_id);

        let mut new_accs = vec![
            (
                pk_exec.clone(),
                AccountSharedData::new(0, 0, &system_program_id),
            ),
            (
                pk_9.clone(),
                AccountSharedData::new(100, 0, &system_program_id),
            ),
            (
                pk_program_data,
                AccountSharedData::new(0, 0, &system_program_id),
            ),
        ];

        let pk_payer = Pubkey::new_unique();
        let account_payer = AccountSharedData::new(100, 0, &system_program_id);
        let pk_buffer = Pubkey::new_unique();
        let account_buffer = AccountSharedData::new(0, 0, &system_program_id);

        let program_signers = vec![&new_accs[0].0, &new_accs[1].0];

        let (accounts, working_accounts_count) = compile_accounts_for_tx_ctx(
            vec![(pk_payer, account_payer), (pk_buffer, account_buffer)],
            vec![
                (
                    system_program_id,
                    native_loader::create_loadable_account_for_test(
                        "system_program_id",
                        &native_loader_id,
                    ),
                ),
                (
                    bpf_loader_upgradeable_id,
                    native_loader::create_loadable_account_for_test(
                        "bpf_loader_upgradeable_id",
                        &native_loader_id,
                    ),
                ),
            ],
        );

        // TODO extract accounts

        let transaction_context = TransactionContext::new(accounts, rent.clone(), 10, 200);

        let mut function_registry =
            FunctionRegistry::<BuiltinFunction<InvokeContext<SDK>>>::default();
        register_builtins(&mut function_registry);
        let loader = Arc::new(BuiltinProgram::new_loader(config, function_registry));
        let mut programs_loaded_for_tx_batch =
            LoadedProgramsForTxBatch::partial_default2(ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            });
        programs_loaded_for_tx_batch.replenish(
            system_program_id,
            Arc::new(LoadedProgram::new_builtin(
                0,
                0,
                system_processor::Entrypoint::vm,
            )),
        );
        programs_loaded_for_tx_batch.replenish(
            bpf_loader_upgradeable_id,
            Arc::new(LoadedProgram::new_builtin(
                0,
                0,
                solana_ee_core::bpf_loader_upgradable::Entrypoint::vm,
            )),
        );
        let programs_modified_by_tx =
            LoadedProgramsForTxBatch::partial_default2(ProgramRuntimeEnvironments {
                program_runtime_v1: loader.clone(),
                program_runtime_v2: loader.clone(),
            });
        let mut invoke_context = InvokeContext::new(
            transaction_context,
            sysvar_cache.clone(),
            &self.sdk,
            compute_budget.clone(),
            programs_loaded_for_tx_batch,
            programs_modified_by_tx,
            Arc::new(FeatureSet::all_enabled()),
            blockhash,
            0,
        );

        let account_keys = invoke_context.get_accounts_keys();

        // TODO
        let program_indices = vec![
            vec![working_accounts_count],
            vec![working_accounts_count + 1],
        ];

        let result =
            MessageProcessor::process_message(&message, &program_indices, &mut invoke_context);
        match result {
            Ok(_) => Ok(()),
            Err(tx_err) => Err(SvmError::TransactionError(tx_err)),
        }
    }

    fn process_svm_error(svm_error: SvmError) -> (Bytes, i32) {
        match svm_error {
            SvmError::TransactionError(err) => (Bytes::new(), ExitCode::UnknownError.into_i32()),
            SvmError::BincodeError(err) => (Bytes::new(), ExitCode::UnknownError.into_i32()),
        }
    }

    fn process_svm_result(result: Result<(), SvmError>) -> (Bytes, i32) {
        match result {
            Ok(_) => (Bytes::new(), ExitCode::Ok.into_i32()),
            Err(err) => Self::process_svm_error(err),
        }
    }

    pub fn exec_svm_bytecode(
        &mut self,
        _context: ContractContext,
        _bytecode_account: &Account,
        input: Bytes,
        _gas: &mut Gas,
        _state: u32,
        _call_depth: u32,
    ) -> (Bytes, i32) {
        let result = self.exec_svm_message(input.as_ref());
        Self::process_svm_result(result)
    }

    pub fn deploy_svm_contract(
        &mut self,
        _target_address: Address,
        inputs: Box<CreateInputs>,
        mut gas: Gas,
        _call_depth: u32,
    ) -> InterpreterResult {
        let input = inputs.init_code;
        let result = self.exec_svm_message(input.as_ref());

        match result {
            Ok(_) => InterpreterResult {
                result: InstructionResult::Stop,
                output: Bytes::default(),
                gas,
            },
            Err(err) => match err {
                SvmError::TransactionError(_err2) => InterpreterResult {
                    result: InstructionResult::FatalExternalError,
                    output: Bytes::default(),
                    gas,
                },
                SvmError::BincodeError(_err2) => InterpreterResult {
                    result: InstructionResult::FatalExternalError,
                    output: Bytes::default(),
                    gas,
                },
            },
        }
    }
}
