use serde::{Deserialize, Serialize};

mod keccak_permute;
mod uint256_mul;
mod halt;
mod write;
mod sha256_extend;
mod sha256_compress;
mod ed_decompress;
mod ed_add;
mod weierstrass_add;
mod weierstrass_double;
mod weierstrass_decompress;
mod fp_op;
mod fp2_addsub;
mod fp2_mul;

fn cast_u8_to_u32(slice: &[u8]) -> Option<&[u32]> {

    if slice.len() % 4 != 0 {
        return None;
    }

    Some(unsafe {
        std::slice::from_raw_parts(
            slice.as_ptr() as *const u32,
            slice.len() / 4
        )
    })
}

#[derive(PartialEq, Copy, Clone, Debug, Serialize, Deserialize)]
pub enum FieldOperation {
    /// Addition.
    Add,
    /// Multiplication.
    Mul,
    /// Subtraction.
    Sub,
    /// Division.
    Div,
}
