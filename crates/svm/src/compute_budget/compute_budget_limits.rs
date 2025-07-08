use solana_program_entrypoint::HEAP_LENGTH;

/// Roughly 0.5us/page, where page is 32K; given roughly 15CU/us, the
/// default heap page cost = 0.5 * 15 ~= 8CU/page
pub const DEFAULT_HEAP_COST: u64 = 8;
pub const DEFAULT_INSTRUCTION_COMPUTE_UNIT_LIMIT: u32 = 200_000;
/// SIMD-170 defines max CUs to be allocated for any builtin program instructions, that
/// have not been migrated to sBPF programs.
pub const MAX_BUILTIN_ALLOCATION_COMPUTE_UNIT_LIMIT: u32 = 3_000;
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
pub const MAX_HEAP_FRAME_BYTES: u32 = 256 * 1024;
pub const MIN_HEAP_FRAME_BYTES: u32 = HEAP_LENGTH as u32;
