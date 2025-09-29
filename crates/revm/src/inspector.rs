use crate::RwasmFrame;
use core::mem::take;
use fluentbase_sdk::U256;
use revm::{
    bytecode::Bytecode,
    context::ContextTr,
    interpreter::{interpreter::ExtBytecode, Stack},
    Inspector,
};

pub(crate) fn inspect_syscall<CTX: ContextTr, INSP: Inspector<CTX>, IN: IntoIterator<Item = U256>>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: &mut INSP,
    evm_opcode: u8,
    input: IN,
) where
    <IN as IntoIterator>::IntoIter: DoubleEndedIterator,
{
    // Save previous bytecode (we return it after)
    let prev_bytecode = take(&mut frame.interpreter.bytecode);
    // Execute inspector steps with modified bytecode and stack
    let bytecode = Bytecode::new_raw([evm_opcode].into());
    frame.interpreter.bytecode = ExtBytecode::new(bytecode);
    frame.interpreter.stack = Stack::new();
    for x in input.into_iter().rev() {
        _ = frame.interpreter.stack.push(x);
    }
    inspector.step(&mut frame.interpreter, ctx);
    // TODO: We should call `step_end` once instruction is over for proper gas & output stack calculation.
    inspector.step_end(&mut frame.interpreter, ctx);
    // Return original bytecode back
    frame.interpreter.bytecode = prev_bytecode;
}
