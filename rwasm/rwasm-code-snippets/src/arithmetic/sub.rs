use crate::{
    common::{u256_be_to_tuple_le, u256_tuple_le_to_be},
    common_sp::{u256_pop, u256_push, SP_VAL_MEM_OFFSET_DEFAULT},
    consts::U64_MAX_VAL,
};

#[no_mangle]
pub fn arithmetic_sub() {
    let b = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);
    let a = u256_pop(SP_VAL_MEM_OFFSET_DEFAULT);

    let a = u256_be_to_tuple_le(a);
    let b = u256_be_to_tuple_le(b);

    let mut borrow: u64 = 0;
    let mut s0: u64 = 0;
    let mut s1: u64 = 0;
    let mut s2: u64 = 0;
    let mut s3: u64 = 0;

    if a.0 >= b.0 {
        s0 = a.0 - b.0;
    } else {
        s0 = U64_MAX_VAL - b.0 + a.0 + (1 - borrow);
        borrow = 1;
    }

    if a.1 > b.1 || (a.1 >= b.1) && borrow <= 0 {
        s1 = a.1 - b.1 - borrow;
        borrow = 0;
    } else {
        s1 = U64_MAX_VAL - b.1 + a.1 + (1 - borrow);
        borrow = 1;
    }

    if a.2 > b.2 || (a.2 >= b.2) && borrow <= 0 {
        s2 = a.2 - b.2 - borrow;
        borrow = 0;
    } else {
        s2 = U64_MAX_VAL - b.2 + a.2 + (1 - borrow);
        borrow = 1;
    }

    if a.3 > b.3 || (a.3 >= b.3) && borrow <= 0 {
        s3 = a.3 - b.3 - borrow;
    } else {
        s3 = U64_MAX_VAL - b.3 + a.3 + (1 - borrow);
    }

    let r = (s0, s1, s2, s3);

    let res = u256_tuple_le_to_be(r);

    u256_push(SP_VAL_MEM_OFFSET_DEFAULT, res);
}
