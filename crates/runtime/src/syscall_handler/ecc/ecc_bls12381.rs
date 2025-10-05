use bls12_381::{
    G1Affine as Sp1G1Affine, G1Projective as Sp1G1Projective, G2Affine as Sp1G2Affine,
    G2Projective as Sp1G2Projective,
};
use blstrs::{G1Affine, G2Affine};
use fluentbase_types::{FP_SIZE, G1_UNCOMPRESSED_SIZE, G2_UNCOMPRESSED_SIZE};
use group::prime::PrimeCurveAffine;

pub fn parse_bls12381_g1_point_uncompressed(input: &[u8; G1_UNCOMPRESSED_SIZE]) -> G1Affine {
    if input.iter().all(|&b| b == 0) {
        G1Affine::identity()
    } else {
        let ct = G1Affine::from_uncompressed(input);
        if ct.is_none().unwrap_u8() == 1 {
            G1Affine::identity()
        } else {
            ct.unwrap()
        }
    }
}

pub fn parse_bls12381_g2_point_uncompressed(input: &[u8; G2_UNCOMPRESSED_SIZE]) -> G2Affine {
    if input.iter().all(|&b| b == 0) {
        G2Affine::identity()
    } else {
        let ct = G2Affine::from_uncompressed(input);
        if ct.is_none().unwrap_u8() == 1 {
            G2Affine::identity()
        } else {
            ct.unwrap()
        }
    }
}

pub fn g2_le_limbs_to_be_uncompressed(
    le_limbs: &[u8; G2_UNCOMPRESSED_SIZE],
) -> [u8; G2_UNCOMPRESSED_SIZE] {
    let mut be_uncompressed = [0u8; G2_UNCOMPRESSED_SIZE];

    // Convert each limb from LE to BE and swap positions
    let mut limb = [0u8; FP_SIZE];

    // x1 -> c0 (with endianness conversion)
    limb.copy_from_slice(&le_limbs[FP_SIZE..2 * FP_SIZE]);
    limb.reverse();
    be_uncompressed[0..FP_SIZE].copy_from_slice(&limb);

    // x0 -> c1 (with endianness conversion)
    limb.copy_from_slice(&le_limbs[0..FP_SIZE]);
    limb.reverse();
    be_uncompressed[FP_SIZE..2 * FP_SIZE].copy_from_slice(&limb);

    // y1 -> c0 (with endianness conversion)
    limb.copy_from_slice(&le_limbs[3 * FP_SIZE..4 * FP_SIZE]);
    limb.reverse();
    be_uncompressed[2 * FP_SIZE..3 * FP_SIZE].copy_from_slice(&limb);

    // y0 -> c1 (with endianness conversion)
    limb.copy_from_slice(&le_limbs[2 * FP_SIZE..3 * FP_SIZE]);
    limb.reverse();
    be_uncompressed[3 * FP_SIZE..4 * FP_SIZE].copy_from_slice(&limb);

    be_uncompressed
}

pub fn g2_be_uncompressed_to_le_limbs(
    be_point: &[u8; G2_UNCOMPRESSED_SIZE],
) -> [u8; G2_UNCOMPRESSED_SIZE] {
    let mut le_point = [0u8; G2_UNCOMPRESSED_SIZE];
    let mut limb = [0u8; FP_SIZE];

    // x0 <= c1, x1 <= c0
    limb.copy_from_slice(&be_point[FP_SIZE..2 * FP_SIZE]); // c1
    limb.reverse();
    le_point[0..FP_SIZE].copy_from_slice(&limb); // x0 LE
    limb.copy_from_slice(&be_point[0..FP_SIZE]); // c0
    limb.reverse();
    le_point[FP_SIZE..2 * FP_SIZE].copy_from_slice(&limb); // x1 LE

    // y0 <= c1, y1 <= c0
    limb.copy_from_slice(&be_point[3 * FP_SIZE..4 * FP_SIZE]); // c1
    limb.reverse();
    le_point[2 * FP_SIZE..3 * FP_SIZE].copy_from_slice(&limb); // y0 LE
    limb.copy_from_slice(&be_point[2 * FP_SIZE..3 * FP_SIZE]); // c0
    limb.reverse();
    le_point[3 * FP_SIZE..4 * FP_SIZE].copy_from_slice(&limb); // y1 LE

    le_point
}

