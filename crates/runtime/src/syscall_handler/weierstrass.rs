mod weierstrass_double;
pub use weierstrass_double::*;
mod weierstrass_add;
pub use weierstrass_add::*;
mod weierstrass_decompress;
pub use weierstrass_decompress::*;

#[cfg(test)]
mod tests {
    use super::{syscall_secp256k1_add_impl, syscall_secp256k1_double_impl};
    use sp1_curves::{params::FieldParameters, weierstrass::secp256k1::Secp256k1BaseField};

    #[test]
    fn secp256k1_off_curve_inputs_preserve_arithmetic_without_panicking() {
        // (0, 1) and (1, 1) are reduced field coordinates, but neither satisfies
        // y^2 = x^3 + 7. These raw syscalls currently permit off-curve inputs.
        let mut p = [0u8; 64];
        p[32] = 1;
        let mut q = p;
        q[0] = 1;

        let minus_one = (Secp256k1BaseField::modulus() - 1u32).to_bytes_le();
        let mut expected_sum = [0u8; 64];
        expected_sum[..32].copy_from_slice(&minus_one);
        expected_sum[32..].copy_from_slice(&minus_one);
        assert_eq!(syscall_secp256k1_add_impl(p, q), Ok(expected_sum));

        let mut expected_double = [0u8; 64];
        expected_double[32..].copy_from_slice(&minus_one);
        assert_eq!(syscall_secp256k1_double_impl(p), Ok(expected_double));
    }
}
