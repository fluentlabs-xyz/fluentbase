/// This file is fully copied from SP1's core executor: sp1/crates/core/executor/src/hook.rs
///
/// But with some modifications:
/// 1. We don't support deprecated SP1 hooks
/// 2. We replace Vec<Vec<u8>> with Vec<u8> to return linear data
/// 3. Instead of panic, we return `ExitCode::MalformedBuiltinParams`
///
/// The rest here we kept as is w/o any modifications.
/// P.S: Because of changes we applied upper we're not able to reuse their crate,
///  also it requires having `HookEnv` context we don't have
///
use crate::{syscall_handler::syscall_process_exit_code, RuntimeContext};
use fluentbase_types::{
    fd::{
        FD_BLS12_381_INVERSE, FD_BLS12_381_SQRT, FD_ECRECOVER_HOOK, FD_ED_DECOMPRESS, FD_FP_INV,
        FD_FP_SQRT, FD_RSA_MUL_MOD,
    },
    ExitCode,
};
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{
    edwards::ed25519::{ed25519_sqrt, Ed25519BaseField},
    params::FieldParameters,
    BigUint, Integer, One,
};

pub fn syscall_write_fd_handler(
    caller: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let (fd, slice_ptr, slice_len) = (
        params[0].i32().unwrap() as u32,
        params[1].i32().unwrap() as u32,
        params[2].i32().unwrap() as u32,
    );
    let mut input = vec![0u8; slice_len as usize];
    caller.memory_read(slice_ptr as usize, &mut input)?;
    caller
        .context_mut(|ctx| syscall_write_fd_impl(ctx, fd, &input))
        .map_err(|err| syscall_process_exit_code(caller, err))?;
    Ok(())
}

pub fn syscall_write_fd_impl(
    ctx: &mut RuntimeContext,
    fd: u32,
    input: &[u8],
) -> Result<(), ExitCode> {
    let output = match fd {
        FD_ECRECOVER_HOOK => hook_ecrecover(input),
        FD_ED_DECOMPRESS => hook_ed_decompress(input),
        FD_RSA_MUL_MOD => hook_rsa_mul_mod(input),
        FD_BLS12_381_SQRT => bls::hook_bls12_381_sqrt(input),
        FD_BLS12_381_INVERSE => bls::hook_bls12_381_inverse(input),
        FD_FP_SQRT => fp_ops::hook_fp_sqrt(input),
        FD_FP_INV => fp_ops::hook_fp_inverse(input),
        _ => return Ok(()),
    }?;
    ctx.execution_result.return_data = output;
    Ok(())
}

/// The hook for the `ecrecover` patches.
///
/// The input should be of the form [(`curve_id_u8` | `r_is_y_odd_u8` << 7) || `r` || `alpha`]
/// where:
/// * `curve_id` is 1 for secp256k1 and 2 for secp256r1
/// * `r_is_y_odd` is 0 if r is even and 1 if r is is odd
/// * r is the x-coordinate of the point, which should be 32 bytes,
/// * alpha := r * r * r * (a * r) + b, which should be 32 bytes.
///
/// Returns vec![vec![1], `y`, `r_inv`] if the point is decompressable
/// and vec![vec![0],`nqr_hint`] if not.
fn hook_ecrecover(buf: &[u8]) -> Result<Vec<u8>, ExitCode> {
    if buf.len() != 65 {
        return Err(ExitCode::MalformedBuiltinParams);
    }

    let curve_id = buf[0] & 0b0111_1111;
    let r_is_y_odd = buf[0] & 0b1000_0000 != 0;

    let r_bytes: [u8; 32] = buf[1..33].try_into().unwrap();
    let alpha_bytes: [u8; 32] = buf[33..65].try_into().unwrap();

    Ok(match curve_id {
        1 => ecrecover::handle_secp256k1(r_bytes, alpha_bytes, r_is_y_odd),
        2 => ecrecover::handle_secp256r1(r_bytes, alpha_bytes, r_is_y_odd),
        _ => unimplemented!("Unsupported curve id: {}", curve_id),
    })
}

