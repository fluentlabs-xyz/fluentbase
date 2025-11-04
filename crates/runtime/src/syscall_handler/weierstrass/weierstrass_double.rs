use crate::RuntimeContext;
use fluentbase_types::{
    BLS12381_G1_RAW_AFFINE_SIZE, BN254_G1_RAW_AFFINE_SIZE, SECP256K1_G1_RAW_AFFINE_SIZE,
    SECP256R1_G1_RAW_AFFINE_SIZE,
};
use num::BigUint;
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{
    weierstrass::{bls12_381::Bls12381, bn254::Bn254, secp256k1::Secp256k1, secp256r1::Secp256r1},
    AffinePoint, EllipticCurve,
};

pub fn syscall_secp256k1_double_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_double_handler::<Secp256k1, { SECP256K1_G1_RAW_AFFINE_SIZE }>(
        ctx, params, result,
    )
}
pub fn syscall_secp256r1_double_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_double_handler::<Secp256r1, { SECP256R1_G1_RAW_AFFINE_SIZE }>(
        ctx, params, result,
    )
}
pub fn syscall_bn254_double_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_double_handler::<Bn254, { BN254_G1_RAW_AFFINE_SIZE }>(ctx, params, result)
}
pub fn syscall_bls12381_double_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_double_handler::<Bls12381, { BLS12381_G1_RAW_AFFINE_SIZE }>(
        ctx, params, result,
    )
}

fn syscall_weierstrass_double_handler<E: EllipticCurve, const POINT_SIZE: usize>(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let p_ptr: u32 = params[0].i32().unwrap() as u32;

    let mut p = [0u8; POINT_SIZE];
    ctx.memory_read(p_ptr as usize, &mut p)?;

    let result = syscall_weierstrass_double_impl::<E, POINT_SIZE>(p);
    ctx.memory_write(p_ptr as usize, &result)?;

    Ok(())
}

pub fn syscall_secp256k1_double_impl(
    p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE],
) -> [u8; SECP256K1_G1_RAW_AFFINE_SIZE] {
    syscall_weierstrass_double_impl::<Secp256k1, { SECP256K1_G1_RAW_AFFINE_SIZE }>(p)
}
pub fn syscall_secp256r1_double_impl(
    p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE],
) -> [u8; SECP256R1_G1_RAW_AFFINE_SIZE] {
    syscall_weierstrass_double_impl::<Secp256r1, { SECP256R1_G1_RAW_AFFINE_SIZE }>(p)
}
pub fn syscall_bn254_double_impl(
    p: [u8; BN254_G1_RAW_AFFINE_SIZE],
) -> [u8; BN254_G1_RAW_AFFINE_SIZE] {
    syscall_weierstrass_double_impl::<Bn254, { BN254_G1_RAW_AFFINE_SIZE }>(p)
}
pub fn syscall_bls12381_double_impl(
    p: [u8; BLS12381_G1_RAW_AFFINE_SIZE],
) -> [u8; BLS12381_G1_RAW_AFFINE_SIZE] {
    syscall_weierstrass_double_impl::<Bls12381, { BLS12381_G1_RAW_AFFINE_SIZE }>(p)
}

