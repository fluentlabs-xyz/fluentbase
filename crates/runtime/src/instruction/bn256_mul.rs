use crate::{utils::syscall_process_exit_code, RuntimeContext};
use ark_bn254::{Fq, Fr, G1Affine, G1Projective};
use ark_ec::{AffineRepr, CurveGroup};
use ark_ff::{PrimeField, Zero};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

/// FQ_LEN specifies the number of bytes needed to represent an
/// Fq element. This is an element in the base field of BN254.
const FQ_LEN: usize = 32;

/// G1_LEN specifies the number of bytes needed to represent a G1 element.
const G1_LEN: usize = 2 * FQ_LEN;

/// SCALAR_LEN specifies the number of bytes needed to represent a scalar.
const SCALAR_LEN: usize = 32;

/// Reads a single `Fq` field element from the input slice.
#[inline]
fn read_fq(input_be: &[u8]) -> Result<Fq, ()> {
    assert_eq!(input_be.len(), FQ_LEN, "input must be {FQ_LEN} bytes");

    let mut input_le = [0u8; FQ_LEN];
    input_le.copy_from_slice(input_be);

    // Reverse in-place to convert from big-endian to little-endian.
    input_le.reverse();

    Fq::deserialize_uncompressed(&input_le[..]).map_err(|_| ())
}

/// Creates a new `G1` point from the given `x` and `y` coordinates.
#[inline]
fn new_g1_point(px: Fq, py: Fq) -> Result<G1Affine, ()> {
    if px.is_zero() && py.is_zero() {
        Ok(G1Affine::zero())
    } else {
        // We cannot use `G1Affine::new` because that triggers an assert if the point is not on the curve.
        let point = G1Affine::new_unchecked(px, py);
        if !point.is_on_curve() || !point.is_in_correct_subgroup_assuming_on_curve() {
            return Err(());
        }
        Ok(point)
    }
}

/// Reads a G1 point from the input slice.
#[inline]
fn read_bn256_point(input: &[u8]) -> Result<G1Affine, ()> {
    let px = read_fq(&input[0..FQ_LEN])?;
    let py = read_fq(&input[FQ_LEN..2 * FQ_LEN])?;
    new_g1_point(px, py)
}

/// Reads a scalar from the input slice.
#[inline]
fn read_scalar(input_be: &[u8]) -> Fr {
    // Interpret as big-endian bytes and reduce modulo r
    let mut be = [0u8; SCALAR_LEN];
    be.copy_from_slice(input_be);
    Fr::from_be_bytes_mod_order(&be)
}

/// Performs point multiplication on a G1 point.
#[inline]
fn bn256_point_mul(p: G1Affine, scalar: Fr) -> G1Affine {
    let p_jacobian: G1Projective = p.into();
    let result = p_jacobian * scalar;
    result.into_affine()
}

/// Encodes a G1 point into a byte array.
#[inline]
fn encode_bn256_point(point: G1Affine) -> [u8; G1_LEN] {
    let mut output = [0u8; G1_LEN];
    let Some((x, y)) = point.xy() else {
        return output;
    };

    let mut x_bytes = [0u8; FQ_LEN];
    x.serialize_uncompressed(&mut x_bytes[..])
        .expect("Failed to serialize x coordinate");

    let mut y_bytes = [0u8; FQ_LEN];
    y.serialize_uncompressed(&mut y_bytes[..])
        .expect("Failed to serialize y coordinate");

    // Convert to big endian by reversing the bytes.
    x_bytes.reverse();
    y_bytes.reverse();

    // Place x in the first half, y in the second half.
    output[0..FQ_LEN].copy_from_slice(&x_bytes);
    output[FQ_LEN..(FQ_LEN * 2)].copy_from_slice(&y_bytes);

    output
}

pub struct SyscallBn256Mul;

impl SyscallBn256Mul {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let q_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; 64];
        caller.memory_read(p_ptr, &mut p)?;

        let mut q = [0u8; 32];
        caller.memory_read(q_ptr, &mut q)?;

        let res = Self::fn_impl(&mut p, &q).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(_) = res {
            caller.memory_write(p_ptr, &p)?;
        }

        Ok(())
    }

    pub fn fn_impl(p: &mut [u8; 64], q: &[u8; 32]) -> Result<[u8; 64], ExitCode> {
        // Direct implementation matching revm precompile exactly
        let p1 = read_bn256_point(p).map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let scalar = read_scalar(q);
        let result = bn256_point_mul(p1, scalar);

        let output = encode_bn256_point(result);
        p.copy_from_slice(&output);
        Ok(output)
    }
}
