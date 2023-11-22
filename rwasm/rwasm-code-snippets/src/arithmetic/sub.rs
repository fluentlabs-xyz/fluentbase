#[no_mangle]
fn arithmetic_sub(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let a0_sign = a0 & 0x8000000000000000;

    let mut borrow = 0;
    let s0;
    let s1;
    let s2;
    let s3;

    if a3 >= b3 {
        s3 = a3 - b3;
    } else {
        s3 = 0xffffffffffffffff - b3 + a3 + 1;
        borrow = 1;
    }

    if a2 >= b2 + borrow {
        s2 = a2 - b2 - borrow;
        borrow = 0;
    } else {
        s2 = 0xffffffffffffffff - b2 + a2 + 1;
        borrow = 1;
    }

    if a1 >= b1 + borrow {
        s1 = a1 - b1 - borrow;
        borrow = 0;
    } else {
        s1 = 0xffffffffffffffff - b1 + a1 + 1;
        borrow = 1;
    }

    if a0 >= b0 + borrow {
        s0 = a0 - b0 - borrow;
        // borrowed = 0;
    } else {
        if a0_sign > 0 {
            s0 = 0xffffffffffffffff - b0 + a0 + 1;
            // borrowed = 1;
        } else {
            // TODO process overflow
            s0 = 0x8000000000000000;
        }
    }

    (s0, s1, s2, s3)
}
