use crate::{
    consts::SP_BASE_MEM_OFFSET_DEFAULT,
    translator::{
        instruction_result::InstructionResult,
        instructions::utilities::wasm_call,
        translator::Translator,
    },
};
use core::slice::Chunks;
use fluentbase_rwasm::rwasm::InstructionSet;
use fluentbase_types::{ExitCode, SysFuncIdx};

pub const EVM_WORD_BYTES: usize = 32;
pub const WASM_I64_BITS: usize = 64;
pub const WASM_I64_BYTES: usize = WASM_I64_BITS / 8;
pub const WASM_I64_IN_EVM_WORD_COUNT: usize = EVM_WORD_BYTES / WASM_I64_BYTES;

pub fn align_to_evm_word_array(
    data: &[u8],
    pad_left_or_right: bool,
) -> Result<[u8; EVM_WORD_BYTES], ()> {
    let data_len = data.len();
    if data_len > EVM_WORD_BYTES {
        return Err(());
    }
    if data_len == EVM_WORD_BYTES {
        let res = TryInto::<[u8; EVM_WORD_BYTES]>::try_into(data);
        if let Err(_) = res {
            return Err(());
        }
        return Ok(res.unwrap());
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

pub fn sp_get_value(is: &mut InstructionSet, apply_delta: Option<i64>) {
    load_i64_const(is, Some(SP_BASE_MEM_OFFSET_DEFAULT as u64));
    apply_delta_value_on_stack(is, apply_delta);
}

pub fn apply_delta_value_on_stack(is: &mut InstructionSet, v: Option<i64>) {
    if let Some(v) = v {
        if v != 0 {
            is.op_i64_const(v.abs() as u64);
            if v < 0 {
                is.op_i64_sub();
            } else {
                is.op_i64_add();
            }
        }
    }
}

pub fn sp_get_offset(is: &mut InstructionSet, apply_delta: Option<i64>) {
    is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT as u64);
    sp_get_value(is, None);
    is.op_i64_sub();
    apply_delta_value_on_stack(is, apply_delta);
}

pub fn sp_drop_u256_gen(count: u64) -> InstructionSet {
    let mut is = InstructionSet::new();
    is.op_i64_const(SP_BASE_MEM_OFFSET_DEFAULT as u64); // for store
    sp_get_value(&mut is, Some(-(EVM_WORD_BYTES as i64 * count as i64)));
    sp_set_value(&mut is, false, None);

    is
}

pub fn sp_drop_u256(is: &mut InstructionSet, count: u64) {
    let is_tmp = sp_drop_u256_gen(count);
    is.extend(&is_tmp);
}

pub fn stop_op_gen(translator: &mut Translator<'_>) {
    translator.instruction_result = InstructionResult::Stop;
    let is = translator.result_instruction_set_mut();
    is.op_return();
    is.op_unreachable();
}
pub fn invalid_op_gen(translator: &mut Translator<'_>) {
    translator.instruction_result = InstructionResult::InvalidFEOpcode;
    let is = translator.result_instruction_set_mut();
    is.op_i32_const(ExitCode::UnknownError as i32);
    wasm_call(translator, None, SysFuncIdx::SYS_HALT);
}
pub fn not_found_op_gen(translator: &mut Translator<'_>) {
    translator.instruction_result = InstructionResult::OpcodeNotFound;
}