fn syscall_weierstrass_double_impl<E: EllipticCurve, const POINT_SIZE: usize>(
    p: [u8; POINT_SIZE],
) -> [u8; POINT_SIZE] {
    let (px, py) = p.split_at(p.len() / 2);
    let p_affine = AffinePoint::<E>::new(BigUint::from_bytes_le(px), BigUint::from_bytes_le(py));
    let result_affine = E::ec_double(&p_affine);
    let (rx, ry) = (result_affine.x, result_affine.y);
    let mut result = [0u8; POINT_SIZE];
    let mut rx = rx.to_bytes_le();
    rx.resize(POINT_SIZE / 2, 0);
    let mut ry = ry.to_bytes_le();
    ry.resize(POINT_SIZE / 2, 0);
    result[..POINT_SIZE / 2].copy_from_slice(&rx);
    result[POINT_SIZE / 2..].copy_from_slice(&ry);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bls12381_double() {
        // generator.
        // 3685416753713387016781088315183077757961620795782546409894578378688607592378376318836054947676345821548104185464507
        // 1339506544944476473020471379941921221584933875938349620426543736416511423956333506472724655353366534992391756441569
        let generator: [u8; 96] = [
            187, 198, 34, 219, 10, 240, 58, 251, 239, 26, 122, 249, 63, 232, 85, 108, 88, 172, 27,
            23, 63, 58, 78, 161, 5, 185, 116, 151, 79, 140, 104, 195, 15, 172, 169, 79, 140, 99,
            149, 38, 148, 215, 151, 49, 167, 211, 241, 23, 225, 231, 197, 70, 41, 35, 170, 12, 228,
            138, 136, 162, 68, 199, 60, 208, 237, 179, 4, 44, 203, 24, 219, 0, 246, 10, 208, 213,
            149, 224, 245, 252, 228, 138, 29, 116, 237, 48, 158, 160, 241, 160, 170, 227, 129, 244,
            179, 8,
        ];

        // 2 * generator (doubled generator).
        // 838589206289216005799424730305866328161735431124665289961769162861615689790485775997575391185127590486775437397838
        // 3450209970729243429733164009999191867485184320918914219895632678707687208996709678363578245114137957452475385814312
        let expected_doubled: [u8; 96] = [
            78, 15, 191, 41, 85, 140, 154, 195, 66, 124, 28, 143, 187, 117, 143, 226, 42, 166, 88,
            195, 10, 45, 144, 67, 37, 1, 40, 145, 48, 219, 33, 151, 12, 69, 169, 80, 235, 200, 8,
            136, 70, 103, 77, 144, 234, 203, 114, 5, 40, 157, 116, 121, 25, 136, 134, 186, 27, 189,
            22, 205, 212, 217, 86, 76, 106, 215, 95, 29, 2, 185, 59, 247, 97, 228, 112, 134, 203,
            62, 186, 34, 56, 142, 157, 119, 115, 166, 253, 34, 163, 115, 198, 171, 140, 157, 106,
            22,
        ];

        let result = syscall_bls12381_double_impl(generator);
        assert_eq!(result, expected_doubled);
    }

    #[test]
    fn test_bn254_double() {
        for _ in 0..10i64.pow(3) {
            // generator.
            // 1
            // 2
            let a: [u8; 64] = [
                1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0,
            ];
            // 2 * generator.
            // 1368015179489954701390400359078579693043519447331113978918064868415326638035
            // 9918110051302171585080402603319702774565515993150576347155970296011118125764
            let b: [u8; 64] = [
                211, 207, 135, 109, 193, 8, 194, 211, 168, 28, 135, 22, 169, 22, 120, 217, 133, 21,
                24, 104, 91, 4, 133, 155, 2, 26, 19, 46, 231, 68, 6, 3, 196, 162, 24, 90, 122, 191,
                62, 255, 199, 143, 83, 227, 73, 164, 166, 104, 10, 156, 174, 178, 150, 95, 132,
                231, 146, 124, 10, 14, 140, 115, 237, 21,
            ];

            let result = syscall_bn254_double_impl(a);
            assert_eq!(result, b);
        }
    }

    #[test]
    fn test_secp256k1_double() {
        for _ in 0..10 {
            // generator.
            // 55066263022277343669578718895168534326250603453777594175500187360389116729240
            // 32670510020758816978083085130507043184471273380659243275938904335757337482424
            let a: [u8; 64] = [
                152, 23, 248, 22, 91, 129, 242, 89, 217, 40, 206, 45, 219, 252, 155, 2, 7, 11, 135,
                206, 149, 98, 160, 85, 172, 187, 220, 249, 126, 102, 190, 121, 184, 212, 16, 251,
                143, 208, 71, 156, 25, 84, 133, 166, 72, 180, 23, 253, 168, 8, 17, 14, 252, 251,
                164, 93, 101, 196, 163, 38, 119, 218, 58, 72,
            ];

            // 2 * generator.
            // 89565891926547004231252920425935692360644145829622209833684329913297188986597
            // 12158399299693830322967808612713398636155367887041628176798871954788371653930
            let b: [u8; 64] = [
                229, 158, 112, 92, 185, 9, 172, 171, 167, 60, 239, 140, 75, 142, 119, 92, 216, 124,
                192, 149, 110, 64, 69, 48, 109, 125, 237, 65, 148, 127, 4, 198, 42, 229, 207, 80,
                169, 49, 100, 35, 225, 208, 102, 50, 101, 50, 246, 247, 238, 234, 108, 70, 25, 132,
                197, 163, 57, 195, 61, 166, 254, 104, 225, 26,
            ];

            let result = syscall_secp256k1_double_impl(a);
            assert_eq!(result, b);
        }
    }

    #[test]
    fn test_secp256r1_double() {
        // generator.
        // 48439561293906451759052585252797914202762949526041747995844080717082404635286
        // 36134250956749795798585127919587881956611106672985015071877198253568414405109
        let a: [u8; 64] = [
            150, 194, 152, 216, 69, 57, 161, 244, 160, 51, 235, 45, 129, 125, 3, 119, 242, 64, 164,
            99, 229, 230, 188, 248, 71, 66, 44, 225, 242, 209, 23, 107, 245, 81, 191, 55, 104, 64,
            182, 203, 206, 94, 49, 107, 87, 51, 206, 43, 22, 158, 15, 124, 74, 235, 231, 142, 155,
            127, 26, 254, 226, 66, 227, 79,
        ];

        // 2 * generator.
        // 56515219790691171413109057904011688695424810155802929973526481321309856242040
        // 3377031843712258259223711451491452598088675519751548567112458094635497583569
        let b: [u8; 64] = [
            120, 153, 102, 71, 252, 72, 11, 166, 53, 27, 242, 119, 226, 105, 137, 192, 195, 26,
            181, 4, 3, 56, 82, 138, 126, 79, 3, 141, 24, 123, 242, 124, 209, 115, 120, 34, 157,
            183, 4, 158, 41, 130, 233, 60, 230, 173, 125, 186, 219, 48, 116, 159, 198, 154, 61, 41,
            64, 208, 142, 219, 16, 85, 119, 7,
        ];

        let result = syscall_secp256r1_double_impl(a);
        assert_eq!(result, b);
    }
}
