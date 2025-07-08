use crate::{
    common::{MAX_CALL_DEPTH, MAX_INSTRUCTION_STACK_DEPTH, STACK_FRAME_SIZE},
    compute_budget_processor::MAX_COMPUTE_UNIT_LIMIT,
};
use solana_program_entrypoint::HEAP_LENGTH;

/// Roughly 0.5us/page, where page is 32K; given roughly 15CU/us, the
/// default heap page cost = 0.5 * 15 ~= 8CU/page
pub const DEFAULT_HEAP_COST: u64 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComputeBudget {
    /// Maximum program instruction invocation stack depth. Invocation stack
    /// depth starts at 1 for transaction instructions and the stack depth is
    /// incremented each time a program invokes an instruction and decremented
    /// when a program returns.
    pub max_instruction_stack_depth: usize,
    /// Maximum cross-program invocation and instructions per transaction
    pub max_instruction_trace_length: usize,
    /// Maximum number of slices hashed per syscall
    pub sha256_max_slices: u64,
    /// Maximum SBF to BPF call depth
    pub max_call_depth: usize,
    /// Size of a stack frame in bytes, must match the size specified in the LLVM SBF backend
    pub stack_frame_size: usize,
    /// Maximum cross-program invocation instruction size
    pub max_cpi_instruction_size: usize,
    /// Number of account data bytes per compute unit charged during a cross-program invocation
    pub cpi_bytes_per_unit: u64,
    /// program heap region size, default: solana_sdk::entrypoint::HEAP_LENGTH
    pub heap_size: u32,
}

impl Default for ComputeBudget {
    fn default() -> Self {
        Self::new(MAX_COMPUTE_UNIT_LIMIT as u64)
    }
}

impl ComputeBudget {
    pub fn new(_compute_unit_limit: u64) -> Self {
        ComputeBudget {
            max_instruction_stack_depth: MAX_INSTRUCTION_STACK_DEPTH,
            max_instruction_trace_length: 64,
            sha256_max_slices: 20_000,
            max_call_depth: MAX_CALL_DEPTH,
            stack_frame_size: STACK_FRAME_SIZE,
            max_cpi_instruction_size: 1280, // IPv6 Min MTU size
            cpi_bytes_per_unit: 250,        // ~50MB at 200,000 units
            heap_size: u32::try_from(HEAP_LENGTH).unwrap(),
        }
    }
}
