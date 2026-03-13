use crate::RwasmFrame;
use core::mem::take;
use fluentbase_sdk::U256;
use revm::{
    bytecode::Bytecode,
    context::ContextTr,
    interpreter::{interpreter::ExtBytecode, Stack},
    Inspector,
};

pub(crate) fn inspect_syscall<
    CTX: ContextTr,
    INSP: Inspector<CTX>,
    IN: IntoIterator<Item = U256>,
    OUT: IntoIterator<Item = U256>,
>(
    frame: &mut RwasmFrame,
    ctx: &mut CTX,
    inspector: &mut INSP,
    evm_opcode: u8,
    input: IN,
    output: OUT,
) where
    <IN as IntoIterator>::IntoIter: DoubleEndedIterator,
    <OUT as IntoIterator>::IntoIter: DoubleEndedIterator,
{
    // Save previous state so synthetic syscall-inspection does not mutate the live frame.
    let prev_bytecode = take(&mut frame.interpreter.bytecode);
    let prev_stack = take(&mut frame.interpreter.stack);

    // Execute inspector steps with synthetic bytecode and input stack snapshot.
    let bytecode = Bytecode::new_raw([evm_opcode].into());
    frame.interpreter.bytecode = ExtBytecode::new(bytecode);
    frame.interpreter.stack = Stack::new();

    // EVM stack top must be the first popped argument, so we push in reverse.
    for x in input.into_iter().rev() {
        _ = frame.interpreter.stack.push(x);
    }

    inspector.step(&mut frame.interpreter, ctx);

    // Provide post-op stack shape for inspectors that persist `step_end` stack snapshots.
    frame.interpreter.stack = Stack::new();
    for x in output.into_iter().rev() {
        _ = frame.interpreter.stack.push(x);
    }

    // TODO: For CALL*/CREATE* opcodes this should ideally run once the child frame settles.
    inspector.step_end(&mut frame.interpreter, ctx);

    // Restore original interpreter state.
    frame.interpreter.bytecode = prev_bytecode;
    frame.interpreter.stack = prev_stack;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RwasmContext, RwasmFrame, RwasmSpecId};
    use revm::{
        bytecode::opcode,
        context::{BlockEnv, CfgEnv, ContextTr, TxEnv},
        database::InMemoryDB,
        interpreter::{interpreter_types::StackTr, Interpreter},
    };

    #[derive(Default)]
    struct RecordingInspector {
        step_opcode: Option<u8>,
        step_stack: Vec<U256>,
        step_end_stack: Vec<U256>,
    }

    impl<CTX: ContextTr> Inspector<CTX> for RecordingInspector {
        fn step(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
            self.step_opcode = Some(interp.bytecode.opcode());
            self.step_stack = interp.stack.data().to_vec();
        }

        fn step_end(&mut self, interp: &mut Interpreter, _context: &mut CTX) {
            self.step_end_stack = interp.stack.data().to_vec();
        }
    }

    #[test]
    fn inspect_syscall_restores_interpreter_state_and_preserves_io_stacks() {
        let mut ctx: RwasmContext<InMemoryDB> = RwasmContext::new(InMemoryDB::default(), RwasmSpecId::PRAGUE);
        ctx.cfg = CfgEnv::new_with_spec(RwasmSpecId::PRAGUE);
        ctx.block = BlockEnv::default();
        ctx.tx = TxEnv::default();

        let mut frame = RwasmFrame::default();
        _ = frame.interpreter.stack.push(U256::from(0xDEAD_BEEFu64));

        let mut inspector = RecordingInspector::default();
        let gas = U256::from(123_456u64);
        let target = U256::from(0xCAFEu64);

        inspect_syscall(
            &mut frame,
            &mut ctx,
            &mut inspector,
            opcode::DELEGATECALL,
            [gas, target, U256::ZERO, U256::ZERO, U256::ZERO, U256::ZERO],
            [U256::ONE],
        );

        assert_eq!(inspector.step_opcode, Some(opcode::DELEGATECALL));
        assert_eq!(inspector.step_stack.len(), 6);
        // Stack layout is bottom..top, so gas must be at the top (last element).
        assert_eq!(inspector.step_stack[5], gas);
        assert_eq!(inspector.step_stack[4], target);
        assert_eq!(inspector.step_end_stack, vec![U256::ONE]);

        // Original frame stack must be fully restored after synthetic inspection.
        assert_eq!(frame.interpreter.stack.data(), &[U256::from(0xDEAD_BEEFu64)]);
    }
}
