use crate::consts::SP_BASE_MEM_OFFSET_DEFAULT;
use core::slice::Chunks;
use fluentbase_rwasm::rwasm::InstructionSet;

pub const EVM_WORD_BYTES: usize = 32;
pub const WASM_I64_BITS: usize = 64;
pub const WASM_I64_BYTES: usize = WASM_I64_BITS / 8;
pub const WASM_I64_IN_EVM_WORD_COUNT: usize = EVM_WORD_BYTES / WASM_I64_BYTES;
pub const WASM_I64_HIGH_32_BIT_MASK: usize = 0xffffffff00000000;
pub const WASM_I64_LOW_32_BIT_MASK: usize = 0xffffffff;

pub fn align_to_evm_word_array(
    data: &[u8],
    pad_left_or_right: bool,
) -> Result<[u8; EVM_WORD_BYTES], ()> {
    let data_len = data.len();
    if data_len > EVM_WORD_BYTES {
        return Err(());
    }
    if data_len == EVM_WORD_BYTES {
        return Ok(data.try_into().unwrap());
    }
    let mut res = [0u8; EVM_WORD_BYTES];
    if pad_left_or_right {
        res[(EVM_WORD_BYTES - data_len)..].copy_from_slice(data);
    } else {
        res[..data_len].copy_from_slice(data);
    }

    Ok(res)
}

pub fn iterate_over_wasm_i64_chunks(data: &[u8; EVM_WORD_BYTES]) -> Chunks<u8> {
    data.chunks(WASM_I64_BYTES).into_iter()
}

pub fn store_i64_const(is: &mut InstructionSet, offset: Option<u64>, value: Option<u64>) {
    if let Some(offset) = offset {
        is.op_i64_const(offset);
    }
    if let Some(value) = value {
        is.op_i64_const(value);
    }
    is.op_i64_store(0);
}

pub fn load_i64_const(is: &mut InstructionSet, offset: Option<u64>) {
    if let Some(offset) = offset {
        is.op_i64_const(offset);
    }
    is.op_i64_load(0);
}

pub fn load_i64_const_be(is: &mut InstructionSet, offset: Option<u64>) {
    if let Some(offset) = offset {
        is.op_i64_const(offset);
    }
    is.op_local_get(1); // duplicate offset
    let mut pow: u64 = 72057594037927936; // 256**7
    for i in 0..WASM_I64_BYTES {
        if i > 0 {
            pow /= 256;
            is.op_local_get(2); // duplicate offset
            is.op_i64_const(i);
            is.op_i64_add();
        }
        is.op_i64_load8_u(0);
        is.op_i64_const(pow);
        is.op_i64_mul();
        if i > 0 {
            is.op_i64_add();
        }
    }
    is.op_local_set(1); // replace offset value left on stack with resulting value
}

pub fn sp_set_value(is: &mut InstructionSet, use_sp_base_offset: bool, value: Option<u64>) {
    store_i64_const(
        is,
        if use_sp_base_offset {
            Some(SP_BASE_MEM_OFFSET_DEFAULT as u64)
        } else {
            None
        },
        value,
    );
}

pub fn sp_get_value(is: &mut InstructionSet) {
    load_i64_const(is, Some(SP_BASE_MEM_OFFSET_DEFAULT as u64))
}

pub fn sp_get_offset(is: &mut InstructionSet) {
    is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT as u64);
    sp_get_value(is);
    is.op_i64_sub();
}

pub fn sp_drop_u256_gen(count: u64) -> InstructionSet {
    let mut is = InstructionSet::new();
    is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT as u64); // for store
    sp_get_value(&mut is);
    is.op_i64_const(EVM_WORD_BYTES as u64 * count);
    is.op_i64_sub();
    sp_set_value(&mut is, false, None);

    is
}

pub fn sp_drop_u256(is: &mut InstructionSet, count: u64) {
    let is_tmp = sp_drop_u256_gen(count);
    is.extend(&is_tmp);

    // is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT as u64); // for store
    // sp_get_value(is);
    // is.op_i64_const(EVM_WORD_BYTES as u64 * count);
    // is.op_i64_sub();
    // sp_set_value(is, false, None);
}
