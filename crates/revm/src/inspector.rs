use crate::RwasmFrame;
use fluentbase_sdk::U256;
use revm::{context::ContextTr, interpreter::Gas, Inspector};

// pub(crate) fn try_execute_rwasm_interruption_with_trace<CTX: ContextTr, INSP: Inspector<CTX>>(
//     frame: &mut RwasmFrame,
//     ctx: &mut CTX,
//     inspector: Option<&mut INSP>,
//     inputs: SystemInterruptionInputs,
// ) -> Result<NextAction, ContextError<<CTX::Db as Database>::Error>> {
//     let Some(inspector) = inspector else {
//         return execute_rwasm_interruption::<CTX>(frame, ctx, inputs);
//     };
//     let mut stack = Stack::new();
//     let evm_opcode = match &inputs.syscall_params.code_hash {
//         &syscall::SYSCALL_ID_STORAGE_READ => {
//             &inputs.syscall_params.input;
//             opcode::SLOAD
//         }
//         &syscall::SYSCALL_ID_STORAGE_WRITE => opcode::SSTORE,
//         &syscall::SYSCALL_ID_CALL => opcode::CALL,
//         &syscall::SYSCALL_ID_STATIC_CALL => opcode::STATICCALL,
//         &syscall::SYSCALL_ID_CALL_CODE => opcode::CALLCODE,
//         &syscall::SYSCALL_ID_DELEGATE_CALL => opcode::DELEGATECALL,
//         &syscall::SYSCALL_ID_CREATE => opcode::CREATE,
//         &syscall::SYSCALL_ID_CREATE2 => opcode::CREATE2,
//         &syscall::SYSCALL_ID_EMIT_LOG => opcode::LOG0,
//         &syscall::SYSCALL_ID_DESTROY_ACCOUNT => opcode::SELFDESTRUCT,
//         &syscall::SYSCALL_ID_BALANCE => opcode::BALANCE,
//         &syscall::SYSCALL_ID_SELF_BALANCE => opcode::SELFBALANCE,
//         &syscall::SYSCALL_ID_CODE_SIZE => opcode::EXTCODESIZE,
//         &syscall::SYSCALL_ID_CODE_HASH => opcode::EXTCODEHASH,
//         &syscall::SYSCALL_ID_CODE_COPY => opcode::EXTCODECOPY,
//         &syscall::SYSCALL_ID_TRANSIENT_READ => opcode::TLOAD,
//         &syscall::SYSCALL_ID_TRANSIENT_WRITE => opcode::TSTORE,
//         &syscall::SYSCALL_ID_BLOCK_HASH => opcode::BLOCKHASH,
//         _ => return execute_rwasm_interruption::<CTX>(frame, ctx, inputs),
//     };
//     frame.interpreter.gas = inputs.gas;
//     let prev_bytecode = frame.interpreter.bytecode.clone();
//     let prev_hash = frame.interpreter.bytecode.hash().clone();
//     let bytecode = Bytecode::Rwasm([evm_opcode].into());
//     frame.interpreter.bytecode = ExtBytecode::new(bytecode);
//     inspector.step(&mut frame.interpreter, ctx);
//     let result = execute_rwasm_interruption::<CTX>(frame, ctx, inputs)?;
//     inspector.step_end(&mut frame.interpreter, ctx);
//     if let Some(prev_hash) = prev_hash {
//         frame.interpreter.bytecode = ExtBytecode::new_with_hash(prev_bytecode, prev_hash);
//     } else {
//         frame.interpreter.bytecode = ExtBytecode::new(prev_bytecode);
//     }
//     Ok(result)
// }

pub(crate) fn inspect_syscall<CTX: ContextTr, INSP: Inspector<CTX>, IN: IntoIterator<Item = U256>>(
    _frame: &mut RwasmFrame,
    _ctx: &mut CTX,
    _inspector: &mut INSP,
    _evm_opcode: u8,
    _gas_limit: u64,
    _gas: Gas,
    _input: IN,
) where
    <IN as IntoIterator>::IntoIter: DoubleEndedIterator,
{
    // frame.interpreter.gas = Gas::new(gas_limit);
    // let prev_bytecode = frame.interpreter.bytecode.clone();
    // let prev_hash = frame.interpreter.bytecode.hash().clone();
    // let bytecode = Bytecode::Rwasm([evm_opcode].into());
    // frame.interpreter.bytecode = ExtBytecode::new(bytecode);
    // frame.interpreter.stack = Stack::new();
    // for x in input.into_iter().rev() {
    //     debug_assert!(frame.interpreter.stack.push(x));
    // }
    // inspector.step(&mut frame.interpreter, ctx);
    // frame.interpreter.stack.clear();
    // if let Some(prev_hash) = prev_hash {
    //     frame.interpreter.bytecode = ExtBytecode::new_with_hash(prev_bytecode, prev_hash);
    // } else {
    //     frame.interpreter.bytecode = ExtBytecode::new(prev_bytecode);
    // }
    // _ = frame.interpreter.gas.record_cost(gas.spent());
    // frame.interpreter.gas.record_refund(gas.refunded());
    // inspector.step_end(&mut frame.interpreter, ctx);
}