mod ecrecover {
    use sp1_curves::{k256, p256};

    /// The non-quadratic residue for the curve for secp256k1 and secp256r1.
    const NQR: [u8; 32] = {
        let mut nqr = [0; 32];
        nqr[31] = 3;
        nqr
    };

    pub(super) fn handle_secp256k1(r: [u8; 32], alpha: [u8; 32], r_y_is_odd: bool) -> Vec<u8> {
        use k256::{
            elliptic_curve::ff::PrimeField, FieldElement as K256FieldElement, Scalar as K256Scalar,
        };

        let r = K256FieldElement::from_bytes(r.as_ref().into()).unwrap();
        debug_assert!(!bool::from(r.is_zero()), "r should not be zero");

        let alpha = K256FieldElement::from_bytes(alpha.as_ref().into()).unwrap();
        assert!(!bool::from(alpha.is_zero()), "alpha should not be zero");

        // nomralize the y-coordinate always to be consistent.
        if let Some(mut y_coord) = alpha.sqrt().into_option().map(|y| y.normalize()) {
            let r = K256Scalar::from_repr(r.to_bytes()).unwrap();
            let r_inv = r.invert().expect("Non zero r scalar");

            if r_y_is_odd != bool::from(y_coord.is_odd()) {
                y_coord = y_coord.negate(1);
                y_coord = y_coord.normalize();
            }

            let mut result = vec![0x1];
            result.copy_from_slice(&*y_coord.to_bytes());
            result.copy_from_slice(&*r_inv.to_bytes());
            result
        } else {
            let nqr_field = K256FieldElement::from_bytes(NQR.as_ref().into()).unwrap();
            let qr = alpha * nqr_field;
            let root = qr
                .sqrt()
                .expect("if alpha is not a square, then qr should be a square");
            let mut result = vec![0x0];
            result.extend_from_slice(&*root.to_bytes());
            result
        }
    }

    pub(super) fn handle_secp256r1(r: [u8; 32], alpha: [u8; 32], r_y_is_odd: bool) -> Vec<u8> {
        use p256::{
            elliptic_curve::ff::PrimeField, FieldElement as P256FieldElement, Scalar as P256Scalar,
        };

        let r = P256FieldElement::from_bytes(r.as_ref().into()).unwrap();
        debug_assert!(!bool::from(r.is_zero()), "r should not be zero");

        let alpha = P256FieldElement::from_bytes(alpha.as_ref().into()).unwrap();
        debug_assert!(!bool::from(alpha.is_zero()), "alpha should not be zero");

        if let Some(mut y_coord) = alpha.sqrt().into_option() {
            let r = P256Scalar::from_repr(r.to_bytes()).unwrap();
            let r_inv = r.invert().expect("Non zero r scalar");

            if r_y_is_odd != bool::from(y_coord.is_odd()) {
                y_coord = -y_coord;
            }

            let mut result = vec![0x1];
            result.copy_from_slice(&*y_coord.to_bytes());
            result.copy_from_slice(&*r_inv.to_bytes());
            result
        } else {
            let nqr_field = P256FieldElement::from_bytes(NQR.as_ref().into()).unwrap();
            let qr = alpha * nqr_field;
            let root = qr
                .sqrt()
                .expect("if alpha is not a square, then qr should be a square");
            let mut result = vec![0x0];
            result.extend_from_slice(&*root.to_bytes());
            result
        }
    }
}

