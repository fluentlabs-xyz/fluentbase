#[no_mangle]
fn bitwise_eq(a0: u64, a1: u64, a2: u64, a3: u64) -> (u64, u64, u64, u64) {
    let s0;
    if a0 == 0 && a1 == 0 && a2 == 0 && a3 == 0 {
        s0 = 1;
    } else {
        s0 = 0;
    }

    return (s0, 0, 0, 0);
}
