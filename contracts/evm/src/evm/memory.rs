use core::cmp::max;
use fluentbase_sdk::SharedAPI;
use revm_interpreter::{
    as_usize_or_fail,
    gas,
    gas_or_fail,
    pop,
    pop_top,
    primitives::U256,
    push,
    resize_memory,
    Interpreter,
};

pub fn mload<SDK: SharedAPI>(interpreter: &mut Interpreter, _sdk: &mut SDK) {
    gas!(interpreter, gas::VERYLOW);
    pop_top!(interpreter, top);
    let offset = as_usize_or_fail!(interpreter, top);
    resize_memory!(interpreter, offset, 32);
    *top = interpreter.shared_memory.get_u256(offset);
}

pub fn mstore<SDK: SharedAPI>(interpreter: &mut Interpreter, _sdk: &mut SDK) {
    gas!(interpreter, gas::VERYLOW);
    pop!(interpreter, offset, value);
    let offset = as_usize_or_fail!(interpreter, offset);
    resize_memory!(interpreter, offset, 32);
    interpreter.shared_memory.set_u256(offset, value);
}

pub fn mstore8<SDK: SharedAPI>(interpreter: &mut Interpreter, _sdk: &mut SDK) {
    gas!(interpreter, gas::VERYLOW);
    pop!(interpreter, offset, value);
    let offset = as_usize_or_fail!(interpreter, offset);
    resize_memory!(interpreter, offset, 1);
    interpreter.shared_memory.set_byte(offset, value.byte(0))
}

pub fn msize<SDK: SharedAPI>(interpreter: &mut Interpreter, _sdk: &mut SDK) {
    gas!(interpreter, gas::BASE);
    push!(interpreter, U256::from(interpreter.shared_memory.len()));
}

// EIP-5656: MCOPY - Memory copying instruction
pub fn mcopy<SDK: SharedAPI>(interpreter: &mut Interpreter, _sdk: &mut SDK) {
    pop!(interpreter, dst, src, len);
    // into usize or fail
    let len = as_usize_or_fail!(interpreter, len);
    // deduce gas
    gas_or_fail!(interpreter, gas::verylowcopy_cost(len as u64));
    if len == 0 {
        return;
    }
    let dst = as_usize_or_fail!(interpreter, dst);
    let src = as_usize_or_fail!(interpreter, src);
    // resize memory
    resize_memory!(interpreter, max(dst, src), len);
    // copy memory in place
    interpreter.shared_memory.copy(dst, src, len);
}
