use crate::{syscall_handler::syscall_process_exit_code, RuntimeContext};
use fluentbase_types::{
    ExitCode, BLS12381_G1_RAW_AFFINE_SIZE, BN254_G1_RAW_AFFINE_SIZE, SECP256K1_G1_RAW_AFFINE_SIZE,
    SECP256R1_G1_RAW_AFFINE_SIZE,
};
use rwasm::{Store, TrapCode, Value};
use sp1_curves::{
    params::FieldParameters,
    weierstrass::{
        bls12_381::{Bls12381, Bls12381BaseField},
        bn254::{Bn254, Bn254BaseField},
        secp256k1::{Secp256k1, Secp256k1BaseField},
        secp256r1::{Secp256r1, Secp256r1BaseField},
    },
    AffinePoint, BigUint, EllipticCurve,
};

pub fn syscall_secp256k1_add_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_add_handler::<Secp256k1, Secp256k1BaseField, { SECP256K1_G1_RAW_AFFINE_SIZE }>(
        ctx, params, result,
    )
}
pub fn syscall_secp256r1_add_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_add_handler::<Secp256r1, Secp256r1BaseField, { SECP256R1_G1_RAW_AFFINE_SIZE }>(
        ctx, params, result,
    )
}
pub fn syscall_bn254_add_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_add_handler::<Bn254, Bn254BaseField, { BN254_G1_RAW_AFFINE_SIZE }>(
        ctx, params, result,
    )
}
pub fn syscall_bls12381_add_handler(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    result: &mut [Value],
) -> Result<(), TrapCode> {
    syscall_weierstrass_add_handler::<Bls12381, Bls12381BaseField, { BLS12381_G1_RAW_AFFINE_SIZE }>(
        ctx, params, result,
    )
}

fn syscall_weierstrass_add_handler<
    E: EllipticCurve,
    P: FieldParameters,
    const POINT_SIZE: usize,
>(
    ctx: &mut impl Store<RuntimeContext>,
    params: &[Value],
    _result: &mut [Value],
) -> Result<(), TrapCode> {
    let p_ptr = params[0].i32().unwrap() as usize;
    let q_ptr = params[1].i32().unwrap() as usize;

    let mut p = [0u8; POINT_SIZE];
    ctx.memory_read(p_ptr, &mut p)?;
    let mut q = [0u8; POINT_SIZE];
    ctx.memory_read(q_ptr, &mut q)?;

    let result = syscall_weierstrass_add_impl::<E, P, POINT_SIZE>(p, q)
        .map_err(|exit_code| syscall_process_exit_code(ctx, exit_code))?;
    ctx.memory_write(p_ptr, &result)?;
    Ok(())
}

/// Secp256k1 curve point addition.
///
/// # Input format
/// Both `p` and `q` must be affine points encoded as `[x || y]` in little-endian,
/// where each coordinate is 32 bytes.
///
/// # Validation
/// Returns `ExitCode::MalformedBuiltinParams` if:
/// - `p == q` (use doubling instead — SP1 doesn't support adding equal points)
/// - Any coordinate >= field modulus
pub fn syscall_secp256k1_add_impl(
    p: [u8; SECP256K1_G1_RAW_AFFINE_SIZE],
    q: [u8; SECP256K1_G1_RAW_AFFINE_SIZE],
) -> Result<[u8; SECP256K1_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_add_impl::<Secp256k1, Secp256k1BaseField, { SECP256K1_G1_RAW_AFFINE_SIZE }>(
        p, q,
    )
}

/// Secp256r1 curve point addition.
///
/// # Input format
/// Both `p` and `q` must be affine points encoded as `[x || y]` in little-endian,
/// where each coordinate is 32 bytes.
///
/// # Validation
/// Returns `ExitCode::MalformedBuiltinParams` if:
/// - `p == q` (use doubling instead — SP1 doesn't support adding equal points)
/// - Any coordinate >= field modulus
pub fn syscall_secp256r1_add_impl(
    p: [u8; SECP256R1_G1_RAW_AFFINE_SIZE],
    q: [u8; SECP256R1_G1_RAW_AFFINE_SIZE],
) -> Result<[u8; SECP256R1_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_add_impl::<Secp256r1, Secp256r1BaseField, { SECP256R1_G1_RAW_AFFINE_SIZE }>(
        p, q,
    )
}

