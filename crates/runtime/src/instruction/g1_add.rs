use crate::{utils::syscall_process_exit_code, RuntimeContext};
use ark_bn254::{Fq, G1Affine, G1Projective};
use ark_ec::{AffineRepr, CurveGroup};
use ark_ff::Zero;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use fluentbase_types::ExitCode;
use rwasm::{Store, TrapCode, TypedCaller, Value};

pub struct SyscallG1Add;

/// FQ_LEN specifies the number of bytes needed to represent an
/// Fq element. This is an element in the base field of BN254.
///
/// Note: The base field is used to define G1 and G2 elements.
const FQ_LEN: usize = 32;

/// G1_LEN specifies the number of bytes needed to represent a G1 element.
///
/// Note: A G1 element contains 2 Fq elements.
const G1_LEN: usize = 2 * FQ_LEN;

/// Reads a single `Fq` field element from the input slice.
///
/// Takes a byte slice and attempts to interpret the first 32 bytes as an
/// elliptic curve field element. Returns an error if the bytes do not form
/// a valid field element.
///
/// # Panics
///
/// Panics if the input is not at least 32 bytes long.
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
///
/// Constructs a point on the G1 curve from its affine coordinates.
///
/// Note: The point at infinity which is represented as (0,0) is
/// handled specifically because `AffineG1` is not capable of
/// representing such a point.
/// In particular, when we convert from `AffineG1` to `G1`, the point
/// will be (0,0,1) instead of (0,1,0)
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
///
/// Parses a G1 point from a byte slice by reading two consecutive field elements
/// representing the x and y coordinates.
///
/// # Panics
///
/// Panics if the input is not at least 64 bytes long.
#[inline]
pub(super) fn read_g1_point(input: &[u8]) -> Result<G1Affine, ()> {
    let px = read_fq(&input[0..FQ_LEN])?;
    let py = read_fq(&input[FQ_LEN..2 * FQ_LEN])?;
    new_g1_point(px, py)
}

/// Encodes a G1 point into a byte array.
///
/// Converts a G1 point in Jacobian coordinates to affine coordinates and
/// serializes the x and y coordinates as big-endian byte arrays.
///
/// Note: If the point is the point at infinity, this function returns
/// all zeroes.
#[inline]
pub(super) fn encode_g1_point(point: G1Affine) -> [u8; G1_LEN] {
    let mut output = [0u8; G1_LEN];
    let Some((x, y)) = point.xy() else {
        return output;
    };

    let mut x_bytes = [0u8; FQ_LEN];
    x.serialize_uncompressed(&mut x_bytes[..])
        .expect("Failed to serialize x coordinate");

    let mut y_bytes = [0u8; FQ_LEN];
    y.serialize_uncompressed(&mut y_bytes[..])
        .expect("Failed to serialize x coordinate");

    // Convert to big endian by reversing the bytes.
    x_bytes.reverse();
    y_bytes.reverse();

    // Place x in the first half, y in the second half.
    output[0..FQ_LEN].copy_from_slice(&x_bytes);
    output[FQ_LEN..(FQ_LEN * 2)].copy_from_slice(&y_bytes);

    output
}

/// Performs point addition on two G1 points.
#[inline]
pub(super) fn g1_point_add(p1: G1Affine, p2: G1Affine) -> G1Affine {
    let p1_jacobian: G1Projective = p1.into();

    let p3 = p1_jacobian + p2;

    p3.into_affine()
}

impl SyscallG1Add {
    pub fn fn_handler(
        caller: &mut TypedCaller<RuntimeContext>,
        params: &[Value],
        _result: &mut [Value],
    ) -> Result<(), TrapCode> {
        let p_ptr = params[0].i32().unwrap() as usize;
        let q_ptr = params[1].i32().unwrap() as usize;

        let mut p = [0u8; 64];
        caller.memory_read(p_ptr, &mut p)?;

        let mut q = [0u8; 64];
        caller.memory_read(q_ptr, &mut q)?;

        let res = Self::fn_impl(&mut p, &q).map_err(|e| syscall_process_exit_code(caller, e));
        if let Ok(_) = res {
            caller.memory_write(p_ptr, &p)?;
        }

        Ok(())
    }

    pub fn fn_impl(p: &mut [u8; 64], q: &[u8; 64]) -> Result<[u8; 64], ExitCode> {
        // Direct implementation matching revm precompile exactly
        let p1 = read_g1_point(p).map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let p2 = read_g1_point(q).map_err(|_| ExitCode::MalformedBuiltinParams)?;
        let result = g1_point_add(p1, p2);

        let output = encode_g1_point(result);
        p.copy_from_slice(&output);
        Ok(output)
    }
}
