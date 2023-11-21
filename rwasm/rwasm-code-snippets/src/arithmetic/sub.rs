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
    let b0_sign = b0 & 0x8000000000000000;

    if a0_sign > b0_sign {
        // -a - b = -(a+b)
    }
    if b0_sign > a0_sign {
        // a - -b = a+b
    }
    // a - b
    // -a + b = b-a

    (0, 0, 0, 0)
}
