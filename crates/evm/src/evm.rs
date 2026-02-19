//! Minimal EVM driver wired for interruptible host calls.
//!
//! EthVM executes analyzed EVM bytecode and yields on host-bound opcodes
//! (calls, storage, logs, etc.). The surrounding runtime performs the
//! operation, and the VM resumes with identical EVM semantics and gas.
use crate::{
    bytecode::AnalyzedBytecode,
    host::{HostWrapper, HostWrapperImpl},
    opcodes::interruptable_instruction_table,
    types::{ExecutionResult, InterruptingInterpreter, InterruptionExtension, InterruptionOutcome},
};
use fluentbase_sdk::{Bytes, ContextReader, ExitCode, SharedAPI, FUEL_DENOM_RATE};
use revm_bytecode::{Bytecode, LegacyAnalyzedBytecode};
use revm_interpreter::{
    interpreter::{ExtBytecode, RuntimeFlags},
    CallInput, Gas, InputsImpl, InstructionTable, Interpreter, InterpreterAction, SharedMemory,
    Stack,
};
use revm_primitives::hardfork::SpecId;

/// EVM interpreter wrapper running with an interruption extension.
pub struct EthVM {
    pub interpreter: Interpreter<InterruptingInterpreter>,
}

unsafe impl Sync for EthVM {}
unsafe impl Send for EthVM {}

impl EthVM {
    /// Create a new VM instance bound to the given context and input.
    /// The bytecode must be pre-analyzed (jump table + hash preserved).
    pub fn new(
        context_input: impl ContextReader,
        input: Bytes,
        analyzed_bytecode: AnalyzedBytecode,
    ) -> Self {
        // Initialize context params and inputs
        let inputs_impl = InputsImpl {
            target_address: context_input.contract_address(),
            bytecode_address: Some(context_input.contract_bytecode_address()),
            caller_address: context_input.contract_caller(),
            input: CallInput::Bytes(input),
            call_value: context_input.contract_value(),
            account_owner: None,
        };
        let is_static = context_input.contract_is_static();
        let gas_limit = context_input.contract_gas_limit();
        // Initialize EVM bytecode and interpreter
        let bytecode = ExtBytecode::new_with_hash(
            Bytecode::LegacyAnalyzed(
                LegacyAnalyzedBytecode::new(
                    analyzed_bytecode.bytecode,
                    analyzed_bytecode.len,
                    analyzed_bytecode.jump_table,
                )
                .into(),
            ),
            analyzed_bytecode.hash,
        );
        let gas = Gas::new(gas_limit);
        let interpreter = Interpreter {
            bytecode,
            gas,
            stack: Stack::new(),
            return_data: Default::default(),
            memory: SharedMemory::new(),
            input: inputs_impl,
            runtime_flag: RuntimeFlags {
                is_static,
                spec_id: SpecId::PRAGUE,
            },
            extend: InterruptionExtension {
                interruption_outcome: None,
                committed_gas: gas,
            },
        };
        Self { interpreter }
    }

    /// Execute until completion, delegating host-bound ops via interruptions.
    /// Returns EVM result plus precise gas/fuel accounting.
    pub fn run_the_loop<SDK: SharedAPI>(mut self, sdk: &mut SDK) -> ExecutionResult {
        let instruction_table = interruptable_instruction_table();
        let mut sdk = HostWrapperImpl::wrap(sdk);
        loop {
            match self.interpreter.run_plain(&instruction_table, &mut sdk) {
                InterpreterAction::Return(result) => {
                    let committed_gas = self.interpreter.extend.committed_gas;
                    break ExecutionResult {
                        result: result.result,
                        output: result.output,
                        committed_gas,
                        gas: result.gas,
                    };
                }
                InterpreterAction::SystemInterruption {
                    code_hash,
                    input,
                    fuel_limit,
                    state,
                } => {
                    self.sync_evm_gas(sdk.sdk_mut());
                    let (fuel_consumed, fuel_refunded, exit_code) =
                        sdk.native_exec(code_hash, input.as_ref(), fuel_limit, state);
                    let mut gas = Gas::new_spent(fuel_consumed / FUEL_DENOM_RATE);
                    gas.record_refund(fuel_refunded / FUEL_DENOM_RATE as i64);
                    // Since the gas here is already synced,
                    // because it's been charged inside the call, we should put into committed
                    {
                        let dirty_gas = &mut self.interpreter.gas;
                        if !dirty_gas.record_cost(gas.spent()) {
                            unreachable!("evm: a fatal gas mis-sync between runtimes, this should never happen");
                        }
                        let committed_gas = &mut self.interpreter.extend.committed_gas;
                        if !committed_gas.record_cost(gas.spent()) {
                            unreachable!("evm: a fatal gas mis-sync between runtimes, this should never happen");
                        }
                    }
                    let output = sdk.return_data();
                    let exit_code = ExitCode::from(exit_code);
                    self.interpreter
                        .extend
                        .interruption_outcome
                        .replace(InterruptionOutcome {
                            output,
                            gas,
                            exit_code,
                            halted_frame: false,
                        });
                }
                InterpreterAction::NewFrame(_) => unreachable!("frames can't be produced"),
            }
        }
    }

    /// Executes 1 step of the interpreter run.
    /// Returns EVM result plus precise gas/fuel accounting.
    #[inline]
    pub fn run_step<'a, SDK>(
        &mut self,
        instruction_table: &InstructionTable<InterruptingInterpreter, HostWrapperImpl<'a, SDK>>,
        sdk: &'a mut SDK,
    ) -> InterpreterAction
    where
        SDK: SharedAPI,
    {
        let mut sdk = HostWrapperImpl::wrap(sdk);
        self.interpreter.run_plain(&instruction_table, &mut sdk)
    }

    /// Commit interpreter gas deltas to the host (fuel) and snapshot the state.
    pub fn sync_evm_gas<SDK: SharedAPI>(&mut self, sdk: &mut SDK) {
        let (gas, committed_gas) = (
            &self.interpreter.gas,
            &mut self.interpreter.extend.committed_gas,
        );
        let remaining_diff = committed_gas.remaining() - gas.remaining();
        // If there is nothing to commit/charge, then just ignore it
        if remaining_diff == 0 {
            return;
        }
        // Charge gas from the runtime
        sdk.charge_fuel(remaining_diff.saturating_mul(FUEL_DENOM_RATE));
        // Remember new committed gas
        *committed_gas = *gas;
    }
}
