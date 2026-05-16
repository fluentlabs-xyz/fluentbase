use crate::RwasmFrame;
use core::mem::{replace, take};
use fluentbase_sdk::U256;
use revm::{
    bytecode::Bytecode,
    context::ContextTr,
    interpreter::{interpreter::ExtBytecode, Stack},
    Inspector,
};

pub(crate) struct SyscallInspectionState {
    prev_bytecode: ExtBytecode,
    prev_stack: Stack,
}

pub(crate) fn inspect_syscall_start<
    CTX: ContextTr,
    INSP: Inspector<CTX>,
    IN: IntoIterator<Item = U256>,
>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: &mut INSP,
    evm_opcode: u8,
    input: IN,
) -> SyscallInspectionState
where
    <IN as IntoIterator>::IntoIter: DoubleEndedIterator,
{
    let prev_bytecode = take(&mut frame.interpreter.bytecode);
    let prev_stack = replace(&mut frame.interpreter.stack, Stack::new());

    let bytecode = Bytecode::new_raw([evm_opcode].into());
    frame.interpreter.bytecode = ExtBytecode::new(bytecode);
    for x in input.into_iter().rev() {
        _ = frame.interpreter.stack.push(x);
    }
    inspector.step(&mut frame.interpreter, ctx);

    SyscallInspectionState {
        prev_bytecode,
        prev_stack,
    }
}

pub(crate) fn inspect_syscall_end<CTX: ContextTr, INSP: Inspector<CTX>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: &mut INSP,
    state: SyscallInspectionState,
) {
    inspector.step_end(&mut frame.interpreter, ctx);
    frame.interpreter.bytecode = state.prev_bytecode;
    frame.interpreter.stack = state.prev_stack;
}

pub(crate) fn inspect_syscall<CTX: ContextTr, INSP: Inspector<CTX>, IN: IntoIterator<Item = U256>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: &mut INSP,
    evm_opcode: u8,
    input: IN,
) where
    <IN as IntoIterator>::IntoIter: DoubleEndedIterator,
{
    let state = inspect_syscall_start(frame, ctx, inspector, evm_opcode, input);
    inspect_syscall_end(frame, ctx, inspector, state);
}
