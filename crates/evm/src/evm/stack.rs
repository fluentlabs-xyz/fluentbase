use crate::{gas, EVM};
use fluentbase_sdk::{SharedAPI, U256};

pub fn pop<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    if let Err(result) = evm.stack.pop() {
        evm.state = result;
    }
}

/// EIP-3855: PUSH0 instruction
///
/// Introduce a new instruction which pushes the constant value 0 onto the stack.
pub fn push0<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::BASE);
    if let Err(result) = evm.stack.push(U256::ZERO) {
        evm.state = result;
    }
}

pub fn push<const N: usize, SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    // SAFETY: In analysis, we append trailing bytes to the bytecode so that this is safe to do
    // without bounds checking.
    let ip = evm.ip;
    if let Err(result) = evm
        .stack
        .push_slice(unsafe { core::slice::from_raw_parts(ip, N) })
    {
        evm.state = result;
        return;
    }
    evm.ip = unsafe { ip.add(N) };
}

pub fn dup<const N: usize, SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    if let Err(result) = evm.stack.dup(N) {
        evm.state = result;
    }
}

pub fn swap<const N: usize, SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    if let Err(result) = evm.stack.swap(N) {
        evm.state = result;
    }
}
