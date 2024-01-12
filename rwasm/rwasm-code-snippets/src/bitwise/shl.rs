use crate::{
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::{BYTE_MAX_VAL, U256_BYTES_COUNT},
};

#[no_mangle]
fn bitwise_shl() {
    let a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let mut res = [0u8; U256_BYTES_COUNT as usize];

    let mut v = [0u8; 8];
    v.clone_from_slice(&a[0..8]);
    let a3: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&a[8..16]);
    let a2: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&a[16..24]);
    let a1: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&a[24..32]);
    let a0: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&b[0..8]);
    let b3: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&b[8..16]);
    let b2: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&b[16..24]);
    let b1: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&b[24..32]);
    let b0: u64 = u64::from_be_bytes(v);

    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    if a3 != 0 || a2 != 0 || a1 != 0 || a0 > BYTE_MAX_VAL {
    } else if a0 >= 192 {
        let shift = a0 - 192;
        s3 = b0 << shift;
    } else if a0 >= 128 {
        let shift = a0 - 128;
        let shift_inv = 64 - shift;
        s2 = b0 << shift;
        s3 = b1 << shift | b0 >> shift_inv;
    } else if a0 >= 64 {
        let shift = a0 - 64;
        let shift_inv = 64 - shift;
        s1 = b0 << shift;
        s2 = b1 << shift | b0 >> shift_inv;
        s3 = b2 << shift | b1 >> shift_inv;
    } else {
        let shift = a0;
        let shift_inv = 64 - shift;
        s0 = b0 << shift;
        s1 = b1 << shift | b0 >> shift_inv;
        s2 = b2 << shift | b1 >> shift_inv;
        s3 = b3 << shift | b2 >> shift_inv;
    }

    let r = (s0, s1, s2, s3);

    res[0..8].copy_from_slice(&r.3.to_be_bytes());
    res[8..16].copy_from_slice(&r.2.to_be_bytes());
    res[16..24].copy_from_slice(&r.1.to_be_bytes());
    res[24..32].copy_from_slice(&r.0.to_be_bytes());

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