/// Checks if a compressed Edwards point can be decompressed.
///
/// # Arguments
/// * `env` - The environment in which the hook is invoked.
/// * `buf` - The buffer containing the compressed Edwards point.
///    - The compressed Edwards point is 32 bytes.
///    - The high bit of the last byte is the sign bit.
///
/// Returns vec![vec![1]] if the point is decompressable.
/// Returns vec![vec![0], `v_inv`, `nqr_hint`] if the point is not decompressable.
///
/// WARNING: This function merely hints at the validity of the compressed point. These values must
/// be constrained by the zkVM for correctness.
pub fn hook_ed_decompress(buf: &[u8]) -> Result<Vec<u8>, ExitCode> {
    const NQR_CURVE_25519: u8 = 2;
    let modulus = Ed25519BaseField::modulus();

    let mut bytes: [u8; 32] = buf[..32].try_into().unwrap();
    // Mask the sign bit.
    bytes[31] &= 0b0111_1111;

    // The AIR asserts canon inputs, so hint here if it cant be satisfied.
    let y = BigUint::from_bytes_le(&bytes);
    if y >= modulus {
        return Ok(vec![0u8]);
    }

    let v = BigUint::from_bytes_le(&buf[32..]);
    // This is computed as dy^2 - 1
    // so it should always be in the field.
    if v >= modulus {
        return Err(ExitCode::MalformedBuiltinParams);
    }

    // For a point to be decompressable, (yy - 1) / (yy * d + 1) must be a quadratic residue.
    let v_inv = v.modpow(&(&modulus - BigUint::from(2u64)), &modulus);
    let u = (&y * &y + &modulus - BigUint::one()) % &modulus;
    let u_div_v = (&u * &v_inv) % &modulus;

    // Note: Our sqrt impl doesnt care about canon representation,
    // however we have already checked that were less than the modulus.
    if ed25519_sqrt(&u_div_v).is_some() {
        return Ok(vec![0x1]);
    }
    let qr = (u_div_v * NQR_CURVE_25519) % &modulus;
    let root = ed25519_sqrt(&qr).unwrap();

    // Pad the results, since this may not be a full 32 bytes.
    let v_inv_bytes = v_inv.to_bytes_le();
    let mut v_inv_padded = [0_u8; 32];
    v_inv_padded[..v_inv_bytes.len()].copy_from_slice(&v_inv.to_bytes_le());

    let root_bytes = root.to_bytes_le();
    let mut root_padded = [0_u8; 32];
    root_padded[..root_bytes.len()].copy_from_slice(&root.to_bytes_le());

    let mut result = vec![0x0];
    result.extend_from_slice(&v_inv_padded);
    result.extend_from_slice(&root_padded);
    Ok(result)
}

/// Given the product of some 256-byte numbers and a modulus, this function does a modular
/// reduction and hints back the values to the vm in order to constrain it.
///
/// # Arguments
///
/// * `env` - The environment in which the hook is invoked.
/// * `buf` - The buffer containing the le bytes of the 512 byte product and the 256 byte modulus.
///
/// Returns The le bytes of the product % modulus (512 bytes)
/// and the quotient floor(product/modulus) (256 bytes).
///
/// WANRING: This function is used to perform a modular reduction outside of the zkVM context.
/// These values must be constrained by the zkVM for correctness.
pub fn hook_rsa_mul_mod(buf: &[u8]) -> Result<Vec<u8>, ExitCode> {
    if buf.len() != 256 + 256 + 256 {
        return Err(ExitCode::MalformedBuiltinParams);
    }

    let prod: &[u8; 512] = buf[..512].try_into().unwrap();
    let m: &[u8; 256] = buf[512..].try_into().unwrap();

    let prod = BigUint::from_bytes_le(prod);
    let m = BigUint::from_bytes_le(m);

    let (q, rem) = prod.div_rem(&m);

    let mut rem = rem.to_bytes_le();
    rem.resize(256, 0);

    let mut q = q.to_bytes_le();
    q.resize(256, 0);

    let mut result = rem;
    result.extend_from_slice(&q);
    Ok(result)
}

mod bls {
    use super::{pad_to_be, BigUint};
    use fluentbase_types::ExitCode;
    use sp1_curves::{params::FieldParameters, weierstrass::bls12_381::Bls12381BaseField, Zero};

    /// A non-quadratic residue for the `12_381` base field in big endian.
    pub const NQR_BLS12_381: [u8; 48] = {
        let mut nqr = [0; 48];
        nqr[47] = 2;
        nqr
    };

    /// The base field modulus for the `12_381` curve, in little endian.
    pub const BLS12_381_MODULUS: &[u8] = Bls12381BaseField::MODULUS;

