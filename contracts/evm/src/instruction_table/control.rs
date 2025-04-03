use crate::{
    as_usize_or_fail,
    evm::{
        result::{InstructionResult, InterpreterResult},
        EVM,
    },
    gas,
    pop,
    push,
    resize_memory,
};
use fluentbase_sdk::{Bytes, SharedAPI, U256};

pub fn jump<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::MID);
    pop!(evm, target);
    jump_inner(evm, target);
}

pub fn jumpi<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::HIGH);
    pop!(evm, target, cond);
    if !cond.is_zero() {
        jump_inner(evm, target);
    }
}

#[inline]
fn jump_inner<SDK: SharedAPI>(evm: &mut EVM<SDK>, target: U256) {
    let target = as_usize_or_fail!(evm, target, InstructionResult::InvalidJump);
    if !evm.analyzed_bytecode.jump_table.is_valid(target) {
        evm.state = InstructionResult::InvalidJump;
        return;
    }
    // SAFETY: `is_valid_jump` ensures that `dest` is in bounds.
    evm.ip = unsafe { evm.analyzed_bytecode.bytecode.as_ptr().add(target) };
}

pub fn jumpdest_or_nop<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::JUMPDEST);
}

pub fn pc<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    // - 1 because we have already advanced the instruction pointer in `Interpreter::step`
    push!(evm, U256::from(evm.program_counter() - 1));
}

#[inline]
fn return_inner<SDK: SharedAPI>(evm: &mut EVM<SDK>, result: InstructionResult) {
    // zero gas cost
    // gas!(interpreter, gas::ZERO);
    pop!(evm, offset, len);
    let len = as_usize_or_fail!(evm, len);
    // important: offset must be ignored if len is zeros
    let mut output = Bytes::default();
    if len != 0 {
        let offset = as_usize_or_fail!(evm, offset);
        resize_memory!(evm, offset, len);
        output = evm.memory.slice(offset, len).to_vec().into()
    }
    evm.state = result;
    evm.output = Some(InterpreterResult {
        output,
        gas: evm.gas,
        result,
    });
}

pub fn ret<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    return_inner(evm, InstructionResult::Return);
}

/// EIP-140: REVERT instruction
pub fn revert<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    return_inner(evm, InstructionResult::Revert);
}

/// Stop opcode. This opcode halts the execution.
pub fn stop<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    evm.state = InstructionResult::Stop;
}

/// Invalid opcode. This opcode halts the execution.
pub fn invalid<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    evm.state = InstructionResult::InvalidFEOpcode;
}

/// Unknown opcode. This opcode halts the execution.
pub fn unknown<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    evm.state = InstructionResult::OpcodeNotFound;
}
