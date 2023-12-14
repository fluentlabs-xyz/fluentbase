use crate::common::mul_test;

// #[no_mangle]
// fn some_fn(a: u64, b: u64) -> u64 {
//     // let mut r = (0, 0, 0, 0);
//     // r.0 = a.0 | b.0;
//     a | b
// }

#[no_mangle]
pub fn arithmetic_exp(
    exp0: u64,
    exp1: u64,
    exp2: u64,
    exp3: u64,
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
) -> (u64, u64, u64, u64) {
    let mut r = (1, 1, 1, 1);
    // r.0 = add_test(r.0, exp0);
    // r.0 = add_test(r.0, v0);
    r = mul_test(r, (exp0, exp1, exp2, exp3));
    r = mul_test(r, (v0, v1, v2, v3));
    // exp(v0, v1, v2, v3, exp0, exp1, exp2, exp3)
    (r.0, r.1, r.2, r.3)
}
