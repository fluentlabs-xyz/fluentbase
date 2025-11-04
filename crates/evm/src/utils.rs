//! Small helpers for building the interruption protocol and keeping gas in sync.
use crate::{host::HostWrapper, types::InterruptionExtension};
use core::{cell::Ref, ops::Range};
use fluentbase_sdk::InterruptionExtractingAdapter;
use revm_interpreter::{
    interpreter_types::{Jumps, LoopControl, MemoryTr},
    Host, InstructionContext, InterpreterAction, InterpreterTypes,
};

/// Convert opcode handler logic into a SystemInterruption and set up re-dispatch.
pub(crate) fn interrupt_into_action<
    WIRE: InterpreterTypes<Extend = InterruptionExtension>,
    H: Host + ?Sized,
    F: FnOnce(&InstructionContext<'_, H, WIRE>, &mut InterruptionExtractingAdapter) -> (u64, i64, i32),
>(
    context: InstructionContext<'_, H, WIRE>,
    f: F,
) {
    // TODO(dmitry123): Is there a better way to extract interruption details?
    //  What to do with serialization overhead?
    let mut sdk = InterruptionExtractingAdapter::default();
    f(&context, &mut sdk);
    // We use the adapter to extract interruption data only.
    // Maybe there is an easier way to do this,
    // but we wanted to avoid code duplicates, especially related to syscall input/output data.
    let data = sdk.extract();
    let action = InterpreterAction::SystemInterruption {
        code_hash: data.code_hash,
        input: data.input,
        fuel_limit: data.fuel_limit,
        state: data.state,
    };
    // We should repeat previous instruction once we have enough data.
    // To achieve this, we jump back to this opcode PC.
    context.interpreter.bytecode.relative_jump(-1);
    context.interpreter.bytecode.set_action(action);
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