    /// Given a field element, in big endian, this function computes the square root.
    ///
    /// - If the field element is the additive identity, this function returns `vec![vec![1],
    ///   vec![0; 48]]`.
    /// - If the field element is a quadratic residue, this function returns `vec![vec![1],
    ///   vec![sqrt(fe)]  ]`.
    /// - If the field element (fe) is not a quadratic residue, this function returns `vec![vec![0],
    ///   vec![sqrt(``NQR_BLS12_381`` * fe)]]`.
    pub fn hook_bls12_381_sqrt(buf: &[u8]) -> Result<Vec<u8>, ExitCode> {
        let field_element = BigUint::from_bytes_be(&buf[..48]);

        // This should be checked in the VM as its easier than dispatching a hook call.
        // But for completeness we include this happy path also.
        if field_element.is_zero() {
            let mut result = vec![1];
            result.resize(48 + 1, 0);
            return Ok(result);
        }

        let modulus = BigUint::from_bytes_le(BLS12_381_MODULUS);

        // Since `BLS12_381_MODULUS` == 3 mod 4,. we can use shanks methods.
        // This means we only need to exponentiate by `(modulus + 1) / 4`.
        let exp = (&modulus + BigUint::from(1u64)) / BigUint::from(4u64);
        let sqrt = field_element.modpow(&exp, &modulus);

        // Shanks methods only works if the field element is a quadratic residue.
        // So we need to check if the square of the sqrt is equal to the field element.
        let square = (&sqrt * &sqrt) % &modulus;
        if square != field_element {
            let nqr = BigUint::from_bytes_be(&NQR_BLS12_381);
            let qr = (&nqr * &field_element) % &modulus;

            // By now, the product of two non-quadratic residues is a quadratic residue.
            // So we can use shanks methods again to get its square root.
            //
            // We pass this root back to the VM to constrain the "failure" case.
            let root = qr.modpow(&exp, &modulus);

            debug_assert!(
                (&root * &root) % &modulus == qr,
                "NQR sanity check failed, this is a bug."
            );

            let mut result = vec![0];
            result.extend(pad_to_be(&root, 48));
            return Ok(result);
        }

        let mut result = vec![1];
        result.extend(pad_to_be(&sqrt, 48));
        Ok(result)
    }

    /// Given a field element, in big endian, this function computes the inverse.
    ///
    /// This functions will panic if the additive identity is passed in.
    pub fn hook_bls12_381_inverse(buf: &[u8]) -> Result<Vec<u8>, ExitCode> {
        let field_element = BigUint::from_bytes_be(&buf[..48]);

        // Zero is not invertible, and we dont want to have to return a status from here.
        if field_element.is_zero() {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let modulus = BigUint::from_bytes_le(BLS12_381_MODULUS);

        // Compute the inverse using Fermat's little theorem, ie, a^(p-2) = a^-1 mod p.
        let inverse = field_element.modpow(&(&modulus - BigUint::from(2u64)), &modulus);

        Ok(pad_to_be(&inverse, 48))
    }
}

/// Pads a big uint to the given length in big endian.
fn pad_to_be(val: &BigUint, len: usize) -> Vec<u8> {
    // First take the byes in little endian
    let mut bytes = val.to_bytes_le();
    // Resize so we get the full padding correctly.
    if len > bytes.len() {
        bytes.resize(len, 0);
    }
    // Convert back to big endian.
    bytes.reverse();

    bytes
}

mod fp_ops {
    use super::{pad_to_be, BigUint, One};
    use fluentbase_types::ExitCode;
    use sp1_curves::Zero;

