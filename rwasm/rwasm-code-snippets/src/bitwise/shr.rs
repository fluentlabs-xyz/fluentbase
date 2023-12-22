use crate::{
    common::shr,
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::U256_BYTES_COUNT,
};

#[no_mangle]
fn bitwise_shr(// v0: u64,
    // v1: u64,
    // v2: u64,
    // v3: u64,
    // shift0: u64,
    // shift1: u64,
    // shift2: u64,
    // shift3: u64,
) /* -> (u64, u64, u64, u64) */
{
    let val = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let shift = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let mut res = [0u8; U256_BYTES_COUNT as usize];

    let mut v = [0u8; 8];
    v.clone_from_slice(&shift[0..8]);
    let shift3: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&shift[8..16]);
    let shift2: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&shift[16..24]);
    let shift1: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&shift[24..32]);
    let shift0: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&val[0..8]);
    let v3: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&val[8..16]);
    let v2: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&val[16..24]);
    let v1: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&val[24..32]);
    let v0: u64 = u64::from_be_bytes(v);

    let r = shr(shift0, shift1, shift2, shift3, v0, v1, v2, v3);

    res[0..8].copy_from_slice(&r.3.to_be_bytes());
    res[8..16].copy_from_slice(&r.2.to_be_bytes());
    res[16..24].copy_from_slice(&r.1.to_be_bytes());
    res[24..32].copy_from_slice(&r.0.to_be_bytes());

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
