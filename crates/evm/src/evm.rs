//! Minimal EVM driver wired for interruptible host calls.
//!
//! EthVM executes analyzed EVM bytecode and yields on host-bound opcodes
//! (calls, storage, logs, etc.). The surrounding runtime performs the
//! operation, and the VM resumes with identical EVM semantics and gas.
//!
//! # Why this interpreter is pinned to a single hardfork
//!
//! Everything here runs at `SpecId::OSAKA` unconditionally, and it is *not* a bug that the
//! chain's active fork is ignored: the delegated EVM is versioned by contract upgrade, not by
//! hardfork.
//!
//! This crate is compiled into the EVM runtime contract (`contracts/evm`), which lives at
//! `PRECOMPILE_EVM_RUNTIME` as ordinary rWASM genesis code. Its bytecode is replaced through
//! `contracts/runtime-upgrade`, so EVM semantics can be changed on a live chain in a forkless
//! manner — deploy a new runtime, and every account delegating to it changes behavior at once.
//! The deployed runtime version *is* the fork boundary.
//!
//! The consequence is deliberate and should not be "fixed": an opcode from a newer hardfork,
//! e.g. `CLZ` (EIP-7939, Osaka), executes even while the chain spec still declares an older
//! fork such as Prague. Fork conditions in the chain spec (`crates/node/src/chainspec.rs`) gate
//! the protocol and native REVM side; they intentionally do not reach into delegated bytecode.
//! Threading a `SpecId` through the shared context to gate opcodes here would reintroduce
//! hardfork coupling that the upgrade mechanism exists to avoid.
//!
//! When bumping this, bump [`crate::evm_gas_params`] to match — opcode availability and the gas
//! schedule must describe the same fork.
use crate::{
    bytecode::AnalyzedBytecode,
    host::HostWrapperImpl,
    opcodes::interruptable_instruction_table,
    types::{ExecutionResult, InterruptingInterpreter, InterruptionExtension},
};
use fluentbase_sdk::{Bytes, ContextReader, SystemAPI, FUEL_DENOM_RATE};
use revm_bytecode::Bytecode;
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
            Bytecode::new_analyzed(
                analyzed_bytecode.bytecode,
                analyzed_bytecode.len as usize,
                analyzed_bytecode.jump_table,
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
                // Intentionally pinned rather than read from the chain's active fork: this
                // runtime is upgraded as a contract, not activated by a hardfork. See the
                // module docs before "fixing" this to follow the chain spec.
                spec_id: SpecId::OSAKA,
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
    #[allow(clippy::never_loop)]
    pub fn run_the_loop<SDK: SystemAPI>(mut self, sdk: &mut SDK) -> ExecutionResult {
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
                InterpreterAction::SystemInterruption => {
                    unimplemented!(
                        "evm: system interruption is not yet supported in `run_the_loop` mode"
                    );
                    /*self.sync_evm_gas(sdk.sdk_mut());
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
                        });*/
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
        SDK: SystemAPI,
    {
        let mut sdk = HostWrapperImpl::wrap(sdk);
        self.interpreter.run_plain(instruction_table, &mut sdk)
    }

    /// Commit interpreter gas deltas to the host (fuel) and snapshot the state.
    pub fn sync_evm_gas<SDK: SystemAPI>(&mut self, sdk: &mut SDK) {
        let (gas, committed_gas) = (
            &self.interpreter.gas,
            &mut self.interpreter.extend.committed_gas,
        );
        let remaining_diff = committed_gas.remaining().saturating_sub(gas.remaining());
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
