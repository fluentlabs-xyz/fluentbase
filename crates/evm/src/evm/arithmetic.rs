use crate::{
    evm::i256::{i256_div, i256_mod},
    gas,
    gas_or_fail,
    pop_top,
    EVM,
};
use fluentbase_sdk::{SharedAPI, U256};

pub fn add<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = op1.wrapping_add(*op2);
}

pub fn mul<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::LOW);
    pop_top!(evm, op1, op2);
    *op2 = op1.wrapping_mul(*op2);
}

pub fn sub<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::VERY_LOW);
    pop_top!(evm, op1, op2);
    *op2 = op1.wrapping_sub(*op2);
}

pub fn div<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::LOW);
    pop_top!(evm, op1, op2);
    if !op2.is_zero() {
        *op2 = op1.wrapping_div(*op2);
    }
}

pub fn sdiv<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::LOW);
    pop_top!(evm, op1, op2);
    *op2 = i256_div(op1, *op2);
}

pub fn rem<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::LOW);
    pop_top!(evm, op1, op2);
    if !op2.is_zero() {
        *op2 = op1.wrapping_rem(*op2);
    }
}

pub fn smod<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::LOW);
    pop_top!(evm, op1, op2);
    *op2 = i256_mod(op1, *op2)
}

pub fn addmod<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::MID);
    pop_top!(evm, op1, op2, op3);
    *op3 = op1.add_mod(op2, *op3)
}

pub fn mulmod<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::MID);
    pop_top!(evm, op1, op2, op3);
    *op3 = op1.mul_mod(op2, *op3)
}

pub fn exp<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    pop_top!(evm, op1, op2);
    gas_or_fail!(evm, gas::exp_cost(*op2));
    *op2 = op1.pow(*op2);
}

/// Implements the `SIGNEXTEND` opcode as defined in the Ethereum Yellow Paper.
///
/// In the yellow paper `SIGNEXTEND` is defined to take two inputs, we will call them
/// `x` and `y`, and produce one output. The first `t` bits of the output (numbering from the
/// left, starting from 0) are equal to the `t`-th bit of `y`, where `t` is equal to
/// `256 - 8(x + 1)`. The remaining bits of the output are equal to the corresponding bits of `y`.
/// Note: if `x >= 32` then the output is equal to `y` since `t <= 0`. To efficiently implement
/// this algorithm in the case `x < 32` we do the following. Let `b` be equal to the `t`-th bit
/// of `y` and let `s = 255 - t = 8x + 7` (this is effectively the same index as `t`, but
/// numbering the bits from the right instead of the left). We can create a bit mask which is all
/// zeros up to and including the `t`-th bit, and all ones afterwards by computing the quantity
/// `2^s - 1`. We can use this mask to compute the output depending on the value of `b`.
/// If `b == 1` then the yellow paper says the output should be all ones up to
/// and including the `t`-th bit, followed by the remaining bits of `y`; this is equal to
/// `y | !mask` where `|` is the bitwise `OR` and `!` is bitwise negation. Similarly, if
/// `b == 0` then the yellow paper says the output should start with all zeros, then end with
/// bits from `b`; this is equal to `y & mask` where `&` is bitwise `AND`.
pub fn signextend<SDK: SharedAPI>(evm: &mut EVM<SDK>) {
    gas!(evm, gas::LOW);
    pop_top!(evm, ext, x);
    // For 31 we also don't need to do anything.
    if ext < U256::from(31) {
        let ext = ext.as_limbs()[0];
        let bit_index = (8 * ext + 7) as usize;
        let bit = x.bit(bit_index);
        let mask = (U256::from(1) << bit_index) - U256::from(1);
        *x = if bit { *x | !mask } else { *x & mask };
    }
}
