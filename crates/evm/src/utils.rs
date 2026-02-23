//! Small helpers for building the interruption protocol and keeping gas in sync.

use crate::{host::HostWrapper, types::InterruptionExtension};
use core::{cell::Ref, ops::Range};
use once_cell::race::OnceBox;
use revm_context_interface::cfg::GasParams;
use revm_interpreter::{
    interpreter_types::{Jumps, LoopControl, MemoryTr},
    Host, InstructionContext, InterpreterAction, InterpreterTypes,
};
use revm_primitives::hardfork::SpecId;

pub fn evm_gas_params() -> &'static GasParams {
    static PRAGUE_GAS_PARAMS: OnceBox<GasParams> = OnceBox::new();
    PRAGUE_GAS_PARAMS.get_or_init(|| GasParams::new_spec(SpecId::PRAGUE).into())
}

/// Convert opcode handler logic into a SystemInterruption and set up re-dispatch.
pub(crate) fn interrupt_into_action<
    WIRE: InterpreterTypes<Extend = InterruptionExtension>,
    H: HostWrapper + ?Sized,
>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // We should repeat the previous instruction once we have enough data.
    // To achieve this, we jump back to this opcode PC.
    context.interpreter.bytecode.relative_jump(-1);
    context
        .interpreter
        .bytecode
        .set_action(InterpreterAction::SystemInterruption);
}

/// View a range of the interpreter’s shared memory as a global slice.
pub(crate) fn global_memory_from_shared_buffer<
    'a,
    WIRE: InterpreterTypes<Extend = InterruptionExtension>,
    H: Host + HostWrapper + ?Sized,
>(
    context: &'a InstructionContext<'_, H, WIRE>,
    in_range: Range<usize>,
) -> Ref<'a, [u8]> {
    if !in_range.is_empty() {
        context.interpreter.memory.global_slice(in_range)
    } else {
        context.interpreter.memory.global_slice(0..0)
    }
}

/// A special case for a 32-bit wasm machine, where we want to fail with proper error
/// code like `CreateInitCodeSizeLimit` or `OutOfOffset` instead of `OutOfGas`.
///
/// * ethereum is (usually) executed on a 64-bit machine where usize is 64-bit
/// * this contract executes on a wasm machine where usize is 32-bit
#[macro_export]
macro_rules! as_u64_or_fail {
    ($interpreter:expr, $v:expr) => {
        match $v.as_limbs() {
            x => {
                if (x[1] == 0) & (x[2] == 0) & (x[3] == 0) {
                    x[0]
                } else {
                    return $interpreter.halt(InstructionResult::InvalidOperandOOG);
                }
            }
        }
    };
}
