#[no_mangle]
fn bitwise_not(a0: u64, a1: u64, a2: u64, a3: u64) -> (u64, u64, u64, u64) {
    return (!a0, !a1, !a2, !a3);
}
