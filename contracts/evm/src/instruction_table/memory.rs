use crate::{as_usize_or_fail, evm::EVM, gas, gas_or_fail, pop, pop_top, push, resize_memory};
use core::cmp::max;
use fluentbase_sdk::{SharedAPI, U256};

pub fn mload<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, top);
    let offset = as_usize_or_fail!(evm, top);
    resize_memory!(evm, offset, 32);
    *top = evm.memory.get_u256(offset);
}

pub fn mstore<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop!(evm, offset, value);
    let offset = as_usize_or_fail!(evm, offset);
    resize_memory!(evm, offset, 32);
    evm.memory.set_u256(offset, value);
}

pub fn mstore8<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop!(evm, offset, value);
    let offset = as_usize_or_fail!(evm, offset);
    resize_memory!(evm, offset, 1);
    evm.memory.set_byte(offset, value.byte(0))
}

pub fn msize<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    push!(evm, U256::from(evm.memory.len()));
}

// EIP-5656: MCOPY - Memory copying instruction
pub fn mcopy<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop!(evm, dst, src, len);
    // into usize or fail
    let len = as_usize_or_fail!(evm, len);
    // deduce gas
    gas_or_fail!(evm, gas::verylowcopy_cost(len as u64));
    if len == 0 {
        return;
    }
    let dst = as_usize_or_fail!(evm, dst);
    let src = as_usize_or_fail!(evm, src);
    // resize memory
    resize_memory!(evm, max(dst, src), len);
    // copy memory in place
    evm.memory.copy(dst, src, len);
}