// Parse into affine points (validated), add in projective, and convert back to affine
pub fn parse_affine_g2(be: &[u8; G2_UNCOMPRESSED_SIZE]) -> G2Affine {
    if be.iter().all(|&b| b == 0) {
        G2Affine::identity()
    } else {
        let ct = G2Affine::from_uncompressed(be);
        if ct.is_none().unwrap_u8() == 1 {
            G2Affine::identity()
        } else {
            ct.unwrap_or(G2Affine::identity())
        }
    }
}

/// Parse BLS12-381 G1 point from uncompressed bytes (96 bytes) using sp1 library
pub fn parse_sp1_g1_point_uncompressed(input: &[u8; G1_UNCOMPRESSED_SIZE]) -> Sp1G1Affine {
    if input.iter().all(|&b| b == 0) {
        Sp1G1Affine::identity()
    } else {
        // sp1 bls12_381 uses from_uncompressed which expects big-endian bytes
        let ct = Sp1G1Affine::from_uncompressed(input);
        if ct.is_some().unwrap_u8() == 1 {
            ct.unwrap()
        } else {
            Sp1G1Affine::identity()
        }
    }
}

/// BLS12-381 G1 point addition using sp1 library
pub fn bls12381_g1_add_sp1(
    p: &[u8; G1_UNCOMPRESSED_SIZE],
    q: &[u8; G1_UNCOMPRESSED_SIZE],
) -> [u8; G1_UNCOMPRESSED_SIZE] {
    let p_aff = parse_sp1_g1_point_uncompressed(p);
    let q_aff = parse_sp1_g1_point_uncompressed(q);

    // Convert to projective, add, and convert back to affine
    let result_proj = Sp1G1Projective::from(p_aff) + Sp1G1Projective::from(q_aff);
    let result_aff = Sp1G1Affine::from(result_proj);

    result_aff.to_uncompressed()
}

/// BLS12-381 G1 point doubling using sp1 library
pub fn bls12381_g1_double_sp1(p: &[u8; G1_UNCOMPRESSED_SIZE]) -> [u8; G1_UNCOMPRESSED_SIZE] {
    let p_aff = parse_sp1_g1_point_uncompressed(p);

    // Convert to projective, double, and convert back to affine
    let result_proj = Sp1G1Projective::from(p_aff).double();
    let result_aff = Sp1G1Affine::from(result_proj);

    result_aff.to_uncompressed()
}

/// Parse BLS12-381 G2 point from uncompressed bytes (192 bytes) using sp1 library
pub fn parse_sp1_g2_point_uncompressed(input: &[u8; G2_UNCOMPRESSED_SIZE]) -> Sp1G2Affine {
    if input.iter().all(|&b| b == 0) {
        Sp1G2Affine::identity()
    } else {
        // Convert from LE limbs to BE uncompressed format expected by sp1 bls12_381
        let be_uncompressed = g2_le_limbs_to_be_uncompressed(input);
        let ct = Sp1G2Affine::from_uncompressed(&be_uncompressed);
        if ct.is_some().unwrap_u8() == 1 {
            ct.unwrap()
        } else {
            Sp1G2Affine::identity()
        }
    }
}

/// BLS12-381 G2 point addition using sp1 library
pub fn bls12381_g2_add_sp1(
    p: &[u8; G2_UNCOMPRESSED_SIZE],
    q: &[u8; G2_UNCOMPRESSED_SIZE],
) -> [u8; G2_UNCOMPRESSED_SIZE] {
    let p_aff = parse_sp1_g2_point_uncompressed(p);
    let q_aff = parse_sp1_g2_point_uncompressed(q);

    // Convert to projective, add, and convert back to affine
    let result_proj = Sp1G2Projective::from(p_aff) + Sp1G2Projective::from(q_aff);
    let result_aff = Sp1G2Affine::from(result_proj);

    // Convert back to LE limbs format
    let be_result = result_aff.to_uncompressed();
    g2_be_uncompressed_to_le_limbs(&be_result)
}

