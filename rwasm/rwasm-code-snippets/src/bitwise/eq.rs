#[no_mangle]
fn bitwise_eq(
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    b0: u64,
    b1: u64,
    b2: u64,
    b3: u64,
) -> (u64, u64, u64, u64) {
    let s0 = 0;
    let s1 = 0;
    let s2 = 0;
    let s3;
    if a0 == b0 && a1 == b1 && a2 == b2 && a3 == b3 {
        s3 = 1;
    } else {
        s3 = 0;
    }

    return (s0, s1, s2, s3);
}