    /// Compute the inverse of a field element.
    ///
    /// # Arguments:
    /// * `buf` - The buffer containing the data needed to compute the inverse.
    ///     - [ len || Element || Modulus ]
    ///     - len is the u32 length of the element and modulus in big endian.
    ///     - Element is the field element to compute the inverse of, interpreted as a big endian
    ///       integer of `len` bytes.
    ///
    /// # Returns:
    /// A single 32 byte vector containing the inverse.
    ///
    /// # Panics:
    /// - If the buffer length is not valid.
    /// - If the element is zero.
    pub fn hook_fp_inverse(buf: &[u8]) -> Result<Vec<u8>, ExitCode> {
        let len: usize = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;

        if buf.len() != 4 + 2 * len {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let buf = &buf[4..];
        let element = BigUint::from_bytes_be(&buf[..len]);
        let modulus = BigUint::from_bytes_be(&buf[len..2 * len]);

        assert!(!element.is_zero(), "FpOp: Inverse called with zero");

        let inverse = element.modpow(&(&modulus - BigUint::from(2u64)), &modulus);

        Ok(pad_to_be(&inverse, len))
    }

    /// Compute the square root of a field element.
    ///
    /// # Arguments:
    /// * `buf` - The buffer containing the data needed to compute the square root.
    ///     - [ len || Element || Modulus || NQR ]
    ///     - len is the length of the element, modulus, and nqr in big endian.
    ///     - Element is the field element to compute the square root of, interpreted as a big
    ///       endian integer of `len` bytes.
    ///     - Modulus is the modulus of the field, interpreted as a big endian integer of `len`
    ///       bytes.
    ///     - NQR is the non-quadratic residue of the field, interpreted as a big endian integer of
    ///       `len` bytes.
    ///
    /// # Assumptions
    /// - NQR is a non-quadratic residue of the field.
    ///
    /// # Returns:
    /// [ `status_u8` || `root_bytes` ]
    ///
    /// If the status is 0, this is the root of NQR * element.
    /// If the status is 1, this is the root of element.
    ///
    /// # Panics:
    /// - If the buffer length is not valid.
    /// - If the element is not less than the modulus.
    /// - If the nqr is not less than the modulus.
    /// - If the element is zero.
    pub fn hook_fp_sqrt(buf: &[u8]) -> Result<Vec<u8>, ExitCode> {
        let len: usize = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;

        if buf.len() != 4 + 3 * len {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        let buf = &buf[4..];
        let element = BigUint::from_bytes_be(&buf[..len]);
        let modulus = BigUint::from_bytes_be(&buf[len..2 * len]);
        let nqr = BigUint::from_bytes_be(&buf[2 * len..3 * len]);

        if element > modulus || nqr > modulus {
            return Err(ExitCode::MalformedBuiltinParams);
        }

        // The sqrt of zero is zero.
        if element.is_zero() {
            let mut result = vec![1];
            result.resize(len + 1, 0);
            return Ok(result);
        }

        // Compute the square root of the element using the general Tonelli-Shanks algorithm.
        // The implementation can be used for any field as it is field-agnostic.
        if let Some(root) = sqrt_fp(&element, &modulus, &nqr) {
            let mut result = vec![1];
            result.extend(pad_to_be(&root, len));
            Ok(result)
        } else {
            let qr = (&nqr * &element) % &modulus;
            let root = sqrt_fp(&qr, &modulus, &nqr).unwrap();
            let mut result = vec![0];
            result.extend(pad_to_be(&root, len));
            Ok(result)
        }
    }

    /// Compute the square root of a field element for some modulus.
    ///
    /// Requires a known non-quadratic residue of the field.
    fn sqrt_fp(element: &BigUint, modulus: &BigUint, nqr: &BigUint) -> Option<BigUint> {
        // If the prime field is of the form p = 3 mod 4, and `x` is a quadratic residue modulo `p`,
        // then one square root of `x` is given by `x^(p+1 / 4) mod p`.
        if modulus % BigUint::from(4u64) == BigUint::from(3u64) {
            let maybe_root = element.modpow(
                &((modulus + BigUint::from(1u64)) / BigUint::from(4u64)),
                modulus,
            );

            return Some(maybe_root).filter(|root| root * root % modulus == *element);
        }

        tonelli_shanks(element, modulus, nqr)
    }

    /// Compute the square root of a field element using the Tonelli-Shanks algorithm.
    ///
    /// # Arguments:
    /// * `element` - The field element to compute the square root of.
    /// * `modulus` - The modulus of the field.
    /// * `nqr` - The non-quadratic residue of the field.
    ///
    /// # Assumptions:
    /// - The element is a quadratic residue modulo the modulus.
    ///
    /// Ref: <https://en.wikipedia.org/wiki/Tonelli%E2%80%93Shanks_algorithm>
    #[allow(clippy::many_single_char_names)]
    fn tonelli_shanks(element: &BigUint, modulus: &BigUint, nqr: &BigUint) -> Option<BigUint> {
        // First, compute the Legendre symbol of the element.
        // If the symbol is not 1, then the element is not a quadratic residue.
        if legendre_symbol(element, modulus) != BigUint::one() {
            return None;
        }

        // Find the values of Q and S such that modulus - 1 = Q * 2^S.
        let mut s = BigUint::zero();
        let mut q = modulus - BigUint::one();
        while &q % &BigUint::from(2u64) == BigUint::zero() {
            s += BigUint::from(1u64);
            q /= BigUint::from(2u64);
        }

        let z = nqr;
        let mut c = z.modpow(&q, modulus);
        let mut r = element.modpow(&((&q + BigUint::from(1u64)) / BigUint::from(2u64)), modulus);
        let mut t = element.modpow(&q, modulus);
        let mut m = s;

        while t != BigUint::one() {
            let mut i = BigUint::zero();
            let mut tt = t.clone();
            while tt != BigUint::one() {
                tt = &tt * &tt % modulus;
                i += BigUint::from(1u64);

                if i == m {
                    return None;
                }
            }

            let b_pow =
                BigUint::from(2u64).pow((&m - &i - BigUint::from(1u64)).try_into().unwrap());
            let b = c.modpow(&b_pow, modulus);

            r = &r * &b % modulus;
            c = &b * &b % modulus;
            t = &t * &c % modulus;
            m = i;
        }

        Some(r)
    }

    /// Compute the Legendre symbol of a field element.
    ///
    /// This indicates if the element is a quadratic in the prime field.
    ///
    /// Ref: <https://en.wikipedia.org/wiki/Legendre_symbol>
    fn legendre_symbol(element: &BigUint, modulus: &BigUint) -> BigUint {
        assert!(!element.is_zero(), "FpOp: Legendre symbol of zero called.");

        element.modpow(&((modulus - BigUint::one()) / BigUint::from(2u64)), modulus)
    }

    #[cfg(test)]
    mod test {
        use super::*;
        use std::str::FromStr;

        #[test]
        fn test_legendre_symbol() {
            // The modulus of the secp256k1 base field.
            let modulus = BigUint::from_str(
                "115792089237316195423570985008687907853269984665640564039457584007908834671663",
            )
            .unwrap();
            let neg_1 = &modulus - BigUint::one();

            let fixtures = [
                (BigUint::from(4u64), BigUint::from(1u64)),
                (BigUint::from(2u64), BigUint::from(1u64)),
                (BigUint::from(3u64), neg_1.clone()),
            ];

            for (element, expected) in fixtures {
                let result = legendre_symbol(&element, &modulus);
                assert_eq!(result, expected);
            }
        }

        #[test]
        fn test_tonelli_shanks() {
            // The modulus of the secp256k1 base field.
            let p = BigUint::from_str(
                "115792089237316195423570985008687907853269984665640564039457584007908834671663",
            )
            .unwrap();

            let nqr = BigUint::from_str("3").unwrap();

            let large_element = &p - BigUint::from(u16::MAX);
            let square = &large_element * &large_element % &p;

            let fixtures = [
                (BigUint::from(2u64), true),
                (BigUint::from(3u64), false),
                (BigUint::from(4u64), true),
                (square, true),
            ];

            for (element, expected) in fixtures {
                let result = tonelli_shanks(&element, &p, &nqr);
                if expected {
                    assert!(result.is_some());

                    let result = result.unwrap();
                    assert!((&result * &result) % &p == element);
                } else {
                    assert!(result.is_none());
                }
            }
        }
    }
}
