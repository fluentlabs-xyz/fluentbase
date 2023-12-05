use core::slice::Chunks;

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
