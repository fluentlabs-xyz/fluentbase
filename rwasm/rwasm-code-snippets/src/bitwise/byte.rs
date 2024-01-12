use crate::{
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};

#[no_mangle]
fn bitwise_byte() {
    let mut byte_index_arg = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let bytes = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let mut r = [0u8; U256_BYTES_COUNT as usize];

    let bi = byte_index_arg[byte_index_arg.len() - 1];
    if bi < U256_BYTES_COUNT as u8 {
        let mut is_zero = true;
        for i in 0..byte_index_arg.len() - 1 {
            if byte_index_arg[i] != 0 {
                is_zero = false;
                break;
            };
        }
        if is_zero {
            r[r.len() - 1] = bytes[bi as usize];
        }
    }

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, r);
}