/// BN254 curve point addition.
///
/// # Input format
/// Both `p` and `q` must be affine points encoded as `[x || y]` in little-endian,
/// where each coordinate is 32 bytes.
///
/// # Validation
/// Returns `ExitCode::MalformedBuiltinParams` if:
/// - `p == q` (use doubling instead — SP1 doesn't support adding equal points)
/// - Any coordinate >= field modulus
pub fn syscall_bn254_add_impl(
    p: [u8; BN254_G1_RAW_AFFINE_SIZE],
    q: [u8; BN254_G1_RAW_AFFINE_SIZE],
) -> Result<[u8; BN254_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_add_impl::<Bn254, Bn254BaseField, { BN254_G1_RAW_AFFINE_SIZE }>(p, q)
}

/// BLS12-381 curve point addition.
///
/// # Input format
/// Both `p` and `q` must be affine points encoded as `[x || y]` in little-endian,
/// where each coordinate is 48 bytes.
///
/// # Validation
/// Returns `ExitCode::MalformedBuiltinParams` if:
/// - `p == q` (use doubling instead — SP1 doesn't support adding equal points)
/// - Any coordinate >= field modulus
pub fn syscall_bls12381_add_impl(
    p: [u8; BLS12381_G1_RAW_AFFINE_SIZE],
    q: [u8; BLS12381_G1_RAW_AFFINE_SIZE],
) -> Result<[u8; BLS12381_G1_RAW_AFFINE_SIZE], ExitCode> {
    syscall_weierstrass_add_impl::<Bls12381, Bls12381BaseField, { BLS12381_G1_RAW_AFFINE_SIZE }>(
        p, q,
    )
}

