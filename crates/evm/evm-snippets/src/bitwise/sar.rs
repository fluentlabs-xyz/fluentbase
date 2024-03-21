use crate::{
    common_sp::{stack_pop_u256, stack_push_u256, SP_BASE_MEM_OFFSET_DEFAULT},
    consts::{BYTE_MAX_VAL, U256_BYTES_COUNT, U64_ALL_BITS_ARE_1, U64_MSBIT_IS_1},
};

#[no_mangle]
fn bitwise_sar() {
    let a = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);
    let b = stack_pop_u256(SP_BASE_MEM_OFFSET_DEFAULT);

    let mut res = [0u8; U256_BYTES_COUNT as usize];

    let mut v = [0u8; 8];

    v.clone_from_slice(&a[0..8]);
    let mut a3: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&a[8..16]);
    let mut a2: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&a[16..24]);
    let mut a1: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&a[24..32]);
    let mut a0: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&b[0..8]);
    let mut b3: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&b[8..16]);
    let mut b2: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&b[16..24]);
    let mut b1: u64 = u64::from_be_bytes(v);
    v.clone_from_slice(&b[24..32]);
    let mut b0: u64 = u64::from_be_bytes(v);

    let mut r = (0, 0, 0, 0);
    let b0_sign = b3 & U64_MSBIT_IS_1;

    if a3 != 0 || a2 != 0 || a1 != 0 || a0 > BYTE_MAX_VAL {
        if b0_sign > 0 {
            r = (
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
            );
        }
    } else if a0 >= 192 {
        let shift = a0 - 192;
        let shift_inv = 64 - shift;
        r.0 = b3 >> shift;
        if b0_sign > 0 {
            r = (
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                U64_ALL_BITS_ARE_1,
                r.0 | U64_ALL_BITS_ARE_1 << shift_inv,
            );
        }
    } else if a0 >= 128 {
        let shift = a0 - 128;
        let shift_inv = 64 - shift;
        r.1 = b3 >> shift;
        r.0 = b3 << shift_inv | b2 >> shift;
        if b0_sign > 0 {
            r.1 = r.1 | U64_ALL_BITS_ARE_1 << shift_inv;
            r = (U64_ALL_BITS_ARE_1, U64_ALL_BITS_ARE_1, r.1, r.0);
        }
    } else if a0 >= 64 {
        let shift = a0 - 64;
        let shift_inv = 64 - shift;
        r.2 = b3 >> shift;
        r.1 = b3 << shift_inv | b2 >> shift;
        r.0 = b2 << shift_inv | b1 >> shift;
        if b0_sign > 0 {
            r.2 = r.2 | U64_ALL_BITS_ARE_1 << shift_inv;
            r = (U64_ALL_BITS_ARE_1, r.2, r.1, r.0);
        }
    } else {
        let shift = a0;
        let shift_inv = 64 - shift;
        r.3 = b3 >> shift;
        r.2 = b3 << shift_inv | b2 >> shift;
        r.1 = b2 << shift_inv | b1 >> shift;
        r.0 = b1 << shift_inv | b0 >> shift;
        if b0_sign > 0 {
            r.3 = r.3 | U64_ALL_BITS_ARE_1 << shift_inv;
        }
    }

    res[0..8].copy_from_slice(&r.3.to_be_bytes());
    res[8..16].copy_from_slice(&r.2.to_be_bytes());
    res[16..24].copy_from_slice(&r.1.to_be_bytes());
    res[24..32].copy_from_slice(&r.0.to_be_bytes());

    stack_push_u256(SP_BASE_MEM_OFFSET_DEFAULT, res);
}