/// BLS12-381 G2 point doubling using sp1 library
pub fn bls12381_g2_double_sp1(p: &[u8; G2_UNCOMPRESSED_SIZE]) -> [u8; G2_UNCOMPRESSED_SIZE] {
    let p_aff = parse_sp1_g2_point_uncompressed(p);

    // Convert to projective, double, and convert back to affine
    let result_proj = Sp1G2Projective::from(p_aff).double();
    let result_aff = Sp1G2Affine::from(result_proj);

    // Convert back to LE limbs format
    let be_result = result_aff.to_uncompressed();
    g2_be_uncompressed_to_le_limbs(&be_result)
}



// /// These tests are taken from:
// /// - sp1/crates/test-artifacts/programs/bls12381-fp/src/main.rs
// /// - sp1/crates/test-artifacts/programs/bls12381-fp2-addsub/src/main.rs
// /// - sp1/crates/test-artifacts/programs/bls12381-fp2-mul/src/main.rs
// /// - sp1/crates/test-artifacts/programs/bls12381-double/src/main.rs
// /// - sp1/crates/test-artifacts/programs/bls12381-mul/src/main.rs
// /// - sp1/crates/test-artifacts/programs/bls12381-pairing/src/main.rs
// ///
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use secp256k1::{rand, rand::Rng};

//     fn biguint_to_bytes_le(x: BigUint) -> [u8; 32] {
//         let mut bytes = x.to_bytes_le();
//         bytes.resize(32, 0);
//         bytes.try_into().unwrap()
//     }

//     #[test]
//     fn test_bls12381_fp() {
//         for _ in 0..50 {
//             // Test with random numbers.
//             let mut rng = rand::thread_rng();
//             let mut x: [u8; 32] = rng.gen();
//             let mut y: [u8; 32] = rng.gen();
//             let modulus: [u8; 32] = rng.gen();

//             // Convert byte arrays to BigUint
//             let modulus_big = BigUint::from_bytes_le(&modulus);
//             let x_big = BigUint::from_bytes_le(&x);
//             x = biguint_to_bytes_le(&x_big % &modulus_big);
//             let y_big = BigUint::from_bytes_le(&y);
//             y = biguint_to_bytes_le(&y_big % &modulus_big);

//             let result_bytes = syscall_uint256_mul_mod_impl(&x, &y, &modulus);

//             let result = (x_big * y_big) % modulus_big;
//             let result_syscall = BigUint::from_bytes_le(&result_bytes);

//             assert_eq!(result, result_syscall);
//         }

//         // Modulus zero tests
//         let modulus = [0u8; 32];
//         let modulus_big: BigUint = BigUint::one() << 256;
//         for _ in 0..50 {
//             // Test with random numbers.
//             let mut rng = rand::thread_rng();
//             let mut x: [u8; 32] = rng.gen();
//             let mut y: [u8; 32] = rng.gen();

//             // Convert byte arrays to BigUint
//             let x_big = BigUint::from_bytes_le(&x);
//             x = biguint_to_bytes_le(&x_big % &modulus_big);
//             let y_big = BigUint::from_bytes_le(&y);
//             y = biguint_to_bytes_le(&y_big % &modulus_big);

//             let result_bytes = syscall_uint256_mul_mod_impl(&x, &y, &modulus);

//             let result = (x_big * y_big) % &modulus_big;
//             let result_syscall = BigUint::from_bytes_le(&result_bytes);

//             assert_eq!(result, result_syscall, "x: {:?}, y: {:?}", x, y);
//         }

//         // Test with random numbers.
//         let mut rng = rand::thread_rng();
//         let x: [u8; 32] = rng.gen();

//         // Hardcoded edge case: Multiplying by 1
//         let modulus = [0u8; 32];

//         let mut one: [u8; 32] = [0; 32];
//         one[0] = 1; // Least significant byte set to 1, represents the number 1
//         let original_x = x; // Copy original x value before multiplication by 1
//         let result_one = syscall_uint256_mul_mod_impl(&x, &one, &modulus);
//         assert_eq!(
//             result_one, original_x,
//             "Multiplying by 1 should yield the same number."
//         );

//         // Hardcoded edge case: Multiplying by 0
//         let zero: [u8; 32] = [0; 32]; // Represents the number 0
//         let result_zero = syscall_uint256_mul_mod_impl(&x, &zero, &modulus);
//         assert_eq!(result_zero, zero, "Multiplying by 0 should yield 0.");
//     }
// }
