use crate::ExecutionResult;
use alloy_primitives::Address;
use fluentbase_sdk::{SyscallInvocationParams, TESTNET_LEGACY_PRECOMPILE_ADDRESSES};
use revm::{
    interpreter::Gas,
    precompile::{PrecompileSpecId, Precompiles},
    primitives::hardfork::SpecId,
};

/// Returns `true` if `address` is part of the executor's system-precompile set.
///
/// P.S: We exclude Fluent system precompiles from this list since it may affect
///  future runtime upgrades and cause redundant forks, because EVM precompiles have
///  enforced empty account state.
pub(crate) fn is_evm_system_precompile(chain_id: u64, spec: SpecId, address: &Address) -> bool {
    // TODO(dmitry123): Remove testnet legacy precompiles once we have new snapshot
    if chain_id == 0x5202 {
        return TESTNET_LEGACY_PRECOMPILE_ADDRESSES.contains(address);
    }
    let precompiles = Precompiles::new(PrecompileSpecId::from_spec_id(spec));
    precompiles.contains(address)
}

/// A system interruption input params
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInterruptionInputs {
    /// The call identifier (used for recover).
    pub call_id: u32,
    /// Interruptions params (code hash, inputs, gas limits, etc.).
    pub syscall_params: SyscallInvocationParams,
    /// A gas snapshot assigned before the interruption.
    /// We need this to calculate the final amount of gas charged for the entire interruption.
    pub gas: Gas,
}

/// An interruption outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInterruptionOutcome {
    /// Original inputs.
    pub inputs: SystemInterruptionInputs,
    /// An interruption execution result.
    /// It can be empty for frame creation,
    /// where we don't know the result until the frame is executed.
    pub result: Option<ExecutionResult>,
    /// Indicates was the frame halted before execution.
    /// When we do CALL-like op we can halt execution during the frame creation, we
    /// should handle this to forward inside the system runtime to make sure all frames
    /// are terminated gracefully.
    pub halted_frame: bool,
}