/// Generic SP1 curve point addition implementation.
///
/// # Input format
/// Both `p` and `q` must be affine points encoded as `[x || y]` in little-endian,
/// where each coordinate is `POINT_SIZE / 2` bytes.
///
/// # Validation
/// Returns `ExitCode::MalformedBuiltinParams` if:
/// - `p == q` (use doubling instead — SP1 doesn't support adding equal points)
/// - Any coordinate >= field modulus
fn syscall_weierstrass_add_impl<E: EllipticCurve, P: FieldParameters, const POINT_SIZE: usize>(
    p: [u8; POINT_SIZE],
    q: [u8; POINT_SIZE],
) -> Result<[u8; POINT_SIZE], ExitCode> {
    let (px, py) = p.split_at(POINT_SIZE / 2);
    let p_affine = AffinePoint::<E>::new(BigUint::from_bytes_le(px), BigUint::from_bytes_le(py));
    let (qx, qy) = q.split_at(POINT_SIZE / 2);
    let q_affine = AffinePoint::<E>::new(BigUint::from_bytes_le(qx), BigUint::from_bytes_le(qy));
    // SP1 doesn't support add of two points, that's why we have to return an error here
    if p_affine.x == q_affine.x && p_affine.y == q_affine.y {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    // Make sure p/q are always less than modulus (to avoid neg underflow)
    let modulus = P::modulus();
    if p_affine.x >= modulus
        || p_affine.y >= modulus
        || q_affine.x >= modulus
        || q_affine.y >= modulus
    {
        return Err(ExitCode::MalformedBuiltinParams);
    }
    let result_affine = p_affine + q_affine;
    let (rx, ry) = (result_affine.x, result_affine.y);
    let mut result = [0u8; POINT_SIZE];
    let mut rx = rx.to_bytes_le();
    rx.resize(POINT_SIZE / 2, 0);
    let mut ry = ry.to_bytes_le();
    ry.resize(POINT_SIZE / 2, 0);
    result[..POINT_SIZE / 2].copy_from_slice(&rx);
    result[POINT_SIZE / 2..].copy_from_slice(&ry);
    Ok(result)
}

/// TESTs
///
/// The tests are taken from:
/// - sp1/crates/test-artifacts/programs/bls12381-add/src/main.rs
/// - sp1/crates/test-artifacts/programs/secp256k1-add/src/main.rs
/// - sp1/crates/test-artifacts/programs/secp256r1-add/src/main.rs
/// - sp1/crates/test-artifacts/programs/bn254-add/src/main.rs
#[cfg(test)]
mod tests {
    use super::*;
    use sp1_curves::{params::FieldParameters, weierstrass::secp256k1::Secp256k1BaseField};

    #[test]
    fn test_bls12381_add() {
        // generator.
        // 3685416753713387016781088315183077757961620795782546409894578378688607592378376318836054947676345821548104185464507
        // 1339506544944476473020471379941921221584933875938349620426543736416511423956333506472724655353366534992391756441569
        const A: [u8; 96] = [
            187, 198, 34, 219, 10, 240, 58, 251, 239, 26, 122, 249, 63, 232, 85, 108, 88, 172, 27,
            23, 63, 58, 78, 161, 5, 185, 116, 151, 79, 140, 104, 195, 15, 172, 169, 79, 140, 99,
            149, 38, 148, 215, 151, 49, 167, 211, 241, 23, 225, 231, 197, 70, 41, 35, 170, 12, 228,
            138, 136, 162, 68, 199, 60, 208, 237, 179, 4, 44, 203, 24, 219, 0, 246, 10, 208, 213,
            149, 224, 245, 252, 228, 138, 29, 116, 237, 48, 158, 160, 241, 160, 170, 227, 129, 244,
            179, 8,
        ];

        // 2 * generator.
        // 838589206289216005799424730305866328161735431124665289961769162861615689790485775997575391185127590486775437397838
        // 3450209970729243429733164009999191867485184320918914219895632678707687208996709678363578245114137957452475385814312
        const B: [u8; 96] = [
            78, 15, 191, 41, 85, 140, 154, 195, 66, 124, 28, 143, 187, 117, 143, 226, 42, 166, 88,
            195, 10, 45, 144, 67, 37, 1, 40, 145, 48, 219, 33, 151, 12, 69, 169, 80, 235, 200, 8,
            136, 70, 103, 77, 144, 234, 203, 114, 5, 40, 157, 116, 121, 25, 136, 134, 186, 27, 189,
            22, 205, 212, 217, 86, 76, 106, 215, 95, 29, 2, 185, 59, 247, 97, 228, 112, 134, 203,
            62, 186, 34, 56, 142, 157, 119, 115, 166, 253, 34, 163, 115, 198, 171, 140, 157, 106,
            22,
        ];

        // 3 * generator.
        // 1527649530533633684281386512094328299672026648504329745640827351945739272160755686119065091946435084697047221031460
        // 487897572011753812113448064805964756454529228648704488481988876974355015977479905373670519228592356747638779818193
        const C: [u8; 96] = [
            36, 82, 78, 2, 201, 192, 210, 150, 155, 23, 162, 44, 11, 122, 116, 129, 249, 63, 91,
            51, 81, 10, 120, 243, 241, 165, 233, 155, 31, 214, 18, 177, 151, 150, 169, 236, 45, 33,
            101, 23, 19, 240, 209, 249, 8, 227, 236, 9, 209, 48, 174, 144, 5, 59, 71, 163, 92, 244,
            74, 99, 108, 37, 69, 231, 230, 59, 212, 15, 49, 39, 156, 157, 127, 9, 195, 171, 221,
            12, 154, 166, 12, 248, 197, 137, 51, 98, 132, 138, 159, 176, 245, 166, 211, 128, 43, 3,
        ];
        let result = syscall_bls12381_add_impl(A, B).unwrap();
        assert_eq!(result, C);
    }

    #[test]
    fn test_bn254_add() {
        // generator.
        // 1
        // 2
        const A: [u8; 64] = [
            1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0,
        ];

        // 2 * generator.
        // 1368015179489954701390400359078579693043519447331113978918064868415326638035
        // 9918110051302171585080402603319702774565515993150576347155970296011118125764
        const B: [u8; 64] = [
            211, 207, 135, 109, 193, 8, 194, 211, 168, 28, 135, 22, 169, 22, 120, 217, 133, 21, 24,
            104, 91, 4, 133, 155, 2, 26, 19, 46, 231, 68, 6, 3, 196, 162, 24, 90, 122, 191, 62,
            255, 199, 143, 83, 227, 73, 164, 166, 104, 10, 156, 174, 178, 150, 95, 132, 231, 146,
            124, 10, 14, 140, 115, 237, 21,
        ];

        // 3 * generator.
        // 3353031288059533942658390886683067124040920775575537747144343083137631628272
        // 19321533766552368860946552437480515441416830039777911637913418824951667761761
        const C: [u8; 64] = [
            240, 171, 21, 25, 150, 85, 211, 242, 121, 230, 184, 21, 71, 216, 21, 147, 21, 189, 182,
            177, 188, 50, 2, 244, 63, 234, 107, 197, 154, 191, 105, 7, 97, 34, 254, 217, 61, 255,
            241, 205, 87, 91, 156, 11, 180, 99, 158, 49, 117, 100, 8, 141, 124, 219, 79, 85, 41,
            148, 72, 224, 190, 153, 183, 42,
        ];
        let result = syscall_bn254_add_impl(A, B).unwrap();
        assert_eq!(result, C);
    }

    #[test]
    fn test_secp256k1_add() {
        const A: [u8; 64] = [
            152, 23, 248, 22, 91, 129, 242, 89, 217, 40, 206, 45, 219, 252, 155, 2, 7, 11, 135,
            206, 149, 98, 160, 85, 172, 187, 220, 249, 126, 102, 190, 121, 184, 212, 16, 251, 143,
            208, 71, 156, 25, 84, 133, 166, 72, 180, 23, 253, 168, 8, 17, 14, 252, 251, 164, 93,
            101, 196, 163, 38, 119, 218, 58, 72,
        ];
        // 2 * generator.
        // 89565891926547004231252920425935692360644145829622209833684329913297188986597
        // 12158399299693830322967808612713398636155367887041628176798871954788371653930
        const B: [u8; 64] = [
            229, 158, 112, 92, 185, 9, 172, 171, 167, 60, 239, 140, 75, 142, 119, 92, 216, 124,
            192, 149, 110, 64, 69, 48, 109, 125, 237, 65, 148, 127, 4, 198, 42, 229, 207, 80, 169,
            49, 100, 35, 225, 208, 102, 50, 101, 50, 246, 247, 238, 234, 108, 70, 25, 132, 197,
            163, 57, 195, 61, 166, 254, 104, 225, 26,
        ];
        // 3 * generator.
        // 112711660439710606056748659173929673102114977341539408544630613555209775888121
        // 25583027980570883691656905877401976406448868254816295069919888960541586679410
        const C: [u8; 64] = [
            249, 54, 224, 188, 19, 241, 1, 134, 176, 153, 111, 131, 69, 200, 49, 181, 41, 82, 157,
            248, 133, 79, 52, 73, 16, 195, 88, 146, 1, 138, 48, 249, 114, 230, 184, 132, 117, 253,
            185, 108, 27, 35, 194, 52, 153, 169, 0, 101, 86, 243, 55, 42, 230, 55, 227, 15, 20,
            232, 45, 99, 15, 123, 143, 56,
        ];
        let result = syscall_secp256k1_add_impl(A, B).unwrap();
        assert_eq!(result, C);
    }

    #[test]
    fn test_secp256r1_add() {
        // generator.
        // 48439561293906451759052585252797914202762949526041747995844080717082404635286
        // 36134250956749795798585127919587881956611106672985015071877198253568414405109
        const A: [u8; 64] = [
            150, 194, 152, 216, 69, 57, 161, 244, 160, 51, 235, 45, 129, 125, 3, 119, 242, 64, 164,
            99, 229, 230, 188, 248, 71, 66, 44, 225, 242, 209, 23, 107, 245, 81, 191, 55, 104, 64,
            182, 203, 206, 94, 49, 107, 87, 51, 206, 43, 22, 158, 15, 124, 74, 235, 231, 142, 155,
            127, 26, 254, 226, 66, 227, 79,
        ];

        // 2 * generator.
        // 56515219790691171413109057904011688695424810155802929973526481321309856242040
        // 3377031843712258259223711451491452598088675519751548567112458094635497583569
        const B: [u8; 64] = [
            120, 153, 102, 71, 252, 72, 11, 166, 53, 27, 242, 119, 226, 105, 137, 192, 195, 26,
            181, 4, 3, 56, 82, 138, 126, 79, 3, 141, 24, 123, 242, 124, 209, 115, 120, 34, 157,
            183, 4, 158, 41, 130, 233, 60, 230, 173, 125, 186, 219, 48, 116, 159, 198, 154, 61, 41,
            64, 208, 142, 219, 16, 85, 119, 7,
        ];

        // 3 * generator.
        // 42877656971275811310262564894490210024759287182177196162425349131675946712428
        // 61154801112014214504178281461992570017247172004704277041681093927569603776562
        const C: [u8; 64] = [
            108, 253, 231, 198, 27, 102, 65, 251, 133, 169, 173, 239, 33, 183, 198, 230, 101, 241,
            75, 29, 149, 239, 247, 200, 68, 10, 51, 166, 209, 228, 203, 94, 50, 80, 125, 162, 39,
            177, 121, 154, 61, 184, 79, 56, 54, 176, 42, 216, 236, 162, 100, 26, 206, 6, 75, 55,
            126, 255, 152, 73, 12, 100, 52, 135,
        ];
        // Tests A + B == C, sum of points of infinity, A + A == 2 * A, and A + (-A) == infinity.
        let result = syscall_secp256r1_add_impl(A, B).unwrap();
        assert_eq!(result, C);
    }

    #[test]
    fn test_add_with_self_dont_panic() {
        const A: [u8; 64] = [
            150, 194, 152, 216, 69, 57, 161, 244, 160, 51, 235, 45, 129, 125, 3, 119, 242, 64, 164,
            99, 229, 230, 188, 248, 71, 66, 44, 225, 242, 209, 23, 107, 245, 81, 191, 55, 104, 64,
            182, 203, 206, 94, 49, 107, 87, 51, 206, 43, 22, 158, 15, 124, 74, 235, 231, 142, 155,
            127, 26, 254, 226, 66, 227, 79,
        ];
        let exit_code = syscall_secp256r1_add_impl(A, A).unwrap_err();
        assert_eq!(exit_code, ExitCode::MalformedBuiltinParams);
    }

    #[test]
    fn test_secp256k1_unreduced_coordinates_trigger_panic() {
        // Build two distinct affine points whose x/y coordinates are deliberately larger than the
        // base-field modulus (all bytes set to 0xff). This mirrors the “unreduced input” scenario.
        let p = [0xffu8; 64];
        let mut q = [0xffu8; 64];
        // Ensure q != p so we do not trip the already-known “points are equal” panic path.
        q[32] = 0xfe;

        let modulus = Secp256k1BaseField::modulus();
        let px = BigUint::from_bytes_le(&p[..32]);
        let py = BigUint::from_bytes_le(&p[32..]);
        let qx = BigUint::from_bytes_le(&q[..32]);
        let qy = BigUint::from_bytes_le(&q[32..]);

        assert!(
            px >= modulus && py >= modulus && qx >= modulus && qy >= modulus,
            "constructed points must have unreduced coordinates"
        );

        let exit_code = syscall_secp256k1_add_impl(p, q).unwrap_err();
        assert_eq!(exit_code, ExitCode::MalformedBuiltinParams);
    }
}
